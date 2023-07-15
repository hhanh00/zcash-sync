use crate::db::data_generated::fb;
use crate::taddr::derive_tkeys;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use zcash_primitives::consensus::{Network, Parameters};

pub fn get_transparent(
    connection: &Connection,
    account: u32,
) -> Result<Option<fb::TransparentDetailsT>> {
    let details = connection
        .query_row(
            "SELECT sk, address FROM taddrs WHERE account = ?1",
            [account],
            |r| {
                Ok(fb::TransparentDetailsT {
                    id: account,
                    sk: r.get(0)?,
                    address: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(details)
}

pub fn create_taddr(network: &Network, connection: &Connection, account: u32) -> Result<()> {
    let account_details = super::account::get_account(connection, account)?;
    if let Some(account_details) = account_details {
        let bip44_path = format!(
            "m/44'/{}'/0'/0/{}",
            network.coin_type(),
            account_details.aindex
        );
        let (sk, address) =
            derive_tkeys(network, account_details.seed.as_ref().unwrap(), &bip44_path)?;
        connection.execute(
            "INSERT INTO taddrs(account, sk, address) VALUES (?1, ?2, ?3)",
            params![account, &sk, &address],
        )?;
    }
    Ok(())
}

pub fn store_taddr(connection: &Connection, account: u32, address: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO taddrs(account, sk, address) VALUES (?1, NULL, ?2)",
        params![account, address],
    )?;
    Ok(())
}

pub fn store_tsk(connection: &Connection, id_account: u32, sk: &str, addr: &str) -> Result<()> {
    connection.execute(
        "UPDATE taddrs SET sk = ?1, address = ?2 WHERE account = ?3",
        params![sk, addr, id_account],
    )?;
    Ok(())
}

pub fn list_txs(connection: &Connection, account: u32) -> Result<fb::PlainTxVecT> {
    let mut s = connection.prepare(
        "SELECT id_tx, t.hash, t.height, timestamp, value, address FROM t_txs t JOIN block_timestamps b \
        ON t.height = b.height WHERE account = ?1 ORDER BY t.height DESC")?;
    let rows = s.query_map([account], |r| {
        let mut tx_hash = r.get::<_, Vec<u8>>(1)?;
        tx_hash.reverse();
        Ok(fb::PlainTxT {
            id: r.get(0)?,
            tx_id: Some(hex::encode(&tx_hash)),
            height: r.get(2)?,
            timestamp: r.get(3)?,
            value: r.get(4)?,
            address: Some(r.get(5)?),
        })
    })?;
    let txs: Result<Vec<_>, _> = rows.collect();
    Ok(fb::PlainTxVecT { txs: Some(txs?) })
}

pub fn list_utxos(connection: &Connection, account: u32) -> Result<fb::PlainNoteVecT> {
    let mut s = connection.prepare(
        "SELECT id_utxo, tx_hash, t.height, timestamp, vout, value FROM t_utxos t JOIN block_timestamps b \
        ON t.height = b.height WHERE account = ?1 AND spent IS NULL ORDER BY t.height DESC")?;
    let rows = s.query_map([account], |r| {
        let mut tx_hash = r.get::<_, Vec<u8>>(1)?;
        tx_hash.reverse();
        Ok(fb::PlainNoteT {
            id: r.get(0)?,
            tx_id: Some(hex::encode(&tx_hash)),
            height: r.get(2)?,
            timestamp: r.get(3)?,
            vout: r.get(4)?,
            value: r.get(5)?,
        })
    })?;
    let notes: Result<Vec<_>, _> = rows.collect();
    Ok(fb::PlainNoteVecT {
        notes: Some(notes?),
    })
}
