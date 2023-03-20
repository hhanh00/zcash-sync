use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};

use anyhow::Result;
use electrum_client::{Client, ElectrumApi};
use lazy_static::lazy_static;

use crate::db::with_coin;

use super::db::store_block;

lazy_static! {
    static ref CLIENT: Mutex<Option<(String, Client)>> = Mutex::new(None);
}

pub fn get_client(new_url: &str) -> Result<MappedMutexGuard<Client>> {
    let mut guard = CLIENT.lock();
    let (url, client) = match guard.take() {
        Some((url, client)) => {
            let client = if url != new_url {
                Client::new(new_url)?
            } else {
                client
            };
            (new_url, client)
        }
        None => {
            let client = Client::new(new_url)?;
            (new_url, client)
        }
    };
    *guard = Some((url.to_string(), client));
    Ok(MutexGuard::map(guard, |g| &mut g.as_mut().unwrap().1))
}

pub async fn sync(coin: u8, url: &str) -> Result<()> {
    let client = get_client(url)?;
    let sub = client.block_headers_subscribe()?;
    with_coin(coin, |c| store_block(c, sub.height, &sub.header))?;
    Ok(())
}

pub fn get_height(url: &str) -> Result<u32> {
    let client = get_client(url)?;
    let sub = client.block_headers_subscribe()?;
    let height = sub.height as u32;
    Ok(height)
}
