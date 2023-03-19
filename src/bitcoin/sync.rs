use anyhow::Result;
use electrum_client::{Client, ElectrumApi};

use crate::db::with_coin;

use super::db::store_block;

pub async fn sync(coin: u8, url: &str) -> Result<()> {
    let client = Client::new(url)?;
    let sub = client.block_headers_subscribe()?;
    with_coin(coin, |c| store_block(c, sub.height, &sub.header))?;
    Ok(())
}

pub fn get_height(url: &str) -> Result<u32> {
    let client = Client::new(url)?;
    let sub = client.block_headers_subscribe()?;
    let height = sub.height as u32;
    Ok(height)
}
