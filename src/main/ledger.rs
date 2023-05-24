use blake2b_simd::Params;
use byteorder::{WriteBytesExt, LE};
use group::{Group, GroupEncoding};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path, env,
};

use halo2_proofs::pasta::pallas::{self};
use orchard::{
    circuit::{Circuit, ProvingKey},
    keys::{Scope, SpendValidatingKey, SpendingKey, SpendAuthorizingKey, FullViewingKey},
    note::{ExtractedNoteCommitment, Nullifier, RandomSeed},
    primitives::redpallas::{Signature, SpendAuth},
    tree::MerklePath,
    value::ValueCommitTrapdoor, Note,
};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use ripemd::Digest;

use anyhow::{anyhow, Result};
use warp_api_ffi::{connect_lightwalletd, ledger::{
    build_ledger_tx, ledger_init
}, RawTransaction, TransactionPlan};

use zcash_primitives::{
    consensus::{BlockHeight, BranchId, Network::MainNetwork},
    transaction::{
        sighash::SignableInput, sighash_v5, txid::TxIdDigester, TransactionData, TxVersion,
    },
};
use zcash_proofs::prover::LocalTxProver;

use group::ff::Field;
use nonempty::NonEmpty;

use hex_literal::hex;
use tokio::task::spawn_blocking;
use tonic::Request;
use zcash_primitives::consensus::Network::YCashMainNetwork;
use warp_api_ffi::ledger::{ledger_get_dfvk};

#[tokio::main]
async fn main() -> Result<()> {
    let filename = std::env::args().nth(1).ok_or(anyhow!("Missing filename"))?;
    let data = std::fs::read_to_string(filename)?;
    let tx_plan: TransactionPlan = serde_json::from_str(&data)?;

    let prover = LocalTxProver::with_default_location().unwrap();
    let proving_key = ProvingKey::build();
    let raw_tx = build_ledger_tx(&YCashMainNetwork, &tx_plan, &prover, &proving_key)?;
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

