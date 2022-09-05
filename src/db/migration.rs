use rusqlite::{params, Connection, OptionalExtension};

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

pub fn init_db(connection: &Connection) -> anyhow::Result<()> {
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

    if version != 4 {
        update_schema_version(connection, 4)?;
        log::info!("Database migrated");
    }

    Ok(())
}
