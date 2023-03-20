use anyhow::Result;
use electrum_client::bitcoin::BlockHeader;
use rusqlite::{params, Connection, OptionalExtension};

use crate::db::data_generated::fb::{AccountT, BackupT, TrpTransactionT};

const LATEST_VERSION: u32 = 1;

pub fn migrate_db(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            id INTEGER PRIMARY KEY NOT NULL,
            version INTEGER NOT NULL)",
        [],
    )?;

    let version = connection
        .query_row(
            "SELECT version FROM schema_version WHERE id = 1",
            [],
            |row| {
                let version: u32 = row.get(0)?;
                Ok(version)
            },
        )
        .optional()?
        .unwrap_or(0);

    if version < 1 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS accounts (
            id_account INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            seed TEXT,
            aindex INTEGER NOT NULL,
            sk TEXT,
            address TEXT NOT NULL UNIQUE)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS blocks (
            height INTEGER PRIMARY KEY,
            hash BLOB NOT NULL,
            timestamp INTEGER NOT NULL)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS transactions (
            id_tx INTEGER PRIMARY KEY,
            account INTEGER NOT NULL,
            txid BLOB NOT NULL,
            height INTEGER NOT NULL,
            timestamp INTEGER NOT NULL,
            value INTEGER NOT NULL,
            address TEXT,
            CONSTRAINT tx_account UNIQUE (txid))",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS historical_prices (
                currency TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                price REAL NOT NULL,
                PRIMARY KEY (currency, timestamp))",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS contacts (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                address TEXT NOT NULL,
                dirty BOOL NOT NULL)",
            [],
        )?;

        connection.execute("CREATE INDEX i_account ON accounts(address)", [])?;
        connection.execute("CREATE INDEX i_transaction ON transactions(account)", [])?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS properties (
                name TEXT PRIMARY KEY,
                value TEXT NOT NULL)",
            [],
        )?;
    }

    if version != LATEST_VERSION {
        connection.execute(
            "INSERT INTO schema_version(id, version) VALUES (1, ?1) \
        ON CONFLICT (id) DO UPDATE SET version = excluded.version",
            [LATEST_VERSION],
        )?;

        connection.cache_flush()?;
        log::info!("Database migrated");
    }

    Ok(())
}

pub fn fetch_accounts(c: &Connection) -> Result<Vec<AccountT>> {
    let mut s = c.prepare("SELECT id_account,name FROM accounts")?;
    let rows = s.query_map([], |row| {
        let id: u32 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok(AccountT {
            id,
            name: Some(name),
            balance: 0,
        })
    })?;
    let mut accounts = vec![];
    for r in rows {
        accounts.push(r?);
    }
    Ok(accounts)
}

pub fn get_backup(connection: &Connection, id_account: u32) -> Result<BackupT> {
    let backup = connection.query_row(
        "SELECT name,seed,aindex,sk FROM accounts WHERE id_account=?1",
        [id_account],
        |row| {
            let name: String = row.get(0)?;
            let seed: String = row.get(1)?;
            let index: u32 = row.get(2)?;
            let sk: String = row.get(3)?;
            let backup = BackupT {
                name: Some(name),
                seed: Some(seed),
                index,
                sk: None,
                fvk: None,
                uvk: None,
                tsk: Some(sk),
            };
            Ok(backup)
        },
    )?;
    Ok(backup)
}

pub fn store_block(c: &Connection, height: usize, header: &BlockHeader) -> Result<()> {
    let hash = header.block_hash().to_vec();
    c.execute(
        "INSERT INTO blocks(height,hash,timestamp) VALUES (?1,?2,?3) \
        ON CONFLICT DO NOTHING",
        params![height, &hash, header.time],
    )?;
    Ok(())
}

pub fn fetch_txs(c: &Connection, id_account: u32) -> Result<Vec<TrpTransactionT>> {
    let mut s = c.prepare(
        "SELECT id_tx,txid,height,timestamp,value,address FROM transactions \
    WHERE account=?1",
    )?;
    let rows = s.query_map([id_account], |row| {
        let id: u32 = row.get(0)?;
        let txid: Vec<u8> = row.get(1)?;
        let height: u32 = row.get(2)?;
        let timestamp: u32 = row.get(3)?;
        let value: i64 = row.get(4)?;
        let address: String = row.get(5)?;
        let tx = TrpTransactionT {
            id,
            txid: Some(txid),
            height,
            timestamp,
            value,
            address: Some(address),
        };
        Ok(tx)
    })?;
    let mut txs = vec![];
    for tx in rows {
        txs.push(tx?);
    }
    Ok(txs)
}

pub fn store_txs<'a, I>(c: &Connection, id_account: u32, txs: I) -> Result<()>
where
    I: IntoIterator<Item = &'a TrpTransactionT> + Clone,
{
    let mut s = c.prepare(
        "INSERT INTO transactions(account,txid,height,timestamp,value,address) \
    VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT (txid) DO NOTHING",
    )?;
    for tx in txs {
        s.execute(params![
            id_account,
            tx.txid.as_ref().unwrap(),
            tx.height,
            tx.timestamp,
            tx.value,
            tx.address
        ])?;
    }
    Ok(())
}

pub fn delete_account(connection: &Connection, id_account: u32) -> Result<()> {
    connection.execute("DELETE FROM accounts WHERE id_account=?1", [id_account])?;
    connection.execute("DELETE FROM transactions WHERE account=?1", [id_account])?;
    Ok(())
}
