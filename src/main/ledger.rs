use std::{
    fs::File,
    io::Read,
    path::Path,
};
use ripemd::Digest;
use secp256k1::SecretKey;
use warp_api_ffi::{TransactionPlan, connect_lightwalletd, build_broadcast_tx, derive_from_secretkey};
use anyhow::Result;
use zcash_client_backend::{encoding::encode_transparent_address, address::RecipientAddress};
use zcash_primitives::{legacy::TransparentAddress, consensus::{Parameters, Network, Network::MainNetwork}};
use zcash_proofs::prover::LocalTxProver;

#[tokio::main]
async fn main() -> Result<()> {
    let network: &Network = &MainNetwork;

    let params_dir = Path::new(&std::env::var("HOME").unwrap()).join(".zcash-params");
    let prover = LocalTxProver::new(
        &params_dir.join("sapling-spend.params"),
        &params_dir.join("sapling-output.params"),
    );
    let mut file = File::open("/tmp/tx.json").unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let tx_plan: TransactionPlan = serde_json::from_str(&data).unwrap();

    let mut client = connect_lightwalletd("https://lwdv3.zecwallet.co").await?;

    build_broadcast_tx(&mut client, &tx_plan, &prover).await?;

    Ok(())
}
