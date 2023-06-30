use crate::db::data_generated::fb::{AccountT, AccountVecT};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub fn init_db(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            id INTEGER PRIMARY KEY NOT NULL,
            version INTEGER NOT NULL)",
        [],
    )?;

    let version = connection
        .query_row("SELECT version FROM schema_version WHERE id = 0", [], |r| {
            r.get::<_, u32>(0)
        })
        .optional()?
        .unwrap_or(0);

    if version < 1 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS properties (
                name TEXT PRIMARY KEY,
                value TEXT NOT NULL)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS accounts (
            id_account INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            seed TEXT,
            sk BLOB,
            address TEXT NOT NULL UNIQUE,
            height INTEGER NOT NULL,
            balance INTEGER NOT NULL)",
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
            "CREATE TABLE IF NOT EXISTS historical_prices (
                currency TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                price REAL NOT NULL,
                PRIMARY KEY (currency, timestamp))",
            [],
        )?;

        connection.execute(
            "CREATE INDEX IF NOT EXISTS i_account ON accounts(address)",
            [],
        )?;
    }

    let latest_version = 1;
    connection.execute(
        "INSERT INTO schema_version(id, version) \
        VALUES (0, ?1) ON CONFLICT(id) DO UPDATE \
        SET version = excluded.version",
        [latest_version],
    )?;

    Ok(())
}

pub fn list_accounts(connection: &Connection) -> Result<AccountVecT> {
    let mut s = connection.prepare("SELECT id_account, name, balance FROM accounts")?;
    let rows = s.query_map([], |r| {
        Ok(AccountT {
            id: r.get(0)?,
            name: Some(r.get(1)?),
            balance: r.get(2)?,
            ..AccountT::default()
        })
    })?;
    let accounts: Result<Vec<_>, _> = rows.collect();

    Ok(AccountVecT {
        accounts: Some(accounts?),
    })
}

pub fn store_keys(
    connection: &Connection,
    name: &str,
    seed: &str,
    sk: &[u8; 32],
    address: &str,
) -> Result<u32> {
    connection.execute(
        "INSERT INTO accounts(name, seed, sk, address, height, balance) \
    VALUES (?1, ?2, ?3, ?4, 0, 0) ON CONFLICT(address) DO NOTHING",
        params![name, seed, sk, address],
    )?;
    let id = connection.last_insert_rowid() as u32;
    Ok(id)
}

pub fn get_address(connection: &Connection, account: u32) -> Result<String> {
    let sk: String = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get(0),
    )?;
    Ok(sk)
}

pub fn get_sk(connection: &Connection, account: u32) -> Result<Vec<u8>> {
    let sk: Vec<u8> = connection.query_row(
        "SELECT sk FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get(0),
    )?;
    Ok(sk)
}
