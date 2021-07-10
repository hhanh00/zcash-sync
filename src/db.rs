use crate::chain::{Nf, NfRef};
use crate::{CTree, Witness};
use rusqlite::{params, Connection, OptionalExtension, NO_PARAMS};
use std::collections::HashMap;
use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::{Diversifier, Node, Note, Rseed};
use zcash_primitives::zip32::{ExtendedFullViewingKey, DiversifierIndex};
use crate::taddr::{derive_tkeys, BIP44_PATH};
use crate::db::migration::{get_schema_version, update_schema_version};
use crate::transaction::TransactionInfo;
use zcash_primitives::memo::Memo;

mod migration;

#[allow(dead_code)]
pub const DEFAULT_DB_PATH: &str = "zec.db";

pub struct DbAdapter {
    pub connection: Connection,
}

pub struct ReceivedNote {
    pub account: u32,
    pub height: u32,
    pub output_index: u32,
    pub diversifier: Vec<u8>,
    pub value: u64,
    pub rcm: Vec<u8>,
    pub nf: Vec<u8>,
    pub spent: Option<u32>,
}

pub struct SpendableNote {
    pub id: u32,
    pub note: Note,
    pub diversifier: Diversifier,
    pub witness: IncrementalWitness<Node>,
}

impl DbAdapter {
    pub fn new(db_path: &str) -> anyhow::Result<DbAdapter> {
        let connection = Connection::open(db_path)?;
        Ok(DbAdapter { connection })
    }

