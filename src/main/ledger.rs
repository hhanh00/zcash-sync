use std::io::Write;
use anyhow::Result;
use bech32::{ToBase32, Variant};
use byteorder::WriteBytesExt;
use warp_api_ffi::{
    connect_lightwalletd, ledger::build_ledger_tx, RawTransaction, TransactionPlan,
};
use zcash_proofs::prover::LocalTxProver;
use tonic::Request;
use zcash_primitives::consensus::Network::YCashMainNetwork;
use clap::Parser;
use warp_api_ffi::ledger::ledger_get_fvks;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional name to operate on
    filename: Option<String>,

    /// Sets a Lightwalletd URL
    #[arg(short, long)]
    server: Option<String>,

    /// Sets an output filename
    #[arg(short, long)]
    out: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let (pubkey, dfvk) = ledger_get_fvks()?;
    let mut uvk = vec![];
    uvk.write_u8(0x00)?;
    uvk.write_all(&dfvk.to_bytes())?;
    uvk.write_u8(0x01)?;
    uvk.write_all(&pubkey.serialize())?;

    let uvk = f4jumble::f4jumble(&uvk)?;
    let uvk = bech32::encode("yfvk", &uvk.to_base32(), Variant::Bech32m)?;
    println!("Your YWallet VK is {uvk}");

    if let Some(filename) = cli.filename.as_ref() {
        let data = std::fs::read_to_string(filename)?;
        let tx_plan: TransactionPlan = serde_json::from_str(&data)?;

        let prover = LocalTxProver::with_default_location().unwrap();
        let raw_tx = build_ledger_tx(&YCashMainNetwork, &tx_plan, pubkey, dfvk, &prover)?;

        if let Some(url) = cli.server.as_ref() {
            let mut client = connect_lightwalletd(&url).await?;

            let response = client
                .send_transaction(Request::new(RawTransaction {
                    data: raw_tx.clone(),
                    height: 0,
                }))
                .await?
                .into_inner();
            println!("{}", response.error_message);
        }

        if let Some(out_filename) = cli.out.as_ref() {
            let mut out = std::fs::File::create(&out_filename)?;
            let tx = base64::encode(&raw_tx);
            write!(out, "{tx}")?;
        }
    }

    Ok(())
}
