use crate::chain::Nf;
use crate::db::data_generated::fb::{CheckpointT, CheckpointVecT};
use crate::db::{wrap_query_no_rows, ReceivedNote, ReceivedNoteShort};
use crate::note_selection::UTXO;
use crate::sync::tree::TreeCheckpoint;
use crate::sync::{CTree, Witness};
use crate::transaction::TransactionDetails;
use crate::{Hash, Source};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row, Statement, Transaction};
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};

pub fn get_last_sync_height(
    connection: &Connection,
    network: &Network,
    max_height: Option<u32>,
) -> Result<u32> {
    let max_height = max_height.unwrap_or(u32::MAX);
    let height = connection.query_row(
        "SELECT MAX(height) FROM blocks WHERE height <= ?1",
        [max_height],
        |row| row.get::<_, Option<u32>>(0),
    )?;
    Ok(height.unwrap_or_else(|| {
        network
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into()
    }))
}

pub fn trim_to_height(connection: &mut Connection, height: u32) -> Result<u32> {
    // snap height to an existing checkpoint
    let height = connection.query_row(
        "SELECT MAX(height) from blocks WHERE height <= ?1",
        [height],
        |r| r.get::<_, Option<u32>>(0),
    )?;
    let height = height.unwrap_or(0);
    log::info!("Rewind to height: {}", height);

    let db_tx = connection.transaction()?;
    db_tx.execute("DELETE FROM blocks WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM sapling_tree WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM orchard_tree WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM sapling_witnesses WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM orchard_witnesses WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM received_notes WHERE height > ?1", [height])?;
    db_tx.execute(
        "UPDATE received_notes SET spent = NULL WHERE spent > ?1",
        [height],
    )?;
    db_tx.execute("DELETE FROM transactions WHERE height > ?1", [height])?;
    db_tx.execute("DELETE FROM messages WHERE height > ?1", [height])?;
    db_tx.commit()?;

    Ok(height)
}

pub fn store_block(
    height: u32,
    hash: &[u8],
    timestamp: u32,
    sapling_tree: &CTree,
    orchard_tree: &CTree,
    db_tx: &Transaction,
) -> Result<()> {
    let mut sapling_bb: Vec<u8> = vec![];
    sapling_tree.write(&mut sapling_bb)?;
    db_tx.execute(
        "INSERT INTO blocks(height, hash, timestamp)
        VALUES (?1, ?2, ?3)",
        params![height, hash, timestamp],
    )?;
    db_tx.execute(
        "INSERT INTO sapling_tree(height, tree) VALUES (?1, ?2)",
        params![height, &sapling_bb],
    )?;
    let mut orchard_bb: Vec<u8> = vec![];
    orchard_tree.write(&mut orchard_bb)?;
    db_tx.execute(
        "INSERT INTO orchard_tree(height, tree) VALUES (?1, ?2)",
        params![height, &orchard_bb],
    )?;
    Ok(())
}

pub fn store_transaction(
    txid: &[u8],
    account: u32,
    height: u32,
    timestamp: u32,
    tx_index: u32,
    db_tx: &Transaction,
) -> Result<u32> {
    db_tx.execute(
        "INSERT INTO transactions(account, txid, height, timestamp, tx_index, value)
        VALUES (?1, ?2, ?3, ?4, ?5, 0) ON CONFLICT DO NOTHING", // ignore conflict when same tx has sapling + orchard outputs
        params![account, txid, height, timestamp, tx_index],
    )?;
    let id_tx = db_tx
        .query_row(
            "SELECT id_tx FROM transactions WHERE account = ?1 AND txid = ?2",
            params![account, txid],
            |row| row.get::<_, u32>(0),
        )
        .map_err(wrap_query_no_rows("store_transaction/id_tx"))?;
    Ok(id_tx)
}

