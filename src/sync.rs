use std::collections::HashMap;
use std::convert::TryInto;
use std::marker::PhantomData;
use anyhow::Result;
use rayon::prelude::*;
use zcash_note_encryption::BatchDomain;
use zcash_primitives::consensus::Parameters;
use crate::{CompactBlock, DbAdapter};
use crate::chain::Nf;
use crate::db::{DbAdapterBuilder, ReceivedNote, ReceivedNoteShort};

pub mod tree;
pub mod trial_decrypt;

pub use trial_decrypt::{ViewKey, DecryptedNote, TrialDecrypter, CompactOutputBytes, OutputPosition};
pub use tree::{Hasher, Node, WarpProcessor, Witness, CTree};
use crate::sync::tree::TreeCheckpoint;

pub struct Synchronizer<N: Parameters, D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]>, VK: ViewKey<D>, DN: DecryptedNote<D, VK>,
    TD: TrialDecrypter<N, D, VK, DN>, H: Hasher> {
    pub decrypter: TD,
    pub warper: WarpProcessor<H>,
    pub vks: Vec<VK>,
    pub db: DbAdapterBuilder,
    pub shielded_pool: String,

    pub note_position: usize,
    pub nullifiers: HashMap<Nf, ReceivedNoteShort>,
    pub tree: CTree,
    pub witnesses: Vec<Witness>,
    pub _phantom: PhantomData<(N, D, DN)>,
}

impl <N: Parameters + Sync,
    D: BatchDomain<ExtractedCommitmentBytes = [u8; 32]> + Sync + Send,
    VK: ViewKey<D> + Sync + Send,
    DN: DecryptedNote<D, VK> + Sync,
    TD: TrialDecrypter<N, D, VK, DN> + Sync,
    H: Hasher> Synchronizer<N, D, VK, DN, TD, H> {
    pub fn new(decrypter: TD, warper: WarpProcessor<H>, vks: Vec<VK>, db: DbAdapterBuilder, shielded_pool: String) -> Self {
        Synchronizer {
            decrypter,
            warper,
            vks,
            db,
            shielded_pool,

            note_position: 0,
            nullifiers: HashMap::default(),
            tree: CTree::new(),
            witnesses: vec![],
            _phantom: Default::default()
        }
    }

    pub fn initialize(&mut self, height: u32) -> Result<()> {
        let db = self.db.build()?;
        let TreeCheckpoint { tree, witnesses } = db.get_tree_by_name(height, &self.shielded_pool)?;
        self.tree = tree;
        self.witnesses = witnesses;
        self.note_position = self.tree.get_position();
        let nfs = db.get_unspent_nullifiers()?;
        for rn in nfs.into_iter() {
            self.nullifiers.insert(rn.nf.clone(), rn);
        }
        Ok(())
    }

    pub fn process(&mut self, blocks: &[CompactBlock]) -> Result<()> {
        if blocks.is_empty() { return Ok(()) }
        let decrypter = self.decrypter.clone();
        let decrypted_blocks: Vec<_> = blocks
            .par_iter()
            .map(|b| decrypter.decrypt_notes(b, &self.vks))
            .collect();
        let mut db = self.db.build()?;
        self.warper.initialize(&self.tree, &self.witnesses);
        let db_tx = db.begin_transaction()?;

        // Detect new received notes
        let mut new_witnesses = vec![];
        for decb in decrypted_blocks.iter() {
            for dectx in decb.txs.iter() {
                let id_tx = DbAdapter::store_transaction(&dectx.tx_id, dectx.account, dectx.height, dectx.timestamp, dectx.tx_index as u32, &db_tx)?;
                let mut balance: i64 = 0;
                for decn in dectx.notes.iter() {
                    let position = decn.position(self.note_position);
                    let rn: ReceivedNote = decn.to_received_note(position as u64);
                    let id_note = DbAdapter::store_received_note(&rn, id_tx, position, &db_tx)?;
                    let nf = Nf(rn.nf.try_into().unwrap());
                    self.nullifiers.insert(nf, ReceivedNoteShort {
                        id: id_note,
                        account: rn.account,
                        nf,
                        value: rn.value
                    });
                    let witness = Witness::new(position, id_note, &decn.cmx());
                    log::info!("Witness {} {} {}", witness.position, witness.id_note, hex::encode(witness.cmx));
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
                        let id_tx = DbAdapter::store_transaction(&tx.hash, rn.account, b.height as u32, b.time, tx_index as u32, &db_tx)?;
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use zcash_primitives::consensus::Network;
    use zcash_primitives::sapling::note_encryption::SaplingDomain;
    use crate::coinconfig::COIN_CONFIG;
    use crate::db::DbAdapterBuilder;
    use crate::init_coin;
    use crate::sapling::{DecryptedSaplingNote, SaplingDecrypter, SaplingHasher, SaplingViewKey};
    use crate::sync::CTree;
    use crate::sync::tree::WarpProcessor;
    use super::Synchronizer;

    type SaplingSynchronizer = Synchronizer<Network, SaplingDomain<Network>, SaplingViewKey, DecryptedSaplingNote,
        SaplingDecrypter<Network>, SaplingHasher>;

    #[test]
    fn test() {
        init_coin(0, "zec.db").unwrap();
        let coin = COIN_CONFIG[0].lock().unwrap();
        let network = coin.chain.network();
        let mut synchronizer = SaplingSynchronizer {
            decrypter: SaplingDecrypter::new(*network),
            warper: WarpProcessor::new(SaplingHasher::default()),
            vks: vec![],
            db: DbAdapterBuilder { coin_type: coin.coin_type, db_path: coin.db_path.as_ref().unwrap().to_owned() },
            shielded_pool: "sapling".to_string(),
            tree: CTree::new(),
            witnesses: vec![],

            note_position: 0,
            nullifiers: HashMap::new(),
            _phantom: Default::default()
        };

        synchronizer.initialize(1000).unwrap();
        synchronizer.process(&vec![]).unwrap();
    }

}
