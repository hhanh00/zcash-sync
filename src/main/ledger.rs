use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf}, vec,
};
use std::io::Write;

use blake2b_simd::Params;
use bls12_381::Scalar;
use byteorder::WriteBytesExt;
use byteorder::LE;
use ff::{PrimeField, Field};
use group::GroupEncoding;
use hex_literal::hex;
use jubjub::{Fr, SubgroupPoint, Fq};
use ledger_apdu::APDUCommand;
use ledger_transport_hid::{TransportNativeHID, hidapi::HidApi};
use orchard::keys::FullViewingKey;
use rand::{rngs::OsRng, RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use reqwest::Client;
use ripemd::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use serde_json::Value;
use sha2::Sha256;
use tonic::{Request, transport::Channel};
use warp_api_ffi::{Destination, Source, TransactionPlan, connect_lightwalletd, RawTransaction, CompactTxStreamerClient, build_broadcast_tx};
use zcash_client_backend::encoding::{decode_extended_spending_key, encode_extended_full_viewing_key, encode_payment_address};
use zcash_note_encryption::EphemeralKeyBytes;
use zcash_params::tx;

use anyhow::{anyhow, Result};
use zcash_primitives::{
    consensus::{BlockHeight, BranchId, MainNetwork, Parameters},
    merkle_tree::IncrementalWitness,
    sapling::{
        note_encryption::sapling_note_encryption, value::{NoteValue, ValueCommitment, ValueSum}, Diversifier, Node, Note,
        PaymentAddress, Rseed, ProofGenerationKey, Nullifier, prover::TxProver, redjubjub::Signature,
    },
    transaction::{
        components::{sapling::{OutputDescriptionV5, Bundle, Authorized as SapAuthorized}, Amount, OutputDescription, SpendDescription},
        TransactionData, TxVersion, Authorized,
    },
    zip32::{DiversifiableFullViewingKey, ExtendedFullViewingKey, ExtendedSpendingKey}, constants::PROOF_GENERATION_KEY_GENERATOR,
};
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::transaction::components::GROTH_PROOF_SIZE;
use zcash_proofs::{prover::LocalTxProver, sapling::SaplingProvingContext};

#[tokio::main]
async fn main() -> Result<()> {
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
