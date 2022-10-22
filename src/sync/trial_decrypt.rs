use std::collections::HashMap;
use crate::chain::Nf;
use std::convert::TryInto;
use std::marker::PhantomData;
use std::time::Instant;
use zcash_note_encryption::batch::try_compact_note_decryption;
use zcash_note_encryption::{BatchDomain, COMPACT_NOTE_SIZE, EphemeralKeyBytes, ShieldedOutput};
use zcash_primitives::consensus::{BlockHeight, Parameters};
use crate::{CompactBlock, CompactSaplingOutput, CompactTx};
use crate::db::ReceivedNote;
use crate::sync::tree::Node;

pub struct DecryptedBlock<D: BatchDomain, VK, DN: DecryptedNote<D, VK>> {
    pub height: u32,
    pub spends: Vec<Nf>,
    pub txs: Vec<DecryptedTx<D, VK, DN>>,
    pub count_outputs: u32,
    pub elapsed: usize,
    _phantom: PhantomData<(D, VK)>,
}

pub struct DecryptedTx<D: BatchDomain, VK, DN: DecryptedNote<D, VK>> {
    pub account: u32,
    pub height: u32,
    pub timestamp: u32,
    pub tx_index: usize,
    pub tx_id: Vec<u8>,
    pub notes: Vec<DN>,
    _phantom: PhantomData<(D, VK)>,
}

pub trait ViewKey<D: BatchDomain>: Clone {
    fn account(&self) -> u32;
    fn ivk(&self) -> D::IncomingViewingKey;
}

#[derive(Clone)]
pub struct OutputPosition {
    pub height: u32,
    pub tx_index: usize,
    pub output_index: usize,
    pub position_in_block: usize,
}

pub trait DecryptedNote<D: BatchDomain, VK>: Send + Sync {
    fn from_parts(vk: VK, note: D::Note, pa: D::Recipient, output_position: OutputPosition, cmx: Node) -> Self;
    fn position(&self, block_offset: usize) -> usize;
    fn cmx(&self) -> Node;
    fn to_received_note(&self, position: u64) -> ReceivedNote;
}

// Deep copy from protobuf message
pub struct CompactOutputBytes {
    pub epk: [u8; 32],
    pub cmx: [u8; 32],
    pub ciphertext: [u8; 52],
}

impl From<&CompactSaplingOutput> for CompactOutputBytes {
    fn from(co: &CompactSaplingOutput) -> Self {
        CompactOutputBytes {
            epk: co.epk.clone().try_into().unwrap(),
            cmx: co.cmu.clone().try_into().unwrap(),
            ciphertext: co.ciphertext.clone().try_into().unwrap(),
        }
    }
}

pub struct CompactShieldedOutput(CompactOutputBytes, OutputPosition);

impl<D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]>> ShieldedOutput<D, COMPACT_NOTE_SIZE>
for CompactShieldedOutput
{
    fn ephemeral_key(&self) -> EphemeralKeyBytes {
        EphemeralKeyBytes(self.0.epk)
    }
    fn cmstar_bytes(&self) -> D::ExtractedCommitmentBytes {
        self.0.cmx
    }
    fn enc_ciphertext(&self) -> &[u8; COMPACT_NOTE_SIZE] {
        &self.0.ciphertext
    }
}

pub trait TrialDecrypter<N: Parameters, D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]>, VK: ViewKey<D>, DN: DecryptedNote<D, VK>>: Clone {
    fn decrypt_notes(
        &self,
        block: &CompactBlock,
        vks: &[VK],
    ) -> DecryptedBlock<D, VK, DN> {
        let height = BlockHeight::from_u32(block.height as u32);
        let mut count_outputs = 0u32;
        let mut spends: Vec<Nf> = vec![];
        let vvks: Vec<_> = vks.iter().map(|vk| vk.ivk().clone()).collect();
        let mut outputs = vec![];
        let mut txs = HashMap::new();
        for (tx_index, vtx) in block.vtx.iter().enumerate() {
            for cs in vtx.spends.iter() {
                let mut nf = [0u8; 32];
                nf.copy_from_slice(&cs.nf);
                spends.push(Nf(nf));
            }

            let tx_outputs = self.outputs(vtx);
            if let Some(fco) = tx_outputs.first() {
                if !fco.epk.is_empty() {
                    for (output_index, cob) in tx_outputs.into_iter().enumerate() {
                        let domain = self.domain(height);
                        let pos = OutputPosition {
                            height: block.height as u32,
                            tx_index,
                            output_index,
                            position_in_block: count_outputs as usize,
                        };
                        let output = CompactShieldedOutput(cob, pos);
                        outputs.push((domain, output));

                        count_outputs += 1;
                    }
                } else {
                    // we filter by transaction, therefore if one epk is empty, every epk is empty
                    // log::info!("Spam Filter tx {}", hex::encode(&vtx.hash));
                    count_outputs += vtx.outputs.len() as u32;
                }
            }
        }

        let start = Instant::now();
        let notes_decrypted =
            try_compact_note_decryption(&vvks, &outputs);
        let elapsed = start.elapsed().as_millis() as usize;

        for (pos, opt_note) in notes_decrypted.iter().enumerate() {
            if let Some(((note, pa), _)) = opt_note {
                let vk = &vks[pos / outputs.len()];
                let account = vk.account();
                let output = &outputs[pos % outputs.len()];
                let tx_index = output.1.1.tx_index;
                let tx_key = (account, tx_index);
                let tx = txs.entry(tx_key).or_insert_with(||
                   DecryptedTx {
                       account,
                       height: block.height as u32,
                       timestamp: block.time,
                       tx_index,
                       tx_id: block.vtx[tx_index].hash.clone(),
                       notes: vec![],
                       _phantom: PhantomData::default(),
                });
                tx.notes.push(DN::from_parts(
                    vk.clone(),
                    note.clone(),
                    pa.clone(),
                    output.1.1.clone(),
                    output.1.0.cmx,
                ));
            }
        }

        DecryptedBlock {
            height: block.height as u32,
            spends,
            txs: txs.into_values().collect(),
            count_outputs,
            elapsed,
            _phantom: PhantomData::default(),
        }
    }

    fn domain(&self, height: BlockHeight) -> D;
    fn spends(&self, vtx: &CompactTx) -> Vec<Nf>;
    fn outputs(&self, vtx: &CompactTx) -> Vec<CompactOutputBytes>;
}

