use std::{
    io::Read, vec,
};
use std::io::Write;
use blake2b_simd::Params;
use byteorder::WriteBytesExt;
use byteorder::LE;
use ff::{PrimeField, Field};
use group::GroupEncoding;
use hex_literal::hex;
use jubjub::{Fr, Fq};
use rand::{rngs::OsRng, RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use ripemd::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use tonic::{Request, transport::Channel};
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::decode_transparent_address;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::{TransparentAddress, Script};
use zcash_primitives::transaction::components::transparent::builder::Unauthorized;
use zcash_primitives::transaction::components::{transparent, TxIn, OutPoint, TxOut};
use zcash_primitives::transaction::txid::TxIdDigester;
use crate::{Destination, Source, TransactionPlan, RawTransaction, CompactTxStreamerClient};
use crate::ledger::transport::*;
use anyhow::{anyhow, Result};

use zcash_primitives::{
    consensus::{BlockHeight, BranchId, MainNetwork},
    merkle_tree::{Hashable, IncrementalWitness},
    sapling::{
        note_encryption::sapling_note_encryption, value::{NoteValue, ValueCommitment, ValueSum}, Diversifier, Node, Note,
        PaymentAddress, Rseed, Nullifier, prover::TxProver, redjubjub::Signature,
    },
    transaction::{
        components::{sapling::{Bundle, Authorized as SapAuthorized}, GROTH_PROOF_SIZE, Amount, OutputDescription, SpendDescription},
        TransactionData, TxVersion, Authorized,
    }, constants::PROOF_GENERATION_KEY_GENERATOR,
};
use zcash_proofs::{prover::LocalTxProver, sapling::SaplingProvingContext};

struct TransparentInputUnAuthorized {
    utxo: OutPoint,
    coin: TxOut,
}

struct SpendDescriptionUnAuthorized {
    cv: ValueCommitment,
    anchor: Fq,
    pub nullifier: Nullifier,
    rk: zcash_primitives::sapling::redjubjub::PublicKey,
    zkproof: [u8; GROTH_PROOF_SIZE],
}

pub async fn build_broadcast_tx(client: &mut CompactTxStreamerClient<Channel>, tx_plan: &TransactionPlan, prover: &LocalTxProver) -> Result<String> {
    ledger_init().await?;
    let pubkey = ledger_get_pubkey().await?;

    let network = MainNetwork; // TODO: Pass network as param
    let secp = Secp256k1::<All>::new();

    let taddr = &tx_plan.taddr;
    let taddr = decode_transparent_address(
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        taddr
    )?.ok_or(anyhow!("Invalid taddr"))?;
    let pkh = match taddr {
        TransparentAddress::PublicKey(pkh) => pkh,
        _ => unreachable!()
    };
    let tin_pubscript = taddr.script();

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

    let mut vin = vec![];
    let mut shielded_spends = vec![];
    for sp in tx_plan.spends.iter() {
        match sp.source {
            Source::Transparent { txid, index } => {
                prevouts_hasher.update(&txid);
                prevouts_hasher.write_u32::<LE>(index)?;
                trscripts_hasher.update(&hex!("1976a914"));
                trscripts_hasher.update(&pkh);
                trscripts_hasher.update(&hex!("88ac"));
                sequences_hasher.update(&hex!("FFFFFFFF"));

                vin.push(TransparentInputUnAuthorized { 
                    utxo: OutPoint::new(txid, index), 
                    coin: TxOut { value: Amount::from_u64(sp.amount).unwrap(), 
                        script_pubkey: tin_pubscript.clone(), // will always use the h/w address
                    } 
                });
                
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
    if !shielded_spends.is_empty() {
        spends_hasher.update(spends_compact_digest.as_bytes());
        spends_hasher.update(spends_non_compact_digest.as_bytes());
    }
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

    let mut vout = vec![];
    let mut shielded_outputs = vec![];
    for output in tx_plan.outputs.iter() {
        if let Destination::Transparent(raw_address) = output.destination {
            if raw_address[0] != 0 {
                anyhow::bail!("Only t1 addresses are supported");
            }
            ledger_add_t_output(output.amount, &raw_address).await?;
            let ta = TransparentAddress::PublicKey(raw_address[1..21].try_into().unwrap());
            vout.push(TxOut { 
                value: Amount::from_u64(output.amount).unwrap(), 
                script_pubkey: ta.script()
            });
        }
    }
    ledger_set_stage(2).await?;
    let has_transparent = !vin.is_empty() || !vout.is_empty();

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

    let mut vins = vec![];
    for tin in vin.iter() {
        let mut txin_hasher = Params::new()
            .hash_length(32)
            .personal(b"Zcash___TxInHash")
            .to_state();

        txin_hasher.update(tin.utxo.hash());
        txin_hasher.update(&tin.utxo.n().to_le_bytes());
        txin_hasher.update(&tin.coin.value.to_i64_le_bytes());
        txin_hasher.update(&[0x19]); // add the script length
        txin_hasher.update(&tin.coin.script_pubkey.0);
        txin_hasher.update(&0xFFFFFFFFu32.to_le_bytes());
        let txin_hash = txin_hasher.finalize();
        println!("TXIN {}", hex::encode(txin_hash));

        let signature = ledger_sign_transparent(txin_hash.as_bytes()).await?;
        let signature = secp256k1::ecdsa::Signature::from_der(&signature)?;
        let mut signature = signature.serialize_der().to_vec();
        signature.extend(&[0x01]); // add SIG_HASH_ALL

        // witness is PUSH(signature) PUSH(pk)
        let script_sig = Script::default() << &*signature << &*pubkey;

        let txin = TxIn::<transparent::Authorized> {
            prevout: tin.utxo.clone(),
            script_sig,
            sequence: 0xFFFFFFFFu32,
        };
        println!("TXIN {:?}", txin);
        vins.push(txin);
    }

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

    // let sk = SecretKey::from_slice(&hex::decode("a2a6cef015bbce367e916d83152094d6492c422beb037edcbbd50593aa02b183").unwrap()).unwrap();
    // let mut transparent_builder = transparent::builder::TransparentBuilder::empty();
    // for vin in vin.iter() {
    //     transparent_builder.add_input(sk.clone(), vin.utxo.clone(), vin.coin.clone()).unwrap();
    // }
    // for vout in vout.iter() {
    //     transparent_builder.add_output(&vout.recipient_address().unwrap(), vout.value).unwrap();
    // }
    // let transparent_bundle = transparent_builder.build();
    // let transparent_bundle = {
    //     let consensus_branch_id =
    //         BranchId::for_height(&network, BlockHeight::from_u32(tx_plan.anchor_height));
    //     let version = TxVersion::suggested_for_branch(consensus_branch_id);
    //     let unauthed_tx: TransactionData<zcash_primitives::transaction::Unauthorized> =
    //     TransactionData::from_parts(
    //         version,
    //         consensus_branch_id,
    //         0,
    //         BlockHeight::from_u32(tx_plan.expiry_height),
    //         transparent_bundle.clone(),
    //         None,
    //         None,
    //         None,
    //     );

    //     let txid_parts = unauthed_tx.digest(TxIdDigester);

    //     transparent_bundle.unwrap().apply_signatures(&unauthed_tx, &txid_parts)
    // };
    // println!("VIN 2 {:?}", &transparent_bundle.vin[0]);

    let transparent_bundle = transparent::Bundle::<transparent::Authorized> { 
        vin: vins, 
        vout, 
        authorization: transparent::Authorized 
    };

    let shielded_spends: Vec<_> = shielded_spends.into_iter().zip(signatures.into_iter()).map(|(sp, spend_auth_sig)| 
        SpendDescription::<SapAuthorized> { cv: sp.cv, anchor: sp.anchor, nullifier: sp.nullifier, rk: sp.rk, zkproof: sp.zkproof, 
            spend_auth_sig }).collect();
    let has_sapling = !shielded_spends.is_empty() || !shielded_outputs.is_empty();

    let value: i64 = value_balance.try_into().unwrap();
    let value = Amount::from_i64(value).unwrap();
    let sighash = ledger_get_sighash().await?;
    println!("TXID {}", hex::encode(&sighash));
    let binding_sig = sapling_context.binding_sig(value, &sighash.try_into().unwrap()).unwrap();

    println!("{} {}", has_transparent, has_sapling);
    println!("{} {} {:?}", shielded_spends.len(), shielded_outputs.len(), value_balance);

    let sapling_bundle = Bundle::<_>::from_parts(
        shielded_spends, shielded_outputs, value, 
        SapAuthorized { binding_sig } );

    let authed_tx: TransactionData<Authorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(tx_plan.expiry_height),
        transparent_bundle: if has_transparent { Some(transparent_bundle) } else { None },
        sprout_bundle: None,
        sapling_bundle: if has_sapling { Some(sapling_bundle) } else { None },
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

    Ok(response.error_message)
}
