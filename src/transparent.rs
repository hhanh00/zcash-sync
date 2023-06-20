mod db;

use crate::db::data_generated::fb::{PlainNoteT, PlainNoteVecT, PlainTxT, PlainTxVecT};
use crate::CoinConfig;
use anyhow::Result;
pub use db::migrate_db;
use rusqlite::Connection;

pub async fn sync(coin: u8, account: u32) -> Result<()> {
    let c = CoinConfig::get(coin);
    let url = c.lwd_url.as_ref().unwrap();
    let db = c.db()?;
    let connection = &db.connection;
    let network = c.chain.network();
    let address: String = connection.query_row(
        "SELECT address FROM taddrs WHERE account = ?1",
        [account],
        |r| r.get(0),
    )?;
    db::fetch_txs(network, connection, url, account, &address)?;
    db::update_timestamps(connection, url).await?;
    Ok(())
}

pub fn get_txs(connection: &Connection, account: u32) -> Result<PlainTxVecT> {
    let mut s = connection.prepare(
        "SELECT id_tx, t.hash, t.height, timestamp, value, address FROM t_txs t JOIN block_timestamps b \
        ON t.height = b.height WHERE account = ?1 ORDER BY t.height DESC")?;
    let rows = s.query_map([account], |r| {
        let mut tx_hash = r.get::<_, Vec<u8>>(1)?;
        tx_hash.reverse();
        Ok(PlainTxT {
            id: r.get(0)?,
            tx_id: Some(hex::encode(&tx_hash)),
            height: r.get(2)?,
            timestamp: r.get(3)?,
            value: r.get(4)?,
            address: Some(r.get(5)?),
        })
    })?;
    let txs: Result<Vec<_>, _> = rows.collect();
    Ok(PlainTxVecT { txs: Some(txs?) })
}

pub fn get_utxos(connection: &Connection, account: u32) -> Result<PlainNoteVecT> {
    let mut s = connection.prepare(
        "SELECT id_utxo, tx_hash, t.height, timestamp, vout, value FROM t_utxos t JOIN block_timestamps b \
        ON t.height = b.height WHERE account = ?1 AND spent IS NULL ORDER BY t.height DESC")?;
    let rows = s.query_map([account], |r| {
        let mut tx_hash = r.get::<_, Vec<u8>>(1)?;
        tx_hash.reverse();
        Ok(PlainNoteT {
            id: r.get(0)?,
            tx_id: Some(hex::encode(&tx_hash)),
            height: r.get(2)?,
            timestamp: r.get(3)?,
            vout: r.get(4)?,
            value: r.get(5)?,
        })
    })?;
    let notes: Result<Vec<_>, _> = rows.collect();
    Ok(PlainNoteVecT {
        notes: Some(notes?),
    })
}
