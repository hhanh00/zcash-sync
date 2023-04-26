use blake2b_simd::Params;
use blake2b_simd::State;
use byteorder::WriteBytesExt;
use byteorder::LE;
use ff::{Field, PrimeField};
use group::GroupEncoding;
use hex_literal::hex;
use jubjub::{Fq, Fr};

use crate::ledger::builder::sapling_bundle::SaplingBuilder;
use crate::ledger::builder::transparent_bundle::TransparentBuilder;
use crate::ledger::transport::*;

use crate::{CompactTxStreamerClient, Destination, RawTransaction, Source, TransactionPlan};
use anyhow::{anyhow, Result};
use rand::{rngs::OsRng, RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use ripemd::{Digest, Ripemd160};
use secp256k1::PublicKey;
use sha2::Sha256;
use tonic::{transport::Channel, Request};
use zcash_client_backend::encoding::{
    encode_extended_full_viewing_key, encode_transparent_address,
};
use zcash_primitives::consensus::Network;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::TransparentAddress;

use zcash_primitives::zip32::ExtendedFullViewingKey;

use zcash_primitives::{
    consensus::{BlockHeight, BranchId, MainNetwork},
    constants::PROOF_GENERATION_KEY_GENERATOR,
    merkle_tree::IncrementalWitness,
    sapling::{
        note_encryption::sapling_note_encryption,
        prover::TxProver,
        redjubjub::Signature,
        value::{NoteValue, ValueCommitment, ValueSum},
        Diversifier, Node, Note, Nullifier, PaymentAddress, Rseed,
    },
    transaction::{
        components::{
            sapling::{Authorized as SapAuthorized, Bundle},
            Amount, OutputDescription, SpendDescription, GROTH_PROOF_SIZE,
        },
        Authorized, TransactionData, TxVersion,
    },
};
use zcash_proofs::{prover::LocalTxProver, sapling::SaplingProvingContext};

mod orchard_bundle;
mod sapling_bundle;
mod transparent_bundle;

#[allow(dead_code)]
pub async fn show_public_keys() -> Result<()> {
    let network = MainNetwork;

    ledger_init().await?;
    let pub_key = ledger_get_pubkey().await?;
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
    let dfvk = ledger_get_dfvk().await?;
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

pub async fn build_broadcast_tx(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    tx_plan: &TransactionPlan,
    prover: &LocalTxProver,
) -> Result<String> {
    ledger_init().await?;
    let pubkey = ledger_get_pubkey().await?;
    let mut transparent_builder = TransparentBuilder::new(network, &pubkey);

    if transparent_builder.taddr != tx_plan.taddr {
        anyhow::bail!("This ledger wallet has a different address");
    }

    // Compute header digest
    let mut h = create_hasher(b"ZTxIdHeadersHash");
    h.update(&hex!("050000800a27a726b4d0d6c200000000"));

    h.write_u32::<LE>(tx_plan.expiry_height)?;
    let header_digest = h.finalize();

    let master_seed = ledger_init_tx(header_digest.as_bytes()).await?;

    // For testing only
    // let esk = "secret-extended-key-main1qwy5cttzqqqqpq8ksfmzqgz90r73yevcw6mvwuv5zuddak9zgl9epp6x308pczzez3hse753heepdk886yf7dmse5qvyl5jsuk5w4ejhtm30cpa862kq0pfu0z4zxxvyd523zeta3rr6lj0vg30mshf6wrlfucg47jv3ldspe0sv464uewwlglr0dzakssj8tdx27vq3dnerfa5z5fgf8vjutlcey3lwn4m6ncg8y4n2cgl64rd768uqg0yfvshljqt3g4x83kngv4guq06xx";
    // let extsk = decode_extended_spending_key(MainNetwork.hrp_sapling_extended_spending_key(), &esk)
    //     .unwrap();
    // let ovk = extsk.expsk.ovk;
    // let proofgen_key = extsk.expsk.proof_generation_key();
    // let dfvk = extsk.to_diversifiable_full_viewing_key();

    let dfvk: zcash_primitives::zip32::DiversifiableFullViewingKey = ledger_get_dfvk().await?;
    let proofgen_key: zcash_primitives::sapling::ProofGenerationKey = ledger_get_proofgen_key().await?;

    let mut sapling_builder = SaplingBuilder::new(prover, dfvk, proofgen_key);

    let o_fvk: [u8; 96] = ledger_get_o_fvk().await?.try_into().unwrap();
    let _o_fvk =
        orchard::keys::FullViewingKey::from_bytes(&o_fvk).ok_or(anyhow!("Invalid Orchard FVK"))?;

    // Derive rseed PRNG
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZRSeedPRNG__Hash")
        .to_state();
    h.update(&master_seed);
    let main_rseed = h.finalize();
    let mut rseed_rng = ChaChaRng::from_seed(main_rseed.as_bytes().try_into().unwrap());

    // Derive alpha PRNG
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZAlphaPRNG__Hash")
        .to_state();
    h.update(&master_seed);
    let alpha = h.finalize();
    let mut alpha_rng = ChaChaRng::from_seed(alpha.as_bytes().try_into().unwrap());

    for sp in tx_plan.spends.iter() {
        match sp.source {
            Source::Transparent { txid, index } => {
                transparent_builder
                    .add_input(txid, index, sp.amount)
                    .await?;
            }
            Source::Sapling {
                diversifier,
                rseed,
                ref witness,
                ..
            } => {
                let alpha = Fr::random(&mut alpha_rng);
                println!("ALPHA {}", hex::encode(&alpha.to_bytes()));
        
                sapling_builder.add_spend(alpha, diversifier, rseed, witness, sp.amount).await?;
            }
            Source::Orchard { .. } => {}
        }
    }
    ledger_set_stage(2).await?;

    for output in tx_plan.outputs.iter() {
        if let Destination::Transparent(raw_address) = output.destination {
            transparent_builder
                .add_output(raw_address, output.amount)
                .await?;
        }
    }
    ledger_set_stage(3).await?;

    for output in tx_plan.outputs.iter() {
        if let Destination::Sapling(raw_address) = output.destination {
            let mut rseed = [0u8; 32];
            rseed_rng.fill_bytes(&mut rseed);
            sapling_builder.add_output(rseed, raw_address, &output.memo, output.amount).await?;
        }
    }
    ledger_set_stage(4).await?;

    transparent_builder.set_merkle_proof().await?;

    ledger_set_stage(5).await?;

    transparent_builder.sign().await?;

    let transparent_bundle = transparent_builder.build();
    let sapling_bundle = sapling_builder.build(tx_plan.net_chg[0]).await?;

    let authed_tx: TransactionData<Authorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle,
        sprout_bundle: None,
        sapling_bundle,
        orchard_bundle: None,
    };

    let tx = authed_tx.freeze().unwrap();
    let mut raw_tx = vec![];
    tx.write_v5(&mut raw_tx)?;

    ledger_end_tx().await?;

    let response = client
        .send_transaction(Request::new(RawTransaction {
            data: raw_tx,
            height: 0,
        }))
        .await?
        .into_inner();
    log::info!("{}", response.error_message);

    Ok(response.error_message)
}
