use crate::db::data_generated::fb::{AccountDetailsT, AccountT, AccountVecT, BackupT, BalanceT};
use crate::db::wrap_query_no_rows;
use anyhow::Result;
use orchard::keys::FullViewingKey;
use rusqlite::{params, Connection, OptionalExtension};
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::sapling::SaplingIvk;
use zcash_primitives::zip32::ExtendedFullViewingKey;

pub fn get_id_by_address(connection: &Connection, address: &str) -> Result<Option<u32>> {
    let id = connection
        .query_row(
            "SELECT id_account accounts WHERE address = ?1",
            params![address],
            |row| row.get(0),
        )
        .map_err(wrap_query_no_rows("get_account_info"))?;
    Ok(id)
}

pub fn get_account(connection: &Connection, id: u32) -> Result<Option<AccountDetailsT>> {
    assert_ne!(id, 0);
    let r = connection
        .query_row(
            "SELECT address, name, seed, aindex, sk, ivk FROM accounts WHERE address = ?1",
            [id],
            |r| {
                Ok(AccountDetailsT {
                    id,
                    name: r.get(1)?,
                    seed: r.get(2)?,
                    aindex: r.get(3)?,
                    sk: r.get(4)?,
                    ivk: r.get(5)?,
                    address: r.get(0)?,
                })
            },
        )
        .optional()?;
    Ok(r)
}

pub fn list_accounts(connection: &Connection) -> Result<Vec<AccountDetailsT>> {
    let mut s = connection
        .prepare("SELECT id_account, name, seed, aindex, sk, ivk, address FROM accounts")?;
    let accounts = s.query_map([], |r| {
        Ok(AccountDetailsT {
            id: r.get(0)?,
            name: r.get(1)?,
            seed: r.get(2)?,
            aindex: r.get(3)?,
            sk: r.get(4)?,
            ivk: r.get(5)?,
            address: r.get(6)?,
        })
    })?;
    let accounts: Result<Vec<_>, _> = accounts.collect();
    Ok(accounts?)
}

pub fn list_account_details(connection: &Connection) -> Result<AccountVecT> {
    let mut stmt = connection.prepare("WITH notes AS (SELECT a.id_account, a.name, a.seed, a.sk, CASE WHEN r.spent IS NULL THEN r.value ELSE 0 END AS nv FROM accounts a LEFT JOIN received_notes r ON a.id_account = r.account), \
                       accounts2 AS (SELECT id_account, name, seed, sk, COALESCE(sum(nv), 0) AS balance FROM notes GROUP by id_account) \
                       SELECT a.id_account, a.name, a.seed, a.sk, a.balance, hw.ledger FROM accounts2 a LEFT JOIN hw_wallets hw ON a.id_account = hw.account")?;
    let accounts = stmt.query_map([], |r| {
        let id: u32 = r.get("id_account")?;
        let name: Option<String> = r.get("name")?;
        let balance: i64 = r.get("balance")?;
        let seed: Option<String> = r.get("seed")?;
        let sk: Option<String> = r.get("sk")?;
        let ledger: Option<bool> = r.get("ledger")?;
        let key_type = if seed.is_some() {
            0
        } else if sk.is_some() {
            1
        } else if ledger == Some(true) {
            2
        } else {
            0x80
        };
        let account = AccountT {
            id,
            name,
            key_type,
            balance: balance as u64,
        };
        Ok(account)
    })?;
    let accounts: Result<Vec<_>, _> = accounts.collect();
    let accounts = AccountVecT {
        accounts: accounts.ok(),
    };
    Ok(accounts)
}

pub fn store_account(
    connection: &Connection,
    name: &str,
    seed: Option<&str>,
    aindex: u32,
    sk: Option<&str>,
    ivk: &str,
    address: &str,
) -> Result<u32> {
    connection.execute(
        "INSERT INTO accounts(name, seed, aindex, sk, ivk, address) \
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![name, seed, aindex, sk, ivk, address],
    )?;
    let id_account = connection.last_insert_rowid() as u32;
    Ok(id_account)
}

