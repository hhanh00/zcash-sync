use crate::db::data_generated::fb::{ShieldedNoteT, ShieldedNoteVecT, ShieldedTxT, ShieldedTxVecT};
use crate::transaction::{GetTransactionDetailRequest, TransactionDetails};
use anyhow::Result;
use rusqlite::{params, Connection, Transaction};
use zcash_primitives::consensus::Network;

pub fn clear_tx_details(connection: &Connection, account: u32) -> Result<()> {
    connection.execute(
        "UPDATE transactions SET address = NULL, memo = NULL WHERE account = ?1",
        [account],
    )?;
    connection.execute("DELETE FROM messages WHERE account = ?1", [account])?;
    Ok(())
}

pub fn list_txid_without_memo(
    connection: &Connection,
    account: u32,
) -> Result<Vec<GetTransactionDetailRequest>> {
    let mut stmt = connection.prepare(
        "SELECT id_tx, txid, height, timestamp, value FROM transactions WHERE memo IS NULL AND account = ?1",
    )?;
    let reqs = stmt.query_map([account], |r| {
        Ok(GetTransactionDetailRequest {
            account,
            id_tx: r.get(0)?,
            txid: r.get::<_, Vec<u8>>(1)?.try_into().unwrap(),
            height: r.get(2)?,
            timestamp: r.get(3)?,
            value: r.get(4)?,
        })
    })?;
    let reqs: Result<Vec<_>, _> = reqs.collect();
    Ok(reqs?)
}

pub fn list_notes(connection: &Connection, id: u32) -> Result<ShieldedNoteVecT> {
    let mut stmt = connection.prepare(
        "SELECT n.id_note, n.height, n.value, t.timestamp, n.orchard, n.excluded, n.spent FROM received_notes n, transactions t \
           WHERE n.account = ?1 AND (n.spent IS NULL OR n.spent = 0) \
           AND n.tx = t.id_tx ORDER BY n.height DESC")?;
    let notes = stmt.query_map([id], |r| {
        let excluded = r.get::<_, Option<bool>>("excluded")?.unwrap_or(false);
        let spent = r.get::<_, Option<bool>>("spent")?.unwrap_or(false);
        Ok(ShieldedNoteT {
            id: r.get("id_note")?,
            height: r.get("height")?,
            value: r.get("value")?,
            timestamp: r.get("timestamp")?,
            orchard: r.get("orchard")?,
            excluded,
            spent,
        })
    })?;
    let notes: Result<Vec<_>, _> = notes.collect();
    let notes = ShieldedNoteVecT { notes: notes.ok() };
    Ok(notes)
}

pub fn list_txs(network: &Network, connection: &Connection, id: u32) -> Result<ShieldedTxVecT> {
    let addresses = super::contact::resolve_addresses(network, connection)?;
    let mut stmt = connection.prepare(
        "SELECT id_tx, txid, height, timestamp, t.address, value, memo FROM transactions t \
        WHERE account = ?1 ORDER BY height DESC",
    )?;
    let txs = stmt.query_map([id], |row| {
        let id_tx: u32 = row.get("id_tx")?;
        let height: u32 = row.get("height")?;
        let mut tx_id: Vec<u8> = row.get("txid")?;
        tx_id.reverse();
        let tx_id = hex::encode(&tx_id);
        let short_tx_id = tx_id[..8].to_string();
        let timestamp: u32 = row.get("timestamp")?;
        let address: Option<String> = row.get("address")?;
        let value: i64 = row.get("value")?;
        let memo: Option<String> = row.get("memo")?;
        let name = address.as_ref().and_then(|a| addresses.get(a)).cloned();
        let tx = ShieldedTxT {
            id: id_tx,
            height,
            tx_id: Some(tx_id),
            short_tx_id: Some(short_tx_id),
            timestamp,
            name,
            value,
            address,
            memo,
        };
        Ok(tx)
    })?;
    let txs: Result<Vec<_>, _> = txs.collect();
    let txs = ShieldedTxVecT { txs: txs.ok() };
    Ok(txs)
}

pub fn update_excluded(connection: &Connection, id: u32, excluded: bool) -> Result<()> {
    connection.execute(
        "UPDATE received_notes SET excluded = ?2 WHERE id_note = ?1",
        params![id, excluded],
    )?;
    Ok(())
}

pub fn invert_excluded(connection: &Connection, id: u32) -> Result<()> {
    connection.execute(
        "UPDATE received_notes SET excluded = NOT(COALESCE(excluded, 0)) WHERE account = ?1",
        [id],
    )?;
    Ok(())
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
