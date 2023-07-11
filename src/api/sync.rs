//! Warp Synchronize

use crate::coinconfig::CoinConfig;
use crate::scan::{AMProgressCallback, Progress};
use crate::sync::CTree;
use crate::{AccountData, BlockId, CompactTxStreamerClient, connect_lightwalletd, DbAdapter};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tonic::transport::Channel;
use tonic::Request;
use zcash_primitives::consensus::Network;
use zcash_primitives::sapling::Note;

/// Asynchronously perform warp sync
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `get_tx`: true to retrieve transaction details
/// * `anchor_offset`: minimum number of confirmations for note selection
/// * `max_cost`: tx that have a higher spending cost are excluded
/// * `progress_callback`: function callback during synchronization
/// * `cancel`: cancellation mutex, set to true to abort
pub async fn coin_sync(
    coin: u8,
    get_tx: bool,
    anchor_offset: u32,
    max_cost: u32,
    progress_callback: impl Fn(Progress) + Send + 'static,
    cancel: oneshot::Receiver<()>,
) -> anyhow::Result<u32> {
    let start_height = get_synced_height(coin)?;
    let (tx_cancel1, rx_cancel1) = mpsc::channel::<()>(1);
    let (tx_cancel2, rx_cancel2) = mpsc::channel::<()>(1);
    tokio::spawn(async move {
        if cancel.await.is_ok() {
            let _ = tx_cancel1.send(()).await;
            let _ = tx_cancel2.send(()).await;
        }
    });

    let p_cb = Arc::new(Mutex::new(progress_callback));
    coin_sync_impl(
        coin,
        get_tx,
        anchor_offset,
        max_cost,
        p_cb.clone(),
        rx_cancel1,
    )
    .await?;
    coin_sync_impl(coin, get_tx, 0, u32::MAX, p_cb.clone(), rx_cancel2).await?;
    let end_height = get_synced_height(coin)?;
    Ok(end_height - start_height)
}

async fn coin_sync_impl(
    coin: u8,
    get_tx: bool,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback,
    cancel: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    crate::scan::sync_async(
        coin,
        get_tx,
        target_height_offset,
        max_cost,
        progress_callback,
        cancel,
    )
    .await
}

/// Return the latest block height synchronized
pub fn get_synced_height(coin: u8) -> anyhow::Result<u32> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.get_last_sync_height().map(|h| h.unwrap_or(0))
}

/// Skip block synchronization and directly mark the chain synchronized
/// Used for new accounts that have no transaction history
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
pub async fn skip_to_last_height(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let last_height = crate::chain::get_latest_height(&mut client).await?;
    fetch_and_store_tree_state(coin, &mut client, last_height).await?;
    Ok(())
}

/// Rewind to a previous block height
///
/// Height is snapped to a closest earlier checkpoint.
/// The effective height is returned
pub fn rewind_to(height: u32) -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let height = c.db()?.trim_to_height(height)?;
    Ok(height)
}

/// Synchronize from a given height
pub async fn rescan_from(coin: u8, height: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
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
    let sapling_tree = CTree::read(&*hex::decode(&tree_state.sapling_tree)?)?; // retrieve orchard state and store it too
    let orchard_tree = if !tree_state.orchard_tree.is_empty() {
        CTree::read(&*hex::decode(&tree_state.orchard_tree)?)? // retrieve orchard state and store it too
    } else {
        CTree::new()
    };
    let db = c.db()?;
    DbAdapter::store_block(
        &db.connection,
        height,
        &block.hash,
        block.time,
        &sapling_tree,
        &orchard_tree,
    )?;
    Ok(())
}

/// Return the date of sapling activation
pub async fn get_activation_date(network: &Network, url: &str) -> anyhow::Result<u32> {
    let mut client = connect_lightwalletd(url).await?;
    let date_time = crate::chain::get_activation_date(network, &mut client).await?;
    Ok(date_time)
}

/// Return the block height for a given timestamp
/// # Arguments
/// * `time`: seconds since epoch
pub async fn get_block_by_time(network: &Network, url: &str, time: u32) -> anyhow::Result<u32> {
    let mut client = connect_lightwalletd(url).await?;
    let date_time = crate::chain::get_block_by_time(network, &mut client, time).await?;
    Ok(date_time)
}

#[allow(dead_code)]
fn trial_decrypt(
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
