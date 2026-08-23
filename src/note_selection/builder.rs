use super::types::*;
use crate::orchard::{OrchardHasher, ORCHARD_ROOTS};
use crate::sapling::{SaplingHasher, SAPLING_ROOTS};
use crate::sync::tree::TreeCheckpoint;
use crate::sync::Witness;
use crate::{AccountData, CoinConfig, DbAdapter, PROVER};
use anyhow::anyhow;
use jubjub::Fr;
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use orchard::note::Nullifier;
use orchard::value::NoteValue;
use orchard::Address;
use rand::{CryptoRng, RngCore};
use ripemd::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use std::str::FromStr;
use zcash_keys::encoding::decode_extended_spending_key;
use zcash_protocol::consensus::{BlockHeight, Network, NetworkConstants, Parameters};
use zcash_transparent::address::{Script, TransparentAddress};
use zcash_transparent::builder::TransparentSigningSet;
use zcash_transparent::bundle::{OutPoint, TxOut};
use incrementalmerkletree::{witness::IncrementalWitness, Hashable};

use sapling::zip32::ExtendedSpendingKey;
use sapling::{Diversifier, Node, PaymentAddress, Rseed};
use zcash_primitives::transaction::builder::{BuildConfig, BundlePadding, Builder};
use zcash_primitives::transaction::fees::fixed::FeeRule;
use zcash_protocol::value::Zatoshis;

/// The fixed transaction fee, in zatoshis, charged by this wallet.
const DEFAULT_FEE: u64 = 1000;

pub struct SecretKeys {
    pub transparent: Option<SecretKey>,
    pub sapling: Option<ExtendedSpendingKey>,
    pub orchard: Option<SpendingKey>,
}

pub struct TxBuilderContext {
    pub height: u32,
    pub sapling_anchor: [u8; 32],
    pub orchard_anchor: Option<[u8; 32]>,
}

impl TxBuilderContext {
    pub fn from_height(coin: u8, height: u32) -> anyhow::Result<Self> {
        let c = CoinConfig::get(coin);
        let mut connection = c.connection();
        let db_tx = connection.transaction()?;
        let TreeCheckpoint { tree, .. } = DbAdapter::get_tree_by_name(&db_tx, height, "sapling")?;
        let hasher = SaplingHasher {};
        let sapling_anchor = tree.root(32, &SAPLING_ROOTS, &hasher);

        let orchard_anchor = if c.chain.has_unified() {
            let TreeCheckpoint { tree, .. } =
                DbAdapter::get_tree_by_name(&db_tx, height, "orchard")?;
            let hasher = OrchardHasher::new();
            Some(tree.root(32, &ORCHARD_ROOTS, &hasher))
        } else {
            None
        };
        let context = TxBuilderContext {
            height,
            sapling_anchor,
            orchard_anchor,
        };
        Ok(context)
    }
}