    pub fn init_db(&self) -> anyhow::Result<()> {
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
            id INTEGER PRIMARY KEY NOT NULL,
            version INTEGER NOT NULL)",
            NO_PARAMS,
        )?;

        let version = get_schema_version(&self.connection)?;
        if version == 1 { return Ok(()); }

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS accounts (
            id_account INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            seed TEXT,
            sk TEXT,
            ivk TEXT NOT NULL UNIQUE,
            address TEXT NOT NULL)",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS blocks (
            height INTEGER PRIMARY KEY,
            hash BLOB NOT NULL,
            timestamp INTEGER NOT NULL,
            sapling_tree BLOB NOT NULL)",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS transactions (
            id_tx INTEGER PRIMARY KEY,
            account INTEGER NOT NULL,
            txid BLOB NOT NULL,
            height INTEGER NOT NULL,
            timestamp INTEGER NOT NULL,
            value INTEGER NOT NULL,
            address TEXT,
            memo TEXT,
            tx_index INTEGER,
            CONSTRAINT tx_account UNIQUE (height, tx_index, account))",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS received_notes (
            id_note INTEGER PRIMARY KEY,
            account INTEGER NOT NULL,
            position INTEGER NOT NULL,
            tx INTEGER NOT NULL,
            height INTEGER NOT NULL,
            output_index INTEGER NOT NULL,
            diversifier BLOB NOT NULL,
            value INTEGER NOT NULL,
            rcm BLOB NOT NULL,
            nf BLOB NOT NULL UNIQUE,
            spent INTEGER,
            CONSTRAINT tx_output UNIQUE (tx, output_index))",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS sapling_witnesses (
            id_witness INTEGER PRIMARY KEY,
            note INTEGER NOT NULL,
            height INTEGER NOT NULL,
            witness BLOB NOT NULL,
            CONSTRAINT witness_height UNIQUE (note, height))",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS diversifiers (
            account INTEGER PRIMARY KEY NOT NULL,
            diversifier_index BLOB NOT NULL)",
            NO_PARAMS,
        )?;

        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS taddrs (
            account INTEGER PRIMARY KEY NOT NULL,
            sk TEXT NOT NULL,
            address TEXT NOT NULL)",
            NO_PARAMS,
        )?;

        update_schema_version(&self.connection, 1)?;

        Ok(())
    }

    pub fn store_account(&self, name: &str, seed: Option<&str>, sk: Option<&str>, ivk: &str, address: &str) -> anyhow::Result<u32> {
        self.connection.execute(
            "INSERT INTO accounts(name, seed, sk, ivk, address) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT DO NOTHING",
            params![name, seed, sk, ivk, address],
        )?;
        let id_tx: u32 = self.connection.query_row(
            "SELECT id_account FROM accounts WHERE sk = ?1",
            params![sk],
            |row| row.get(0),
        )?;
        Ok(id_tx)
    }

    pub fn get_fvks(&self) -> anyhow::Result<HashMap<u32, String>> {
        let mut statement = self.connection.prepare("SELECT id_account, ivk FROM accounts")?;
        let rows = statement.query_map(NO_PARAMS, |row| {
            let account: u32 = row.get(0)?;
            let ivk: String = row.get(1)?;
            Ok((account, ivk))
        })?;
        let mut fvks: HashMap<u32, String> = HashMap::new();
        for r in rows {
            let row = r?;
            fvks.insert(row.0, row.1);
        }
        Ok(fvks)
    }

    pub fn trim_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM blocks WHERE height >= ?1", params![height])?;
        tx.execute(
            "DELETE FROM sapling_witnesses WHERE height >= ?1",
            params![height],
        )?;
        tx.execute(
            "DELETE FROM received_notes WHERE height >= ?1",
            params![height],
        )?;
        tx.execute(
            "DELETE FROM transactions WHERE height >= ?1",
            params![height],
        )?;
        tx.commit()?;

        Ok(())
    }

    pub fn get_txhash(&self, id_tx: u32) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let (account, height, tx_hash) = self.connection.query_row(
            "SELECT account, height, txid FROM transactions WHERE id_tx = ?1", params![id_tx],
            |row| {
                let account: u32 = row.get(0)?;
                let height: u32 = row.get(1)?;
                let tx_hash: Vec<u8> = row.get(2)?;
                Ok((account, height, tx_hash))
            })?;
        Ok((account, height, tx_hash))
    }

    pub fn store_block(
        &self,
        height: u32,
        hash: &[u8],
        timestamp: u32,
        tree: &CTree,
    ) -> anyhow::Result<()> {
        log::debug!("+block");
        let mut bb: Vec<u8> = vec![];
        tree.write(&mut bb)?;
        self.connection.execute(
            "INSERT INTO blocks(height, hash, timestamp, sapling_tree)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT DO NOTHING",
            params![height, hash, timestamp, &bb],
        )?;
        log::debug!("-block");
        Ok(())
    }

    pub fn store_transaction(
        &self,
        txid: &[u8],
        account: u32,
        height: u32,
        timestamp: u32,
        tx_index: u32,
    ) -> anyhow::Result<u32> {
        log::debug!("+transaction");
        self.connection.execute(
            "INSERT INTO transactions(account, txid, height, timestamp, tx_index, value)
        VALUES (?1, ?2, ?3, ?4, ?5, 0)
        ON CONFLICT DO NOTHING",
            params![account, txid, height, timestamp, tx_index],
        )?;
        let id_tx: u32 = self.connection.query_row(
            "SELECT id_tx FROM transactions WHERE txid = ?1",
            params![txid],
            |row| row.get(0),
        )?;
        log::debug!("-transaction {}", id_tx);
        Ok(id_tx)
    }

    pub fn store_received_note(
        &self,
        note: &ReceivedNote,
        id_tx: u32,
        position: usize,
    ) -> anyhow::Result<u32> {
        log::debug!("+received_note {}", id_tx);
        self.connection.execute("INSERT INTO received_notes(account, tx, height, position, output_index, diversifier, value, rcm, nf, spent)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT DO NOTHING", params![note.account, id_tx, note.height, position as u32, note.output_index, note.diversifier, note.value as i64, note.rcm, note.nf, note.spent])?;
        let id_note: u32 = self.connection.query_row(
            "SELECT id_note FROM received_notes WHERE tx = ?1 AND output_index = ?2",
            params![id_tx, note.output_index],
            |row| row.get(0),
        )?;
        log::debug!("-received_note");
        Ok(id_note)
    }

    pub fn store_witnesses(
        &self,
        witness: &Witness,
        height: u32,
        id_note: u32,
    ) -> anyhow::Result<()> {
        log::debug!("+witnesses");
        let mut bb: Vec<u8> = vec![];
        witness.write(&mut bb)?;
        self.connection.execute(
            "INSERT INTO sapling_witnesses(note, height, witness) VALUES (?1, ?2, ?3)
        ON CONFLICT DO NOTHING",
            params![id_note, height, bb],
        )?;
        log::debug!("-witnesses");
        Ok(())
    }

    pub fn store_tx_metadata(&self, id_tx: u32, tx_info: &TransactionInfo) -> anyhow::Result<()> {
        let memo = match &tx_info.memo {
            Memo::Empty => "".to_string(),
            Memo::Text(text) => text.to_string(),
            Memo::Future(_) => "Unrecognized".to_string(),
            Memo::Arbitrary(_) => "Unrecognized".to_string(),
        };
        self.connection.execute(
            "UPDATE transactions SET address = ?1, memo = ?2 WHERE id_tx = ?3", params![tx_info.address, &memo, id_tx]
        )?;
        Ok(())
    }

    pub fn add_value(&self, id_tx: u32, value: i64) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE transactions SET value = value + ?2 WHERE id_tx = ?1",
            params![id_tx, value],
        )?;
        Ok(())
    }

    pub fn get_received_note_value(&self, nf: &Nf) -> anyhow::Result<(u32, i64)> {
        let (account, value) = self.connection.query_row(
            "SELECT account, value FROM received_notes WHERE nf = ?1",
            params![nf.0.to_vec()],
            |row| {
                let account: u32 = row.get(0)?;
                let value: i64 = row.get(1)?;
                Ok((account, value))
            },
        )?;
        Ok((account, value))
    }

    pub fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        let balance: Option<i64> = self.connection.query_row(
            "SELECT SUM(value) FROM received_notes WHERE (spent IS NULL OR spent = 0) AND account = ?1",
            params![account],
            |row| row.get(0),
        )?;
        Ok(balance.unwrap_or(0) as u64)
    }

    pub fn get_spendable_balance(&self, account: u32, anchor_height: u32) -> anyhow::Result<u64> {
        let balance: Option<i64> = self.connection.query_row(
            "SELECT SUM(value) FROM received_notes WHERE spent IS NULL AND height <= ?1 AND account = ?2",
            params![anchor_height, account],
            |row| row.get(0),
        )?;
        Ok(balance.unwrap_or(0) as u64)
    }

    pub fn get_last_sync_height(&self) -> anyhow::Result<Option<u32>> {
        let height: Option<u32> =
            self.connection
                .query_row("SELECT MAX(height) FROM blocks", NO_PARAMS, |row| {
                    row.get(0)
                })?;
        Ok(height)
    }

    pub fn get_db_height(&self) -> anyhow::Result<u32> {
        let height: u32 = self.get_last_sync_height()?.unwrap_or_else(|| {
            crate::NETWORK
                .activation_height(NetworkUpgrade::Sapling)
                .unwrap()
                .into()
        });
        Ok(height)
    }

    pub fn get_db_hash(&self, height: u32) -> anyhow::Result<Option<[u8; 32]>> {
        let hash: Option<Vec<u8>> = self.connection.query_row("SELECT hash FROM blocks WHERE height = ?1", params![height], |row| row.get(0)).optional()?;
        Ok(hash.map(|h| {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&h);
            hash
        }))
    }

    pub fn get_tree(&self) -> anyhow::Result<(CTree, Vec<Witness>)> {
        let res = self.connection.query_row(
            "SELECT height, sapling_tree FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
            NO_PARAMS, |row| {
                let height: u32 = row.get(0)?;
                let tree: Vec<u8> = row.get(1)?;
                Ok((height, tree))
            }).optional()?;
        Ok(match res {
            Some((height, tree)) => {
                let tree = CTree::read(&*tree)?;
                let mut statement = self.connection.prepare(
                    "SELECT id_note, witness FROM sapling_witnesses w, received_notes n WHERE w.height = ?1 AND w.note = n.id_note AND (n.spent IS NULL OR n.spent = 0)")?;
                let ws = statement.query_map(params![height], |row| {
                    let id_note: u32 = row.get(0)?;
                    let witness: Vec<u8> = row.get(1)?;
                    Ok(Witness::read(id_note, &*witness).unwrap())
                })?;
                let mut witnesses: Vec<Witness> = vec![];
                for w in ws {
                    witnesses.push(w?);
                }
                (tree, witnesses)
            }
            None => (CTree::new(), vec![]),
        })
    }

    pub fn get_nullifiers(&self) -> anyhow::Result<HashMap<Nf, NfRef>> {
        let mut statement = self
            .connection
            .prepare("SELECT id_note, account, nf FROM received_notes WHERE spent IS NULL OR spent = 0")?;
        let nfs_res = statement.query_map(NO_PARAMS, |row| {
            let id_note: u32 = row.get(0)?;
            let account: u32 = row.get(1)?;
            let nf_vec: Vec<u8> = row.get(2)?;
            let mut nf = [0u8; 32];
            nf.clone_from_slice(&nf_vec);
            let nf_ref = NfRef {
                id_note,
                account,
            };
            Ok((nf_ref, nf))
        })?;
        let mut nfs: HashMap<Nf, NfRef> = HashMap::new();
        for n in nfs_res {
            let n = n?;
            nfs.insert(Nf(n.1), n.0);
        }

        Ok(nfs)
    }

    pub fn get_nullifier_amounts(&self, account: u32, unspent_only: bool) -> anyhow::Result<HashMap<Vec<u8>, u64>> {
        let mut sql = "SELECT value, nf FROM received_notes WHERE account = ?1".to_string();
        if unspent_only {
            sql += "AND (spent IS NULL OR spent = 0)";
        }
        let mut statement = self
            .connection
            .prepare(&sql)?;
        let nfs_res = statement.query_map(params![account], |row| {
            let amount: i64 = row.get(0)?;
            let nf: Vec<u8> = row.get(1)?;
            Ok((amount, nf))
        })?;
        let mut nfs: HashMap<Vec<u8>, u64> = HashMap::new();
        for n in nfs_res {
            let n = n?;
            nfs.insert(n.1, n.0 as u64);
        }

        Ok(nfs)
    }

    pub fn get_spendable_notes(
        &self,
        account: u32,
        anchor_height: u32,
        fvk: &ExtendedFullViewingKey,
    ) -> anyhow::Result<Vec<SpendableNote>> {
        let mut statement = self.connection.prepare(
            "SELECT id_note, diversifier, value, rcm, witness FROM received_notes r, sapling_witnesses w WHERE spent IS NULL AND account = ?2
            AND w.height = (
	            SELECT MAX(height) FROM sapling_witnesses WHERE height <= ?1
            ) AND r.id_note = w.note")?;
        let notes = statement.query_map(params![anchor_height, account], |row| {
            let id_note: u32 = row.get(0)?;

            let diversifier: Vec<u8> = row.get(1)?;
            let value: i64 = row.get(2)?;
            let rcm: Vec<u8> = row.get(3)?;
            let witness: Vec<u8> = row.get(4)?;

            let mut diversifer_bytes = [0u8; 11];
            diversifer_bytes.copy_from_slice(&diversifier);
            let diversifier = Diversifier(diversifer_bytes);
            let mut rcm_bytes = [0u8; 32];
            rcm_bytes.copy_from_slice(&rcm);
            let rcm = jubjub::Fr::from_bytes(&rcm_bytes).unwrap();
            let rseed = Rseed::BeforeZip212(rcm);
            let witness = IncrementalWitness::<Node>::read(&*witness).unwrap();

            let pa = fvk.fvk.vk.to_payment_address(diversifier).unwrap();
            let note = pa.create_note(value as u64, rseed).unwrap();
            Ok(SpendableNote {
                id: id_note,
                note,
                diversifier,
                witness,
            })
        })?;
        let mut spendable_notes: Vec<SpendableNote> = vec![];
        for n in notes {
            spendable_notes.push(n?);
        }

        Ok(spendable_notes)
    }

    pub fn mark_spent(&self, id: u32, height: u32) -> anyhow::Result<()> {
        log::debug!("+mark_spent");
        self.connection.execute(
            "UPDATE received_notes SET spent = ?1 WHERE id_note = ?2",
            params![height, id],
        )?;
        log::debug!("-mark_spent");
        Ok(())
    }

    pub fn get_backup(&self, account: u32) -> anyhow::Result<(Option<String>, Option<String>, String)> {
        log::debug!("+get_backup");
        let (seed, sk, ivk) = self.connection.query_row(
            "SELECT seed, sk, ivk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let seed: Option<String> = row.get(0)?;
                let sk: Option<String> = row.get(0)?;
                let ivk: String = row.get(0)?;
                Ok((seed, sk, ivk))
            },
        )?;
        log::debug!("-get_backup");
        Ok((seed, sk, ivk))
    }

    pub fn get_seed(&self, account: u32) -> anyhow::Result<Option<String>> {
        log::info!("+get_seed");
        let seed = self.connection.query_row(
            "SELECT seed FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let sk: String = row.get(0)?;
                Ok(sk)
            },
        ).optional()?;
        log::info!("-get_seed");
        Ok(seed)
    }

    pub fn get_sk(&self, account: u32) -> anyhow::Result<String> {
        log::info!("+get_sk");
        let sk = self.connection.query_row(
            "SELECT sk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let sk: String = row.get(0)?;
                Ok(sk)
            },
        )?;
        log::info!("-get_sk");
        Ok(sk)
    }

    pub fn get_ivk(&self, account: u32) -> anyhow::Result<String> {
        log::debug!("+get_ivk");
        let ivk = self.connection.query_row(
            "SELECT ivk FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let ivk: String = row.get(0)?;
                Ok(ivk)
            },
        )?;
        log::debug!("-get_ivk");
        Ok(ivk)
    }

    pub fn get_address(&self, account: u32) -> anyhow::Result<String> {
        log::debug!("+get_address");
        let address = self.connection.query_row(
            "SELECT address FROM accounts WHERE id_account = ?1",
            params![account],
            |row| {
                let address: String = row.get(0)?;
                Ok(address)
            },
        )?;
        log::debug!("-get_address");
        Ok(address)
    }

    pub fn get_diversifier(&self, account: u32) -> anyhow::Result<DiversifierIndex> {
        let diversifier_index = self.connection.query_row(
            "SELECT diversifier_index FROM diversifiers WHERE account = ?1",
            params![account],
            |row| {
                let d: Vec<u8> = row.get(0)?;
                let mut div = [0u8; 11];
                div.copy_from_slice(&d);
                Ok(div)
            },
        ).optional()?.unwrap_or_else(|| [0u8; 11]);
        Ok(DiversifierIndex(diversifier_index))
    }

    pub fn store_diversifier(&self, account: u32, diversifier_index: &DiversifierIndex) -> anyhow::Result<()> {
        let diversifier_bytes = diversifier_index.0.to_vec();
        self.connection.execute(
            "INSERT INTO diversifiers(account, diversifier_index) VALUES (?1, ?2) ON CONFLICT \
            (account) DO UPDATE SET diversifier_index = excluded.diversifier_index",
            params![account, diversifier_bytes])?;
        Ok(())
    }

    pub fn get_taddr(&self, account: u32) -> anyhow::Result<Option<String>> {
        let address = self.connection.query_row(
            "SELECT address FROM taddrs WHERE account = ?1",
            params![account],
            |row| {
                let address: String = row.get(0)?;
                Ok(address)
            }).optional()?;
        Ok(address)
    }

    pub fn get_tsk(&self, account: u32) -> anyhow::Result<String> {
        let sk = self.connection.query_row(
            "SELECT sk FROM taddrs WHERE account = ?1",
            params![account],
            |row| {
                let address: String = row.get(0)?;
                Ok(address)
            })?;
        Ok(sk)
    }

    pub fn create_taddr(&self, account: u32) -> anyhow::Result<()> {
        let seed = self.get_seed(account)?;
        if let Some(seed) = seed {
            let (sk, address) = derive_tkeys(&seed, BIP44_PATH)?;
            self.connection.execute("INSERT INTO taddrs(account, sk, address) VALUES (?1, ?2, ?3) \
            ON CONFLICT DO NOTHING", params![account, &sk, &address])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{DbAdapter, ReceivedNote, DEFAULT_DB_PATH};
    use crate::{CTree, Witness};

    #[test]
    fn test_db() {
        let mut db = DbAdapter::new(DEFAULT_DB_PATH).unwrap();
        db.init_db().unwrap();
        db.trim_to_height(0).unwrap();

        db.store_block(1, &[0u8; 32], 0, &CTree::new()).unwrap();
        let id_tx = db.store_transaction(&[0; 32], 1, 1, 0, 20).unwrap();
        db.store_received_note(
            &ReceivedNote {
                account: 1,
                height: 1,
                output_index: 0,
                diversifier: vec![],
                value: 0,
                rcm: vec![],
                nf: vec![],
                spent: None,
            },
            id_tx,
            5,
        )
            .unwrap();
        let witness = Witness {
            position: 10,
            id_note: 0,
            note: None,
            tree: CTree::new(),
            filled: vec![],
            cursor: CTree::new(),
        };
        db.store_witnesses(&witness, 1000, 1).unwrap();
    }

    #[test]
    fn test_balance() {
        let db = DbAdapter::new(DEFAULT_DB_PATH).unwrap();
        let balance = db.get_balance(1).unwrap();
        println!("{}", balance);
    }
}
