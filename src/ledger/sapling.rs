#![allow(unused_imports)]

use anyhow::anyhow;
use bip39::{Mnemonic, Language, Seed};
use byteorder::{ByteOrder, LE, LittleEndian, WriteBytesExt};
use ed25519_bip32::{DerivationScheme, XPrv};
use sha2::{Sha256, Sha512};
use hmac::{Hmac, Mac};
use hmac::digest::{crypto_common, FixedOutput, MacMarker, Update};
use blake2b_simd::Params;
use jubjub::{ExtendedPoint, Fr, SubgroupPoint};
use group::GroupEncoding;
use ledger_apdu::{APDUAnswer, APDUCommand};
use rand::rngs::OsRng;
use zcash_primitives::zip32::{ExtendedSpendingKey, ChildIndex, ExtendedFullViewingKey, ChainCode, DiversifierKey};
use zcash_client_backend::encoding::{decode_extended_full_viewing_key, decode_payment_address, encode_extended_spending_key, encode_payment_address};
use zcash_primitives::consensus::Network::MainNetwork;
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::constants::{PROOF_GENERATION_KEY_GENERATOR, SPENDING_KEY_GENERATOR};
use zcash_primitives::keys::OutgoingViewingKey;
use zcash_primitives::sapling::keys::{ExpandedSpendingKey, FullViewingKey};
use zcash_primitives::sapling::{Diversifier, Node, Note, PaymentAddress, ProofGenerationKey, Rseed, ViewingKey};
use serde::{Serialize, Deserialize};
use serde::__private::de::Content::ByteBuf;
use zcash_primitives::memo::Memo;
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::note_encryption::sapling_note_encryption;
use zcash_primitives::sapling::prover::TxProver;
use zcash_primitives::sapling::redjubjub::{PublicKey, Signature};
use zcash_primitives::transaction::builder::Builder;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;
use zcash_primitives::transaction::components::OutputDescription;
use zcash_primitives::transaction::components::sapling::GrothProofBytes;
use zcash_proofs::sapling::SaplingProvingContext;
use crate::{Tx, TxIn, TxOut};

const HARDENED: u32 = 0x8000_0000;
const NETWORK: &Network = &MainNetwork;
const EXPIRY: u32 = 50;
const LEDGER_IP: &str = "192.168.0.101";

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct APDURequest {
    apduHex: String,
}

#[derive(Serialize, Deserialize)]
struct APDUReply {
    data: String,
    error: Option<String>,
}

// fn get_ivk(app: &LedgerApp) -> anyhow::Result<String> {
//     let command = ApduCommand {
//         cla: 0x85,
//         ins: 0xf0,
//         p1: 1,
//         p2: 0,
//         length: 4,
//         data: vec![0, 0, 0, 0]
//     };
//     let res = app.exchange(command)?;
//     let mut raw_ivk = [0u8; 32];
//     raw_ivk.copy_from_slice(&res.apdu_data());
//     let ivk = jubjub::Fr::from_bytes(&raw_ivk).unwrap();
//     let ivk = SaplingIvk(ivk);
//     let fvk = ExtendedFullViewingKey {
//         depth: 0,
//         parent_fvk_tag: (),
//         child_index: (),
//         chain_code: (),
//         fvk: FullViewingKey {},
//         dk: DiversifierKey()
//     };
//     println!("{}", address);
//
//     Ok(address)
// }

const CURVE_SEEDKEY: &[u8] = b"ed25519 seed";
const ZCASH_PERSO: &[u8] = b"Zcash_ExpandSeed";

type HMAC256 = Hmac<Sha256>;
type HMAC512 = Hmac<Sha512>;

fn hmac_sha2<T: Update + FixedOutput + MacMarker + crypto_common::KeyInit>(data: &mut [u8]) {
    let mut hmac = T::new_from_slice(CURVE_SEEDKEY).unwrap();
    hmac.update(&data);
    data.copy_from_slice(&hmac.finalize().into_bytes());
}

macro_rules! prf_expand {
    ($($y:expr),*) => (
    {
        let mut res = [0u8; 64];
        let mut hasher = Params::new()
            .hash_length(64)
            .personal(ZCASH_PERSO)
            .to_state();
        $(
            hasher.update($y);
        )*
        res.copy_from_slice(&hasher.finalize().as_bytes());
        res
    })
}

struct ExtSpendingKey {
    chain: [u8; 32],
    ovk: [u8; 32],
    dk: [u8; 32],
    ask: Fr,
    nsk: Fr,
}

