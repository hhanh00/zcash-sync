use rusqlite::{Connection, params, OptionalExtension};
use crate::{Witness, CTree};

pub struct DbAdapter {
    connection: Connection,
}

pub struct ReceivedNote {
    pub height: u32,
    pub output_index: u32,
    pub diversifier: Vec<u8>,
    pub value: u64,
    pub rcm: Vec<u8>,
    pub nf: Vec<u8>,
    pub is_change: bool,
    pub memo: Vec<u8>,
    pub spent: bool,
}

impl DbAdapter {
    pub fn new(db_path: &str) -> anyhow::Result<DbAdapter> {
        let connection = Connection::open(db_path)?;
        Ok(DbAdapter {
            connection,
        })
    }

    pub fn init_db(&self) -> anyhow::Result<()> {
        self.connection.execute("CREATE TABLE IF NOT EXISTS blocks (
            height INTEGER PRIMARY KEY,
            hash BLOB NOT NULL,
            sapling_tree BLOB NOT NULL)", [])?;

        self.connection.execute("CREATE TABLE IF NOT EXISTS transactions (
            id_tx INTEGER PRIMARY KEY,
            txid BLOB NOT NULL UNIQUE,
            height INTEGER,
            tx_index INTEGER)", [])?;

        self.connection.execute("CREATE TABLE IF NOT EXISTS received_notes (
            id_note INTEGER PRIMARY KEY,
            position INTEGER NOT NULL,
            tx INTEGER NOT NULL,
            height INTEGER NOT NULL,
            output_index INTEGER NOT NULL,
            diversifier BLOB NOT NULL,
            value INTEGER NOT NULL,
            rcm BLOB NOT NULL,
            nf BLOB NOT NULL UNIQUE,
            is_change INTEGER NOT NULL,
            memo BLOB,
            spent INTEGER,
            FOREIGN KEY (tx) REFERENCES transactions(id_tx),
            FOREIGN KEY (spent) REFERENCES transactions(id_tx),
            CONSTRAINT tx_output UNIQUE (tx, output_index))", [])?;

        self.connection.execute("CREATE TABLE IF NOT EXISTS sapling_witnesses (
            id_witness INTEGER PRIMARY KEY,
            note INTEGER NOT NULL,
            height INTEGER NOT NULL,
            witness BLOB NOT NULL,
            FOREIGN KEY (note) REFERENCES received_notes(id_note),
            CONSTRAINT witness_height UNIQUE (note, height))", [])?;

        Ok(())
    }

    pub fn trim_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM blocks WHERE height >= ?1", params![height])?;
        tx.execute("DELETE FROM sapling_witnesses WHERE height >= ?1", params![height])?;
        tx.execute("DELETE FROM received_notes WHERE height >= ?1", params![height])?;
        tx.execute("DELETE FROM transactions WHERE height >= ?1", params![height])?;
        tx.commit()?;

        Ok(())
    }

    pub fn store_block(&self, height: u32, hash: &[u8], tree: &CTree) -> anyhow::Result<()> {
        let mut bb: Vec<u8> = vec![];
        tree.write(&mut bb)?;
        self.connection.execute("INSERT INTO blocks(height, hash, sapling_tree)
        VALUES (?1, ?2, ?3)
        ON CONFLICT DO NOTHING", params![height, hash, &bb])?;
        Ok(())
    }

    pub fn store_transaction(&self, txid: &[u8], height: u32, tx_index: u32) -> anyhow::Result<u32> {
        self.connection.execute("INSERT INTO transactions(txid, height, tx_index)
        VALUES (?1, ?2, ?3)
        ON CONFLICT DO NOTHING", params![txid, height, tx_index])?;
        let id_tx: u32 = self.connection.query_row("SELECT id_tx FROM transactions WHERE txid = ?1", params![txid], |row| row.get(0))?;
        Ok(id_tx)
    }

    pub fn store_received_note(&self, note: &ReceivedNote, id_tx: u32, position: usize) -> anyhow::Result<u32> {
        self.connection.execute("INSERT INTO received_notes(tx, height, position, output_index, diversifier, value, rcm, nf, is_change, memo, spent)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT DO NOTHING", params![id_tx, note.height, position, note.output_index, note.diversifier, note.value, note.rcm, note.nf, note.is_change, note.memo, note.spent])?;
        let id_note: u32 = self.connection.query_row("SELECT id_note FROM received_notes WHERE tx = ?1 AND output_index = ?2", params![id_tx, note.output_index], |row| row.get(0))?;
        Ok(id_note)
    }

    pub fn store_witnesses(&self, witness: &Witness, height: u32, id_note: u32) -> anyhow::Result<()> {
        let mut bb: Vec<u8> = vec![];
        witness.write(&mut bb)?;
        println!("{} {}", height, id_note);
        self.connection.execute("INSERT INTO sapling_witnesses(note, height, witness) VALUES (?1, ?2, ?3)
        ON CONFLICT DO NOTHING", params![id_note, height, bb])?;
        Ok(())
    }

    pub fn get_balance(&self) -> anyhow::Result<u64> {
        let balance: u64 = self.connection.query_row("SELECT SUM(value) FROM received_notes WHERE spent = 0", [], |row| row.get(0))?;
        Ok(balance)
    }

    pub fn get_last_height(&self) -> anyhow::Result<Option<u32>> {
        let height: Option<u32> = self.connection.query_row("SELECT MAX(height) FROM blocks", [], |row| row.get(0)).optional()?;
        Ok(height)
    }

    pub fn get_tree(&self) -> anyhow::Result<(CTree, Vec<Witness>)> {
        let res = self.connection.query_row(
            "SELECT height, sapling_tree FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
            [], |row| {
                let height: u32 = row.get(0)?;
                let tree: Vec<u8> = row.get(1)?;
                Ok((height, tree))
            }).optional()?;
        Ok(match res {
            Some((height, tree)) => {
                let tree = CTree::read(&*tree)?;
                let mut statement = self.connection.prepare(
                "SELECT id_note, position, witness FROM sapling_witnesses w, received_notes n WHERE w.height = ?1 AND w.note = n.id_note")?;
                let ws = statement.query_map(params![height], |row| {
                    let id_note: u32 = row.get(0)?;
                    let position: u32 = row.get(1)?;
                    let witness: Vec<u8> = row.get(2)?;
                    Ok(Witness::read(position as usize, id_note, &*witness).unwrap())
                })?;
                let mut witnesses: Vec<Witness> = vec![];
                for w in ws {
                    witnesses.push(w?);
                }
                (tree, witnesses)
            },
            None => (CTree::new(), vec![])
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{DbAdapter, ReceivedNote};
    use crate::{Witness, CTree};

    const DB_PATH: &str = "zec.db";

    #[test]
    fn test_db() {
        let mut db = DbAdapter::new(DB_PATH).unwrap();
        db.init_db().unwrap();
        db.trim_to_height(0).unwrap();

        db.store_block(1, &[0u8; 32], &CTree::new()).unwrap();
        let id_tx = db.store_transaction(&[0; 32], 1, 20).unwrap();
        db.store_received_note(&ReceivedNote {
            height: 1,
            output_index: 0,
            diversifier: vec![],
            value: 0,
            rcm: vec![],
            nf: vec![],
            is_change: false,
            memo: vec![],
            spent: false
        }, id_tx, 5).unwrap();
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
        let db = DbAdapter::new(DB_PATH).unwrap();
        let balance = db.get_balance().unwrap();
        println!("{}", balance);
    }
}
