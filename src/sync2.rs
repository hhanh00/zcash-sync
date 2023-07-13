use crate::chain::{Blocks, download_chain, get_latest_height};
use crate::orchard::{DecryptedOrchardNote, OrchardDecrypter, OrchardHasher, OrchardViewKey};
use crate::sapling::{DecryptedSaplingNote, SaplingDecrypter, SaplingHasher, SaplingViewKey};
// use crate::scan::{AMProgressCallback, Blocks, Progress};
use crate::sync::tree::TreeCheckpoint;
use crate::sync::{Synchronizer, WarpProcessor};
use crate::transaction::get_transaction_details;
use crate::{connect_lightwalletd, db};
use anyhow::Result;
use orchard::note_encryption::OrchardDomain;
use rusqlite::Connection;
use tokio::sync::mpsc;
use zcash_primitives::consensus::Network;
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use crate::db::data_generated::fb::ProgressT;

type ProgressCallback = dyn Fn(ProgressT);

type SaplingSynchronizer = Synchronizer<
    Network,
    SaplingDomain<Network>,
    SaplingViewKey,
    DecryptedSaplingNote,
    SaplingDecrypter<Network>,
    SaplingHasher,
    'S',
>;

type OrchardSynchronizer = Synchronizer<
    Network,
    OrchardDomain,
    OrchardViewKey,
    DecryptedOrchardNote,
    OrchardDecrypter<Network>,
    OrchardHasher,
    'O',
>;

pub async fn warp_sync_inner<'a>(
    network: Network,
    connection: &'a mut Connection,
    url: &'a str,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: &'a ProgressCallback,
    has_orchard: bool,
    cancel: mpsc::Receiver<()>,
) -> Result<u32> {
    let mut client = connect_lightwalletd(url).await?;
    let (start_height, prev_hash, vks) = {
        let height = db::checkpoint::get_last_sync_height(&network, connection, None)?;
        let block_hash = db::checkpoint::get_block(connection, height)?;
        let hash = block_hash.map(|bh| bh.hash);
        let vks = db::account::get_fvks(connection, &network)?;
        (height, hash, vks)
    };
    let end_height = get_latest_height(&mut client).await?;
    let end_height = (end_height - target_height_offset).max(start_height);
    log::info!("{start_height} - {end_height}");
    if start_height >= end_height {
        return Ok(start_height);
    }

    log::info!("Scan started");
    let mut height = start_height;
    let (blocks_tx, mut blocks_rx) = mpsc::channel::<Blocks>(1);
    let downloader = tokio::spawn(async move {
        download_chain(
            &mut client,
            start_height,
            end_height,
            prev_hash,
            max_cost,
            cancel,
            blocks_tx,
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    });

    let mut progress = ProgressT {
        height: 0,
        trial_decryptions: 0,
        downloaded: 0,
    };

    let sapling_vks: Vec<_> = vks
        .iter()
        .map(|vk| SaplingViewKey {
            account: vk.account,
            fvk: vk.sfvk.clone(),
            ivk: vk.sivk.clone(),
        })
        .collect();
    let orchard_vks: Vec<_> = vks
        .iter()
        .filter_map(|vk| {
            vk.ofvk.as_ref().map(|ofvk| OrchardViewKey {
                account: vk.account,
                fvk: ofvk.clone(),
            })
        })
        .collect();

    while let Some(blocks) = blocks_rx.recv().await {
        let first_block = blocks.0.first().unwrap(); // cannot be empty because blocks are not
        println!("Height: {}", first_block.height);
        let last_block = blocks.0.last().unwrap();
        let last_hash: [u8; 32] = last_block.hash.clone().try_into().unwrap();
        let last_height = last_block.height as u32;
        let last_timestamp = last_block.time;

        progress.downloaded += blocks.1 as u64;
        progress.height = last_height;

        let unspent_notes = db::checkpoint::list_unspent_nullifiers(connection)?;
        {
            // Sapling
            let mut sapling_synchronizer = {
                let TreeCheckpoint { tree, witnesses } =
                    db::checkpoint::get_tree::<'S'>(connection, height)?;
                let decrypter = SaplingDecrypter::new(network);
                let warper = WarpProcessor::new(SaplingHasher::default());
                SaplingSynchronizer::new_from_parts(
                    decrypter,
                    warper,
                    sapling_vks.clone(),
                    tree,
                    unspent_notes.clone(),
                    witnesses,
                    "sapling",
                )
            };
            let orchard_synchronizer = {
                if has_orchard {
                    // Orchard
                    let TreeCheckpoint { tree, witnesses } =
                        db::checkpoint::get_tree::<'O'>(connection, height)?;
                    let decrypter = OrchardDecrypter::new(network);
                    let warper = WarpProcessor::new(OrchardHasher::new());
                    let synchronizer = OrchardSynchronizer::new_from_parts(
                        decrypter,
                        warper,
                        orchard_vks.clone(),
                        tree,
                        unspent_notes,
                        witnesses,
                        "orchard",
                    );
                    Some(synchronizer)
                } else {
                    None
                }
            };

            let db_tx = connection.transaction()?;
            log::info!("Process sapling start");
            progress.trial_decryptions += sapling_synchronizer.process2(&blocks.0, &db_tx)? as u64;
            log::info!("Process sapling end");
            if let Some(mut orchard_synchronizer) = orchard_synchronizer {
                log::info!("Process orchard start");
                progress.trial_decryptions +=
                    orchard_synchronizer.process2(&blocks.0, &db_tx)? as u64;
                log::info!("Process orchard end");
            }
            db::checkpoint::store_block_timestamp(last_height, &last_hash, last_timestamp, &db_tx)?;
            db_tx.commit()?;

            height = last_height;
        }
        progress_callback(progress.clone());
    }
    log::info!("Scan finishing");

    downloader.await??;
    println!("Scan finished");

    db::checkpoint::purge_old_witnesses(connection, height)?;

    Ok(end_height)
}

/// Rewind to a previous block height
///
/// Height is snapped to a closest earlier checkpoint.
/// The effective height is returned
pub fn rewind_to(network: &Network, connection: &mut Connection, height: u32) -> Result<u32> {
    let checkpoint_height = crate::db::checkpoint::get_last_sync_height(network, connection, Some(height))?;
    let height = crate::db::checkpoint::trim_to_height(connection, checkpoint_height)?;
    Ok(height)
}

/// Synchronize from a given height
pub async fn rescan_from(network: &Network, connection: &mut Connection, url: &str, height: u32) -> anyhow::Result<()> {
    crate::db::purge::truncate_data(connection)?;
    crate::chain::fetch_tree_state(url, height).await?;
    Ok(())
}

