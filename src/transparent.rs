mod db;

use crate::connect_lightwalletd;
use crate::db::data_generated::fb::{PlainNoteT, PlainNoteVecT, PlainTxT, PlainTxVecT};
use anyhow::{anyhow, Result};
pub use db::migrate_db;
use rusqlite::Connection;
use zcash_primitives::consensus::Network;

pub async fn sync(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
) -> Result<()> {
    let transparent_details =
        crate::db::transparent::get_transparent(connection, account)?.ok_or(anyhow!("No taddr"))?;
    let address = transparent_details.address.unwrap();
    db::fetch_txs(network, connection, url, account, &address)?;
    db::update_timestamps(connection, url).await?;
    Ok(())
}

pub async fn get_balance(connection: &Connection, url: &str, id_account: u32) -> Result<u64> {
    let mut client = connect_lightwalletd(url).await?;
    let details = crate::db::transparent::get_transparent(connection, id_account)?;
    let address = details.and_then(|d| d.address);
    let balance = match address {
        None => 0u64,
        Some(address) => crate::taddr::get_taddr_balance(&mut client, &address).await?,
    };
    Ok(balance)
}