fn derive_child(esk: &mut ExtSpendingKey, path: &[u32]) {
    let mut a = [0u8; 32];
    let mut n = [0u8; 32];
    a.copy_from_slice(&esk.ask.to_bytes());
    n.copy_from_slice(&esk.nsk.to_bytes());

    for &p in path {
        println!("==> ask: {}", hex::encode(esk.ask.to_bytes()));
        let hardened = (p & 0x8000_0000) != 0;
        let c = p & 0x7FFF_FFFF;
        assert!(hardened);
        //make index LE
        //zip32 child derivation
        let mut le_i = [0; 4];
        LittleEndian::write_u32(&mut le_i, c + (1 << 31));
        println!("==> chain: {}", hex::encode(esk.chain));
        println!("==> a: {}", hex::encode(a));
        println!("==> n: {}", hex::encode(n));
        println!("==> ovk: {}", hex::encode(esk.ovk));
        println!("==> dk: {}", hex::encode(esk.dk));
        println!("==> i: {}", hex::encode(le_i));
        let h = prf_expand!(&esk.chain, &[0x11], &a, &n, &esk.ovk, &esk.dk, &le_i);
        println!("==> tmp: {}", hex::encode(h));
        let mut key = [0u8; 32];
        key.copy_from_slice(&h[..32]);
        esk.chain.copy_from_slice(&h[32..]);
        let ask_cur = Fr::from_bytes_wide(&prf_expand!(&key, &[0x13]));
        let nsk_cur = Fr::from_bytes_wide(&prf_expand!(&key, &[0x14]));
        esk.ask += ask_cur;
        esk.nsk += nsk_cur;

        let t = prf_expand!(&key, &[0x15], &esk.ovk);
        esk.ovk.copy_from_slice(&t[..32]);
        let t = prf_expand!(&key, &[0x16], &esk.dk);
        esk.dk.copy_from_slice(&t[..32]);

        a.copy_from_slice(&scalar_to_bytes(&prf_expand!(&key, &[0x00])));
        n.copy_from_slice(&scalar_to_bytes(&prf_expand!(&key, &[0x01])));
    }
}

fn scalar_to_bytes(k: &[u8; 64]) -> [u8; 32] {
    let t = Fr::from_bytes_wide(k);
    t.to_bytes()
}

struct SpendData {
    position: u64,
    cv: ExtendedPoint,
    anchor: [u8; 32],
    nullifier: [u8; 32],
    rk: ExtendedPoint,
    zkproof: [u8; 192],
}

