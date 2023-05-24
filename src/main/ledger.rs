use orchard::circuit::ProvingKey;

use anyhow::{anyhow, Result};
use warp_api_ffi::{
    connect_lightwalletd, ledger::build_ledger_tx, RawTransaction, TransactionPlan,
};

use zcash_proofs::prover::LocalTxProver;

use tonic::Request;
use zcash_primitives::consensus::Network::YCashMainNetwork;

#[tokio::main]
async fn main() -> Result<()> {
    let filename = std::env::args().nth(1).unwrap_or("/tmp/tx.json".to_string());
    let data = std::fs::read_to_string(filename)?;
    let tx_plan: TransactionPlan = serde_json::from_str(&data)?;

    let prover = LocalTxProver::with_default_location().unwrap();
    let raw_tx = build_ledger_tx(&YCashMainNetwork, &tx_plan, &prover)?;
    let mut client = connect_lightwalletd("https://lite.ycash.xyz:9067").await?;

    let response = client
        .send_transaction(Request::new(RawTransaction {
            data: raw_tx,
            height: 0,
        }))
        .await?
        .into_inner();
    println!("{}", response.error_message);

    Ok(())
}
