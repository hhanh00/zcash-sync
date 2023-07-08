use anyhow::{anyhow, Result};
use ethers::prelude::*;
use rusqlite::{params, Connection};
use std::thread;
use tokio::runtime::Runtime;

pub fn get_latest_height(url: &str) -> Result<u32> {
    let provider = Provider::<Http>::try_from(url)?;
    let height = thread::spawn(|| {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move { provider.get_block_number().await })
    })
    .join()
    .map_err(|_e| anyhow::anyhow!("Error"))?;
    Ok(height?.as_u32())
}

pub fn sync(connection: &Connection, url: &str) -> Result<u32> {
    let provider = Provider::<Http>::try_from(url)?;
    let block_hash = thread::spawn(|| {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let height = provider.get_block_number().await?;
            let block_hash = provider
                .get_block(height)
                .await?
                .ok_or(anyhow!("Unknown block"))?;
            Ok::<_, anyhow::Error>(block_hash)
        })
    })
    .join()
    .unwrap()?;
    let height = block_hash.number.unwrap().as_u32();
    let hash = block_hash.hash.unwrap();
    let time = block_hash.time()?.timestamp();
    connection.execute(
        "INSERT INTO blocks (height, hash, timestamp) \
    VALUES (?1, ?2, ?3) ON CONFLICT (height) DO UPDATE SET \
    hash = excluded.hash, timestamp = excluded.timestamp",
        params![height, hash.as_bytes(), time],
    )?;
    Ok(height)
}