pub fn update_account_name(connection: &Connection, id: u32, name: &str) -> Result<()> {
    connection.execute(
        "UPDATE accounts SET name = ?2 WHERE id_account = ?1",
        params![id, name],
    )?;
    Ok(())
}

/// Sub Account
///
pub fn get_next_aindex(connection: &Connection, seed: &str) -> Result<u32> {
    let index = connection.query_row(
        "SELECT MAX(aindex) FROM accounts WHERE seed = ?1",
        [seed],
        |r| {
            let aindex = r.get::<_, Option<u32>>(0)?.map(|i| i + 1);
            Ok(aindex.unwrap_or(0))
        },
    )?;
    Ok(index)
}

/// View Only
pub fn convert_to_watchonly(connection: &Connection, id_account: u32) -> Result<()> {
    connection.execute(
        "UPDATE accounts SET seed = NULL, sk = NULL WHERE id_account = ?1",
        [id_account],
    )?;
    connection.execute(
        "UPDATE orchard_addrs SET sk = NULL WHERE account = ?1",
        [id_account],
    )?;
    connection.execute(
        "UPDATE taddrs SET sk = NULL WHERE account = ?1",
        [id_account],
    )?;
    Ok(())
}

pub struct AccountViewKey {
    pub account: u32,
    pub sfvk: ExtendedFullViewingKey,
    pub sivk: SaplingIvk,
    pub ofvk: Option<FullViewingKey>,
}

/// List Full Viewing Keys
///
pub fn get_fvks(connection: &Connection, network: &Network) -> Result<Vec<AccountViewKey>> {
    let hrp_fvk = network.hrp_sapling_extended_full_viewing_key();
    let mut statement = connection
        .prepare("SELECT a.id_account, a.ivk, o.fvk FROM accounts a LEFT JOIN orchard_addrs o ON a.id_account = o.account")?;
    let rows = statement.query_map([], |row| {
        let sivk: String = row.get::<_, String>(1)?;
        let sfvk = decode_extended_full_viewing_key(hrp_fvk, &sivk).unwrap();
        let sivk = sfvk.fvk.vk.ivk();
        let ofvk = row
            .get::<_, Option<Vec<u8>>>(2)?
            .map(|kb| FullViewingKey::from_bytes(&kb.try_into().unwrap()).unwrap());
        Ok(AccountViewKey {
            account: row.get(0)?,
            sfvk,
            sivk,
            ofvk,
        })
    })?;
    let fvks: Result<Vec<_>, _> = rows.collect();
    Ok(fvks?)
}

/// Addresses
///
pub fn get_available_addrs(connection: &Connection, account: u32) -> Result<u8> {
    let has_transparent = connection
        .query_row(
            "SELECT 1 FROM taddrs WHERE account = ?1",
            [account],
            |_row| Ok(()),
        )
        .optional()?
        .is_some();
    let has_sapling = connection
        .query_row(
            "SELECT 1 FROM accounts WHERE account = ?1",
            [account],
            |_row| Ok(()),
        )
        .optional()?
        .is_some();
    let has_orchard = connection
        .query_row(
            "SELECT 1 FROM orchard_addrs WHERE account = ?1",
            [account],
            |_row| Ok(()),
        )
        .optional()?
        .is_some();
    let res = if has_transparent { 1 } else { 0 }
        | if has_sapling { 2 } else { 0 }
        | if has_orchard { 4 } else { 0 };
    Ok(res)
}

/// Balances
///
pub fn get_balance(connection: &Connection, account: u32) -> Result<u64> {
    let balance = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE (spent IS NULL OR spent = 0) AND account = ?1",
        [account],
        |row| row.get::<_, Option<u64>>(0),
    )?;
    Ok(balance.unwrap_or(0))
}

