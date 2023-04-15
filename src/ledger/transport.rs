use anyhow::{anyhow, Result};
use jubjub::Fr;
use jubjub::SubgroupPoint;
use group::GroupEncoding;
use ledger_apdu::APDUCommand;
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use reqwest::Client;
use serde_json::Value;
use byteorder::WriteBytesExt;
use byteorder::LE;
use zcash_primitives::sapling::ProofGenerationKey;
use zcash_primitives::zip32::DiversifiableFullViewingKey;
use std::io::Write;
use hex_literal::hex;

async fn apdu_usb(data: &[u8]) -> Vec<u8> {
    let api = HidApi::new().unwrap();
    let transport = TransportNativeHID::new(&api).unwrap();
    let command = APDUCommand {
        cla: data[0],
        ins: data[1],
        p1: data[2],
        p2: data[3],
        data: &data[5..],
    };
    println!("ins {}", data[1]);
    let response = transport.exchange(&command).unwrap();
    println!("ret {}", response.retcode());
    response.data().to_vec()
}

const TEST_SERVER_IP: &str = "127.0.0.1";

async fn apdu(data: &[u8]) -> Vec<u8> {
    let client = Client::new();
    let response = client.post(&format!("http://{}:5000/apdu", TEST_SERVER_IP))
    .body(format!("{{\"data\": \"{}\"}}", hex::encode(data)))
    .send()
    .await
    .unwrap();
    let response_body: Value = response.json().await.unwrap();
    let data = response_body["data"].as_str().unwrap();
    let data = hex::decode(data).unwrap();
    data[..data.len()-2].to_vec()
}

pub async fn ledger_init() -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.clear();
    bb.write_all(&hex!("E005000000"))?;
    apdu(&bb).await;

    Ok(())
}

pub async fn ledger_get_dfvk() -> Result<DiversifiableFullViewingKey> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E006000000"))?;
    let dfvk_vec = apdu(&bb).await;
    let mut dfvk = [0; 128];
    dfvk.copy_from_slice(&dfvk_vec);

    let dfvk = DiversifiableFullViewingKey::from_bytes(&dfvk).ok_or(anyhow!("Invalid diversifiable fvk"))?;
    Ok(dfvk)
}

pub async fn ledger_get_address() -> Result<String> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E007000000"))?;
    let address = apdu(&bb).await;
    let address = String::from_utf8_lossy(&address);
    Ok(address.to_string())
}

pub async fn ledger_get_proofgen_key() -> Result<ProofGenerationKey> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E011000000"))?;
    let proofgen_key = apdu(&bb).await;
    let proofgen_key = ProofGenerationKey {
        ak: SubgroupPoint::from_bytes(proofgen_key[0..32].try_into().unwrap()).unwrap(),
        nsk: Fr::from_bytes(proofgen_key[32..64].try_into().unwrap()).unwrap(),
    };
    Ok(proofgen_key)
}

pub async fn ledger_init_tx(header_digest: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E008000020"))?;
    bb.write_all(header_digest)?;
    let main_seed = apdu(&bb).await;
    Ok(main_seed)
}

pub async fn ledger_add_t_input(amount: u64) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E009000008"))?;
    bb.write_u64::<LE>(amount)?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_add_t_output(amount: u64, address: &[u8]) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00A000020"))?;
    bb.write_u64::<LE>(amount)?;
    bb.write_all(address)?;
    bb.write_all(&hex!("000000"))?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_add_s_output(amount: u64, epk: &[u8], address: &[u8], enc_compact: &[u8]) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00B00008C"))?;
    bb.write_u64::<LE>(amount)?;
    bb.write_all(epk)?;
    bb.write_all(address)?;
    bb.write_all(&hex!("0000000000"))?;
    bb.write_all(enc_compact)?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_set_transparent_merkle_proof(prevouts_digest: &[u8], pubscripts_digest: &[u8], sequences_digest: &[u8]) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00D000060"))?;
    bb.write_all(prevouts_digest)?;
    bb.write_all(pubscripts_digest)?;
    bb.write_all(sequences_digest)?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_set_sapling_merkle_proof(spends_digest: &[u8], memos_digest: &[u8], outputs_nc_digest: &[u8]) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00E000060"))?;
    bb.write_all(spends_digest)?;
    bb.write_all(memos_digest)?;
    bb.write_all(outputs_nc_digest)?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_set_net_sapling(net: i64) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00C000008"))?;
    bb.write_i64::<LE>(net)?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_set_stage(stage: u8) -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E00F"))?;
    bb.write_u8(stage)?;
    bb.write_all(&hex!("0000"))?;
    apdu(&bb).await;
    Ok(())
}

pub async fn ledger_get_sighash() -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E010000000"))?;
    let sighash = apdu(&bb).await;
    Ok(sighash)
}

pub async fn ledger_sign_sapling() -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E012000000"))?;
    let signature = apdu(&bb).await;
    Ok(signature)
}

pub async fn ledger_cmu(data: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E0800000"))?;
    bb.write_u8(data.len() as u8)?;
    bb.write_all(data)?;
    let cmu = apdu(&bb).await;
    Ok(cmu)
}

pub async fn ledger_jubjub_hash(data: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E0810000"))?;
    bb.write_u8(data.len() as u8)?;
    bb.write_all(data)?;
    let cmu = apdu(&bb).await;
    Ok(cmu)
}

pub async fn ledger_pedersen_hash(data: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E0820000"))?;
    bb.write_u8(data.len() as u8)?;
    bb.write_all(data)?;
    let cmu = apdu(&bb).await;
    Ok(cmu)
}

pub async fn ledger_get_taddr() -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E013000000"))?;
    let pkh = apdu(&bb).await;
    Ok(pkh)
}
