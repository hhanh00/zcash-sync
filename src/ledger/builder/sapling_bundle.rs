use std::io::Write;
use blake2b_simd::State;

use ff::PrimeField;
use group::GroupEncoding;

use jubjub::{Fq, Fr};
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::sapling::ProofGenerationKey;
use zcash_primitives::zip32::{DiversifiableFullViewingKey, ExtendedSpendingKey, Scope};

use crate::ledger::transport::*;

use anyhow::{anyhow, Result};
use rand::{Rng, RngCore};

use zcash_primitives::{
    consensus::MainNetwork,
    merkle_tree::IncrementalWitness,
    sapling::{
        note_encryption::sapling_note_encryption,
        prover::TxProver,
        redjubjub::Signature,
        value::{NoteValue, ValueCommitment, ValueSum},
        Diversifier, Node, Note, Nullifier, PaymentAddress, Rseed,
    },
    transaction::components::{
        sapling::{Authorized as SapAuthorized, Bundle},
        Amount, OutputDescription, SpendDescription, GROTH_PROOF_SIZE,
    },
};
use zcash_primitives::constants::SPENDING_KEY_GENERATOR;
use ::zcash_primitives::transaction::components::sapling;
use zcash_primitives::consensus::{BlockHeight, Network};
use zcash_primitives::consensus::Network::YCashMainNetwork;
use zcash_primitives::sapling::keys::ExpandedSpendingKey;
use zcash_primitives::transaction::components::sapling::Authorized;
use zcash_primitives::transaction::components::sapling::builder::Unauthorized as SaplingUnauthorized;
use zcash_primitives::transaction::{TransactionData, Unauthorized};
use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v4::v4_signature_hash;
use zcash_proofs::{prover::LocalTxProver, sapling::SaplingProvingContext};

use super::create_hasher;

struct SpendDescriptionUnAuthorized {
    cv: ValueCommitment,
    anchor: Fq,
    pub nullifier: Nullifier,
    rk: zcash_primitives::sapling::redjubjub::PublicKey,
    alpha: Fr,
    zkproof: [u8; GROTH_PROOF_SIZE],
}

pub struct Unauth {
    builder: sapling::builder::SaplingBuilder<Network>,
}

pub struct Proven {
    ctx: SaplingProvingContext,
}

pub struct SaplingBuilder<'a, A> {
    prover: &'a LocalTxProver,
    dfvk: DiversifiableFullViewingKey,
    proofgen_key: ProofGenerationKey,
    esk: ExtendedSpendingKey,
    pub auth: A,
    // signatures: Vec<Signature>,
}

impl<'a> SaplingBuilder<'a, Unauth> {
    pub fn new(
        network: &Network,
        prover: &'a LocalTxProver,
        dfvk: DiversifiableFullViewingKey,
        proofgen_key: ProofGenerationKey,
        height: u32,
    ) -> Self {
        let builder =
            sapling::builder::SaplingBuilder::<_>::new(network.clone(), BlockHeight::from_u32(height));
        // a dummy ExtendedSpendingKey
        let esk = ExtendedSpendingKey::read([0; 169].as_slice()).unwrap();
        SaplingBuilder {
            prover,
            dfvk,
            proofgen_key,
            esk,
            auth: Unauth {
                builder
            },
        }
    }

    pub fn add_spend<R: RngCore>(
        &mut self,
        diversifier: [u8; 11],
        rseed: [u8; 32],
        witness: &[u8],
        amount: u64,
        mut rng: R,
    ) -> Result<()> {
        let diversifier = Diversifier(diversifier);
        let z_address = self
            .dfvk
            .fvk
            .vk
            .to_payment_address(diversifier)
            .ok_or(anyhow!("Invalid diversifier"))?;
        let rseed = Rseed::BeforeZip212(Fr::from_bytes(&rseed).unwrap());
        let note = Note::from_parts(z_address, NoteValue::from_raw(amount), rseed);
        let witness = IncrementalWitness::<Node>::read(&witness[..])?;
        let merkle_path = witness.path().ok_or(anyhow!("Invalid merkle path"))?;
        self.auth.builder.add_spend_with_pgk(rng, self.esk.clone(), self.proofgen_key.clone(), diversifier, note, merkle_path).unwrap();
        Ok(())
    }

    pub fn add_output<R: RngCore>(
        &mut self,
        rseed: [u8; 32],
        raw_address: [u8; 43],
        memo: &MemoBytes,
        amount: u64,
        mut rng: R,
    ) -> Result<()> {
        let ovk = self.esk.expsk.ovk.clone();
        let recipient = PaymentAddress::from_bytes(&raw_address).unwrap();
        let value = NoteValue::from_raw(amount);
        self.auth.builder.add_output(rng, Some(ovk), recipient, value, memo.clone()).unwrap();

        Ok(())
    }

    pub fn prepare<R: RngCore>(self, height: u32, mut rng: R) -> (SaplingBuilder<'a, Proven>, Option<Bundle<SaplingUnauthorized>>) {
        let mut ctx = SaplingProvingContext::new();
        let bundle = self.auth.builder.build(self.prover, &mut ctx, rng, BlockHeight::from_u32(height), None).unwrap();
        let builder = SaplingBuilder::<Proven> {
            prover: self.prover,
            dfvk: self.dfvk,
            proofgen_key: self.proofgen_key,
            esk: self.esk,
            auth: Proven { ctx }
        };
        (builder, bundle)
    }
}

impl <'a> SaplingBuilder<'a, Proven> {
    pub fn sign(&mut self, tx_data: &TransactionData<Unauthorized>) -> Result<Option<Bundle<Authorized>>> {
        let bundle = match tx_data.sapling_bundle.as_ref() {
            Some(bundle) => {
                let hash = v4_signature_hash(tx_data, &SignableInput::Shielded {});
                let binding_sig = self.prover
                    .binding_sig(&mut self.auth.ctx, bundle.value_balance, &hash.as_bytes().try_into().unwrap()).unwrap();
                let mut signatures = vec![];
                for sp in bundle.shielded_spends.iter() {
                    let alpha = sp.spend_auth_sig.alpha;
                    let signature = ledger_sign_sapling(hash.as_bytes(), &alpha.to_bytes())?;
                    let signature = Signature::read(&*signature)?;
                    signatures.push(signature);
                }
                let bundle = Bundle::<Authorized> {
                    shielded_spends: bundle.shielded_spends.iter().zip(signatures).map(|(sp, sig)|
                        SpendDescription {
                            cv: sp.cv.clone(),
                            anchor: sp.anchor.clone(),
                            nullifier: sp.nullifier.clone(),
                            rk: sp.rk.clone(),
                            zkproof: sp.zkproof.clone(),
                            spend_auth_sig: sig
                        }

                    ).collect(),
                    shielded_outputs: bundle.shielded_outputs.clone(),
                    value_balance: bundle.value_balance,
                    authorization: Authorized { binding_sig }
                };
                Some(bundle)
            }
            None => None
        };
        Ok(bundle)
    }
}
