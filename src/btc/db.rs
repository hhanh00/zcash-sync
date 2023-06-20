use crate::btc::key::AccountKey;
use crate::btc::BTCNET;
use crate::db::data_generated::fb::{
    AccountT, AccountVecT, BackupT, HeightT, PlainNoteT, PlainNoteVecT, PlainTxT, PlainTxVecT,
};
use anyhow::Result;
use electrum_client::bitcoin::address::{WitnessProgram, WitnessVersion};
use electrum_client::bitcoin::block::Header;
use electrum_client::bitcoin::script::PushBytesBuf;
use electrum_client::bitcoin::secp256k1::SecretKey;
use electrum_client::bitcoin::{PrivateKey, Transaction, Txid};
use rusqlite::{params, Connection, OptionalExtension};

pub fn init_db(connection: &Connection) -> anyhow::Result<()> {
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
            pkh BLOB NOT NULL,
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
            value INTEGER NOT NULL)",
            [],
        )?;

        connection.execute(
            "CREATE TABLE IF NOT EXISTS utxos (
            id_utxo INTEGER PRIMARY KEY NOT NULL,
            account INTEGER NOT NULL,
            height INTEGER NOT NULL,
            timestamp INTEGER NOT NULL,
            txid BLOB NOT NULL,
            vout INTEGER NOT NULL,
            value INTEGER,
            spent INTEGER,
            CONSTRAINT utxo_output UNIQUE (txid, vout))",
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
        connection.execute(
            "CREATE INDEX IF NOT EXISTS i_transaction ON transactions(account)",
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

pub fn store_keys(connection: &Connection, name: &str, keys: &AccountKey) -> Result<u32> {
    connection.execute(
        "INSERT INTO accounts(name, seed, sk, pkh, address) \
    VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(address) DO NOTHING",
        params![
            name,
            keys.passphrase,
            keys.secret_key,
            keys.pkh,
            keys.address
        ],
    )?;
    let id = connection.last_insert_rowid() as u32;
    Ok(id)
}

pub fn delete_secrets(connection: &Connection, id: u32) -> Result<()> {
    connection.execute(
        "UPDATE accounts SET seed = NULL, sk = NULL WHERE id_account = ?1",
        [id],
    )?;
    Ok(())
}

pub fn list_accounts(connection: &Connection) -> Result<AccountVecT> {
    let mut s = connection.prepare(
        "WITH n AS (SELECT a.id_account, a.name, a.seed, a.sk, CASE WHEN r.spent IS NULL THEN r.value ELSE 0 END AS nv FROM accounts a LEFT JOIN utxos r ON a.id_account = r.account) \
               SELECT id_account, name, COALESCE(sum(nv), 0) AS balance FROM n GROUP by id_account")?;
    let accounts = s.query_map([], |r| {
        Ok(AccountT {
            id: r.get(0)?,
            name: r.get(1)?,
            balance: r.get(2)?,
            key_type: 0,
        })
    })?;
    let accounts: Result<Vec<_>, _> = accounts.collect();
    Ok(AccountVecT {
        accounts: Some(accounts?),
    })
}

pub fn get_account(connection: &Connection, account: u32) -> Result<AccountT> {
    let account = connection.query_row(
        "SELECT name FROM accounts WHERE id_account = ?1",
        [account],
        |r| {
            Ok(AccountT {
                id: account,
                name: Some(r.get(0)?),
                ..AccountT::default()
            })
        },
    )?;
    Ok(account)
}

pub fn has_account(connection: &Connection, id: u32) -> Result<bool> {
    let res = connection
        .query_row("SELECT 1 FROM accounts WHERE id_account = ?1", [id], |_r| {
            Ok(())
        })
        .optional()?;
    Ok(res.is_some())
}

pub fn update_name(connection: &Connection, id: u32, name: &str) -> Result<()> {
    connection.execute(
        "UPDATE accounts SET NAME = ?2 WHERE id_account = ?1",
        params![id, name],
    )?;
    Ok(())
}

pub fn delete_account(connection: &Connection, id: u32) -> Result<()> {
    connection.execute("DELETE FROM accounts WHERE id_account = ?1", [id])?;
    Ok(())
}

pub fn get_height(connection: &Connection) -> Result<Option<HeightT>> {
    let h = connection
        .query_row(
            "SELECT height, timestamp FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
            [],
            |r| {
                Ok(HeightT {
                    height: r.get(0)?,
                    timestamp: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(h)
}

pub fn get_block_hash(connection: &Connection, height: u32) -> Result<Vec<u8>> {
    let h: Vec<u8> =
        connection.query_row("SELECT hash FROM blocks WHERE height = ?1", [height], |r| {
            r.get(0)
        })?;
    Ok(h)
}

pub fn rewind_to(connection: &Connection, height: u32) -> Result<()> {
    connection.execute("DELETE FROM blocks WHERE height > ?1", [height])?;
    connection.execute("DELETE FROM transactions WHERE height > ?1", [height])?;
    connection.execute("DELETE FROM utxos WHERE height > ?1", [height])?;
    connection.execute("UPDATE utxos SET spent = NULL WHERE spent > ?1", [height])?;
    Ok(())
}

pub fn store_header(connection: &Connection, height: u32, header: &Header) -> Result<()> {
    let hash = header.block_hash();
    let hash: &[u8] = hash.as_ref();
    connection.execute(
        "INSERT INTO blocks(height, hash, timestamp) \
    VALUES (?1, ?2, ?3)",
        params![height, hash, header.time],
    )?;
    Ok(())
}

pub fn get_accounts(connection: &Connection) -> Result<Vec<u32>> {
    let mut s = connection.prepare("SELECT id_account FROM accounts")?;
    let m = s.query_map([], |r| r.get::<_, u32>(0))?;
    let v: std::result::Result<Vec<_>, _> = m.collect();
    Ok(v?)
}

pub fn get_backup(
    connection: &Connection,
    account: u32,
    _map_sk: fn(Vec<u8>) -> String,
) -> Result<BackupT> {
    let backup = connection.query_row(
        "SELECT name, seed, sk FROM accounts WHERE id_account = ?1",
        [account],
        |r| {
            Ok(BackupT {
                name: Some(r.get(0)?),
                seed: r.get(1)?,
                sk: r.get::<_, Option<Vec<u8>>>(2)?.map(|sk| {
                    let sk = SecretKey::from_slice(&sk).unwrap();
                    let privk = PrivateKey::new(sk, BTCNET);
                    privk.to_wif()
                }),
                ..BackupT::default()
            })
        },
    )?;
    Ok(backup)
}

pub fn get_sk(connection: &Connection, account: u32) -> Result<Option<SecretKey>> {
    let sk = connection.query_row(
        "SELECT sk FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, Option<Vec<u8>>>(0),
    )?;
    let sk = sk.map(|sk| SecretKey::from_slice(&sk).unwrap());
    Ok(sk)
}

pub fn get_wp(connection: &Connection, account: u32) -> Result<WitnessProgram> {
    let pkh = connection.query_row(
        "SELECT pkh FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, Vec<u8>>(0),
    )?;
    let pb: PushBytesBuf = pkh.try_into().unwrap();
    let program = WitnessProgram::new(WitnessVersion::V0, pb)?;
    Ok(program)
}

pub fn get_address(connection: &Connection, account: u32) -> Result<String> {
    let address = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, String>(0),
    )?;
    Ok(address)
}

pub fn get_tx_txid(connection: &Connection, account: u32, txid: &Txid) -> Result<Option<PlainTxT>> {
    let txid: &[u8] = txid.as_ref();
    let tx = connection
        .query_row(
            "SELECT id_tx, height, timestamp, value \
        FROM transactions WHERE account = ?1 AND txid = ?2",
            params![account, txid],
            |r| {
                let id: u32 = r.get(0)?;
                let height: u32 = r.get(1)?;
                let timestamp: u32 = r.get(2)?;
                let value: i64 = r.get(3)?;

                Ok(PlainTxT {
                    id,
                    tx_id: Some(hex::encode(txid)),
                    height,
                    timestamp,
                    value,
                    address: None,
                })
            },
        )
        .optional()?;
    Ok(tx)
}

pub fn store_utxo(
    connection: &Connection,
    account: u32,
    height: u32,
    timestamp: u32,
    txid: &Txid,
    vout: u32,
    value: u64,
) -> Result<()> {
    let tx_hash: &[u8] = txid.as_ref();
    connection.execute(
        "INSERT INTO utxos(account, height, timestamp, txid, vout, value) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![account, height, timestamp, tx_hash, vout, value as i64],
    )?;
    Ok(())
}

pub fn store_tx(
    connection: &Connection,
    account: u32,
    height: u32,
    timestamp: u32,
    tx: &Transaction,
    value: i64,
) -> Result<u32> {
    let txid = tx.txid();
    let tx_hash: &[u8] = txid.as_ref();
    connection.execute(
        "INSERT INTO transactions(account, txid, height, timestamp, value) \
    VALUES (?1, ?2, ?3, ?4, ?5)",
        params![account, tx_hash, height, timestamp, value],
    )?;
    let id = connection.last_insert_rowid() as u32;
    Ok(id)
}

pub fn get_balance(connection: &Connection, account: u32) -> Result<u64> {
    let balance = connection.query_row(
        "SELECT SUM(value) FROM utxos WHERE account = ?1 AND spent IS NULL",
        [account],
        |r| r.get::<_, Option<i64>>(0),
    )?;
    Ok(balance.unwrap_or(0) as u64)
}

pub fn get_txs(connection: &Connection, account: u32) -> Result<PlainTxVecT> {
    let mut s = connection.prepare(
        "SELECT id_tx, txid, height, timestamp, value FROM transactions WHERE account = ?1 ORDER BY height DESC",
    )?;
    let rows = s.query_map([account], |r| {
        let id: u32 = r.get(0)?;
        let mut txid: Vec<u8> = r.get(1)?;
        txid.reverse();
        let height: u32 = r.get(2)?;
        let timestamp: u32 = r.get(3)?;
        let value: i64 = r.get(4)?;

        Ok(PlainTxT {
            id,
            tx_id: Some(hex::encode(txid)),
            height,
            timestamp,
            value,
            address: None,
        })
    })?;
    let txs: Result<Vec<_>, _> = rows.collect();
    let res = PlainTxVecT { txs: Some(txs?) };
    Ok(res)
}

pub fn get_utxos(connection: &Connection, account: u32) -> Result<PlainNoteVecT> {
    let mut s = connection.prepare(
        "SELECT id_utxo, height, timestamp, txid, vout, value FROM utxos \
    WHERE account = ?1 AND spent IS NULL ORDER BY height DESC",
    )?;
    let rows = s.query_map([account], |r| {
        let id: u32 = r.get(0)?;
        let height: u32 = r.get(1)?;
        let timestamp: u32 = r.get(2)?;
        let txid: Vec<u8> = r.get(3)?;
        let vout: u32 = r.get(4)?;
        let value: i64 = r.get(5)?;
        Ok(PlainNoteT {
            id,
            tx_id: Some(hex::encode(txid)),
            height,
            timestamp,
            vout,
            value: value as u64,
        })
    })?;
    let notes: Result<Vec<_>, _> = rows.collect();
    Ok(PlainNoteVecT {
        notes: Some(notes?),
    })
}

pub fn get_property(connection: &Connection, name: &str) -> Result<String> {
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM properties WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.unwrap_or_default())
}

pub fn set_property(connection: &Connection, name: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO properties(name, value) VALUES (?1, ?2) \
    ON CONFLICT(name) DO UPDATE SET value = excluded.value",
        params![name, value],
    )?;
    Ok(())
}

pub fn truncate(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM blocks", [])?;
    connection.execute("DELETE FROM transactions", [])?;
    connection.execute("DELETE FROM utxos", [])?;
    connection.execute("DELETE FROM historical_prices", [])?;
    Ok(())
}
