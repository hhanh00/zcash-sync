use crate::ledger::builder::sapling_bundle::SaplingBuilder;
use crate::ledger::builder::transparent_bundle::TransparentBuilder;
use crate::ledger::transport::*;
use crate::{Destination, Source, TransactionPlan};
use anyhow::Result;
use blake2b_simd::Params;
use blake2b_simd::State;
use ff::Field;
use jubjub::Fr;
use orchard::circuit::ProvingKey;
use rand::rngs::OsRng;
use rand::RngCore;
use ripemd::{Digest, Ripemd160};
use secp256k1::PublicKey;
use sha2::Sha256;
use zcash_client_backend::encoding::{
    encode_extended_full_viewing_key, encode_transparent_address,
};
use zcash_primitives::consensus::Network;
use zcash_primitives::consensus::Network::YCashMainNetwork;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_primitives::{
    consensus::{BlockHeight, BranchId, MainNetwork},
    transaction::{Authorized, TransactionData, TxVersion},
};
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
    _proving_key: &ProvingKey,
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
    let proofgen_key: zcash_primitives::sapling::ProofGenerationKey = ledger_get_proofgen_key()?;

    let mut sapling_builder =
        SaplingBuilder::new(network, prover, dfvk, proofgen_key, tx_plan.anchor_height);

    let mut rseed_rng = OsRng;
    let mut alpha_rng = OsRng;

    for sp in tx_plan.spends.iter() {
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
                // println!("ALPHA {}", hex::encode(&alpha.to_bytes()));

                sapling_builder.add_spend(diversifier, rseed, witness, sp.amount, &mut rng)?;
            }
            Source::Orchard { .. } => {
                anyhow::bail!("Orchard is unsupported");
            }
        }
    }

    for output in tx_plan.outputs.iter() {
        if let Destination::Transparent(raw_address) = output.destination {
            transparent_builder.add_output(raw_address, output.amount)?;
        }
    }

    for output in tx_plan.outputs.iter() {
        match output.destination {
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
            }
            Destination::Orchard(_raw_address) => {
                anyhow::bail!("Orchard is unsupported");
            }
            _ => {}
        }
    }

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
