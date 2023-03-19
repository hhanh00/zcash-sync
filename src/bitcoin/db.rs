use anyhow::Result;
use electrum_client::bitcoin::BlockHeader;
use rusqlite::{params, Connection, OptionalExtension};

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
            tx_index INTEGER,
            CONSTRAINT tx_account UNIQUE (height, tx_index, account))",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS received_notes (
            id_note INTEGER PRIMARY KEY,
            account INTEGER NOT NULL,
            height INTEGER NOT NULL,
            txid BLOB NOT NULL,
            output_index INTEGER NOT NULL,
            value INTEGER NOT NULL,
            spent INTEGER,
            excluded BOOL,
            CONSTRAINT tx_output UNIQUE (txid, output_index))",
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

        connection.execute(
            "CREATE INDEX i_received_notes ON received_notes(account)",
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

pub fn store_block(
    c: &Connection,
    height: usize,
    header: &BlockHeader,
) -> std::result::Result<(), anyhow::Error> {
    let hash = header.block_hash().to_vec();
    c.execute(
        "INSERT INTO blocks(height,hash,timestamp) VALUES (?1,?2,?3) \
        ON CONFLICT DO NOTHING",
        params![height, &hash, header.time],
    )?;
    Ok(())
}
