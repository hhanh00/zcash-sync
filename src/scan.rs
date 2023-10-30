use crate::chain::get_latest_height;
use crate::db::AccountViewKey;

use crate::chain::{download_chain, DecryptNode};
use crate::db::data_generated::fb::*;
use crate::taddr::get_taddr_balance;
use crate::transaction::get_transaction_details;
use crate::{
    connect_lightwalletd, ChainError, CoinConfig, CompactBlock, CompactSaplingOutput, CompactTx,
    Connection, DbAdapter,
};

use anyhow::anyhow;
use lazy_static::lazy_static;
use orchard::note_encryption::OrchardDomain;
use rusqlite::{params, OptionalExtension};
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

#[derive(Clone)]
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

type SaplingSynchronizer<'a> = Synchronizer<
    'a,
    Network,
    SaplingDomain<Network>,
    SaplingViewKey,
    DecryptedSaplingNote,
    SaplingDecrypter<Network>,
    SaplingHasher,
>;

type OrchardSynchronizer<'a> = Synchronizer<
    'a,
    Network,
    OrchardDomain,
    OrchardViewKey,
    DecryptedOrchardNote,
    OrchardDecrypter<Network>,
    OrchardHasher,
>;

pub async fn sync_async<'a>(
    coin: u8,
    get_tx: bool,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback, // TODO
    cancel: &'static std::sync::Mutex<bool>,
) -> anyhow::Result<()> {
    let result = sync_async_inner(
        coin,
        get_tx,
        target_height_offset,
        max_cost,
        progress_callback,
        cancel,
    )
    .await;
    if let Err(ref e) = result {
        if let Some(ChainError::Reorg) = e.downcast_ref::<ChainError>() {
            log::info!("Drop latest checkpoint");
            let c = CoinConfig::get(coin);
            let mut db = c.db()?;
            db.drop_last_checkpoint()?;
        }
    }
    result
}

async fn sync_async_inner<'a>(
    coin: u8,
    get_tx: bool,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback, // TODO
    cancel: &'static std::sync::Mutex<bool>,
) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let ld_url = c.lwd_url.as_ref().unwrap().clone();

    let network = *c.chain.network();

    let mut client = connect_lightwalletd(&ld_url).await?;
    let (start_height, prev_hash, sapling_vks, orchard_vks) = {
        let db = c.db()?;
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

        let mut connection = c.connection();
        let db_tx = connection.transaction()?;
        // Sapling
        log::info!("Sapling");
        {
            let decrypter = SaplingDecrypter::new(network);
            let warper = WarpProcessor::new(SaplingHasher::default());
            let mut synchronizer = SaplingSynchronizer::new(
                decrypter,
                warper,
                sapling_vks.clone(),
                &db_tx,
                "sapling".to_string(),
            );
            synchronizer.initialize(height)?;
            progress.trial_decryptions += synchronizer.process(&blocks.0)? as u64;
        }

        if c.chain.has_unified() {
            // Orchard
            log::info!("Orchard");
            {
                let decrypter = OrchardDecrypter::new(network);
                let warper = WarpProcessor::new(OrchardHasher::new());
                let mut synchronizer = OrchardSynchronizer::new(
                    decrypter,
                    warper,
                    orchard_vks.clone(),
                    &db_tx,
                    "orchard".to_string(),
                );
                synchronizer.initialize(height)?;
                log::info!("Process orchard start");
                progress.trial_decryptions += synchronizer.process(&blocks.0)? as u64;
                log::info!("Process orchard end");
            }
        }

        DbAdapter::store_block_timestamp(&db_tx, last_height, &last_hash, last_timestamp)?;
        db_tx.commit()?;
        height = last_height;
        let cb = progress_callback.lock().await;
        cb(progress.clone());
    }

    downloader.await??;

    if get_tx {
        get_transaction_details(coin).await?;
    }
    let connection = c.connection();
    DbAdapter::purge_old_witnesses(&connection, height)?;

    Ok(())
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

pub async fn coin_trp_sync(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let connection = c.connection();
    let mut query_addresses = connection.prepare("SELECT account, address FROM taddrs")?;
    let res = query_addresses.query_map([], |r| {
        let account = r.get::<_, u32>(0)?;
        let address = r.get::<_, String>(1)?;
        Ok((account, address))
    })?;
    let mut update_balance =
        connection.prepare("UPDATE taddrs SET balance = ?2 WHERE account = ?1")?;
    for r in res {
        let (account, address) = r?;
        let balance = get_taddr_balance(&mut client, &address).await?;
        update_balance.execute(params![account, balance])?;
    }
    Ok(())
}

fn get_balance(
    connection: &Connection,
    account: u32,
    height: u32,
    orchard: u8,
    include_unconfirmed: bool
) -> anyhow::Result<u64> {
    let spend_predicate = if include_unconfirmed { "(spent IS NULL OR spent = 0)" } else { "spent IS NULL" };
    let balance = connection.query_row(
        &format!("SELECT SUM(value) FROM received_notes WHERE account = ?1 AND {spend_predicate} AND orchard = ?3 AND height <= ?2 AND (excluded IS NULL OR NOT excluded)"),
        params![account, height, orchard], |row| {
            let value = row.get::<_, Option<u64>>(0)?;
            Ok(value.unwrap_or(0))
        }).optional()?.unwrap_or(0);
    Ok(balance)
}

pub fn get_pool_balances(
    coin: u8,
    account: u32,
    confirmations: u32,
    include_unconfirmed: bool,
) -> anyhow::Result<PoolBalanceT> {
    let c = CoinConfig::get(coin);
    let connection = c.connection();
    let db = DbAdapter::new(c.coin_type, connection)?;
    let h = db.get_db_height()? - confirmations;
    let connection = db.inner();
    let sapling = get_balance(&connection, account, h, 0, include_unconfirmed)?;
    let orchard = get_balance(&connection, account, h, 1, include_unconfirmed)?;
    let transparent = connection
        .query_row(
            "SELECT balance FROM taddrs WHERE account = ?1",
            [account],
            |r| r.get::<_, Option<u64>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or_default();

    Ok(PoolBalanceT {
        transparent,
        sapling,
        orchard,
    })
}