pub async fn build_tx_ledger(tx: &mut Tx, prover: &impl TxProver) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Vec::<u8>::new();
    let tin_count = tx.t_inputs.len();
    let s_in_count = tx.inputs.len();
    let mut s_out_count = tx.outputs.len();
    // TODO: Support t in/outputs
    assert_eq!(tin_count, 0);
    buffer.push(0u8);
    buffer.push(0u8);
    buffer.push(s_in_count as u8);
    buffer.push((s_out_count + 1) as u8); // +1 for change

    let mut change = 0;
    for sin in tx.inputs.iter() {
        buffer.write_u32::<LE>(HARDENED)?;
        let fvk = decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &sin.fvk).unwrap().unwrap();
        let (_, pa) = fvk.default_address();
        let address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
        assert_eq!(pa.to_bytes().len(), 43);
        buffer.extend_from_slice(&pa.to_bytes());
        buffer.write_u64::<LE>(sin.amount)?;
        change += sin.amount as i64;
    }

    // assert_eq!(buffer.len(), 4+55*s_in_count);

    for sout in tx.outputs.iter() {
        let pa = decode_payment_address(NETWORK.hrp_sapling_payment_address(), &sout.addr).unwrap().unwrap();
        assert_eq!(pa.to_bytes().len(), 43);
        buffer.extend_from_slice(&pa.to_bytes());
        buffer.write_u64::<LE>(sout.amount)?;
        buffer.push(0xF6); // no memo
        buffer.push(0x01); // ovk present
        buffer.extend_from_slice(&hex::decode(&sout.ovk)?);
        change -= sout.amount as i64;
    }
    assert_eq!(buffer.len(), 4 + 55 * s_in_count + 85 * (s_out_count));

    change -= i64::from(DEFAULT_FEE);
    assert!(change >= 0);

    let output_change = TxOut {
        addr: tx.change.clone(),
        amount: change as u64,
        ovk: tx.ovk.clone(),
        memo: "".to_string(),
    };
    tx.outputs.push(output_change);
    s_out_count += 1;

    let pa_change = decode_payment_address(NETWORK.hrp_sapling_payment_address(), &tx.change).unwrap().unwrap();
    buffer.extend_from_slice(&pa_change.to_bytes());
    buffer.write_u64::<LE>(change as u64)?;
    buffer.push(0xF6); // no memo
    buffer.push(0x01); // ovk present
    buffer.extend_from_slice(&hex::decode(&tx.ovk)?);

    assert_eq!(buffer.len(), 4 + 55 * s_in_count + 85 * s_out_count);
    log::debug!("txlen {}", buffer.len());

    let mut chunks: Vec<_> = buffer.chunks(250).collect();
    chunks.insert(0, &[]); // starts with empty chunk
    for (index, c) in chunks.iter().enumerate() {
        let p1 = match index {
            0 => 0,
            _ if index == chunks.len() - 1 => 2,
            _ => 1,
        };
        log::debug!("data {}", hex::encode(c));
        let command = APDUCommand {
            cla: 0x85,
            ins: 0xA0,
            p1,
            p2: 0,
            data: c.to_vec(),
        };
        let rep = send_request(&command).await;
        log::debug!("{}", rep.retcode());
    }

    let mut buffer = Vec::<u8>::new();
    let mut context = prover.new_sapling_proving_context();
    let mut spend_datas: Vec<SpendData> = vec![];
    for i in 0..s_in_count {
        let txin = &tx.inputs[i];
        let command = APDUCommand {
            cla: 0x85,
            ins: 0xA1,
            p1: 0,
            p2: 0,
            data: vec![],
        };
        let rep = send_request(&command).await;
        log::debug!("{}", rep.retcode());
        let ak = &rep.apdu_data()[0..32];
        let nsk = &rep.apdu_data()[32..64];
        let rcv = &rep.apdu_data()[64..96];
        let ar = &rep.apdu_data()[96..128];

        let ak = SubgroupPoint::from_bytes(&slice_to_hash(&ak)).unwrap();
        let nsk = Fr::from_bytes(&slice_to_hash(&nsk)).unwrap();
        let rcv = Fr::from_bytes(&slice_to_hash(&rcv)).unwrap();
        let ar = Fr::from_bytes(&slice_to_hash(&ar)).unwrap();
        let spend_data = get_spend_proof(&tx, i, ak, nsk, ar, rcv, &mut context, prover);

        let rseed = string_to_hash(&txin.rseed);
        buffer.extend_from_slice(&rseed);
        buffer.write_u64::<LE>(spend_data.position)?;
        spend_datas.push(spend_data);
    }

    for spd in spend_datas.iter() {
        buffer.extend_from_slice(&spd.cv.to_bytes());
        buffer.extend_from_slice(&spd.anchor);
        buffer.extend_from_slice(&spd.nullifier);
        buffer.extend_from_slice(&spd.rk.to_bytes());
        buffer.extend_from_slice(&spd.zkproof);
    }

    let mut output_descriptions: Vec<OutputDescription<GrothProofBytes>> = vec![];
    for i in 0..s_out_count {
        let command = APDUCommand {
            cla: 0x85,
            ins: 0xA2,
            p1: 0,
            p2: 0,
            data: vec![],
        };
        let rep = send_request(&command).await;
        log::debug!("{}", rep.retcode());
        let rcv = &rep.apdu_data()[0..32];
        let rseed = &rep.apdu_data()[32..64];
        let rcv = Fr::from_bytes(&slice_to_hash(&rcv)).unwrap();
        let rseed = slice_to_hash(&rseed);
        let output_description = get_output_description(tx, i, change as u64, rcv, rseed, &mut context, prover);
        buffer.extend_from_slice(&output_description.cv.to_bytes());
        buffer.extend_from_slice(&output_description.cmu.to_bytes());
        buffer.extend_from_slice(&output_description.ephemeral_key.0);
        buffer.extend_from_slice(&output_description.enc_ciphertext);
        buffer.extend_from_slice(&output_description.out_ciphertext);
        buffer.extend_from_slice(&output_description.zkproof);
        output_descriptions.push(output_description);
    }

    let hash_data = get_hash_data(tx.height, u64::from(DEFAULT_FEE),
                                  &spend_datas, &output_descriptions);
    buffer.extend_from_slice(&hash_data);
    let sig_hash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(&hex::decode("5a6361736853696748617368a675ffe9")?) // consensus branch id = canopy
        .hash(&hash_data)
        .as_bytes());

    let mut tx_hash = [0u8; 32];

    let mut chunks: Vec<_> = buffer.chunks(250).collect();
    chunks.insert(0, &[]); // starts with empty chunk
    for (index, c) in chunks.iter().enumerate() {
        let p1 = match index {
            0 => 0,
            _ if index == chunks.len() - 1 => 2,
            _ => 1,
        };
        log::debug!("data {}", hex::encode(c));
        let command = APDUCommand {
            cla: 0x85,
            ins: 0xA3,
            p1,
            p2: 0,
            data: c.to_vec(),
        };
        let rep = send_request(&command).await;
        log::debug!("{}", rep.retcode());
        if p1 == 2 {
            tx_hash.copy_from_slice(&rep.apdu_data()[0..32]);
        }
    }

    let mut signatures: Vec<Vec<u8>> = vec![];
    for _i in 0..s_in_count {
        let command = APDUCommand {
            cla: 0x85,
            ins: 0xA4,
            p1: 0,
            p2: 0,
            data: vec![],
        };
        let rep = send_request(&command).await;
        log::debug!("{}", rep.retcode());
        let signature = &rep.apdu_data()[0..64];
        signatures.push(rep.apdu_data()[0..64].to_vec())
    }

    log::debug!("tx hash: {}", hex::encode(tx_hash));
    log::debug!("sig hash: {}", hex::encode(sig_hash));

    let binding_signature = prover.binding_sig(&mut context, DEFAULT_FEE, &sig_hash).map_err(|_| anyhow!("Cannot create binding signature"))?;
    let mut sig_buffer: Vec<u8> = vec![];
    binding_signature.write(&mut sig_buffer).unwrap();
    log::debug!("binding_signature: {}", hex::encode(&sig_buffer));

    let tx = get_tx_data(tx.height, u64::from(DEFAULT_FEE), &spend_datas, &output_descriptions,
                         &signatures, &binding_signature);
    Ok(tx)
}

