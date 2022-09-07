// Sync

use crate::coinconfig::CoinConfig;
use crate::db::PlainNote;
use crate::scan::AMProgressCallback;
use crate::{AccountData, BlockId, CTree, CompactTxStreamerClient, DbAdapter};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;
use zcash_primitives::sapling::Note;

const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub async fn coin_sync(
    coin: u8,
    get_tx: bool,
    anchor_offset: u32,
    max_cost: u32,
    progress_callback: impl Fn(u32) + Send + 'static,
    cancel: &'static std::sync::Mutex<bool>,
) -> anyhow::Result<()> {
    let cb = Arc::new(Mutex::new(progress_callback));
    coin_sync_impl(
        coin,
        get_tx,
        DEFAULT_CHUNK_SIZE,
        anchor_offset,
        max_cost,
        cb.clone(),
        cancel,
    )
    .await?;
    coin_sync_impl(
        coin,
        get_tx,
        DEFAULT_CHUNK_SIZE,
        0,
        u32::MAX,
        cb.clone(),
        cancel,
    )
    .await?;
    Ok(())
}

async fn coin_sync_impl(
    coin: u8,
    get_tx: bool,
    chunk_size: u32,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback,
    cancel: &'static std::sync::Mutex<bool>,
) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    crate::scan::sync_async(
        c.coin_type,
        chunk_size,
        get_tx,
        c.db_path.as_ref().unwrap(),
        target_height_offset,
        max_cost,
        progress_callback,
        cancel,
        c.lwd_url.as_ref().unwrap(),
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

pub fn get_synced_height() -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    db.get_last_sync_height().map(|h| h.unwrap_or(0))
}

pub async fn skip_to_last_height(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let last_height = crate::chain::get_latest_height(&mut client).await?;
    fetch_and_store_tree_state(coin, &mut client, last_height).await?;
    Ok(())
}

// if exact = true, do not snap the height to a pre existing checkpoint
// and get the tree_state from the server
// this option is used when we rescan from a given height
// We ignore any transaction that occurred before
pub async fn rewind_to(height: u32) -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let height = c.db()?.trim_to_height(height)?;
    Ok(height)
}

pub async fn rescan_from(height: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    c.db()?.truncate_sync_data()?;
    let mut client = c.connect_lwd().await?;
    fetch_and_store_tree_state(c.coin, &mut client, height).await?;
    Ok(())
}

async fn fetch_and_store_tree_state(
    coin: u8,
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let block_id = BlockId {
        height: height as u64,
        hash: vec![],
    };
    let block = client.get_block(block_id.clone()).await?.into_inner();
    let tree_state = client
        .get_tree_state(Request::new(block_id))
        .await?
        .into_inner();
    let tree = CTree::read(&*hex::decode(&tree_state.sapling_tree)?)?;
    let db = c.db()?;
    DbAdapter::store_block(&db.connection, height, &block.hash, block.time, &tree)?;
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

pub fn trial_decrypt(
    height: u32,
    cmu: &[u8],
    epk: &[u8],
    ciphertext: &[u8],
) -> anyhow::Result<Option<Note>> {
    let c = CoinConfig::get_active();
    let AccountData { fvk, .. } = c.db().unwrap().get_account_info(c.id_account)?;
    let note =
        crate::scan::trial_decrypt_one(c.chain.network(), height, &fvk, cmu, epk, ciphertext)?;
    Ok(note)
}
