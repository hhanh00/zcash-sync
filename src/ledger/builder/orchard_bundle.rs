use std::{fs::File, io::Read};

use blake2b_simd::Params;
use byteorder::{WriteBytesExt, LE};
use group::{Group, GroupEncoding};
use orchard::{
    builder::{
        InProgress, SigningMetadata, SigningParts, SpendInfo, Unauthorized as OrchardUnauthorized,
        Unproven,
    },
    bundle::{Authorization, Authorized, Flags},
    circuit::{Circuit, Instance, ProvingKey},
    keys::{
        Diversifier, FullViewingKey, Scope, SpendAuthorizingKey, SpendValidatingKey, SpendingKey,
    },
    note::{ExtractedNoteCommitment, Nullifier, RandomSeed, TransmittedNoteCiphertext},
    note_encryption::OrchardNoteEncryption,
    primitives::redpallas::{Signature, SpendAuth},
    tree::MerklePath,
    value::{NoteValue, ValueCommitTrapdoor, ValueCommitment, ValueSum},
    Action, Address, Anchor, Bundle, Note, Proof,
};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use ripemd::Digest;

use crate::{
    connect_lightwalletd, decode_orchard_merkle_path, ledger::*, RawTransaction, TransactionPlan,
};
use anyhow::Result;
use tonic::Request;

use group::ff::Field;
use hex_literal::hex;
use nonempty::NonEmpty;
use zcash_primitives::{
    consensus::{BlockHeight, BranchId},
    transaction::{
        components::Amount, sighash::SignableInput, sighash_v5, txid::TxIdDigester,
        Authorized as TxAuthorized, Transaction, TransactionData, TxVersion, Unauthorized,
    },
};

use crate::{Destination, Source};

#[derive(Debug)]
pub struct NoAuth;

impl Authorization for NoAuth {
    type SpendAuth = ();
}

