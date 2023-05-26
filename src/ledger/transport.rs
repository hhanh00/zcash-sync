use anyhow::{anyhow, Result};

use group::GroupEncoding;
use hex_literal::hex;
use jubjub::Fr;
use jubjub::SubgroupPoint;
use ledger_apdu::APDUCommand;
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use serde_json::Value;
use std::io::Write;
use zcash_primitives::sapling::ProofGenerationKey;
use zcash_primitives::zip32::DiversifiableFullViewingKey;

fn handle_error_code(code: u16) -> Result<()> {
    match code {
        0x9000 => Ok(()),
        0x6D02 => Err(anyhow!("Zcash Application NOT OPEN")),
        0x6985 => Err(anyhow!("Tx REJECTED by User")),
        0x5515 => Err(anyhow!("Ledger is LOCKED")),
        _ => Err(anyhow!("Ledger device returned error code {:#06x}", code)),
    }
}

#[cfg(not(feature="speculos"))]
fn apdu(data: &[u8]) -> Result<Vec<u8>> {
    let api = HidApi::new()?;
    let transport = TransportNativeHID::new(&api)?;
    let command = APDUCommand {
        cla: data[0],
        ins: data[1],
        p1: data[2],
        p2: data[3],
        data: &data[5..],
    };
    // println!("ins {} {}", data[1], hex::encode(data));
    let response = transport.exchange(&command)?;
    let error_code = response.retcode();
    log::info!("error_code {}", error_code);
    handle_error_code(error_code)?;
    let rep = response.data().to_vec();
    // println!("rep {}", hex::encode(&rep));
    Ok(rep)
}

const TEST_SERVER_IP: Option<&'static str> = option_env!("LEDGER_IP");

#[cfg(feature="speculos")]
#[allow(dead_code)]
fn apdu(data: &[u8]) -> Result<Vec<u8>> {
    let response = ureq::post(&format!("http://{}:5000/apdu", TEST_SERVER_IP.unwrap()))
        .send_string(&format!("{{\"data\": \"{}\"}}", hex::encode(data)))?
        .into_string()?;
    println!("ins {} {}", data[1], hex::encode(data));
    let response_body: Value = serde_json::from_str(&response)?;
    let data = response_body["data"]
        .as_str()
        .ok_or(anyhow!("No data field"))?;
    let data = hex::decode(data)?;
    println!("rep {}", hex::encode(&data));
    let error_code = u16::from_be_bytes(data[data.len() - 2..].try_into().unwrap());
    handle_error_code(error_code)?;
    Ok(data[..data.len() - 2].to_vec())
}

pub fn ledger_init() -> Result<()> {
    let mut bb: Vec<u8> = vec![];
    bb.clear();
    bb.write_all(&hex!("E005000000"))?;
    apdu(&bb)?;

    Ok(())
}

pub fn ledger_get_pubkey() -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E006000000"))?;
    let pk = apdu(&bb)?;
    Ok(pk)
}

pub fn ledger_get_dfvk() -> Result<DiversifiableFullViewingKey> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E007000000"))?;
    let dfvk_vec = apdu(&bb)?;
    let mut dfvk = [0; 128];
    dfvk.copy_from_slice(&dfvk_vec);

    let dfvk = DiversifiableFullViewingKey::from_bytes(&dfvk)
        .ok_or(anyhow!("Invalid diversifiable fvk"))?;
    Ok(dfvk)
}

pub fn ledger_get_proofgen_key() -> Result<ProofGenerationKey> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E009000000"))?;
    let proofgen_key = apdu(&bb)?;
    let proofgen_key = ProofGenerationKey {
        ak: SubgroupPoint::from_bytes(proofgen_key[0..32].try_into().unwrap()).unwrap(),
        nsk: Fr::from_bytes(proofgen_key[32..64].try_into().unwrap()).unwrap(),
    };
    Ok(proofgen_key)
}

pub fn ledger_sign_transparent(sighash: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E021000020"))?;
    bb.write_all(sighash)?;
    let signature = apdu(&bb)?;
    Ok(signature)
}

pub fn ledger_sign_sapling(sighash: &[u8], alpha: &[u8]) -> Result<Vec<u8>> {
    let mut bb: Vec<u8> = vec![];
    bb.write_all(&hex!("E022000040"))?;
    bb.write_all(sighash)?;
    bb.write_all(alpha)?;
    let signature = apdu(&bb)?;
    Ok(signature)
}