fn get_spend_proof<T: TxProver>(tx: &Tx, i: usize, ak: SubgroupPoint, nsk: Fr, ar: Fr, rcv: Fr, context: &mut T::SaplingProvingContext, prover: &T) -> SpendData
{
    let txin = &tx.inputs[i];

    let fvk = decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &txin.fvk).unwrap().unwrap();
    let mut diversifier = [0u8; 11];
    hex::decode_to_slice(&txin.diversifier, &mut diversifier).unwrap();
    let diversifier = Diversifier(diversifier);
    let pa = fvk.fvk.vk.to_payment_address(diversifier).unwrap();
    let mut rseed_bytes = [0u8; 32];
    hex::decode_to_slice(&txin.rseed, &mut rseed_bytes).unwrap();
    let rseed = Fr::from_bytes(&rseed_bytes).unwrap();
    let note = pa
        .create_note(txin.amount, Rseed::BeforeZip212(rseed))
        .unwrap();
    let w = hex::decode(&txin.witness).unwrap();
    let witness = IncrementalWitness::<Node>::read(&*w).unwrap();
    let merkle_path = witness.path().unwrap();
    let position = merkle_path.position;
    let cmu = Node::new(note.cmu().into());
    let anchor = merkle_path.root(cmu).into();

    let pgk = ProofGenerationKey {
        ak,
        nsk
    };
    let value = txin.amount;
    let vk = pgk.to_viewing_key();

    let (spend_proof, cv, rk) = prover.spend_proof_with_rcv(context, rcv,
                                                            pgk, diversifier, Rseed::BeforeZip212(rseed), ar, value, anchor, merkle_path
    ).unwrap();
    let spend_data = SpendData {
        position,
        cv,
        anchor: anchor.to_bytes(),
        nullifier: note.nf(&vk, position).0,
        rk: rk.0,
        zkproof: spend_proof,
    };
    spend_data
}