pub fn store_received_note(
    note: &ReceivedNote,
    id_tx: u32,
    position: usize,
    db_tx: &Transaction,
) -> Result<u32> {
    let orchard = note.rho.is_some();
    db_tx.execute("INSERT INTO received_notes(account, tx, height, position, output_index, diversifier, value, rcm, rho, nf, orchard, spent) \
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                  params![note.account, id_tx, note.height, position as u32, note.output_index,
            note.diversifier, note.value as i64, note.rcm, note.rho, note.nf, orchard, note.spent])?;
    let id_note = db_tx
        .query_row(
            "SELECT id_note FROM received_notes WHERE tx = ?1 AND output_index = ?2 AND orchard = ?3",
            params![id_tx, note.output_index, orchard],
            |row| row.get::<_, u32>(0),
        )
        .map_err(wrap_query_no_rows("store_received_note/id_note"))?;
    Ok(id_note)
}

pub fn store_witness<const POOL: char>(
    witness: &Witness,
    height: u32,
    id_note: u32,
    db_tx: &Transaction,
) -> Result<()> {
    let shielded_pool = if POOL == 'S' { "sapling" } else { "orchard" };
    let mut bb: Vec<u8> = vec![];
    witness.write(&mut bb)?;
    db_tx.execute(
        &format!(
            "INSERT INTO {shielded_pool}_witnesses(note, height, witness) VALUES (?1, ?2, ?3)"
        ),
        params![id_note, height, bb],
    )?;
    Ok(())
}

pub fn store_tree<const POOL: char>(height: u32, tree: &CTree, db_tx: &Transaction) -> Result<()> {
    let shielded_pool = if POOL == 'S' { "sapling" } else { "orchard" };
    let mut bb: Vec<u8> = vec![];
    tree.write(&mut bb)?;
    db_tx.execute(
        &format!("INSERT INTO {shielded_pool}_tree(height, tree) VALUES (?1,?2)"),
        params![height, &bb],
    )?;
    Ok(())
}

// TODO: Used?
pub fn store_block_timestamp(
    height: u32,
    hash: &[u8],
    timestamp: u32,
    db_tx: &Transaction,
) -> Result<()> {
    db_tx.execute(
        "INSERT INTO blocks(height, hash, timestamp) VALUES (?1,?2,?3)",
        params![height, hash, timestamp],
    )?;
    Ok(())
}

pub struct BlockHash {
    pub hash: [u8; 32],
    pub timestamp: u32,
}

