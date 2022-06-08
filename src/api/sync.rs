// Sync

use crate::coinconfig::CoinConfig;
use crate::scan::AMProgressCallback;
use crate::{BlockId, CTree, CompactTxStreamerClient};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;

const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub async fn coin_sync(
    coin: u8,
    get_tx: bool,
    anchor_offset: u32,
    progress_callback: impl Fn(u32) + Send + 'static,
) -> anyhow::Result<()> {
    let cb = Arc::new(Mutex::new(progress_callback));
    coin_sync_impl(coin, get_tx, DEFAULT_CHUNK_SIZE, anchor_offset, cb.clone()).await?;
    coin_sync_impl(coin, get_tx, DEFAULT_CHUNK_SIZE, 0, cb.clone()).await?;
    Ok(())
}

async fn coin_sync_impl(
    coin: u8,
    get_tx: bool,
    chunk_size: u32,
    target_height_offset: u32,
    progress_callback: AMProgressCallback,
) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    crate::scan::sync_async(
        c.coin_type,
        chunk_size,
        get_tx,
        &c.db_path,
        target_height_offset,
        progress_callback,
        &c.lwd_url,
    )
    .await?;
    Ok(())
}

pub async fn get_latest_height() -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    let last_height = crate::chain::get_latest_height(&mut client).await?;
    Ok(last_height)
}

pub async fn skip_to_last_height(coin: u8) -> anyhow::Result<()> {
    let c = if coin == 0xFF {
        CoinConfig::get_active()
    } else {
        CoinConfig::get(coin)
    };
    let mut client = c.connect_lwd().await?;
    let last_height = crate::chain::get_latest_height(&mut client).await?;
    fetch_and_store_tree_state(&mut client, last_height).await?;
    Ok(())
}

pub async fn rewind_to_height(height: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    c.db()?.trim_to_height(height)?;
    fetch_and_store_tree_state(&mut client, height).await?;
    Ok(())
}

async fn fetch_and_store_tree_state(
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let block_id = BlockId {
        height: height as u64,
        hash: vec![],
    };
    let block = client.get_block(block_id.clone()).await?.into_inner();
    let tree_state = client
        .get_tree_state(Request::new(block_id))
        .await?
        .into_inner();
    let tree = CTree::read(&*hex::decode(&tree_state.tree)?)?;
    c.db()?
        .store_block(height, &block.hash, block.time, &tree)?;
    Ok(())
}

pub async fn get_activation_date() -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    let date_time = crate::chain::get_activation_date(c.chain.network(), &mut client).await?;
    Ok(date_time)
}

pub async fn get_block_by_time(time: u32) -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    let date_time = crate::chain::get_block_by_time(c.chain.network(), &mut client, time).await?;
    Ok(date_time)
}