fn get_output_description<T: TxProver>(tx: &Tx, i: usize, change_amount: u64, rcv: Fr, rseed: [u8; 32], context: &mut T::SaplingProvingContext, prover: &T)
                                       -> OutputDescription<GrothProofBytes> {
    let txout = if i == tx.outputs.len() {
        TxOut {
            addr: tx.change.clone(),
            amount: change_amount,
            ovk: tx.ovk.clone(),
            memo: "".to_string(),
        }
    } else { tx.outputs[i].clone() };
    let ovk = OutgoingViewingKey(string_to_hash(&tx.ovk));
    let pa = decode_payment_address(NETWORK.hrp_sapling_payment_address(), &txout.addr).unwrap().unwrap();
    let rseed = Rseed::AfterZip212(rseed);
    let note = pa
        .create_note(txout.amount, rseed)
        .unwrap();

    let encryptor = sapling_note_encryption::<_, zcash_primitives::consensus::MainNetwork>(
        Some(ovk),
        note.clone(),
        pa.clone(),
        Memo::Empty.encode(),
        &mut OsRng,
    );

    let cmu = note.cmu();
    let epk = *encryptor.epk();

    let (zkproof, cv) = prover.output_proof_with_rcv(context, rcv, *encryptor.esk(), pa.clone(), note.rcm(), txout.amount);
    let enc_ciphertext = encryptor.encrypt_note_plaintext();
    let out_ciphertext = encryptor.encrypt_outgoing_plaintext(&cv, &cmu, &mut OsRng);

    OutputDescription {
        cv,
        cmu,
        ephemeral_key: epk.to_bytes().into(),
        enc_ciphertext,
        out_ciphertext,
        zkproof
    }
}

fn string_to_hash(s: &str) -> [u8; 32] {
    slice_to_hash(&hex::decode(s).unwrap())
}

fn slice_to_hash(s: &[u8]) -> [u8; 32] {
    let mut b = [0u8; 32];
    b.copy_from_slice(s);
    b
}

async fn send_request(command: &APDUCommand<Vec<u8>>) -> APDUAnswer<Vec<u8>> {
    let port = 9000;
    let apdu_hex = hex::encode(command.serialize());
    let client = reqwest::Client::new();
    let rep = client.post(format!("http://{}:{}", LEDGER_IP, port)).json(&APDURequest {
        apduHex: apdu_hex,
    }).header("Content-Type", "application/json").send().await.unwrap();
    let rep: APDUReply = rep.json().await.unwrap();
    let answer = APDUAnswer::from_answer(hex::decode(rep.data).unwrap());
    answer.unwrap()
}

fn get_hash_data(expiry_height: u32, sapling_value_balance: u64, spend_datas: &[SpendData], output_descriptions: &[OutputDescription<GrothProofBytes>]) -> Vec<u8> {
    let prevout_hash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(b"ZcashPrevoutHash")
        .hash(&[])
        .as_bytes());
    let out_hash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(b"ZcashOutputsHash")
        .hash(&[])
        .as_bytes());
    let sequence_hash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(b"ZcashSequencHash")
        .hash(&[])
        .as_bytes());

    let mut data: Vec<u8> = vec![];
    for sp in spend_datas.iter() {
        data.extend_from_slice(&sp.cv.to_bytes());
        data.extend_from_slice(&sp.anchor);
        data.extend_from_slice(&sp.nullifier);
        data.extend_from_slice(&sp.rk.to_bytes());
        data.extend_from_slice(&sp.zkproof);
    }
    let shieldedspendhash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(b"ZcashSSpendsHash")
        .hash(&data).as_bytes());

    let mut data: Vec<u8> = vec![];
    for output_description in output_descriptions.iter() {
        data.extend_from_slice(&output_description.cv.to_bytes());
        data.extend_from_slice(&output_description.cmu.to_bytes());
        data.extend_from_slice(&output_description.ephemeral_key.0);
        data.extend_from_slice(&output_description.enc_ciphertext);
        data.extend_from_slice(&output_description.out_ciphertext);
        data.extend_from_slice(&output_description.zkproof);
    }
    let shieldedoutputhash: [u8; 32] = slice_to_hash(Params::new()
        .hash_length(32)
        .personal(b"ZcashSOutputHash")
        .hash(&data).as_bytes());

    let mut tx_hash_data: Vec<u8> = vec![];

    tx_hash_data.write_u32::<LE>(0x80000004).unwrap();
    tx_hash_data.write_u32::<LE>(0x892F2085).unwrap();
    tx_hash_data.extend_from_slice(&prevout_hash);
    tx_hash_data.extend_from_slice(&sequence_hash);
    tx_hash_data.extend_from_slice(&out_hash);
    tx_hash_data.extend_from_slice(&[0u8; 32]);
    tx_hash_data.extend_from_slice(&shieldedspendhash);
    tx_hash_data.extend_from_slice(&shieldedoutputhash);
    tx_hash_data.write_u32::<LE>(0).unwrap();
    tx_hash_data.write_u32::<LE>(expiry_height + EXPIRY).unwrap();
    tx_hash_data.write_u64::<LE>(sapling_value_balance).unwrap();
    tx_hash_data.write_u32::<LE>(1).unwrap();

    assert_eq!(tx_hash_data.len(), 220);

    tx_hash_data
}

