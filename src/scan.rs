use crate::builder::BlockProcessor;
use crate::chain::{DecryptedBlock, Nf, NfRef};
use crate::db::{AccountViewKey, DbAdapter, PlainNote, ReceivedNote};
use serde::Serialize;
use std::cmp::Ordering;

use crate::transaction::retrieve_tx_info;
use crate::{
    connect_lightwalletd, download_chain, get_latest_height, CompactBlock, CompactSaplingOutput,
    CompactTx, DecryptNode, Witness,
};
use ff::PrimeField;

use anyhow::anyhow;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::panic;
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_params::coin::{get_coin_chain, CoinType};
use zcash_primitives::consensus::{Network, Parameters};

use zcash_primitives::sapling::{Node, Note};

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
    height: u32,
    trial_decryptions: u64,
    downloaded: usize,
}

pub type ProgressCallback = dyn Fn(Progress) + Send;
pub type AMProgressCallback = Arc<Mutex<ProgressCallback>>;

#[derive(PartialEq, PartialOrd, Debug, Hash, Eq)]
pub struct TxIdHeight {
    id_tx: u32,
    height: u32,
    index: u32,
}

pub async fn sync_async(
    coin_type: CoinType,
    _chunk_size: u32,
    get_tx: bool,
    db_path: &str,
    target_height_offset: u32,
    max_cost: u32,
    progress_callback: AMProgressCallback,
    cancel: &'static std::sync::Mutex<bool>,
    ld_url: &str,
) -> anyhow::Result<()> {
    let ld_url = ld_url.to_owned();
    let db_path = db_path.to_string();
    let network = {
        let chain = get_coin_chain(coin_type);
        *chain.network()
    };

    let mut client = connect_lightwalletd(&ld_url).await?;
    let (start_height, prev_hash, vks) = {
        let db = DbAdapter::new(coin_type, &db_path)?;
        let height = db.get_db_height()?;
        let hash = db.get_db_hash(height)?;
        let vks = db.get_fvks()?;
        (height, hash, vks)
    };

    let end_height = get_latest_height(&mut client).await?;
    let end_height = (end_height - target_height_offset).max(start_height);
    if start_height >= end_height {
        return Ok(());
    }
    let n_ivks = vks.len();

    let decrypter = DecryptNode::new(vks);

    let (decryptor_tx, mut decryptor_rx) = mpsc::channel::<Blocks>(1);
    let (processor_tx, mut processor_rx) = mpsc::channel::<(Vec<DecryptedBlock>, usize)>(1);

    let db_path2 = db_path.clone();

    let downloader = tokio::spawn(async move {
        log::info!("download_scheduler");
        download_chain(
            &mut client,
            n_ivks,
            start_height,
            end_height,
            prev_hash,
            max_cost,
            decryptor_tx,
            cancel,
        )
        .await?;
        Ok::<_, anyhow::Error>(())
    });

    let proc_callback = progress_callback.clone();

    let decryptor = DECRYPTER_RUNTIME.spawn(async move {
        while let Some(blocks) = decryptor_rx.recv().await {
            let dec_blocks = decrypter.decrypt_blocks(&network, blocks.0); // this function may block
            let batch_decrypt_elapsed: usize = dec_blocks.iter().map(|b| b.elapsed).sum();
            log::info!("  Batch Decrypt: {} ms", batch_decrypt_elapsed);
            let _ = processor_tx.send((dec_blocks, blocks.1)).await;
        }
        Ok::<_, anyhow::Error>(())
    });

    let mut progress = Progress {
        height: 0,
        trial_decryptions: 0,
        downloaded: 0,
    };

    let processor = tokio::spawn(async move {
        let mut db = DbAdapter::new(coin_type, &db_path2)?;
        let mut nfs = db.get_nullifiers()?;

        while let Some((dec_blocks, blocks_size)) = processor_rx.recv().await {
            if dec_blocks.is_empty() {
                continue;
            }
            progress.downloaded += blocks_size;
            let (mut tree, witnesses) = db.get_tree()?;
            let mut bp = BlockProcessor::new(&tree, &witnesses);
            let mut absolute_position_at_block_start = tree.get_position();

            log::info!("start processing - {}", dec_blocks[0].height);
            log::info!("Time {:?}", chrono::offset::Local::now());
            let start = Instant::now();

            let mut new_ids_tx: HashMap<u32, TxIdHeight> = HashMap::new();
            let mut witnesses: Vec<Witness> = vec![];

            {
                // db tx scope
                let db_tx = db.begin_transaction()?;
                let outputs = dec_blocks
                    .iter()
                    .map(|db| db.count_outputs as usize)
                    .sum::<usize>();
                {
                    progress.trial_decryptions += (n_ivks * outputs) as u64;
                }
                for b in dec_blocks.iter() {
                    let mut my_nfs: Vec<Nf> = vec![];
                    for nf in b.spends.iter() {
                        if let Some(&nf_ref) = nfs.get(nf) {
                            log::info!("NF FOUND {} {}", nf_ref.id_note, b.height);
                            DbAdapter::mark_spent(nf_ref.id_note, b.height, &db_tx)?;
                            my_nfs.push(*nf);
                            nfs.remove(nf);
                        }
                    }
                    if !b.notes.is_empty() {
                        log::debug!("{} {}", b.height, b.notes.len());
                    }

                    for n in b.notes.iter() {
                        let p = absolute_position_at_block_start + n.position_in_block;

                        let note = &n.note;
                        let rcm = note.rcm().to_repr();
                        let nf = note.nf(&n.ivk.fvk.vk, p as u64);

                        let id_tx = DbAdapter::store_transaction(
                            &n.txid,
                            n.account,
                            n.height,
                            b.compact_block.time,
                            n.tx_index as u32,
                            &db_tx,
                        )?;
                        new_ids_tx.insert(
                            id_tx,
                            TxIdHeight {
                                id_tx,
                                height: n.height,
                                index: n.tx_index as u32,
                            },
                        );
                        let id_note = DbAdapter::store_received_note(
                            &ReceivedNote {
                                account: n.account,
                                height: n.height,
                                output_index: n.output_index as u32,
                                diversifier: n.pa.diversifier().0.to_vec(),
                                value: note.value,
                                rcm: rcm.to_vec(),
                                nf: nf.0.to_vec(),
                                spent: None,
                            },
                            id_tx,
                            n.position_in_block,
                            &db_tx,
                        )?;
                        DbAdapter::add_value(id_tx, note.value as i64, &db_tx)?;
                        nfs.insert(
                            Nf(nf.0),
                            NfRef {
                                id_note,
                                account: n.account,
                            },
                        );

                        let w = Witness::new(p as usize, id_note, Some(n.clone()));
                        witnesses.push(w);
                    }

                    if !my_nfs.is_empty() {
                        for (tx_index, tx) in b.compact_block.vtx.iter().enumerate() {
                            for cs in tx.spends.iter() {
                                let mut nf = [0u8; 32];
                                nf.copy_from_slice(&cs.nf);
                                let nf = Nf(nf);
                                if my_nfs.contains(&nf) {
                                    let (account, note_value) =
                                        DbAdapter::get_received_note_value(&nf, &db_tx)?;
                                    let txid = &*tx.hash;
                                    let id_tx = DbAdapter::store_transaction(
                                        txid,
                                        account,
                                        b.height,
                                        b.compact_block.time,
                                        tx_index as u32,
                                        &db_tx,
                                    )?;
                                    new_ids_tx.insert(
                                        id_tx,
                                        TxIdHeight {
                                            id_tx,
                                            height: b.height,
                                            index: tx_index as u32,
                                        },
                                    );
                                    DbAdapter::add_value(id_tx, -(note_value as i64), &db_tx)?;
                                }
                            }
                        }
                    }

                    absolute_position_at_block_start += b.count_outputs as usize;
                }
                log::info!("Dec end : {}", start.elapsed().as_millis());

                db_tx.commit()?;
            }

            let start = Instant::now();
            let mut nodes: Vec<Node> = vec![];
            for block in dec_blocks.iter() {
                let cb = &block.compact_block;
                for tx in cb.vtx.iter() {
                    for co in tx.outputs.iter() {
                        let mut cmu = [0u8; 32];
                        cmu.copy_from_slice(&co.cmu);
                        let node = Node::new(cmu);
                        nodes.push(node);
                    }
                }
            }

            if !nodes.is_empty() {
                bp.add_nodes(&mut nodes, &witnesses);
            }
            // println!("NOTES = {}", nodes.len());

            log::info!("Witness : {}", start.elapsed().as_millis());

            let start = Instant::now();
            if get_tx && !new_ids_tx.is_empty() {
                let mut ids: Vec<_> = new_ids_tx.into_iter().map(|(_, v)| v).collect();
                ids.sort_by(|a, b| {
                    let c = a.height.cmp(&b.height);
                    if c == Ordering::Equal {
                        return a.index.cmp(&b.index);
                    }
                    c
                });
                let ids: Vec<_> = ids.into_iter().map(|e| e.id_tx).collect();
                let mut client = connect_lightwalletd(&ld_url).await?;
                retrieve_tx_info(coin_type, &mut client, &db_path2, &ids).await?;
            }
            log::info!("Transaction Details : {}", start.elapsed().as_millis());

            let (new_tree, new_witnesses) = bp.finalize();
            tree = new_tree;
            witnesses = new_witnesses;

            if let Some(dec_block) = dec_blocks.last() {
                {
                    let block = &dec_block.compact_block;
                    let mut db_transaction = db.begin_transaction()?;
                    let height = block.height as u32;
                    for w in witnesses.iter() {
                        DbAdapter::store_witnesses(&db_transaction, w, height, w.id_note)?;
                    }
                    DbAdapter::store_block(
                        &mut db_transaction,
                        height,
                        &block.hash,
                        block.time,
                        &tree,
                    )?;
                    db_transaction.commit()?;
                    // db_transaction is dropped here
                }
                log::info!("progress: {}", dec_block.height);
                let callback = proc_callback.lock().await;
                callback(progress.clone());
            }
        }

        let callback = progress_callback.lock().await;
        callback(progress);

        db.purge_old_witnesses(end_height - 100)?;

        Ok::<_, anyhow::Error>(())
    });

    let res = tokio::try_join!(downloader, decryptor, processor);
    match res {
        Ok((d, dc, p)) => {
            if let Err(err) = d {
                log::info!("Downloader error = {}", err);
                return Err(err);
            }
            if let Err(err) = dc {
                log::info!("Decryptor error = {}", err);
                return Err(err);
            }
            if let Err(err) = p {
                log::info!("Processor error = {}", err);
                return Err(err);
            }
        }
        Err(err) => {
            log::info!("Sync error = {}", err);
            if err.is_panic() {
                panic::resume_unwind(err.into_panic());
            }
            anyhow::bail!("Join Error");
        }
    }

    log::info!("Sync completed");

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
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk)?
            .ok_or(anyhow!("Invalid FVK"))?;
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
