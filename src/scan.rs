use zcash_primitives::sapling::Node;
use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::{DecryptNode, LWD_URL, get_latest_height, download_chain, calculate_tree_state_v2, CompactBlock, NETWORK, connect_lightwalletd, Witness, advance_tree};
use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use tokio::sync::mpsc;
use std::time::Instant;
use std::ops::Range;
use log::info;
use crate::db::{DbAdapter, ReceivedNote};
use ff::PrimeField;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use crate::chain::Nf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn scan_all(fvks: &[ExtendedFullViewingKey]) -> anyhow::Result<()> {
    let decrypter = DecryptNode::new(fvks.to_vec());

    let total_start = Instant::now();
    let mut client = CompactTxStreamerClient::connect(LWD_URL).await?;
    let start_height: u32 = crate::NETWORK
        .activation_height(NetworkUpgrade::Sapling)
        .unwrap()
        .into();
    let end_height = get_latest_height(&mut client).await?;

    let start = Instant::now();
    let cbs = download_chain(&mut client, start_height, end_height).await?;
    info!("Download chain: {} ms", start.elapsed().as_millis());

    let start = Instant::now();
    let blocks = decrypter.decrypt_blocks(&cbs);
    info!("Decrypt Notes: {} ms", start.elapsed().as_millis());

    let witnesses = calculate_tree_state_v2(&cbs, &blocks);

    info!("# Witnesses {}", witnesses.len());
    for w in witnesses.iter() {
        let mut bb: Vec<u8> = vec![];
        w.write(&mut bb)?;
        log::debug!("{}", hex::encode(&bb));
    }

    info!("Total: {} ms", total_start.elapsed().as_millis());

    Ok(())
}

struct Blocks(Vec<CompactBlock>);
struct BlockMetadata {
    height: u32,
    hash: [u8; 32],
}

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blocks of len {}", self.0.len())
    }
}

pub type ProgressCallback = Arc<Mutex<dyn Fn(u32) + Send>>;

pub async fn sync_async(ivk: &str, chunk_size: u32, db_path: &str, target_height_offset: u32, progress_callback: ProgressCallback) -> anyhow::Result<()> {
    let db_path = db_path.to_string();
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
            ?.ok_or_else(|| anyhow::anyhow!("Invalid key"))?;
    let decrypter = DecryptNode::new(vec![fvk]);

    let mut client = connect_lightwalletd().await?;
    let start_height = {
        let db = DbAdapter::new(&db_path)?;
        db.get_db_height()?
    };
    let end_height = get_latest_height(&mut client).await?;
    let end_height = (end_height - target_height_offset).max(start_height);

    let (downloader_tx, mut download_rx) = mpsc::channel::<Range<u32>>(2);
    let (processor_tx, mut processor_rx) = mpsc::channel::<Blocks>(1);
    let (completed_tx, mut completed_rx) = mpsc::channel::<()>(1);

    let downloader = tokio::spawn(async move {
        let mut client = connect_lightwalletd().await?;
        while let Some(range) = download_rx.recv().await {
            log::info!("+ {:?}", range);
            let blocks = download_chain(&mut client, range.start, range.end).await?;
            log::info!("- {:?}", range);
            let b = Blocks(blocks);
            processor_tx.send(b).await?;
        }
        log::info!("download completed");
        drop(processor_tx);

        Ok::<_, anyhow::Error>(())
    });

    let proc_callback = progress_callback.clone();

    let processor = tokio::spawn(async move {
        let db = DbAdapter::new(&db_path)?;
        let mut nfs = db.get_nullifiers()?;

        let (mut tree, mut witnesses) = db.get_tree()?;
        let mut absolute_position_at_block_start = tree.get_position();
        let mut last_block: Option<BlockMetadata> = None;
        while let Some(blocks) = processor_rx.recv().await {
            log::info!("{:?}", blocks);
            if blocks.0.is_empty() { continue }

            let dec_blocks = decrypter.decrypt_blocks(&blocks.0);
            for b in dec_blocks.iter() {
                for nf in b.spends.iter() {
                    if let Some(&id) = nfs.get(nf) {
                        println!("NF FOUND {} {}", id, b.height);
                        db.mark_spent(id, b.height)?;
                    }
                }
                if !b.notes.is_empty() {
                    log::info!("{} {}", b.height, b.notes.len());
                    for nf in b.spends.iter() {
                        println!("{}", hex::encode(nf.0));
                    }
                }
                for n in b.notes.iter() {
                    let p = absolute_position_at_block_start + n.position_in_block;

                    let note = &n.note;
                    let id_tx = db.store_transaction(&n.txid, n.height, n.tx_index as u32)?;
                    let rcm = note.rcm().to_repr();
                    let nf = note.nf(&n.ivk.fvk.vk, p as u64);

                    let id_note = db.store_received_note(&ReceivedNote {
                        height: n.height,
                        output_index: n.output_index as u32,
                        diversifier: n.pa.diversifier().0.to_vec(),
                        value: note.value,
                        rcm: rcm.to_vec(),
                        nf: nf.0.to_vec(),
                        spent: None
                    }, id_tx, n.position_in_block)?;
                    nfs.insert(Nf(nf.0), id_note);

                    let w = Witness::new(p as usize, id_note, Some(n.clone()));
                    witnesses.push(w);
                }
                absolute_position_at_block_start += b.count_outputs as usize;
            }

            let mut nodes: Vec<Node> = vec![];
            for cb in blocks.0.iter() {
                for tx in cb.vtx.iter() {
                    for co in tx.outputs.iter() {
                        let mut cmu = [0u8; 32];
                        cmu.copy_from_slice(&co.cmu);
                        let node = Node::new(cmu);
                        nodes.push(node);
                    }
                }
            }

            let (new_tree, new_witnesses) = advance_tree(tree, &witnesses, &mut nodes);
            tree = new_tree;
            witnesses = new_witnesses;

            if let Some(block) = blocks.0.last() {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&block.hash);
                last_block = Some(BlockMetadata {
                    height: block.height as u32,
                    hash,
                });
            }
            let callback = proc_callback.lock().await;
            callback(blocks.0[0].height as u32);
        }

        // Finalize scan
        let (new_tree, new_witnesses) = advance_tree(tree, &witnesses, &mut []);
        tree = new_tree;
        witnesses = new_witnesses;

        if let Some(last_block) = last_block {
            let last_height = last_block.height;
            db.store_block(last_height, &last_block.hash, &tree)?;
            for w in witnesses.iter() {
                db.store_witnesses(w, last_height, w.id_note)?;
            }
        }

        // let callback = progress_callback.lock()?;
        // callback(end_height);
        log::info!("Witnesses {}", witnesses.len());
        drop(completed_tx);

        Ok::<_, anyhow::Error>(())
    });

    let mut height = start_height;
    while height < end_height {
        let s = height;
        let e = (height + chunk_size).min(end_height);
        let range = s..e;

        downloader_tx.send(range).await?;

        height = e;
    }
    drop(downloader_tx);
    log::info!("req downloading completed");

    completed_rx.recv().await;
    log::info!("completed");

    let res = tokio::try_join!(downloader, processor);
    match res {
        Ok((a, b)) => {
            if let Err(err) = a { log::info!("Downloader error = {}", err) }
            if let Err(err) = b { log::info!("Processor error = {}", err) }
        },
        Err(err) => log::info!("Sync error = {}", err),
    }

    Ok(())
}

pub async fn latest_height() -> anyhow::Result<u32> {
    let mut client = connect_lightwalletd().await?;
    let height = get_latest_height(&mut client).await?;
    Ok(height)
}
