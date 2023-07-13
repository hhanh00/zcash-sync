use crate::chain::Nf;
use crate::db::{ReceivedNote, ReceivedNoteShort};
use crate::{db, CompactBlock, DbAdapter};
use allo_isolate::IntoDart;
use anyhow::Result;
use flatbuffers::FlatBufferBuilder;
use rayon::prelude::*;
use rusqlite::{Connection, Transaction};
use std::collections::HashMap;
use std::convert::TryInto;
use std::marker::PhantomData;
use tokio::sync::oneshot;
use zcash_note_encryption::BatchDomain;
use zcash_primitives::consensus::Parameters;

pub mod tree;
pub mod trial_decrypt;

use crate::api::dart_ffi::POST_COBJ;
use crate::db::data_generated::fb::ProgressT;
use crate::sync::tree::TreeCheckpoint;
pub use tree::{CTree, Hasher, Node, WarpProcessor, Witness};
pub use trial_decrypt::{
    CompactOutputBytes, DecryptedNote, OutputPosition, TrialDecrypter, ViewKey,
};

pub struct Synchronizer<
    N: Parameters,
    D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]>,
    VK: ViewKey<D>,
    DN: DecryptedNote<D, VK>,
    TD: TrialDecrypter<N, D, VK, DN>,
    H: Hasher,
    const POOL: char,
> {
    pub decrypter: TD,
    pub warper: WarpProcessor<H>,
    pub vks: Vec<VK>,
    pub shielded_pool: &'static str,

    pub note_position: usize,
    pub nullifiers: HashMap<Nf, ReceivedNoteShort>,
    pub tree: CTree,
    pub witnesses: Vec<Witness>,
    pub _phantom: PhantomData<(N, D, DN)>,
}

