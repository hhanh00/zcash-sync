use std::io::Write;
use orchard::circuit::ProvingKey;
use anyhow::Result;
use bech32::{ToBase32, Variant};
use byteorder::WriteBytesExt;
use orchard::keys::FullViewingKey;
use secp256k1::PublicKey;
use zcash_primitives::consensus::Network::MainNetwork;
use zcash_proofs::prover::LocalTxProver;
use tokio::task::spawn_blocking;
use tonic::Request;
use zcash_primitives::zip32::DiversifiableFullViewingKey;
use crate::ledger::transport::*;

pub fn ledger_get_fvks() -> Result<(PublicKey, DiversifiableFullViewingKey)> {
    ledger_init()?;
    let pubkey = ledger_get_pubkey()?;
    let pubkey = PublicKey::from_slice(&pubkey)?;
    let dfvk: DiversifiableFullViewingKey = ledger_get_dfvk()?;
    Ok((pubkey, dfvk))
}
