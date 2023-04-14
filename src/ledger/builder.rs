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
use crate::{Destination, Source, TransactionPlan, connect_lightwalletd, RawTransaction, CompactTxStreamerClient};
use zcash_client_backend::encoding::{decode_extended_spending_key, encode_extended_full_viewing_key, encode_payment_address};
use zcash_note_encryption::EphemeralKeyBytes;
use zcash_params::tx;

use crate::ledger::transport::*;
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

struct SpendDescriptionUnAuthorized {
    cv: ValueCommitment,
    anchor: Fq,
    pub nullifier: Nullifier,
    rk: zcash_primitives::sapling::redjubjub::PublicKey,
    zkproof: [u8; GROTH_PROOF_SIZE],
}

pub async fn build_broadcast_tx(client: &mut CompactTxStreamerClient<Channel>, tx_plan: &TransactionPlan, prover: &LocalTxProver) -> Result<()> {
    ledger_init().await?;
    let address = ledger_get_address().await?;
    println!("address {}", address);

    let secp = Secp256k1::<All>::new();

    // TODO: Not used atm
    let sk: SecretKey = "ee729122985b068958c6c62afe6ee70927d3e7f9405c0b9a09e822cd2f5ce2ae"
        .parse()
        .unwrap();
    let pub_key = PublicKey::from_secret_key(&secp, &sk);
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let tpub_key: [u8; 20] = pub_key.into();

    // Compute header digest
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdHeadersHash")
        .to_state();
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

    let dfvk = ledger_get_dfvk().await?;
    let ovk = dfvk.fvk.ovk;
    let proofgen_key = ledger_get_proofgen_key().await?;

    let fvk = dfvk.fvk;
    let nf_key = proofgen_key.to_viewing_key().nk;

    println!("NSK {}", hex::encode(proofgen_key.nsk.to_bytes()));
    assert_eq!(PROOF_GENERATION_KEY_GENERATOR * proofgen_key.nsk, fvk.vk.nk.0);

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
    println!("ALPHA SEED {}", hex::encode(&alpha.as_bytes()));
    let mut alpha_rng = ChaChaRng::from_seed(alpha.as_bytes().try_into().unwrap());

    let mut prevouts_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdPrevoutHash")
        .to_state();

    let mut trscripts_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxTrScriptsHash")
        .to_state();

    let mut sequences_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSequencHash")
        .to_state();

    let mut spends_compact_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSSpendCHash")
        .to_state();

    let mut spends_non_compact_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSSpendNHash")
        .to_state();

    let mut sapling_context = SaplingProvingContext::new();
    let mut value_balance = ValueSum::zero();

    let mut shielded_spends = vec![];
    for sp in tx_plan.spends.iter() {
        match sp.source {
            Source::Transparent { txid, index } => {
                prevouts_hasher.update(&txid);
                prevouts_hasher.write_u32::<LE>(index)?;
                trscripts_hasher.update(&hex!("1976a914"));
                trscripts_hasher.update(&tpub_key);
                trscripts_hasher.update(&hex!("88ac"));
                sequences_hasher.update(&hex!("FFFFFFFF"));

                ledger_add_t_input(sp.amount).await?;
            }
            Source::Sapling { diversifier, rseed, ref witness, .. } => {
                let diversifier = Diversifier(diversifier);
                let z_address = fvk.vk.to_payment_address(diversifier).ok_or(anyhow!("Invalid diversifier"))?;
                let rseed = Rseed::BeforeZip212(Fr::from_bytes(&rseed).unwrap());
                let note = Note::from_parts(z_address, NoteValue::from_raw(sp.amount), rseed);
                let witness = IncrementalWitness::<Node>::read(&witness[..])?;
                let merkle_path = witness.path().ok_or(anyhow!("Invalid merkle path"))?;

                let node = Node::from_cmu(&note.cmu());
                let anchor = Fq::from_bytes(&merkle_path.root(node).repr).unwrap();

                let nullifier = note.nf(&nf_key, merkle_path.position);

                let alpha = Fr::random(&mut alpha_rng);
                println!("ALPHA {}", hex::encode(&alpha.to_bytes()));

                let (zkproof, cv, rk) = prover.spend_proof(&mut sapling_context, proofgen_key.clone(), diversifier, rseed, alpha, 
                    sp.amount, anchor, merkle_path.clone(), OsRng).map_err(|_| anyhow!("Error generating spend"))?;
                value_balance = (value_balance + note.value()).ok_or(anyhow!("Invalid amount"))?;

                spends_compact_hasher.update(nullifier.as_ref());
                spends_non_compact_hasher.update(&cv.to_bytes());
                spends_non_compact_hasher.update(&anchor.to_repr());
                rk.write(&mut spends_non_compact_hasher)?;

                shielded_spends.push(SpendDescriptionUnAuthorized { cv, anchor, nullifier, rk, zkproof });
            }
            Source::Orchard { .. } => { unimplemented!() },
        }
    }
    ledger_set_stage(1).await?;

    let prevouts_digest = prevouts_hasher.finalize();
    println!("PREVOUTS {}", hex::encode(prevouts_digest));
    let pubscripts_digest = trscripts_hasher.finalize();
    println!("PUBSCRIPTS {}", hex::encode(pubscripts_digest));
    let sequences_digest = sequences_hasher.finalize();
    println!("SEQUENCES {}", hex::encode(sequences_digest));

    let spends_compact_digest = spends_compact_hasher.finalize();
    println!("C SPENDS {}", hex::encode(spends_compact_digest));
    let spends_non_compact_digest = spends_non_compact_hasher.finalize();
    println!("NC SPENDS {}", hex::encode(spends_non_compact_digest));

    let mut spends_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSSpendsHash")
        .to_state();
    spends_hasher.update(spends_compact_digest.as_bytes());
    spends_hasher.update(spends_non_compact_digest.as_bytes());
    let spends_digest = spends_hasher.finalize();
    println!("SPENDS {}", hex::encode(spends_digest));

    let mut output_memos_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSOutM__Hash")
        .to_state();

    let mut output_non_compact_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdSOutN__Hash")
        .to_state();

    let mut shielded_outputs = vec![];
    for output in tx_plan.outputs.iter() {
        if let Destination::Transparent(raw_address) = output.destination {
            ledger_add_t_output(output.amount, &raw_address[1..21]).await?;
        }
    }
    ledger_set_stage(2).await?;

    for output in tx_plan.outputs.iter() {
        if let Destination::Sapling(raw_address) = output.destination {
            let recipient = PaymentAddress::from_bytes(&raw_address).unwrap();
            let mut rseed = [0u8; 32];
            rseed_rng.fill_bytes(&mut rseed);
            let rseed = Rseed::AfterZip212(rseed);
    
            let value = NoteValue::from_raw(output.amount);
            value_balance = (value_balance - value).ok_or(anyhow!("Invalid amount"))?;

            let note = Note::from_parts(recipient, value, rseed);
            let rcm = note.rcm();
            let cmu = note.cmu();
            println!("cmu {}", hex::encode(cmu.to_bytes()));

            let encryptor =
                sapling_note_encryption::<_, MainNetwork>(Some(ovk.clone()), note, recipient, output.memo.clone(), &mut OsRng);
    
            let (zkproof, cv) = prover.output_proof(
                &mut sapling_context,
                encryptor.esk().0,
                recipient,
                rcm,
                output.amount,
                &mut OsRng,
            );
    
            let enc_ciphertext = encryptor.encrypt_note_plaintext();
            let out_ciphertext = encryptor.encrypt_outgoing_plaintext(&cv, &cmu, &mut OsRng);
        
            let epk = encryptor.epk();

            ledger_add_s_output(output.amount, &epk.to_bytes().0, &raw_address, &enc_ciphertext[0..52]).await?;

            let memo = &enc_ciphertext[52..564];
            output_memos_hasher.update(memo);
        
            output_non_compact_hasher.update(&cv.as_inner().to_bytes());
            output_non_compact_hasher.update(&enc_ciphertext[564..]);
            output_non_compact_hasher.update(&out_ciphertext);

            let ephemeral_key = epk.to_bytes();
            shielded_outputs.push(OutputDescription { cv, cmu, ephemeral_key, enc_ciphertext, out_ciphertext, zkproof });
        }
    }
    ledger_set_stage(3).await?;

    let memos_digest = output_memos_hasher.finalize();
    println!("MEMOS {}", hex::encode(memos_digest));
    let outputs_nc_digest = output_non_compact_hasher.finalize();
    println!("NC OUTPUTS {}", hex::encode(outputs_nc_digest));

    ledger_set_transparent_merkle_proof(prevouts_digest.as_bytes(), 
        pubscripts_digest.as_bytes(), sequences_digest.as_bytes()).await?;
    ledger_set_sapling_merkle_proof(spends_digest.as_bytes(), memos_digest.as_bytes(), outputs_nc_digest.as_bytes()).await?;

    ledger_set_net_sapling(-tx_plan.net_chg[0]).await?;

    let mut signatures = vec![];
    for _sp in shielded_spends.iter() {
        let signature = ledger_sign_sapling().await?;
        println!("SIGNATURE {}", hex::encode(&signature));
        let signature = Signature::read(&*signature)?;
        // Signature verification
        // let rk = sp.rk();
        // let mut message: Vec<u8> = vec![];
        // message.write_all(&rk.0.to_bytes())?;
        // message.write_all(sig_hash.as_ref())?;
        // println!("MSG {}", hex::encode(&message));
        // let verified = rk.verify_with_zip216(&message, &signature, SPENDING_KEY_GENERATOR, true);
        // assert!(verified);
        signatures.push(signature);
    }

    let shielded_spends = shielded_spends.into_iter().zip(signatures.into_iter()).map(|(sp, spend_auth_sig)| 
        SpendDescription::<_> { cv: sp.cv, anchor: sp.anchor, nullifier: sp.nullifier, rk: sp.rk, zkproof: sp.zkproof, 
            spend_auth_sig }).collect();

    let value: i64 = value_balance.try_into().unwrap();
    let value = Amount::from_i64(value).unwrap();
    let sighash = ledger_get_sighash().await?;
    let binding_sig = sapling_context.binding_sig(value, &sighash.try_into().unwrap()).unwrap();

    let sapling_bundle = Bundle::<_>::from_parts(
        shielded_spends, shielded_outputs, value, 
        SapAuthorized { binding_sig } );

    let authed_tx: TransactionData<Authorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle: None,
        sprout_bundle: None,
        sapling_bundle: Some(sapling_bundle),
        orchard_bundle: None,
    };

    let tx = authed_tx.freeze().unwrap();
    let mut raw_tx = vec![];
    tx.write_v5(&mut raw_tx)?;

    let response = client.send_transaction(Request::new(RawTransaction { 
        data: raw_tx, 
        height: 0, 
    })).await?.into_inner();
    println!("{}", response.error_message);

    Ok(())
}
