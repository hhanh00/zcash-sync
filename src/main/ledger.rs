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

use anyhow::Result;
use warp_api_ffi::{
    connect_lightwalletd,
    ledger::{
        build_broadcast_tx, ledger_add_o_action, ledger_confirm_fee, ledger_init, ledger_init_tx,
        ledger_set_net_orchard, ledger_set_orchard_merkle_proof, ledger_set_stage, ledger_test_math, ledger_get_o_fvk, ledger_get_debug, ledger_cmu,
    },
    TransactionPlan,
};

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

#[tokio::main]
async fn main() {
    let mut rng = ChaCha20Rng::from_seed([4u8; 32]);
    let (_, _, note) = Note::dummy(&mut rng, None);
    let cmx: ExtractedNoteCommitment = note.commitment().into();
    println!("cmx {:?}", cmx);

    let address = note.recipient().to_raw_address_bytes();
    let value = note.value().inner();
    let rseed = note.rseed().as_bytes();
    let rho = note.rho().to_bytes();

    println!("{}", hex::encode(&address));
    println!("{} {}", value, hex::encode(value.to_le_bytes()));
    println!("{}", hex::encode(rseed));
    println!("{}", hex::encode(&rho));

    let mut buffer = vec![];
    buffer.write(&address).unwrap();
    buffer.write_u64::<LE>(value).unwrap();
    buffer.write(rseed).unwrap();
    buffer.write(&rho).unwrap();

    let cmu = ledger_cmu(&buffer).await.unwrap();
    println!("cmx {:?}", hex::encode(&cmu));
}

