use crate::orchard::{derive_orchard_keys, OrchardKeyBytes};
use crate::unified::UnifiedAddressType;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use zcash_primitives::consensus::{Network, Parameters};

pub fn get_orchard(connection: &Connection, account: u32) -> Result<Option<OrchardKeyBytes>> {
    let key = connection
        .query_row(
            "SELECT sk, fvk FROM orchard_addrs WHERE account = ?1",
            [account],
            |row| {
                let sk = row.get::<_, Option<Vec<u8>>>(0)?;
                let fvk = row.get::<_, Vec<u8>>(1)?;
                Ok(OrchardKeyBytes {
                    sk: sk.map(|sk| sk.try_into().unwrap()),
                    fvk: fvk.try_into().unwrap(),
                })
            },
        )
        .optional()?;
    Ok(key)
}

pub fn create_orchard(connection: &Connection, network: &Network, account: u32) -> Result<()> {
    let account_details = super::account::get_account(connection, account)?;
    if let Some(account_details) = account_details {
        let keys = derive_orchard_keys(
            network.coin_type(),
            account_details.seed.as_ref().unwrap(),
            account_details.aindex,
        );
        connection.execute(
            "INSERT INTO orchard_addrs(account, sk, fvk) VALUES (?1, ?2, ?3)",
            params![account, &keys.sk, &keys.fvk],
        )?;
    }
    Ok(())
}

pub fn store_orchard_fvk(connection: &Connection, account: u32, fvk: &[u8; 96]) -> Result<()> {
    connection.execute(
        "INSERT INTO orchard_addrs(account, sk, fvk) VALUES (?1, NULL, ?2) ON CONFLICT DO NOTHING",
        params![account, fvk],
    )?;
    Ok(())
}

pub fn get_ua_settings(connection: &Connection, account: u32) -> Result<UnifiedAddressType> {
    let tpe = connection.query_row(
        "SELECT transparent, sapling, orchard FROM ua_settings WHERE account = ?1",
        [account],
        |r| {
            Ok(UnifiedAddressType {
                transparent: r.get(0)?,
                sapling: r.get(1)?,
                orchard: r.get(2)?,
            })
        },
    )?;
    Ok(tpe)
}

pub fn store_ua_settings(
    connection: &Connection,
    account: u32,
    transparent: bool,
    sapling: bool,
    orchard: bool,
) -> Result<()> {
    connection.execute(
        "INSERT INTO ua_settings(account, transparent, sapling, orchard) VALUES (?1, ?2, ?3, ?4)",
        params![account, transparent, sapling, orchard],
    )?;
    Ok(())
}
