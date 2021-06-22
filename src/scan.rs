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
        w.write(&mut bb).unwrap();
        log::debug!("{}", hex::encode(&bb));
    }

    info!("Total: {} ms", total_start.elapsed().as_millis());

    Ok(())
}

struct Blocks(Vec<CompactBlock>);

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blocks of len {}", self.0.len())
    }
}

fn get_db_height(db_path: &str) -> anyhow::Result<u32> {
    let db = DbAdapter::new(db_path).unwrap();
    let height: u32 = db.get_last_height()?.unwrap_or_else(|| {
        crate::NETWORK
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into()
    });
    Ok(height)
}

pub async fn sync_async(ivk: &str, chunk_size: u32, db_path: &str, progress_callback: impl Fn(u32) + Send + 'static) -> anyhow::Result<()> {
    let db_path = db_path.to_string();
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
            .unwrap()
            .unwrap();
    let decrypter = DecryptNode::new(vec![fvk]);

    let mut client = connect_lightwalletd().await?;
    let start_height = get_db_height(&db_path)?;
    let end_height = get_latest_height(&mut client).await?;

    let (downloader_tx, mut download_rx) = mpsc::channel::<Range<u32>>(2);
    let (processor_tx, mut processor_rx) = mpsc::channel::<Blocks>(2);
    let (completed_tx, mut completed_rx) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        let mut client = connect_lightwalletd().await.unwrap();
        while let Some(range) = download_rx.recv().await {
            log::info!("{:?}", range);
            let blocks = download_chain(&mut client, range.start, range.end).await.unwrap();
            let b = Blocks(blocks);
            processor_tx.send(b).await.unwrap();
        }
        log::info!("download completed");
        drop(processor_tx);

        // Ok::<_, anyhow::Error>(())
    });

    tokio::spawn(async move {
        let db = DbAdapter::new(&db_path).unwrap();
        let (mut tree, mut witnesses) = db.get_tree().unwrap();
        let mut pos = tree.get_position();
        // let mut tree = CTree::new();
        // let mut witnesses: Vec<Witness> = vec![];
        while let Some(blocks) = processor_rx.recv().await {
            log::info!("{:?}", blocks);
            if blocks.0.is_empty() { continue }

            let dec_blocks = decrypter.decrypt_blocks(&blocks.0);
            for b in dec_blocks.iter() {
                if !b.notes.is_empty() {
                    log::info!("{} {}", b.height, b.notes.len());
                }
                for n in b.notes.iter() {
                    let p = pos + n.position;

                    let note = &n.note;
                    let id_tx = db.store_transaction(&n.txid, n.height, n.tx_index as u32).unwrap();
                    let rcm = note.rcm().to_repr();
                    let nf = note.nf(&n.ivk.fvk.vk, n.position as u64);

                    let id_note = db.store_received_note(&ReceivedNote {
                        height: n.height,
                        output_index: n.output_index as u32,
                        diversifier: n.pa.diversifier().0.to_vec(),
                        value: note.value,
                        rcm: rcm.to_vec(),
                        nf: nf.0.to_vec(),
                        is_change: false, // TODO: it's change the ovk matches too
                        memo: vec![],
                        spent: false
                    }, id_tx, n.position).unwrap();

                    let w = Witness::new(p as usize, id_note, Some(n.clone()));
                    witnesses.push(w);
                }
                pos += b.count_outputs as usize;
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

            let last_block = blocks.0.last().unwrap();
            let last_height = last_block.height as u32;
            db.store_block(last_height, &last_block.hash, &tree).unwrap();
            for w in witnesses.iter() {
                db.store_witnesses(w, last_height, w.id_note).unwrap();
            }

            progress_callback(blocks.0[0].height as u32);
        }

        progress_callback(end_height);
        log::info!("Witnesses {}", witnesses.len());
        drop(completed_tx);
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

    Ok(())
}

pub async fn latest_height() -> u32 {
    let mut client = connect_lightwalletd().await.unwrap();
    get_latest_height(&mut client).await.unwrap()
}
