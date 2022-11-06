use crate::coinconfig::get_prover;
use super::types::*;
use super::{decode};
use crate::orchard::{get_proving_key, OrchardHasher, ORCHARD_ROOTS};
use crate::sapling::{SaplingHasher, SAPLING_ROOTS};
use crate::sync::tree::TreeCheckpoint;
use crate::sync::Witness;
use crate::{broadcast_tx, init_coin, set_active, set_coin_lwd_url, AccountData, CoinConfig};
use anyhow::anyhow;
use jubjub::Fr;
use orchard::builder::Builder as OrchardBuilder;
use orchard::bundle::Flags;
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use orchard::note::Nullifier;
use orchard::value::NoteValue;
use orchard::{Address, Anchor, Bundle};
use rand::rngs::OsRng;
use rand::{CryptoRng, RngCore};
use ripemd::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use std::str::FromStr;
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_primitives::consensus::{BlockHeight, BranchId, Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::prover::TxProver;
use zcash_primitives::sapling::{Diversifier, Node, PaymentAddress, Rseed};
use zcash_primitives::transaction::builder::Builder;
use zcash_primitives::transaction::components::{Amount, OutPoint, TxOut};
use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v5::v5_signature_hash;
use zcash_primitives::transaction::txid::TxIdDigester;
use zcash_primitives::transaction::{Transaction, TransactionData, TxVersion};
use zcash_primitives::zip32::{ExtendedFullViewingKey, ExtendedSpendingKey};
use crate::note_selection::fee::FeeFlat;
use crate::note_selection::{build_tx_plan, fetch_utxos};

pub struct SecretKeys {
    pub transparent: Option<SecretKey>,
    pub sapling: ExtendedSpendingKey,
    pub orchard: Option<SpendingKey>,
}

pub struct TxBuilderContext {
    pub height: u32,
    pub sapling_anchor: [u8; 32],
    pub orchard_anchor: [u8; 32],
}

impl TxBuilderContext {
    pub fn from_height(coin: u8, height: u32) -> anyhow::Result<Self> {
        let c = CoinConfig::get(coin);
        let db = c.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        let TreeCheckpoint { tree, .. } = db.get_tree_by_name(height, "sapling")?;
        let hasher = SaplingHasher {};
        let sapling_anchor = tree.root(32, &SAPLING_ROOTS, &hasher);
        let TreeCheckpoint { tree, .. } = db.get_tree_by_name(height, "orchard")?;
        let hasher = OrchardHasher::new();
        let orchard_anchor = tree.root(32, &ORCHARD_ROOTS, &hasher);
        let context = TxBuilderContext {
            height,
            sapling_anchor,
            orchard_anchor,
        };
        Ok(context)
    }
}

const EXPIRY_HEIGHT: u32 = 50;

pub fn build_tx(
    network: &Network,
    skeys: &SecretKeys,
    plan: &TransactionPlan,
    context: TxBuilderContext,
    mut rng: impl RngCore + CryptoRng + Clone,
) -> anyhow::Result<Vec<u8>> {
    let secp = Secp256k1::<All>::new();
    let transparent_address = skeys.transparent.map(|tkey| {
        let pub_key = PublicKey::from_secret_key(&secp, &tkey);
        let pub_key = pub_key.serialize();
        let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
        TransparentAddress::PublicKey(pub_key.into())
    });

    let sapling_fvk = ExtendedFullViewingKey::from(&skeys.sapling);
    let sapling_ovk = sapling_fvk.fvk.ovk;

    let okeys = skeys.orchard.map(|sk| {
        let orchard_fvk = FullViewingKey::from(&sk);
        let orchard_ovk = orchard_fvk.clone().to_ovk(Scope::External);
        (orchard_fvk, orchard_ovk)
    });
    let (orchard_fvk, orchard_ovk) = match okeys {
        Some((a, b)) => (Some(a), Some(b)),
        _ => (None, None),
    };

    let mut has_orchard = false;
    let mut builder = Builder::new(*network, BlockHeight::from_u32(context.height));
    let anchor: Anchor = orchard::tree::MerkleHashOrchard::from_bytes(&context.orchard_anchor)
        .unwrap()
        .into();
    let mut orchard_builder = OrchardBuilder::new(Flags::from_parts(true, true), anchor);
    for spend in plan.spends.iter() {
        match &spend.source {
            Source::Transparent { txid, index } => {
                let utxo = OutPoint::new(*txid, *index);
                let coin = TxOut {
                    value: Amount::from_u64(spend.amount).unwrap(),
                    script_pubkey: transparent_address
                        .ok_or(anyhow!("No transparent key"))
                        .map(|ta| ta.script())?,
                };
                builder.add_transparent_input(skeys.transparent.unwrap(), utxo, coin)?;
            }
            Source::Sapling {
                diversifier,
                rseed,
                witness,
                ..
            } => {
                let diversifier = Diversifier(*diversifier);
                let sapling_address = sapling_fvk.fvk.vk.to_payment_address(diversifier).unwrap();
                let rseed = Rseed::BeforeZip212(Fr::from_bytes(rseed).unwrap());
                let note = sapling_address.create_note(spend.amount, rseed).unwrap();
                let witness = IncrementalWitness::<Node>::read(witness.as_slice())?;
                let merkle_path = witness.path().unwrap();
                builder.add_sapling_spend(skeys.sapling.clone(), diversifier, note, merkle_path)?;
            }
            Source::Orchard {
                id_note,
                diversifier,
                rho,
                rseed,
                witness,
            } => {
                has_orchard = true;
                let diversifier = orchard::keys::Diversifier::from_bytes(*diversifier);
                let sender_address = orchard_fvk
                    .as_ref()
                    .ok_or(anyhow!("No Orchard key"))
                    .map(|fvk| fvk.address(diversifier, Scope::External))?;
                let value = NoteValue::from_raw(spend.amount);
                let rho = Nullifier::from_bytes(&rho).unwrap();
                let rseed = orchard::note::RandomSeed::from_bytes(*rseed, &rho).unwrap();
                let note = orchard::Note::from_parts(sender_address, value, rho, rseed).unwrap();
                let witness = Witness::from_bytes(*id_note, &witness)?;
                let auth_path: Vec<_> = witness
                    .auth_path(32, &ORCHARD_ROOTS, &OrchardHasher::new())
                    .iter()
                    .map(|n| orchard::tree::MerkleHashOrchard::from_bytes(n).unwrap())
                    .collect();
                let merkle_path = orchard::tree::MerklePath::from_parts(
                    witness.position as u32,
                    auth_path.try_into().unwrap(),
                );
                orchard_builder
                    .add_spend(orchard_fvk.clone().unwrap(), note, merkle_path)
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        }
    }

    for output in plan.outputs.iter() {
        let value = Amount::from_u64(output.amount).unwrap();
        match &output.destination {
            Destination::Transparent(addr) => {
                let transparent_address = TransparentAddress::PublicKey(*addr);
                builder.add_transparent_output(&transparent_address, value)?;
            }
            Destination::Sapling(addr) => {
                let sapling_address = PaymentAddress::from_bytes(addr).unwrap();
                builder.add_sapling_output(
                    Some(sapling_ovk),
                    sapling_address,
                    value,
                    output.memo.clone(),
                )?;
            }
            Destination::Orchard(addr) => {
                has_orchard = true;
                let orchard_address = Address::from_raw_address_bytes(addr).unwrap();
                orchard_builder
                    .add_recipient(
                        orchard_ovk.clone(),
                        orchard_address,
                        NoteValue::from_raw(output.amount),
                        Some(*output.memo.as_array()),
                    )
                    .map_err(|_| anyhow!("Orchard::add_recipient"))?;
            }
        }
    }

    let transparent_bundle = builder.transparent_builder.build();
    let mut ctx = get_prover().new_sapling_proving_context();
    let sapling_bundle = builder
        .sapling_builder
        .build(
            get_prover(),
            &mut ctx,
            &mut rng,
            BlockHeight::from_u32(context.height),
            None,
        )
        .unwrap();

    let mut orchard_bundle: Option<Bundle<_, Amount>> = None;
    if has_orchard {
        orchard_bundle = Some(orchard_builder.build(rng.clone()).unwrap());
    }

    let unauthed_tx: TransactionData<zcash_primitives::transaction::Unauthorized> =
        TransactionData::from_parts(
            TxVersion::Zip225,
            BranchId::Nu5,
            0,
            BlockHeight::from_u32(context.height + EXPIRY_HEIGHT),
            transparent_bundle,
            None,
            sapling_bundle,
            orchard_bundle,
        );

    let txid_parts = unauthed_tx.digest(TxIdDigester);
    let sig_hash = v5_signature_hash(&unauthed_tx, &SignableInput::Shielded, &txid_parts);
    let sig_hash: [u8; 32] = sig_hash.as_bytes().try_into().unwrap();

    let transparent_bundle = unauthed_tx
        .transparent_bundle()
        .map(|tb| tb.clone().apply_signatures(&unauthed_tx, &txid_parts));

    let sapling_bundle = unauthed_tx.sapling_bundle().map(|sb| {
        sb.clone()
            .apply_signatures(get_prover(), &mut ctx, &mut rng, &sig_hash)
            .unwrap()
            .0
    });

    let mut orchard_signing_keys = vec![];
    if let Some(sk) = skeys.orchard {
        orchard_signing_keys.push(SpendAuthorizingKey::from(&sk));
    }

    let orchard_bundle = unauthed_tx.orchard_bundle().map(|ob| {
        let proven = ob
            .clone()
            .create_proof(get_proving_key(), rng.clone())
            .unwrap();
        proven
            .apply_signatures(rng.clone(), sig_hash, &orchard_signing_keys)
            .unwrap()
    });

    let tx_data: TransactionData<zcash_primitives::transaction::Authorized> =
        TransactionData::from_parts(
            TxVersion::Zip225,
            BranchId::Nu5,
            0,
            BlockHeight::from_u32(context.height + EXPIRY_HEIGHT),
            transparent_bundle,
            None,
            sapling_bundle,
            orchard_bundle,
        );
    let tx = Transaction::from_data(tx_data).unwrap();

    let mut tx_bytes = vec![];
    tx.write(&mut tx_bytes).unwrap();

    Ok(tx_bytes)
}

pub fn get_secret_keys(coin: u8, account: u32) -> anyhow::Result<SecretKeys> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;

    let transparent_sk = db
        .get_tsk(account)?
        .map(|tsk| SecretKey::from_str(&tsk).unwrap());

    let AccountData { sk, .. } = db.get_account_info(account)?;
    let sapling_sk = sk.ok_or(anyhow!("No secret key"))?;
    let sapling_sk = decode_extended_spending_key(
        c.chain.network().hrp_sapling_extended_spending_key(),
        &sapling_sk,
    )
    .unwrap();

    let orchard_sk = db
        .get_orchard(account)?
        .map(|ob| SpendingKey::from_bytes(ob.sk).unwrap());

    let sk = SecretKeys {
        transparent: transparent_sk,
        sapling: sapling_sk,
        orchard: orchard_sk,
    };
    Ok(sk)
}

#[tokio::test]
async fn dummy_test() {
    let _ = env_logger::try_init();
    init_coin(0, "./zec.db").unwrap();
    set_coin_lwd_url(0, "http://127.0.0.1:9067");

    let c = CoinConfig::get(0);
    log::info!("Start test");
    let height = {
        let db = c.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        db.get_last_sync_height().unwrap().unwrap()
    };
    log::info!("Height {}", height);

    const REGTEST_CHANGE: &str = "uregtest1mxy5wq2n0xw57nuxa4lqpl358zw4vzyfgadsn5jungttmqcv6nx6cpx465dtpzjzw0vprjle4j4nqqzxtkuzm93regvgg4xce0un5ec6tedquc469zjhtdpkxz04kunqqyasv4rwvcweh3ue0ku0payn29stl2pwcrghyzscrrju9ar57rn36wgz74nmynwcyw27rjd8yk477l97ez8";
    let mut config = TransactionBuilderConfig::new(REGTEST_CHANGE);

    log::info!("Getting signing keys");
    let keys = get_secret_keys(0, 1).unwrap();

    log::info!("Building signing context");
    let context = TxBuilderContext::from_height(0, height).unwrap();

    log::info!("Getting available notes");
    let utxos = fetch_utxos(0, 1, height, true, 0).await.unwrap();
    // let notes: Vec<_> = utxos.into_iter().filter(|utxo| utxo.source.pool() == Pool::Orchard).collect();

    log::info!("Preparing outputs");
    let mut orders = vec![];
    orders.push(
        Order::new(
            1,
            "tmWXoSBwPoCjJCNZjw4P7heoVMcT2Ronrqq",
            10000,
            MemoBytes::empty(),
        )
    );
    orders.push(Order::new(2, "zregtestsapling1qzy9wafd2axnenul6t6wav76dys6s8uatsq778mpmdvmx4k9myqxsd9m73aqdgc7gwnv53wga4j", 20000, MemoBytes::empty()));
    orders.push(Order::new(3, "uregtest1mzt5lx5s5u8kczlfr82av97kjckmfjfuq8y9849h6cl9chhdekxsm6r9dklracflqwplrnfzm5rucp5txfdm04z5myrde8y3y5rayev8", 30000, MemoBytes::empty()));
    orders.push(Order::new(4, "uregtest1yvucqfqnmq5ldc6fkvuudlsjhxg56hxph9ymmcnmpzpywd752ym8sr5l5d24wqn4enz3gakk6alf5hlpw2cjs3jjrcdae3nksrefyum5x400f9gs3ak9yllcr8czhrlnjufuuy7n5mh", 40000, MemoBytes::empty()));
    orders.push(Order::new(5, "uregtest1wqgc0cm50a7a647qrdglgj62fl40q8njsrcfkt2mzlsmj979rdmsdwuysypc6ewxjxz0zc48kmm35jwx4q6c4fgqwkmmqyhwlep4n2hc0229vf6cahcnesr38y7gyzfx6pa8zg9jvv9", 50000, MemoBytes::empty()));
    orders.push(Order::new(6, "uregtest1usu9eyxgqu48sa8lqug6ccjc7vcam3mt3a5t7jvyxj7pq5dgdtkjgkqzsyh9pfeav9970xddp2c9h5x44drwnz4f0zwc894k3vt380g6kfsg9j9fmnpljye9r56d94njsv40uaam392xvmky2v38dh3yhayz44z6xv402slujuhwy3mg", 60000, MemoBytes::empty()));
    orders.push(Order::new(7, "uregtest1mxy5wq2n0xw57nuxa4lqpl358zw4vzyfgadsn5jungttmqcv6nx6cpx465dtpzjzw0vprjle4j4nqqzxtkuzm93regvgg4xce0un5ec6tedquc469zjhtdpkxz04kunqqyasv4rwvcweh3ue0ku0payn29stl2pwcrghyzscrrju9ar57rn36wgz74nmynwcyw27rjd8yk477l97ez8", 70000, MemoBytes::empty()));

    log::info!("Building tx plan");
    let tx_plan =
        build_tx_plan::<FeeFlat>("", 0, &utxos, &mut orders, &config).unwrap();
    log::info!("Plan: {}", serde_json::to_string(&tx_plan).unwrap());

    log::info!("Building tx");
    let tx = build_tx(c.chain.network(), &keys, &tx_plan, context, OsRng).unwrap();
    println!("{}", hex::encode(&tx));
}

#[tokio::test]
async fn submit_tx() {
    let _ = env_logger::try_init();
    init_coin(0, "./zec.db").unwrap();
    set_coin_lwd_url(0, "http://127.0.0.1:9067");
    set_active(0);
    let tx = "";

    let r = broadcast_tx(&hex::decode(tx).unwrap()).await.unwrap();
    log::info!("{}", r);
}
