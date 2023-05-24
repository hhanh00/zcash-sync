use std::io::Write;
use crate::ledger::builder::sapling_bundle::SaplingBuilder;
use crate::ledger::builder::transparent_bundle::TransparentBuilder;
use crate::ledger::transport::*;
use crate::{Destination, Source, TransactionPlan};
use anyhow::{anyhow, Result};
use bech32::{ToBase32, Variant};
use blake2b_simd::Params;
use blake2b_simd::State;
use byteorder::WriteBytesExt;
use ff::Field;
use jubjub::Fr;
use orchard::circuit::ProvingKey;
use rand::rngs::OsRng;
use rand::RngCore;
use ripemd::{Digest, Ripemd160};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use secp256k1::PublicKey;
use sha2::Sha256;
use zcash_client_backend::encoding::{AddressCodec, encode_extended_full_viewing_key, encode_payment_address, encode_transparent_address};
use zcash_client_backend::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::Network;
use zcash_primitives::consensus::Network::YCashMainNetwork;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_primitives::{
    consensus::{BlockHeight, BranchId, MainNetwork},
    transaction::{Authorized, TransactionData, TxVersion},
};
use zcash_primitives::legacy::keys::AccountPubKey;
use zcash_primitives::sapling::PaymentAddress;
use zcash_primitives::transaction::components::Amount;
use zcash_proofs::prover::LocalTxProver;

mod sapling_bundle;
mod transparent_bundle;

#[allow(dead_code)]
pub fn show_public_keys() -> Result<()> {
    let network = YCashMainNetwork;

    ledger_init()?;
    let pub_key = ledger_get_pubkey()?;
    let pub_key = PublicKey::from_slice(&pub_key)?;
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let address = TransparentAddress::PublicKey(pub_key.into());
    let address = encode_transparent_address(
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        &address,
    );
    println!("address {}", address);
    let dfvk = ledger_get_dfvk()?;
    let efvk = ExtendedFullViewingKey::from_diversifiable_full_viewing_key(&dfvk);
    let efvk = encode_extended_full_viewing_key(
        MainNetwork.hrp_sapling_extended_full_viewing_key(),
        &efvk,
    );
    println!("efvk {}", efvk);
    Ok(())
}

pub fn create_hasher(perso: &[u8]) -> State {
    let h = Params::new().hash_length(32).personal(perso).to_state();
    h
}

pub fn build_ledger_tx(
    network: &Network,
    tx_plan: &TransactionPlan,
    prover: &LocalTxProver,
) -> Result<Vec<u8>> {
    ledger_init()?;
    let pubkey = ledger_get_pubkey()?;
    let mut transparent_builder = TransparentBuilder::new(network, &pubkey);

    let mut rng = OsRng;
    if transparent_builder.taddr_str != tx_plan.taddr {
        anyhow::bail!(
            "This ledger wallet has a different address {} != {}",
            transparent_builder.taddr_str,
            tx_plan.taddr
        );
    }

    let dfvk: zcash_primitives::zip32::DiversifiableFullViewingKey = ledger_get_dfvk()?;

    let mut uvk = vec![];
    uvk.write_u8(0x00)?;
    uvk.write_all(&dfvk.to_bytes())?;
    uvk.write_u8(0x01)?;
    uvk.write_all(&pubkey)?;

    let uvk = f4jumble::f4jumble(&uvk)?;
    let uvk = bech32::encode("yfvk", &uvk.to_base32(), Variant::Bech32m)?;
    println!("Your YWallet VK is {}", uvk);

    let proofgen_key: zcash_primitives::sapling::ProofGenerationKey = ledger_get_proofgen_key()?;

    let mut sapling_builder =
        SaplingBuilder::new(network, prover, dfvk, proofgen_key, tx_plan.anchor_height);

    let mut rseed_rng = OsRng;
    let mut alpha_rng = OsRng;
    let mut fee = 0i64;

    println!("============================================================================================");
    for sp in tx_plan.spends.iter() {
        fee += sp.amount as i64;
        match sp.source {
            Source::Transparent { txid, index } => {
                transparent_builder.add_input(txid, index, sp.amount)?;
            }
            Source::Sapling {
                diversifier,
                rseed,
                ref witness,
                ..
            } => {
                let _alpha = Fr::random(&mut alpha_rng);
                sapling_builder.add_spend(diversifier, rseed, witness, sp.amount, &mut rng)?;
            }
            Source::Orchard { .. } => {
                anyhow::bail!("Orchard is unsupported");
            }
        }
    }

    for output in tx_plan.outputs.iter() {
        fee -= output.amount as i64;
        let amount = Decimal::from_i128_with_scale(output.amount as i128, 8);
        match output.destination {
            Destination::Transparent(raw_address) => {
                transparent_builder.add_output(raw_address, output.amount)?;
                let ta = TransparentAddress::PublicKey(raw_address[1..21].try_into().unwrap());
                println!("{} : {}", ta.encode(network), amount);
            }
            Destination::Sapling(raw_address) => {
                let mut rseed = [0u8; 32];
                rseed_rng.fill_bytes(&mut rseed);
                sapling_builder.add_output(
                    rseed,
                    raw_address,
                    &output.memo,
                    output.amount,
                    &mut rng,
                )?;
                let za = PaymentAddress::from_bytes(&raw_address).unwrap();
                println!("{} : {}", encode_payment_address(network.hrp_sapling_payment_address(), &za), amount);
            }
            Destination::Orchard(_raw_address) => {
                anyhow::bail!("Orchard is unsupported");
            }
            _ => {}
        }
    }
    let fee = Decimal::from_i128_with_scale(fee as i128, 8);
    println!("Fee: {}", fee);
    println!("============================================================================================");

    let (transparent_builder, transparent_bundle) = transparent_builder.prepare();
    let (mut sapling_builder, sapling_bundle) =
        sapling_builder.prepare(tx_plan.anchor_height, OsRng);

    let unauth_tx = TransactionData::<::zcash_primitives::transaction::Unauthorized> {
        version: TxVersion::Sapling,
        consensus_branch_id: BranchId::YCanopy,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle,
        sprout_bundle: None,
        sapling_bundle,
        orchard_bundle: None,
    };

    println!("Enter OK to continue");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if line.trim() != "OK" {
        println!("Transaction aborted");
        std::process::exit(1);
    }

    let transparent_bundle = transparent_builder.sign(&unauth_tx)?;
    let sapling_bundle = sapling_builder.sign(&unauth_tx)?;

    let tx = TransactionData::<Authorized> {
        version: unauth_tx.version,
        consensus_branch_id: unauth_tx.consensus_branch_id,
        lock_time: unauth_tx.lock_time,
        expiry_height: unauth_tx.expiry_height,
        transparent_bundle,
        sprout_bundle: None,
        sapling_bundle,
        orchard_bundle: None,
    };

    let tx = tx.freeze().unwrap();
    let mut raw_tx = vec![];
    tx.write_v4(&mut raw_tx)?;

    Ok(raw_tx)
}
