use crate::orchard::derive_orchard_keys;
use rusqlite::{params, Connection, OptionalExtension};
use zcash_primitives::consensus::{Network, Parameters};

pub fn get_schema_version(connection: &Connection) -> anyhow::Result<u32> {
    let version: Option<u32> = connection
        .query_row(
            "SELECT version FROM schema_version WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(version.unwrap_or(0))
}

pub fn update_schema_version(connection: &Connection, version: u32) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO schema_version(id, version) VALUES (1, ?1) \
    ON CONFLICT (id) DO UPDATE SET version = excluded.version",
        params![version],
    )?;
    Ok(())
}

pub fn reset_db(connection: &Connection) -> anyhow::Result<()> {
    // don't drop account data: accounts, taddrs, secret_shares
    connection.execute("DROP TABLE blocks", [])?;
    connection.execute("DROP TABLE transactions", [])?;
    connection.execute("DROP TABLE received_notes", [])?;
    connection.execute("DROP TABLE sapling_witnesses", [])?;
    connection.execute("DROP TABLE diversifiers", [])?;
    connection.execute("DROP TABLE historical_prices", [])?;
    update_schema_version(connection, 0)?;
    Ok(())
}

const LATEST_VERSION: u32 = 9;

pub fn init_db(connection: &Connection, network: &Network, has_ua: bool) -> anyhow::Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
            id INTEGER PRIMARY KEY NOT NULL,
            version INTEGER NOT NULL)",
        [],
    )?;

    let version = get_schema_version(connection)?;

    if version < 1 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS accounts (
            id_account INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            seed TEXT,
            aindex INTEGER NOT NULL,
            sk TEXT,
            ivk TEXT NOT NULL UNIQUE,
            address TEXT NOT NULL)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS blocks (
            height INTEGER PRIMARY KEY,
            hash BLOB NOT NULL,
            timestamp INTEGER NOT NULL,
            sapling_tree BLOB NOT NULL)",
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
            memo TEXT,
            tx_index INTEGER,
            CONSTRAINT tx_account UNIQUE (height, tx_index, account))",
            [],
        )?;

        connection.execute(
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
            excluded BOOL,
            CONSTRAINT tx_output UNIQUE (tx, output_index))",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS sapling_witnesses (
            id_witness INTEGER PRIMARY KEY,
            note INTEGER NOT NULL,
            height INTEGER NOT NULL,
            witness BLOB NOT NULL,
            CONSTRAINT witness_height UNIQUE (note, height))",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS diversifiers (
            account INTEGER PRIMARY KEY NOT NULL,
            diversifier_index BLOB NOT NULL)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS taddrs (
            account INTEGER PRIMARY KEY NOT NULL,
            sk TEXT NOT NULL,
            address TEXT NOT NULL)",
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
    }

    if version < 2 {
        connection.execute(
            "CREATE INDEX i_received_notes ON received_notes(account)",
            [],
        )?;
        connection.execute("CREATE INDEX i_account ON accounts(address)", [])?;
        connection.execute("CREATE INDEX i_contact ON contacts(address)", [])?;
        connection.execute("CREATE INDEX i_transaction ON transactions(account)", [])?;
        connection.execute("CREATE INDEX i_witness ON sapling_witnesses(height)", [])?;
    }

    if version < 3 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY,
            account INTEGER NOT NULL,
            sender TEXT,
            recipient TEXT NOT NULL,
            subject TEXT NOT NULL,
            body TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            height INTEGER NOT NULL,
            read BOOL NOT NULL)",
            [],
        )?;
        // Don't index because it *really* slows down inserts
        // connection.execute(
        //     "CREATE INDEX i_messages ON messages(account)",
        //     [],
        // )?;
    }

    if version < 4 {
        connection.execute("ALTER TABLE messages ADD id_tx INTEGER", [])?;
    }

    if version < 5 {
        connection.execute(
            "CREATE TABLE orchard_addrs(
            account INTEGER PRIMARY KEY,
            sk BLOB,
            fvk BLOB NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE ua_settings(
            account INTEGER PRIMARY KEY,
            transparent BOOL NOT NULL,
            sapling BOOL NOT NULL,
            orchard BOOL NOT NULL)",
            [],
        )?;
        if has_ua {
            upgrade_accounts(&connection, network)?;
        }
        connection.execute(
            "CREATE TABLE sapling_tree(
            height INTEGER PRIMARY KEY,
            tree BLOB NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE orchard_tree(
            height INTEGER PRIMARY KEY,
            tree BLOB NOT NULL)",
            [],
        )?;
        connection.execute(
            "INSERT INTO sapling_tree SELECT height, sapling_tree FROM blocks",
            [],
        )?;
        connection.execute("ALTER TABLE blocks DROP sapling_tree", [])?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS new_received_notes (
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
            rho BLOB,
            orchard BOOL NOT NULL DEFAULT false,
            spent INTEGER,
            excluded BOOL,
            CONSTRAINT tx_output UNIQUE (tx, orchard, output_index))",
            [],
        )?;
        connection.execute(
            "INSERT INTO new_received_notes(
            id_note, account, position, tx, height, output_index, diversifier, value,
            rcm, nf, spent, excluded
        ) SELECT * FROM received_notes",
            [],
        )?;
        connection.execute("DROP TABLE received_notes", [])?;
        connection.execute(
            "ALTER TABLE new_received_notes RENAME TO received_notes",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS orchard_witnesses (
            id_witness INTEGER PRIMARY KEY,
            note INTEGER NOT NULL,
            height INTEGER NOT NULL,
            witness BLOB NOT NULL,
            CONSTRAINT witness_height UNIQUE (note, height))",
            [],
        )?;
        connection.execute(
            "CREATE INDEX IF NOT EXISTS i_orchard_witness ON orchard_witnesses(height)",
            [],
        )?;
        connection.execute(
            "ALTER TABLE messages ADD incoming BOOL NOT NULL DEFAULT true",
            [],
        )?;
    }

    if version < 6 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS send_templates (
                id_send_template INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                address TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fiat_amount DECIMAL NOT NULL,
                fee_included BOOL NOT NULL,
                fiat TEXT,
                include_reply_to BOOL NOT NULL,
                subject TEXT NOT NULL,
                body TEXT NOT NULL)",
            [],
        )?;
    }

    if version < 7 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS properties (
                name TEXT PRIMARY KEY,
                value TEXT NOT NULL)",
            [],
        )?;
    }

    if version < 8 {
        connection.execute(
            "CREATE TABLE IF NOT EXISTS new_taddrs (
            account INTEGER PRIMARY KEY NOT NULL,
            sk TEXT,
            address TEXT NOT NULL)",
            [],
        )?;
        connection.execute(
            "INSERT INTO new_taddrs(
            account, sk, address
        ) SELECT * FROM taddrs",
            [],
        )?;
        connection.execute("DROP TABLE taddrs", [])?;
        connection.execute("ALTER TABLE new_taddrs RENAME TO taddrs", [])?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS hw_wallets(
            account INTEGER PRIMARY KEY NOT NULL,
            ledger BOOL NOT NULL)",
            [],
        )?;
    }

    if version < 9 {
        crate::transparent::migrate_db(connection)?;
    }

    if version != LATEST_VERSION {
        update_schema_version(connection, LATEST_VERSION)?;
        connection.cache_flush()?;
        log::info!("Database migrated");
    }

    // We may get a database that has no valid schema version from a version of single currency Z/YWallet
    // because we kept the same app name in Google/Apple Stores. The upgraded app fails to recognize the db tables
    // At least we monkey patch the accounts table to let the user access the backup page and recover his seed phrase
    let c = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('accounts') WHERE name = 'aindex'",
        params![],
        |row| {
            let c: u32 = row.get(0)?;
            Ok(c)
        },
    )?;
    if c == 0 {
        connection.execute(
            "ALTER TABLE accounts ADD aindex INTEGER NOT NULL DEFAULT (0)",
            params![],
        )?;
    }

    Ok(())
}