pub async fn build_orchard() -> Result<()> {
    dotenv::dotenv()?;
    let mut prng = ChaCha20Rng::from_seed([0; 32]);
    let mut rseed_rng = ChaCha20Rng::from_seed([1; 32]);
    let mut alpha_rng = ChaCha20Rng::from_seed([2; 32]);
    let _sig_rng = ChaCha20Rng::from_seed([3; 32]);

    let spending_key = hex::decode(dotenv::var("SPENDING_KEY").unwrap()).unwrap();
    let spk = SpendingKey::from_bytes(spending_key.try_into().unwrap()).unwrap();
    let ask = SpendAuthorizingKey::from(&spk);
    println!("ASK {:?}", ask);

    let mut file = File::open("/tmp/tx.json").unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let tx_plan: TransactionPlan = serde_json::from_str(&data).unwrap();

    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdHeadersHash")
        .to_state();
    h.update(&hex!("050000800a27a726b4d0d6c200000000"));
    h.write_u32::<LE>(tx_plan.expiry_height)?;
    let header_digest = h.finalize();

    let orchard_fvk: [u8; 96] = hex::decode(tx_plan.orchard_fvk)
        .unwrap()
        .try_into()
        .unwrap();
    let orchard_fvk = FullViewingKey::from_bytes(&orchard_fvk).unwrap();

    let anchor = Anchor::from_bytes(tx_plan.orchard_anchor).unwrap();

    let spends: Vec<_> = tx_plan
        .spends
        .iter()
        .filter_map(|s| match s.source {
            Source::Orchard {
                id_note: _,
                diversifier,
                rseed,
                rho,
                ref witness,
            } => {
                let diversifier = Diversifier::from_bytes(diversifier);
                let address = orchard_fvk.address(diversifier, Scope::External);
                let rho = Nullifier::from_bytes(&rho).unwrap();
                let rseed = RandomSeed::from_bytes(rseed, &rho).unwrap();
                let note =
                    Note::from_parts(address, NoteValue::from_raw(s.amount), rho, rseed).unwrap();
                let merkle_path = decode_orchard_merkle_path(0, &witness).unwrap();
                Some(OrchardSpend {
                    fvk: orchard_fvk.clone(),
                    note,
                    merkle_path,
                })
            }
            _ => None,
        })
        .collect();

    let outputs: Vec<_> = tx_plan
        .outputs
        .iter()
        .filter_map(|o| match o.destination {
            Destination::Orchard(address) => {
                let address = Address::from_raw_address_bytes(&address).unwrap();
                let output = OrchardOutput {
                    recipient: address,
                    amount: NoteValue::from_raw(o.amount),
                    memo: o.memo.as_array().clone(),
                };
                Some(output)
            }
            _ => None,
        })
        .collect();

    let _zero_bsk = ValueCommitTrapdoor::zero().into_bsk();

    let mut orchard_memos_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdOrcActMHash")
        .to_state();
    let mut orchard_nc_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdOrcActNHash")
        .to_state();

    let num_actions = spends.len().max(outputs.len());
    let mut actions = vec![];
    let mut circuits = vec![];
    let mut instances = vec![];
    let mut net_value = ValueSum::default();
    let mut sum_rcv = ValueCommitTrapdoor::zero();

    let mut padded_outputs = vec![];
    for i in 0..num_actions {
        // pad with dummy spends/outputs
        let spend = if i < spends.len() {
            spends[i].clone()
        } else {
            OrchardSpend::dummy(&mut prng)
        };

        let output = if i < outputs.len() {
            outputs[i].clone()
        } else {
            OrchardOutput::dummy(&mut prng)
        };
        padded_outputs.push(output.clone());

        let rcv = ValueCommitTrapdoor::random(&mut prng);
        sum_rcv = sum_rcv + &rcv;
        let alpha = pasta_curves::Fq::random(&mut alpha_rng);
        let ak: SpendValidatingKey = orchard_fvk.clone().into();
        let rk = ak.randomize(&alpha);

        let rho = spend.note.nullifier(&orchard_fvk);
        let mut rseed = [0u8; 32];
        rseed_rng.fill_bytes(&mut rseed);
        let rseed = RandomSeed::from_bytes(rseed, &rho).unwrap();

        let v_net: ValueSum = spend.note.value() - output.amount;
        net_value = (net_value + v_net).unwrap();
        let cv_net = ValueCommitment::derive(v_net, rcv.clone());

        let spend_info = SpendInfo::new(
            spend.fvk.clone(),
            spend.note.clone(),
            spend.merkle_path.clone(),
        )
        .unwrap();
        let output_note = Note::from_parts(
            output.recipient.clone(),
            output.amount.clone(),
            rho.clone(),
            rseed,
        )
        .unwrap();
        let cmx: ExtractedNoteCommitment = output_note.commitment().into();

        let encryptor = OrchardNoteEncryption::new(
            Some(orchard_fvk.to_ovk(Scope::External)),
            output_note.clone(),
            output.recipient.clone(),
            output.memo.clone(),
        );

        let epk = encryptor.epk().to_bytes().0;
        let enc = encryptor.encrypt_note_plaintext();
        let out = encryptor.encrypt_outgoing_plaintext(&cv_net.clone(), &cmx, &mut prng);
        let encrypted_note = TransmittedNoteCiphertext {
            epk_bytes: epk.clone(),
            enc_ciphertext: enc.clone(),
            out_ciphertext: out.clone(),
        };

        let rk_bytes: [u8; 32] = rk.clone().0.into();
        orchard_memos_hasher.update(&enc[52..564]);
        orchard_nc_hasher.update(&cv_net.to_bytes());
        orchard_nc_hasher.update(&rk_bytes);
        orchard_nc_hasher.update(&enc[564..]);
        orchard_nc_hasher.update(&out);

        println!(
            "d/pkd {}",
            hex::encode(&output.recipient.to_raw_address_bytes())
        );
        println!("rho {}", hex::encode(&rho.to_bytes()));
        println!(
            "amount {}",
            hex::encode(&output.amount.inner().to_le_bytes())
        );
        println!("rseed {}", hex::encode(&rseed.as_bytes()));
        println!("cmx {}", hex::encode(&cmx.to_bytes()));

        let action: Action<SigningMetadata> = Action::from_parts(
            rho.clone(),
            rk.clone(),
            cmx.clone(),
            encrypted_note,
            cv_net.clone(),
            SigningMetadata {
                dummy_ask: None,
                parts: SigningParts { ak, alpha },
            },
        );
        actions.push(action);

        let circuit =
            Circuit::from_action_context(spend_info, output_note, alpha, rcv.clone()).unwrap();
        circuits.push(circuit);
        let instance = Instance::from_parts(anchor, cv_net, rho.clone(), rk, cmx, true, true);
        instances.push(instance);
    }
    let actions = NonEmpty::from_slice(&actions).unwrap();

    let pk = ProvingKey::build();
    let proof = Proof::create(&pk, &circuits, &instances, &mut prng).unwrap();
    let nv = i64::try_from(net_value).unwrap();
    let amount = Amount::from_i64(nv).unwrap();

    let flags = Flags::from_parts(true, true);
    let bsk = sum_rcv.into_bsk();
    let bundle: Bundle<_, Amount> = Bundle::from_parts(
        actions,
        flags,
        amount,
        anchor,
        InProgress::<Unproven, OrchardUnauthorized> {
            proof: Unproven { circuits: vec![] },
            sigs: OrchardUnauthorized { bsk: bsk.clone() },
        },
    );

    let tx_data: TransactionData<Unauthorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle: None,
        sprout_bundle: None,
        sapling_bundle: None,
        orchard_bundle: Some(bundle.clone()),
    };

    let txid_parts = tx_data.digest(TxIdDigester);
    let sig_hash = sighash_v5::v5_signature_hash(&tx_data, &SignableInput::Shielded, &txid_parts);
    let sig_hash = sig_hash.as_bytes();
    let binding_signature = bsk.sign(&mut prng, &sig_hash);

    ledger_init().await.unwrap();
    ledger_init_tx(header_digest.as_bytes()).await.unwrap();
    ledger_set_orchard_merkle_proof(
        &anchor.to_bytes(),
        orchard_memos_hasher.finalize().as_bytes(),
        orchard_nc_hasher.finalize().as_bytes(),
    )
    .await
    .unwrap();

    // no t-in
    ledger_set_stage(2).await.unwrap();
    // no t-out
    ledger_set_stage(3).await.unwrap();
    // no s-out
    ledger_set_stage(4).await.unwrap();

    for (a, o) in bundle.actions().iter().zip(padded_outputs.iter()) {
        let nf = a.nullifier().to_bytes();
        let epk = a.encrypted_note().epk_bytes;
        let _address = ledger_add_o_action(
            &nf,
            o.amount.inner(),
            &epk,
            &o.recipient.to_raw_address_bytes(),
            &a.encrypted_note().enc_ciphertext[0..52],
        )
        .await
        .unwrap();
    }
    ledger_set_stage(5).await.unwrap();
    ledger_set_net_orchard(-tx_plan.net_chg[1]).await.unwrap();
    ledger_confirm_fee().await.unwrap();

    let mut auth_actions = vec![];
    for a in bundle.actions() {
        println!("ask {:?}", ask);
        println!("alpha {:?}", a.authorization().parts.alpha);
        let rsk = ask.randomize(&a.authorization().parts.alpha);
        println!("rsk {:?}", rsk);
        // let signature: Signature<SpendAuth> = [0; 64].into();
        // let signature = rsk.sign(&mut sig_rng, sig_hash);
        let sig_bytes: [u8; 64] = ledger_sign_orchard().await.unwrap().try_into().unwrap();
        let signature: Signature<SpendAuth> = sig_bytes.into();
        let auth_action = Action::from_parts(
            a.nullifier().clone(),
            a.rk().clone(),
            a.cmx().clone(),
            a.encrypted_note().clone(),
            a.cv_net().clone(),
            signature,
        );
        auth_actions.push(auth_action);
    }
    let auth_actions = NonEmpty::from_slice(&auth_actions).unwrap();

    let bundle: Bundle<_, Amount> = Bundle::from_parts(
        auth_actions,
        Flags::from_parts(true, true),
        amount,
        anchor.clone(),
        Authorized::from_parts(proof, binding_signature),
    );

    let tx_data: TransactionData<TxAuthorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle: None,
        sprout_bundle: None,
        sapling_bundle: None,
        orchard_bundle: Some(bundle),
    };
    let tx = Transaction::from_data(tx_data).unwrap();

    let mut tx_bytes = vec![];
    tx.write(&mut tx_bytes).unwrap();

    let _orchard_memos_hash = orchard_memos_hasher.finalize();
    let _orchard_nc_hash = orchard_nc_hasher.finalize();

    let mut client = connect_lightwalletd("https://lwdv3.zecwallet.co").await?;
    let response = client
        .send_transaction(Request::new(RawTransaction {
            data: tx_bytes,
            height: 0,
        }))
        .await?
        .into_inner();

    println!("LWD send transaction {:?}", response);

    Ok(())
}

#[derive(Clone, Debug)]
struct OrchardSpend {
    fvk: FullViewingKey,
    note: Note,
    merkle_path: MerklePath,
}

impl OrchardSpend {
    pub fn dummy<R: RngCore>(rng: &mut R) -> Self {
        let (_, fvk, dummy_note) = Note::dummy(rng, None);
        let dummy_path = MerklePath::dummy(rng);
        OrchardSpend {
            fvk,
            note: dummy_note,
            merkle_path: dummy_path,
        }
    }
}

#[derive(Clone, Debug)]
struct OrchardOutput {
    recipient: Address,
    amount: NoteValue,
    memo: [u8; 512],
}

impl OrchardOutput {
    pub fn dummy<R: RngCore>(rng: &mut R) -> Self {
        let (_, _, dummy_note) = Note::dummy(rng, None);
        let _address = dummy_note.recipient();
        let mut memo = [0u8; 512];
        memo[0] = 0xF6;

        OrchardOutput {
            recipient: dummy_note.recipient(),
            amount: dummy_note.value(),
            memo,
        }
    }
}
