use crate::chain::get_latest_height;
use crate::db::AccountViewKey;
use serde::Serialize;

use crate::chain::{download_chain, DecryptNode};
use crate::transaction::{get_transaction_details, retrieve_tx_info, GetTransactionDetailRequest};
use crate::{
    connect_lightwalletd, CoinConfig, CompactBlock, CompactSaplingOutput, CompactTx,
    DbAdapterBuilder,
};

use anyhow::anyhow;
use lazy_static::lazy_static;
use orchard::note_encryption::OrchardDomain;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{Network, Parameters};

use crate::orchard::{DecryptedOrchardNote, OrchardDecrypter, OrchardHasher, OrchardViewKey};
use crate::sapling::{DecryptedSaplingNote, SaplingDecrypter, SaplingHasher, SaplingViewKey};
use crate::sync::{Synchronizer, WarpProcessor};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use zcash_primitives::sapling::Note;

pub struct Blocks(pub Vec<CompactBlock>, pub usize);

lazy_static! {
    static ref DECRYPTER_RUNTIME: Runtime = Builder::new_multi_thread().build().unwrap();
}

#[derive(Debug)]
struct TxIdSet(Vec<u32>);

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blocks of len {}", self.0.len())
    }
}

#[derive(Clone, Serialize)]
pub struct Progress {
    pub height: u32,
    pub trial_decryptions: u64,
    pub downloaded: usize,
}

pub type ProgressCallback = dyn Fn(Progress) + Send;
pub type AMProgressCallback = Arc<Mutex<ProgressCallback>>;

#[derive(PartialEq, PartialOrd, Debug, Hash, Eq)]
pub struct TxIdHeight {
    id_tx: u32,
    height: u32,
    index: u32,
}

type SaplingSynchronizer = Synchronizer<
    Network,
    SaplingDomain<Network>,
    SaplingViewKey,
    DecryptedSaplingNote,
    SaplingDecrypter<Network>,
    SaplingHasher,
>;

type OrchardSynchronizer = Synchronizer<
    Network,
    OrchardDomain,
    OrchardViewKey,
    DecryptedOrchardNote,
    OrchardDecrypter<Network>,
    OrchardHasher,
>;

pub async fn sync_async<'a>(
    coin: u8,
    _chunk_size: u32,
    get_tx: bool, // TODO
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback, // TODO
    cancel: &'static std::sync::Mutex<bool>,
) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let ld_url = c.lwd_url.as_ref().unwrap().clone();
    let db_path = c.db_path.as_ref().unwrap().clone();

    let network = *c.chain.network();

    let mut client = connect_lightwalletd(&ld_url).await?;
    let (start_height, prev_hash, sapling_vks, orchard_vks) = {
        let db = c.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        let height = db.get_db_height()?;
        let hash = db.get_db_hash(height)?;
        let sapling_vks = db.get_sapling_fvks()?;
        let orchard_vks = db.get_orchard_fvks()?;
        (height, hash, sapling_vks, orchard_vks)
    };
    let end_height = get_latest_height(&mut client).await?;
    let end_height = (end_height - target_height_offset).max(start_height);
    if start_height >= end_height {
        return Ok(());
    }

    let mut height = start_height;
    let (blocks_tx, mut blocks_rx) = mpsc::channel::<Blocks>(1);
    tokio::spawn(async move {
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

    let db_builder = DbAdapterBuilder {
        coin_type: c.coin_type,
        db_path: db_path.clone(),
    };
    let mut progress = Progress {
        height: 0,
        trial_decryptions: 0,
        downloaded: 0,
    };

    while let Some(blocks) = blocks_rx.recv().await {
        let first_block = blocks.0.first().unwrap(); // cannot be empty because blocks are not
        log::info!("Height: {}", first_block.height);
        let last_block = blocks.0.last().unwrap();
        let last_hash: [u8; 32] = last_block.hash.clone().try_into().unwrap();
        let last_height = last_block.height as u32;
        let last_timestamp = last_block.time;

        progress.downloaded += blocks.1;
        progress.height = last_height;

        // Sapling
        log::info!("Sapling");
        {
            let decrypter = SaplingDecrypter::new(network);
            let warper = WarpProcessor::new(SaplingHasher::default());
            let mut synchronizer = SaplingSynchronizer::new(
                decrypter,
                warper,
                sapling_vks.clone(),
                db_builder.clone(),
                "sapling".to_string(),
            );
            synchronizer.initialize(height)?;
            progress.trial_decryptions += synchronizer.process(&blocks.0)? as u64;
        }

        // Orchard
        log::info!("Orchard");
        {
            let decrypter = OrchardDecrypter::new(network);
            let warper = WarpProcessor::new(OrchardHasher::new());
            let mut synchronizer = OrchardSynchronizer::new(
                decrypter,
                warper,
                orchard_vks.clone(),
                db_builder.clone(),
                "orchard".to_string(),
            );
            synchronizer.initialize(height)?;
            progress.trial_decryptions += synchronizer.process(&blocks.0)? as u64;
        }

        let db = db_builder.build()?;
        db.store_block_timestamp(last_height, &last_hash, last_timestamp)?;
        height = last_height;
        let cb = progress_callback.lock().await;
        cb(progress.clone());
    }

    if get_tx {
        get_transaction_details(coin).await?;
    }
    Ok(())
}

pub async fn latest_height(ld_url: &str) -> anyhow::Result<u32> {
    let mut client = connect_lightwalletd(ld_url).await?;
    let height = get_latest_height(&mut client).await?;
    Ok(height)
}

#[allow(dead_code)]
// test function
pub fn trial_decrypt_one(
    network: &Network,
    height: u32,
    fvk: &str,
    cmu: &[u8],
    epk: &[u8],
    ciphertext: &[u8],
) -> anyhow::Result<Option<Note>> {
    let mut vks = HashMap::new();
    let fvk =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk)
            .map_err(|_| anyhow!("Bech32 Decode Error"))?;
    let ivk = fvk.fvk.vk.ivk();
    vks.insert(
        0,
        AccountViewKey {
            fvk,
            ivk,
            viewonly: false,
        },
    );
    let dn = DecryptNode::new(vks);
    let block = vec![CompactBlock {
        proto_version: 0, // don't care about most of these fields
        height: height as u64,
        hash: vec![],
        prev_hash: vec![],
        time: 0,
        header: vec![],
        vtx: vec![CompactTx {
            index: 0,
            hash: vec![],
            fee: 0,
            spends: vec![],
            actions: vec![],
            outputs: vec![CompactSaplingOutput {
                cmu: cmu.to_vec(),
                epk: epk.to_vec(),
                ciphertext: ciphertext.to_vec(),
            }],
        }],
    }];
    let decrypted_block = dn.decrypt_blocks(network, block);
    let decrypted_block = decrypted_block.first().unwrap();
    let note = decrypted_block.notes.first().map(|dn| dn.note.clone());
    Ok(note)
}