pub fn get_balances(
    connection: &Connection,
    id: u32,
    confirmed_height: u32,
    filter_excluded: bool,
) -> Result<BalanceT> {
    let excluded_cond = if filter_excluded {
        " AND (excluded IS NULL OR NOT(excluded))"
    } else {
        ""
    };
    let shielded = connection
        .query_row(
            &("SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL"
                .to_owned() + excluded_cond),
            [id],
            |row| row.get::<_, Option<u64>>(0),
        )?
        .unwrap_or_default();
    let unconfirmed_spent = connection
        .query_row(
            &("SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent = 0"
                .to_owned()
                + excluded_cond),
            [id],
            |row| row.get::<_, Option<u64>>(0),
        )?
        .unwrap_or_default();
    let balance = shielded + unconfirmed_spent;
    let under_confirmed = connection.query_row(
        &("SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL AND height > ?2".to_owned()
            + excluded_cond),
        params![id, confirmed_height],
        |row| row.get::<_, Option<u64>>(0)
    )?.unwrap_or_default();
    let excluded = connection
        .query_row(
            "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL \
        AND height <= ?2 AND excluded",
            params![id, confirmed_height],
            |row| row.get::<_, Option<u64>>(0),
        )?
        .unwrap_or_default();
    let sapling = connection.query_row(
        &("SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 0 AND height <= ?2".to_owned()
            + excluded_cond),
        params![id, confirmed_height],
        |row| row.get::<_, Option<u64>>(0)
    )?.unwrap_or_default();
    let orchard = connection.query_row(
        &("SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 1 AND height <= ?2".to_owned()
            + excluded_cond),
        params![id, confirmed_height],
        |row| row.get::<_, Option<u64>>(0)
    )?.unwrap_or_default();
    let balance = BalanceT {
        shielded,
        unconfirmed_spent,
        balance,
        under_confirmed,
        excluded,
        sapling,
        orchard,
    };
    Ok(balance)
}

/// Active
///
pub fn get_active_account(connection: &Connection) -> Result<u32> {
    let id = connection
        .query_row(
            "SELECT value FROM properties WHERE name = 'account'",
            [],
            |row| {
                let value: String = row.get(0)?;
                let value: u32 = value.parse().unwrap();
                Ok(value)
            },
        )
        .optional()?
        .unwrap_or(0);
    let id = get_available_account_id(connection, id)?;
    set_active_account(connection, id)?;
    Ok(id)
}

pub fn set_active_account(connection: &Connection, id: u32) -> Result<()> {
    connection.execute(
        "INSERT INTO properties(name, value) VALUES ('account',?1) \
    ON CONFLICT (name) DO UPDATE SET value = excluded.value",
        [id],
    )?;
    Ok(())
}

pub fn get_available_account_id(connection: &Connection, id: u32) -> Result<u32> {
    let r = connection
        .query_row("SELECT 1 FROM accounts WHERE id_account = ?1", [id], |_| {
            Ok(())
        })
        .optional()?;
    if r.is_some() {
        return Ok(id);
    }
    let id = connection
        .query_row("SELECT MAX(id_account) FROM accounts", [], |row| {
            row.get::<_, u32>(0)
        })
        .optional()?
        .unwrap_or(0);
    Ok(id)
}

pub fn get_backup(
    connection: &Connection,
    account: u32,
    map_sk: fn(Vec<u8>) -> String,
) -> Result<BackupT> {
    let backup = connection.query_row(
        "SELECT name, seed, sk FROM accounts WHERE id_account = ?1",
        [account],
        |r| {
            Ok(BackupT {
                name: Some(r.get(0)?),
                seed: r.get(1)?,
                sk: r.get::<_, Option<Vec<u8>>>(2)?.map(map_sk),
                ..BackupT::default()
            })
        },
    )?;
    Ok(backup)
}