fn get_tx_data(expiry_height: u32, sapling_value_balance: u64, spend_datas: &[SpendData], output_descriptions: &[OutputDescription<GrothProofBytes>],
               signatures: &[Vec<u8>], binding_signature: &Signature) -> Vec<u8> {
    let mut tx_data: Vec<u8> = vec![];
    tx_data.write_u32::<LE>(0x80000004).unwrap();
    tx_data.write_u32::<LE>(0x892F2085).unwrap();
    tx_data.push(0);
    tx_data.push(0);
    tx_data.write_u32::<LE>(0).unwrap();
    tx_data.write_u32::<LE>(expiry_height + EXPIRY).unwrap();
    tx_data.write_u64::<LE>(sapling_value_balance).unwrap();
    tx_data.push(spend_datas.len() as u8); // TODO Support compactsize
    for (sp, sig) in spend_datas.iter().zip(signatures) {
        let mut sp_bytes: Vec<u8> = vec![];
        sp_bytes.extend_from_slice(&sp.cv.to_bytes());
        sp_bytes.extend_from_slice(&sp.anchor);
        sp_bytes.extend_from_slice(&sp.nullifier);
        sp_bytes.extend_from_slice(&sp.rk.to_bytes());
        sp_bytes.extend_from_slice(&sp.zkproof);
        sp_bytes.extend_from_slice(&sig);
        assert_eq!(sp_bytes.len(), 384);
        tx_data.extend_from_slice(&sp_bytes);
    }
    tx_data.push(output_descriptions.len() as u8); // TODO Support compactsize
    for output_description in output_descriptions.iter() {
        tx_data.extend_from_slice(&output_description.cv.to_bytes());
        tx_data.extend_from_slice(&output_description.cmu.to_bytes());
        tx_data.extend_from_slice(&output_description.ephemeral_key.0);
        tx_data.extend_from_slice(&output_description.enc_ciphertext);
        tx_data.extend_from_slice(&output_description.out_ciphertext);
        tx_data.extend_from_slice(&output_description.zkproof);
    }
    tx_data.push(0);
    let mut sig_buffer: Vec<u8> = vec![];
    binding_signature.write(&mut sig_buffer).unwrap();
    tx_data.extend_from_slice(&sig_buffer);
    tx_data
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Read;
    use blake2b_simd::Params;
    use group::GroupEncoding;
    use jubjub::{Fr, ExtendedPoint, SubgroupPoint};
    use ledger_apdu::*;
    use zcash_primitives::constants::SPENDING_KEY_GENERATOR;
    use zcash_primitives::sapling::redjubjub::{PublicKey, Signature};
    use zcash_proofs::prover::LocalTxProver;
    use crate::ledger::{build_tx_ledger, send_request, slice_to_hash};
    use crate::Tx;

    #[tokio::test]
    async fn get_version() {
        let command = APDUCommand {
            cla: 0x85,
            ins: 0x00,
            p1: 0,
            p2: 0,
            data: vec![],
        };
        let answer = send_request(&command).await;
        assert_eq!(answer.retcode(), 0x9000);
        println!("{}.{}", answer.apdu_data()[1], answer.apdu_data()[2]);
        assert_eq!(answer.apdu_data()[1], 3);
    }

    #[tokio::test]
    async fn get_addr() {
        let command = APDUCommand {
            cla: 0x85,
            ins: 0x11,
            p1: 0,
            p2: 0,
            data: vec![0, 0, 0, 0],
        };
        let answer = send_request(&command).await;
        let address = String::from_utf8(answer.apdu_data()[43..].to_ascii_lowercase()).unwrap();
        println!("{}", address);
        assert_eq!(address, "zs1m8d7506t4rpcgaag392xae698gx8j5at63qpg54ssprg6eqej0grmkfu76tq6p495z3w6s8qlll");
    }

    #[tokio::test]
    async fn load_tx() {
        let file = File::open("tx.json").unwrap();
        let mut tx: Tx = serde_json::from_reader(&file).unwrap();
        let prover = LocalTxProver::with_default_location().unwrap();
        build_tx_ledger(&mut tx, &prover).await.unwrap();
    }
}