/// Blocks
///
pub fn get_block(connection: &Connection, height: u32) -> Result<Option<BlockHash>> {
    let block = connection
        .query_row(
            "SELECT hash, timestamp FROM blocks WHERE height = ?1",
            [height],
            |r| {
                Ok(BlockHash {
                    hash: r.get::<_, Vec<u8>>(0)?.try_into().unwrap(),
                    timestamp: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(block)
}

/// Witnesses
///
pub fn get_tree<const POOL: char>(connection: &Connection, height: u32) -> Result<TreeCheckpoint> {
    let shielded_pool = if POOL == 'S' { "sapling" } else { "orchard" };
    let tree = connection.query_row(
        &format!("SELECT tree FROM {shielded_pool}_tree WHERE height = ?1"),
        [height],
        |row| row.get::<_, Vec<u8>>(0),
    )?;

    let tree = CTree::read(&*tree)?;
    let mut statement = connection.prepare(
        &format!("SELECT id_note, witness FROM {shielded_pool}_witnesses w, received_notes n WHERE w.height = ?1 AND w.note = n.id_note AND (n.spent IS NULL OR n.spent = 0)"))?;
    let witnesses = statement.query_map(params![height], |row| {
        let id_note: u32 = row.get::<_, u32>(0)?;
        let witness: Vec<u8> = row.get::<_, Vec<u8>>(1)?;
        let w = Witness::read(id_note, &*witness).unwrap();
        Ok(w)
    })?;
    let witnesses: Result<Vec<_>, _> = witnesses.collect();
    Ok(TreeCheckpoint {
        tree,
        witnesses: witnesses?,
    })
}

/// Transactions
///
pub fn update_transaction_with_memo(
    connection: &Connection,
    details: &TransactionDetails,
) -> Result<()> {
    connection.execute(
        "UPDATE transactions SET address = ?1, memo = ?2 WHERE id_tx = ?3",
        params![details.address, details.memo, details.id_tx],
    )?;
    Ok(())
}

pub fn add_value(id_tx: u32, value: i64, db_tx: &Transaction) -> Result<()> {
    db_tx.execute(
        "UPDATE transactions SET value = value + ?2 WHERE id_tx = ?1",
        params![id_tx, value],
    )?;
    Ok(())
}

pub fn mark_spent(id: u32, height: u32, db_tx: &Transaction) -> Result<()> {
    db_tx.execute(
        "UPDATE received_notes SET spent = ?1 WHERE id_note = ?2",
        [height, id],
    )?;
    Ok(())
}

/// Trial Decryption
///
pub fn list_nullifier_amounts(
    connection: &Connection,
    account: u32,
    unspent_only: bool,
) -> Result<Vec<(Hash, u64)>> {
    let mut sql = "SELECT value, nf FROM received_notes WHERE account = ?1".to_owned();
    if unspent_only {
        sql += " AND (spent IS NULL OR spent = 0)";
    }
    let mut statement = connection.prepare(&sql)?;
    let nfs = statement.query_map([account], |r| {
        let amount = r.get::<_, u64>(0)?;
        let nf: Hash = r.get::<_, Vec<u8>>(1)?.try_into().unwrap();
        Ok((nf, amount))
    })?;
    let nfs: Result<Vec<_>, _> = nfs.collect();
    Ok(nfs?)
}

pub fn list_unspent_nullifiers(connection: &Connection) -> Result<Vec<ReceivedNoteShort>> {
    let sql =
        "SELECT id_note, account, nf, value FROM received_notes WHERE spent IS NULL OR spent = 0";
    let mut statement = connection.prepare(sql)?;
    let nfs = statement.query_map([], |r| {
        Ok(ReceivedNoteShort {
            id: r.get(0)?,
            account: r.get(1)?,
            nf: Nf(r.get::<_, Vec<u8>>(2)?.try_into().unwrap()),
            value: r.get(3)?,
        })
    })?;
    let nfs: Result<Vec<_>, _> = nfs.collect();
    Ok(nfs?)
}

pub fn get_unspent_received_notes<const POOL: char>(
    connection: &Connection,
    account: u32,
    checkpoint_height: u32,
) -> Result<Vec<UTXO>> {
    let (mut statement, map_row): (Statement, fn(&Row) -> Result<UTXO, rusqlite::Error>) =
        match POOL {
            'S' => {
                let s = connection.prepare(
                "SELECT id_note, diversifier, value, rcm, witness FROM received_notes r, sapling_witnesses w WHERE spent IS NULL AND account = ?2 AND rho IS NULL
            AND (r.excluded IS NULL OR NOT r.excluded) AND w.height = ?1
            AND r.id_note = w.note")?;
                let r = |r: &Row| {
                    let id_note = r.get::<_, u32>(0)?;
                    let source = Source::Sapling {
                        id_note,
                        diversifier: r.get::<_, Vec<u8>>(1)?.try_into().unwrap(),
                        rseed: r.get::<_, Vec<u8>>(3)?.try_into().unwrap(),
                        witness: r.get::<_, Vec<u8>>(4)?.try_into().unwrap(),
                    };
                    Ok(UTXO {
                        id: id_note,
                        source,
                        amount: r.get(2)?,
                    })
                };
                (s, r)
            }
            'O' => {
                let s = connection.prepare(
                "SELECT id_note, diversifier, value, rcm, rho, witness FROM received_notes r, orchard_witnesses w WHERE spent IS NULL AND account = ?2 AND rho IS NOT NULL
            AND (r.excluded IS NULL OR NOT r.excluded) AND w.height = ?1
            AND r.id_note = w.note")?;
                let r = |r: &Row| {
                    let id_note = r.get::<_, u32>(0)?;
                    let source = Source::Orchard {
                        id_note,
                        diversifier: r.get::<_, Vec<u8>>(1)?.try_into().unwrap(),
                        rseed: r.get::<_, Vec<u8>>(3)?.try_into().unwrap(),
                        rho: r.get::<_, Vec<u8>>(4)?.try_into().unwrap(),
                        witness: r.get::<_, Vec<u8>>(5)?.try_into().unwrap(),
                    };
                    Ok(UTXO {
                        id: id_note,
                        source,
                        amount: r.get(2)?,
                    })
                };
                (s, r)
            }

            _ => unreachable!(),
        };
    let notes = statement.query_map([checkpoint_height, account], map_row)?;
    let notes: Result<Vec<_>, _> = notes.collect();
    Ok(notes?)
}

pub fn list_checkpoints(connection: &Connection) -> Result<CheckpointVecT> {
    let mut stmt = connection.prepare("SELECT height, timestamp FROM blocks ORDER by height")?;
    let checkpoints = stmt.query_map([], |row| {
        let height: u32 = row.get(0)?;
        let timestamp: u32 = row.get(1)?;

        let checkpoint = CheckpointT { height, timestamp };
        Ok(checkpoint)
    })?;
    let checkpoints: Result<Vec<_>, _> = checkpoints.collect();
    let checkpoints = CheckpointVecT {
        checkpoints: checkpoints.ok(),
    };
    Ok(checkpoints)
}

/// Cleanup
///
pub fn purge_old_witnesses(connection: &mut Connection, height: u32) -> Result<()> {
    const BLOCKS_PER_HOUR: u32 = 60 * 60 / 75;
    const BLOCKS_PER_DAY: u32 = 24 * BLOCKS_PER_HOUR;
    const BLOCKS_PER_MONTH: u32 = 30 * BLOCKS_PER_DAY;
    let db_tx = connection.transaction()?;
    // Keep the last hour
    for i in 2..=24 {
        // 1 checkpoint per hour
        prune_interval(
            height - i * BLOCKS_PER_HOUR,
            height - (i - 1) * BLOCKS_PER_HOUR,
            &db_tx,
        )?;
    }
    for i in 2..=30 {
        // 1 checkpoint per day
        prune_interval(
            height - i * BLOCKS_PER_DAY,
            height - (i - 1) * BLOCKS_PER_DAY,
            &db_tx,
        )?;
    }
    for i in 2..=12 {
        // 1 checkpoint per 30 days
        prune_interval(
            height - i * BLOCKS_PER_MONTH,
            height - (i - 1) * BLOCKS_PER_MONTH,
            &db_tx,
        )?;
    }
    db_tx.commit()?;
    Ok(())
}

// Only keep the oldest checkpoint in [low, high)
fn prune_interval(low: u32, high: u32, db_tx: &Transaction) -> Result<()> {
    let keep_height = db_tx.query_row(
        "SELECT MIN(height) FROM blocks WHERE height >= ?1 AND height < ?2",
        params![low, high],
        |row| row.get::<_, Option<u32>>(0),
    )?;
    if let Some(keep_height) = keep_height {
        db_tx.execute(
            "DELETE FROM sapling_witnesses WHERE height >= ?1 AND height < ?2 AND height != ?3",
            params![low, high, keep_height],
        )?;
        db_tx.execute(
            "DELETE FROM orchard_witnesses WHERE height >= ?1 AND height < ?2 AND height != ?3",
            params![low, high, keep_height],
        )?;
        db_tx.execute(
            "DELETE FROM blocks WHERE height >= ?1 AND height < ?2 AND height != ?3",
            params![low, high, keep_height],
        )?;
        db_tx.execute(
            "DELETE FROM sapling_tree WHERE height >= ?1 AND height < ?2 AND height != ?3",
            params![low, high, keep_height],
        )?;
        db_tx.execute(
            "DELETE FROM orchard_tree WHERE height >= ?1 AND height < ?2 AND height != ?3",
            params![low, high, keep_height],
        )?;
    }
    Ok(())
}
