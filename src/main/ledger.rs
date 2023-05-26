use std::io::Write;
use orchard::circuit::ProvingKey;
use anyhow::Result;
use bech32::{ToBase32, Variant};
use byteorder::WriteBytesExt;
use orchard::keys::FullViewingKey;
use secp256k1::PublicKey;
use warp_api_ffi::{connect_lightwalletd, ledger::build_ledger_tx, RawTransaction, TransactionPlan};
use zcash_primitives::consensus::Network::MainNetwork;
use zcash_proofs::prover::LocalTxProver;
use tokio::task::spawn_blocking;
use tonic::Request;
use zcash_primitives::zip32::DiversifiableFullViewingKey;
use warp_api_ffi::ledger::ledger_get_fvks;

#[tokio::main]
async fn main() -> Result<()> {
    let (pubkey, dfvk, ofvk) = ledger_get_fvks()?;
    let mut uvk = vec![];
    uvk.write_u8(0x00)?;
    uvk.write_all(&dfvk.to_bytes())?;
    uvk.write_u8(0x01)?;
    uvk.write_all(&pubkey.serialize())?;
    if let Some(ofvk) = ofvk.as_ref() {
        uvk.write_u8(0x02)?;
        uvk.write_all(&ofvk.to_bytes())?;
    }

    let uvk = f4jumble::f4jumble(&uvk)?;
    let uvk = bech32::encode("yfvk", &uvk.to_base32(), Variant::Bech32m)?;
    println!("Your YWallet VK is {}", uvk);

    let filename = std::env::args().nth(1);
    if let Some(filename) = filename {
        let prover = LocalTxProver::with_default_location().unwrap();
        let proving_key = ProvingKey::build();
        let data = std::fs::read_to_string(&filename)?;
        let tx_plan: TransactionPlan = serde_json::from_str(&data)?;

        let raw_tx = build_ledger_tx(&MainNetwork, &tx_plan, &pubkey, &dfvk, ofvk.clone(), &prover, &proving_key)?;
        let mut client = connect_lightwalletd("https://lwdv3.zecwallet.co").await?;

        let response = client
            .send_transaction(Request::new(RawTransaction {
                data: raw_tx,
                height: 0,
            }))
            .await?
            .into_inner();
        println!("{}", response.error_message);
    }

    Ok(())
}