pub fn build_tx(
    network: &Network,
    skeys: &SecretKeys,
    plan: &TransactionPlan,
    mut rng: impl RngCore + CryptoRng,
) -> anyhow::Result<Vec<u8>> {
    let secp = Secp256k1::<All>::new();

    let sapling_fvk = skeys
        .sapling
        .as_ref()
        .map(|sk| sk.to_extended_full_viewing_key());
    let sapling_ovk = sapling_fvk.as_ref().map(|efvk| efvk.fvk.ovk.clone());

    let okeys = skeys.orchard.map(|sk| {
        let orchard_fvk = FullViewingKey::from(&sk);
        let orchard_ovk = orchard_fvk.clone().to_ovk(Scope::External);
        (orchard_fvk, orchard_ovk)
    });
    let (orchard_fvk, orchard_ovk) = match okeys {
        Some((a, b)) => (Some(a), Some(b)),
        _ => (None, None),
    };

    let account_tsk = skeys.transparent;
    let mut signing_set = TransparentSigningSet::new();

    // Collect the inputs; the Sapling anchor is computed from the first
    // spend's merkle path.
    let mut sapling_anchor = None;
    let mut sapling_spends: Vec<(sapling::keys::FullViewingKey, sapling::Note, sapling::MerklePath)> =
        vec![];
    let mut orchard_spends: Vec<(
        orchard::keys::FullViewingKey,
        orchard::Note,
        orchard::tree::MerklePath,
    )> = vec![];
    let mut transparent_inputs: Vec<(secp256k1::PublicKey, OutPoint, TxOut)> = vec![];

    for spend in plan.spends.iter() {
        match &spend.source {
            Source::Transparent { txid, index } => {
                let sk = spend
                    .key
                    .as_ref()
                    .map(|k| SecretKey::from_slice(k).unwrap());
                // Use secret key from UTXO or fail over with secret key from account
                let tsk = sk
                    .or(account_tsk)
                    .ok_or(anyhow!("No transparent secret key"))?;
                let pub_key = signing_set.add_key(tsk);
                let address = TransparentAddress::PublicKeyHash(
                    Ripemd160::digest(&Sha256::digest(&pub_key.serialize())).into(),
                );
                let utxo = OutPoint::new(*txid, *index);
                let coin = TxOut::new(
                    Zatoshis::from_u64(spend.amount).unwrap(),
                    Script::from(address.script()),
                );
                transparent_inputs.push((pub_key, utxo, coin));
            }
            Source::Sapling {
                diversifier,
                rseed,
                witness,
                ..
            } => {
                let diversifier = Diversifier(*diversifier);
                let sapling_address = sapling_fvk
                    .as_ref()
                    .ok_or(anyhow!("No sapling key"))?
                    .fvk
                    .vk
                    .to_payment_address(diversifier)
                    .unwrap();
                let rseed = Rseed::BeforeZip212(Fr::from_bytes(rseed).unwrap());
                let note =
                    sapling_address.create_note(sapling::value::NoteValue::from_raw(spend.amount), rseed);
                let witness = zcash_primitives::merkle_tree::read_incremental_witness::<
                    Node,
                    _,
                    32,
                >(witness.as_slice())?;
                let merkle_path = witness.path().unwrap();
                if sapling_anchor.is_none() {
                    sapling_anchor =
                        Some(sapling::Anchor::from(merkle_path.root(Node::empty_leaf())));
                }
                sapling_spends.push((sapling_fvk.as_ref().unwrap().fvk.clone(), note, merkle_path));
            }
            Source::Orchard {
                id_note,
                diversifier,
                rho,
                rseed,
                witness,
            } => {
                let diversifier = orchard::keys::Diversifier::from_bytes(*diversifier);
                let sender_address = orchard_fvk
                    .as_ref()
                    .ok_or(anyhow!("No Orchard key"))
                    .map(|fvk| fvk.address(diversifier, Scope::External))?;
                let value = NoteValue::from_raw(spend.amount);
                let rho = orchard::note::Rho::from_bytes(&rho).unwrap();
                let rseed = orchard::note::RandomSeed::from_bytes(*rseed, &rho).unwrap();
                let note = orchard::Note::from_parts(
                    sender_address,
                    value,
                    rho,
                    rseed,
                    orchard::note::NoteVersion::V2,
                )
                .unwrap();
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
                orchard_spends.push((orchard_fvk.as_ref().unwrap().clone(), note, merkle_path));
            }
        }
    }

    let orchard_anchor = orchard::Anchor::from_bytes(plan.orchard_anchor).unwrap();
    let build_config = BuildConfig::Standard {
        sapling_anchor,
        orchard_anchor: Some(orchard_anchor),
        ironwood_anchor: None,
        orchard_padding: BundlePadding::DEFAULT,
        ironwood_padding: BundlePadding::DEFAULT,
    };
    let mut builder = Builder::new(*network, BlockHeight::from_u32(plan.anchor_height), build_config);

    for (pub_key, utxo, coin) in transparent_inputs {
        builder
            .add_transparent_p2pkh_input(pub_key, utxo, coin)
            .map_err(|e| anyhow!(e.to_string()))?;
    }

    for (fvk, note, merkle_path) in sapling_spends {
        builder
            .add_sapling_spend::<anyhow::Error>(fvk, note, merkle_path)
            .map_err(|e| anyhow!(e.to_string()))?;
    }

    for (fvk, note, merkle_path) in orchard_spends {
        builder
            .add_orchard_spend::<anyhow::Error>(fvk, note, merkle_path)
            .map_err(|e| anyhow!(e.to_string()))?;
    }

    for output in plan.outputs.iter() {
        let value = Zatoshis::from_u64(output.amount).unwrap();
        match &output.destination {
            Destination::Transparent(_addr) => {
                let transparent_address = output.destination.transparent();
                builder
                    .add_transparent_output(&transparent_address, value)
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
            Destination::Sapling(addr) => {
                let sapling_address = PaymentAddress::from_bytes(addr).unwrap();
                builder
                    .add_sapling_output::<anyhow::Error>(
                        sapling_ovk,
                        sapling_address,
                        value,
                        output.memo.clone(),
                    )
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
            Destination::Orchard(addr) => {
                let orchard_address = Address::from_raw_address_bytes(addr).unwrap();
                builder
                    .add_orchard_output::<anyhow::Error>(
                        orchard_ovk.clone(),
                        orchard_address,
                        value,
                        output.memo.clone(),
                    )
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        }
    }

    let prover = PROVER.lock();
    let prover = prover.as_ref().unwrap();
    let sapling_extsks = vec![
        skeys.sapling.clone().ok_or(anyhow!("No Sapling Key"))?;
        plan.spends
            .iter()
            .filter(|s| matches!(s.source, Source::Sapling { .. }))
            .count()
    ];
    let orchard_saks = if let Some(sk) = skeys.orchard {
        vec![
            SpendAuthorizingKey::from(&sk);
            plan.spends
                .iter()
                .filter(|s| matches!(s.source, Source::Orchard { .. }))
                .count()
        ]
    } else {
        vec![]
    };
    let fee_rule = FeeRule::non_standard(Zatoshis::from_u64(DEFAULT_FEE).unwrap());
    let build_result = builder.build(
        &signing_set,
        &sapling_extsks,
        &orchard_saks,
        &mut rng,
        prover,
        prover,
        &fee_rule,
    )?;

    let mut tx_bytes = vec![];
    build_result.transaction().write(&mut tx_bytes).unwrap();

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
        .and_then(|ob| ob.sk.map(|sk| SpendingKey::from_bytes(sk).unwrap()));

    let sk = SecretKeys {
        transparent: transparent_sk,
        sapling: Some(sapling_sk),
        orchard: orchard_sk,
    };
    Ok(sk)
}
