use crate::db::data_generated::fb::TransparentDetailsT;
use crate::taddr::derive_tkeys;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use zcash_primitives::consensus::{Network, Parameters};

pub fn get_transparent(
    connection: &Connection,
    account: u32,
) -> Result<Option<TransparentDetailsT>> {
    let details = connection
        .query_row(
            "SELECT sk, address FROM taddrs WHERE account = ?1",
            [account],
            |r| {
                Ok(TransparentDetailsT {
                    id: account,
                    sk: r.get(0)?,
                    address: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(details)
}

pub fn create_taddr(connection: &Connection, network: &Network, account: u32) -> Result<()> {
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