impl<
        N: Parameters + Sync,
        D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]> + Sync + Send,
        VK: ViewKey<D> + Sync + Send,
        DN: DecryptedNote<D, VK> + Sync,
        TD: TrialDecrypter<N, D, VK, DN> + Sync,
        H: Hasher,
        const POOL: char,
    > Synchronizer<N, D, VK, DN, TD, H, POOL>
{
    pub fn new(
        decrypter: TD,
        warper: WarpProcessor<H>,
        vks: Vec<VK>,
        shielded_pool: &'static str,
    ) -> Self {
        Synchronizer {
            decrypter,
            warper,
            vks,
            shielded_pool,

            note_position: 0,
            nullifiers: HashMap::default(),
            tree: CTree::new(),
            witnesses: vec![],
            _phantom: Default::default(),
        }
    }

    pub fn new_from_parts(
        decrypter: TD,
        warper: WarpProcessor<H>,
        vks: Vec<VK>,
        tree: CTree,
        received_notes: Vec<ReceivedNoteShort>,
        witnesses: Vec<Witness>,
        shielded_pool: &'static str,
    ) -> Self {
        Synchronizer {
            decrypter,
            warper,
            vks,
            shielded_pool,
            note_position: tree.get_position(),
            nullifiers: received_notes
                .into_iter()
                .map(|rn| (rn.nf.clone(), rn))
                .collect(),
            tree,
            witnesses,
            _phantom: Default::default(),
        }
    }

    pub fn initialize(&mut self, height: u32, db: &mut DbAdapter) -> Result<()> {
        let TreeCheckpoint { tree, witnesses } =
            db.get_tree_by_name(height, &self.shielded_pool)?;
        self.tree = tree;
        self.witnesses = witnesses;
        self.note_position = self.tree.get_position();
        let nfs = db.get_unspent_nullifiers()?;
        for rn in nfs.into_iter() {
            self.nullifiers.insert(rn.nf.clone(), rn);
        }
        Ok(())
    }

    pub fn process2(&mut self, blocks: &[CompactBlock], db_tx: &Transaction) -> Result<usize> {
        if blocks.is_empty() {
            return Ok(0);
        }
        let decrypter = self.decrypter.clone();
        let decrypted_blocks: Vec<_> = blocks
            .par_iter()
            .map(|b| decrypter.decrypt_notes(b, &self.vks))
            .collect();
        let count_outputs: usize = decrypted_blocks
            .iter()
            .map(|b| b.count_outputs)
            .sum::<u32>() as usize;

        self.warper.initialize(&self.tree, &self.witnesses);

        // Detect new received notes
        let mut new_witnesses = vec![];
        for decb in decrypted_blocks.iter() {
            for dectx in decb.txs.iter() {
                let id_tx = db::checkpoint::store_transaction(
                    &dectx.tx_id,
                    dectx.account,
                    dectx.height,
                    dectx.timestamp,
                    dectx.tx_index as u32,
                    &db_tx,
                )?;
                let mut balance: i64 = 0;
                for decn in dectx.notes.iter() {
                    let position = decn.position(self.note_position);
                    let rn: ReceivedNote = decn.to_received_note(position as u64);
                    let id_note =
                        db::checkpoint::store_received_note(&rn, id_tx, position, &db_tx)?;
                    let nf = Nf(rn.nf.try_into().unwrap());
                    self.nullifiers.insert(
                        nf,
                        ReceivedNoteShort {
                            id: id_note,
                            account: rn.account,
                            nf,
                            value: rn.value,
                        },
                    );
                    let witness = Witness::new(position, id_note, &decn.cmx());
                    log::info!(
                        "Witness {} {} {}",
                        witness.position,
                        witness.id_note,
                        hex::encode(witness.cmx)
                    );
                    new_witnesses.push(witness);
                    balance += rn.value as i64;
                }
                db::transaction::add_value(id_tx, balance, &db_tx)?;
            }
            self.note_position += decb.count_outputs as usize;
        }

        // Detect spends and collect note commitments
        let mut new_cmx = vec![];
        let mut height = 0;
        let mut hash = [0u8; 32];
        for b in blocks.iter() {
            for (tx_index, tx) in b.vtx.iter().enumerate() {
                for sp in self.decrypter.spends(tx).iter() {
                    if let Some(rn) = self.nullifiers.get(sp) {
                        let id_tx = db::checkpoint::store_transaction(
                            &tx.hash,
                            rn.account,
                            b.height as u32,
                            b.time,
                            tx_index as u32,
                            &db_tx,
                        )?;
                        db::transaction::add_value(id_tx, -(rn.value as i64), &db_tx)?;
                        db::transaction::mark_spent(rn.id, b.height as u32, &db_tx)?;
                        self.nullifiers.remove(sp);
                    }
                }
                new_cmx.extend(self.decrypter.outputs(tx).into_iter().map(|cob| cob.cmx));
            }
            height = b.height as u32;
            hash.copy_from_slice(&b.hash);
        }

        // Run blocks through warp sync
        self.warper.add_nodes(&mut new_cmx, &new_witnesses);
        let (updated_tree, updated_witnesses) = self.warper.finalize();

        // Store witnesses
        for w in updated_witnesses.iter() {
            db::checkpoint::store_witness::<POOL>(w, height, w.id_note, &db_tx)?;
        }
        db::checkpoint::store_tree::<POOL>(height, &updated_tree, &db_tx)?;
        self.tree = updated_tree;
        self.witnesses = updated_witnesses;

        Ok(count_outputs * self.vks.len())
    }

    pub fn process(&mut self, blocks: &[CompactBlock], db: &mut DbAdapter) -> Result<usize> {
        if blocks.is_empty() {
            return Ok(0);
        }
        let decrypter = self.decrypter.clone();
        let decrypted_blocks: Vec<_> = blocks
            .par_iter()
            .map(|b| decrypter.decrypt_notes(b, &self.vks))
            .collect();
        let count_outputs: usize = decrypted_blocks
            .iter()
            .map(|b| b.count_outputs)
            .sum::<u32>() as usize;

        self.warper.initialize(&self.tree, &self.witnesses);
        let db_tx = db.begin_transaction()?;

        // Detect new received notes
        let mut new_witnesses = vec![];
        for decb in decrypted_blocks.iter() {
            for dectx in decb.txs.iter() {
                let id_tx = DbAdapter::store_transaction(
                    &dectx.tx_id,
                    dectx.account,
                    dectx.height,
                    dectx.timestamp,
                    dectx.tx_index as u32,
                    &db_tx,
                )?;
                let mut balance: i64 = 0;
                for decn in dectx.notes.iter() {
                    let position = decn.position(self.note_position);
                    let rn: ReceivedNote = decn.to_received_note(position as u64);
                    let id_note = DbAdapter::store_received_note(&rn, id_tx, position, &db_tx)?;
                    let nf = Nf(rn.nf.try_into().unwrap());
                    self.nullifiers.insert(
                        nf,
                        ReceivedNoteShort {
                            id: id_note,
                            account: rn.account,
                            nf,
                            value: rn.value,
                        },
                    );
                    let witness = Witness::new(position, id_note, &decn.cmx());
                    log::info!(
                        "Witness {} {} {}",
                        witness.position,
                        witness.id_note,
                        hex::encode(witness.cmx)
                    );
                    new_witnesses.push(witness);
                    balance += rn.value as i64;
                }
                DbAdapter::add_value(id_tx, balance, &db_tx)?;
            }
            self.note_position += decb.count_outputs as usize;
        }

        // Detect spends and collect note commitments
        let mut new_cmx = vec![];
        let mut height = 0;
        let mut hash = [0u8; 32];
        for b in blocks.iter() {
            for (tx_index, tx) in b.vtx.iter().enumerate() {
                for sp in self.decrypter.spends(tx).iter() {
                    if let Some(rn) = self.nullifiers.get(sp) {
                        let id_tx = DbAdapter::store_transaction(
                            &tx.hash,
                            rn.account,
                            b.height as u32,
                            b.time,
                            tx_index as u32,
                            &db_tx,
                        )?;
                        DbAdapter::add_value(id_tx, -(rn.value as i64), &db_tx)?;
                        DbAdapter::mark_spent(rn.id, b.height as u32, &db_tx)?;
                        self.nullifiers.remove(sp);
                    }
                }
                new_cmx.extend(self.decrypter.outputs(tx).into_iter().map(|cob| cob.cmx));
            }
            height = b.height as u32;
            hash.copy_from_slice(&b.hash);
        }

        // Run blocks through warp sync
        self.warper.add_nodes(&mut new_cmx, &new_witnesses);
        let (updated_tree, updated_witnesses) = self.warper.finalize();

        // Store witnesses
        for w in updated_witnesses.iter() {
            DbAdapter::store_witness(w, height, w.id_note, &db_tx, &self.shielded_pool)?;
        }
        DbAdapter::store_tree(height, &updated_tree, &db_tx, &self.shielded_pool)?;
        self.tree = updated_tree;
        self.witnesses = updated_witnesses;

        db_tx.commit()?;
        Ok(count_outputs * self.vks.len())
    }
}

// pub async fn warp(
//     coin: u8,
//     get_tx: bool,
//     anchor_offset: u32,
//     max_cost: u32,
//     port: i64,
//     rx_cancel: oneshot::Receiver<()>,
// ) -> Result<u32> {
//     crate::api::sync::coin_sync(
//         coin,
//         get_tx,
//         anchor_offset,
//         max_cost,
//         move |progress| {
//             let progress = ProgressT {
//                 height: progress.height,
//                 trial_decryptions: progress.trial_decryptions,
//                 downloaded: progress.downloaded as u64,
//             };
//             let mut builder = FlatBufferBuilder::new();
//             let root = progress.pack(&mut builder);
//             builder.finish(root, None);
//             let v = builder.finished_data().to_vec();
//             let mut progress = v.into_dart();
//             if port != 0 {
//                 unsafe {
//                     if let Some(p) = POST_COBJ {
//                         p(port, &mut progress);
//                     }
//                 }
//             }
//         },
//         rx_cancel,
//     )
//     .await
// }