fn upgrade_accounts(connection: &Connection, network: &Network) -> anyhow::Result<()> {
    let mut statement = connection.prepare("SELECT a.id_account, a.seed, a.aindex, t.address FROM accounts a LEFT JOIN taddrs t ON a.id_account = t.account")?;
    let rows = statement.query_map([], |row| {
        let id_account: u32 = row.get(0)?;
        let seed: Option<String> = row.get(1)?;
        let aindex: u32 = row.get(2)?;
        let transparent_address: Option<String> = row.get(3)?;
        Ok((id_account, seed, aindex, transparent_address.is_some()))
    })?;
    let mut res = vec![];
    for row in rows {
        res.push(row?);
    }

    for (id_account, seed, aindex, has_transparent) in res {
        let has_orchard = seed.is_some();
        if let Some(seed) = seed {
            let orchard_keys = derive_orchard_keys(network.coin_type(), &seed, aindex);
            connection.execute(
                "INSERT INTO orchard_addrs(account, sk, fvk) VALUES (?1,?2,?3)",
                params![id_account, &orchard_keys.sk, &orchard_keys.fvk],
            )?;
        }
        connection.execute(
            "INSERT INTO ua_settings(account, transparent, sapling, orchard) VALUES (?1,?2,?3,?4)",
            params![id_account, has_transparent, true, has_orchard],
        )?;
    }
    Ok(())
}
