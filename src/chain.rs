use crate::commitment::{CTree, Witness};
use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::lw_rpc::*;
use crate::{advance_tree, NETWORK, LWD_URL};
use ff::PrimeField;
use group::GroupEncoding;
use log::info;
use rayon::prelude::*;
use std::time::Instant;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::Request;
use thiserror::Error;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{BlockHeight, NetworkUpgrade, Parameters};
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::note_encryption::try_sapling_compact_note_decryption;
use zcash_primitives::sapling::{Node, Note, PaymentAddress};
use zcash_primitives::transaction::components::sapling::CompactOutputDescription;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use std::collections::HashMap;

const MAX_CHUNK: u32 = 50000;

pub async fn get_latest_height(
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<u32> {
    let chainspec = ChainSpec {};
    let rep = client.get_latest_block(Request::new(chainspec)).await?;
    let block_id = rep.into_inner();
    Ok(block_id.height as u32)
}

#[derive(Error, Debug)]
pub enum ChainError {
    #[error("Blockchain reorganization")]
    Reorg,
    #[error("Synchronizer busy")]
    Busy,
}

/* download [start_height+1, end_height] inclusive */
pub async fn download_chain(
    client: &mut CompactTxStreamerClient<Channel>,
    start_height: u32,
    end_height: u32,
    mut prev_hash: Option<[u8; 32]>,
) -> anyhow::Result<Vec<CompactBlock>> {
    let mut cbs: Vec<CompactBlock> = Vec::new();
    let mut s = start_height + 1;
    while s <= end_height {
        let e = (s + MAX_CHUNK - 1).min(end_height);
        let range = BlockRange {
            start: Some(BlockId {
                height: s as u64,
                hash: vec![],
            }),
            end: Some(BlockId {
                height: e as u64,
                hash: vec![],
            }),
        };
        let mut block_stream = client
            .get_block_range(Request::new(range))
            .await?
            .into_inner();
        while let Some(block) = block_stream.message().await? {
            if prev_hash.is_some() && block.prev_hash.as_slice() != prev_hash.unwrap() {
                anyhow::bail!(ChainError::Reorg);
            }
            let mut ph = [0u8; 32];
            ph.copy_from_slice(&block.hash);
            prev_hash = Some(ph);
            cbs.push(block);
        }
        s = e + 1;
    }
    Ok(cbs)
}

pub struct DecryptNode {
    fvks: HashMap<u32, ExtendedFullViewingKey>,
}

#[derive(Eq, Hash, PartialEq, Copy, Clone)]
pub struct Nf(pub [u8; 32]);

#[derive(Copy, Clone)]
pub struct NfRef {
    pub id_note: u32,
    pub account: u32
}

pub struct DecryptedBlock<'a> {
    pub height: u32,
    pub notes: Vec<DecryptedNote>,
    pub count_outputs: u32,
    pub spends: Vec<Nf>,
    pub compact_block: &'a CompactBlock,
}

#[derive(Clone)]
pub struct DecryptedNote {
    pub account: u32,
    pub ivk: ExtendedFullViewingKey,
    pub note: Note,
    pub pa: PaymentAddress,
    pub position_in_block: usize,

    pub height: u32,
    pub txid: Vec<u8>,
    pub tx_index: usize,
    pub output_index: usize,
}

pub fn to_output_description(co: &CompactOutput) -> CompactOutputDescription {
    let mut cmu = [0u8; 32];
    cmu.copy_from_slice(&co.cmu);
    let cmu = bls12_381::Scalar::from_repr(cmu).unwrap();
    let mut epk = [0u8; 32];
    epk.copy_from_slice(&co.epk);
    let epk = jubjub::ExtendedPoint::from_bytes(&epk).unwrap();
    let od = CompactOutputDescription {
        epk,
        cmu,
        enc_ciphertext: co.ciphertext.to_vec(),
    };
    od
}

fn decrypt_notes<'a>(
    block: &'a CompactBlock,
    fvks: &HashMap<u32, ExtendedFullViewingKey>,
) -> DecryptedBlock<'a> {
    let height = BlockHeight::from_u32(block.height as u32);
    let mut count_outputs = 0u32;
    let mut spends: Vec<Nf> = vec![];
    let mut notes: Vec<DecryptedNote> = vec![];
    for (tx_index, vtx) in block.vtx.iter().enumerate() {
        for cs in vtx.spends.iter() {
            let mut nf = [0u8; 32];
            nf.copy_from_slice(&cs.nf);
            spends.push(Nf(nf));
        }

        for (output_index, co) in vtx.outputs.iter().enumerate() {
            for (&account, fvk) in fvks.iter() {
                let ivk = &fvk.fvk.vk.ivk();
                let od = to_output_description(co);
                if let Some((note, pa)) =
                    try_sapling_compact_note_decryption(&NETWORK, height, ivk, &od)
                {
                    notes.push(DecryptedNote {
                        account,
                        ivk: fvk.clone(),
                        note,
                        pa,
                        position_in_block: count_outputs as usize,
                        height: block.height as u32,
                        tx_index,
                        txid: vtx.hash.clone(),
                        output_index,
                    });
                }
            }
            count_outputs += 1;
        }
    }
    DecryptedBlock {
        height: block.height as u32,
        spends,
        notes,
        count_outputs,
        compact_block: block,
    }
}

impl DecryptNode {
    pub fn new(fvks: HashMap<u32, ExtendedFullViewingKey>) -> DecryptNode {
        DecryptNode { fvks }
    }

    pub fn decrypt_blocks<'a>(&self, blocks: &'a [CompactBlock]) -> Vec<DecryptedBlock<'a>> {
        let mut decrypted_blocks: Vec<DecryptedBlock> = blocks
            .par_iter()
            .map(|b| decrypt_notes(b, &self.fvks))
            .collect();
        decrypted_blocks.sort_by(|a, b| a.height.cmp(&b.height));
        decrypted_blocks
    }
}

#[allow(dead_code)]
async fn get_tree_state(client: &mut CompactTxStreamerClient<Channel>, height: u32) -> String {
    let block_id = BlockId {
        height: height as u64,
        hash: vec![],
    };
    let rep = client
        .get_tree_state(Request::new(block_id))
        .await
        .unwrap()
        .into_inner();
    rep.tree
}

pub async fn send_transaction(
    client: &mut CompactTxStreamerClient<Channel>,
    raw_tx: &[u8],
    height: u32,
) -> anyhow::Result<String> {
    let raw_tx = RawTransaction {
        data: raw_tx.to_vec(),
        height: height as u64,
    };
    let rep = client
        .send_transaction(Request::new(raw_tx))
        .await?
        .into_inner();
    let code = rep.error_code;
    if code == 0 {
        Ok(rep.error_message)
    }
    else {
        Err(anyhow::anyhow!(rep.error_message))
    }
}

/* Using the IncrementalWitness */
#[allow(dead_code)]
fn calculate_tree_state_v1(
    cbs: &[CompactBlock],
    blocks: &[DecryptedBlock],
    height: u32,
    mut tree_state: CommitmentTree<Node>,
) -> Vec<IncrementalWitness<Node>> {
    let mut witnesses: Vec<IncrementalWitness<Node>> = vec![];
    for (cb, block) in cbs.iter().zip(blocks) {
        assert_eq!(cb.height as u32, block.height);
        if block.height < height {
            continue;
        } // skip before height
        let mut notes = block.notes.iter();
        let mut n = notes.next();
        let mut i = 0usize;
        for tx in cb.vtx.iter() {
            for co in tx.outputs.iter() {
                let mut cmu = [0u8; 32];
                cmu.copy_from_slice(&co.cmu);
                let node = Node::new(cmu);
                tree_state.append(node).unwrap();
                for w in witnesses.iter_mut() {
                    w.append(node).unwrap();
                }
                if let Some(nn) = n {
                    if i == nn.position_in_block {
                        let w = IncrementalWitness::from_tree(&tree_state);
                        witnesses.push(w);
                        n = notes.next();
                    }
                }
                i += 1;
            }
        }
    }
    // let mut bb: Vec<u8> = vec![];
    // tree_state.write(&mut bb).unwrap();
    // hex::encode(bb)

    witnesses
}

pub fn calculate_tree_state_v2(cbs: &[CompactBlock], blocks: &[DecryptedBlock]) -> Vec<Witness> {
    let mut p = 0usize;
    let mut nodes: Vec<Node> = vec![];
    let mut positions: Vec<usize> = vec![];

    let start = Instant::now();
    for (cb, block) in cbs.iter().zip(blocks) {
        assert_eq!(cb.height as u32, block.height);
        if !block.notes.is_empty() {
            println!("{} {}", block.height, block.notes.len());
        }
        let mut notes = block.notes.iter();
        let mut n = notes.next();
        let mut i = 0usize;
        for tx in cb.vtx.iter() {
            for co in tx.outputs.iter() {
                let mut cmu = [0u8; 32];
                cmu.copy_from_slice(&co.cmu);
                let node = Node::new(cmu);
                nodes.push(node);

                if let Some(nn) = n {
                    if i == nn.position_in_block {
                        positions.push(p);
                        n = notes.next();
                    }
                }
                i += 1;
                p += 1;
            }
        }
    }
    info!(
        "Build CMU list: {} ms - {} nodes",
        start.elapsed().as_millis(),
        nodes.len()
    );

    let witnesses: Vec<_> = positions
        .iter()
        .map(|p| Witness::new(*p, 0, None))
        .collect();
    let (_, new_witnesses) = advance_tree(&CTree::new(), &witnesses, &mut nodes, true);
    info!("Tree State & Witnesses: {} ms", start.elapsed().as_millis());
    new_witnesses
}

pub async fn connect_lightwalletd() -> anyhow::Result<CompactTxStreamerClient<Channel>> {
    let mut channel = tonic::transport::Channel::from_shared(LWD_URL)?;
    if LWD_URL.starts_with("https") {
        let pem = include_bytes!("ca.pem");
        let ca = Certificate::from_pem(pem);
        let tls = ClientTlsConfig::new().ca_certificate(ca);
        channel = channel.tls_config(tls)?;
    }
    let client = CompactTxStreamerClient::connect(channel).await?;
    Ok(client)
}

pub async fn sync(fvks: &HashMap<u32, String>) -> anyhow::Result<()> {
    let fvks: HashMap<_, _> = fvks.iter().map(|(&account, fvk)| {
        let fvk =
            decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap()
                .unwrap();
        (account, fvk)
    }).collect();
    let decrypter = DecryptNode::new(fvks);
    let mut client = connect_lightwalletd().await?;
    let start_height: u32 = crate::NETWORK
        .activation_height(NetworkUpgrade::Sapling)
        .unwrap()
        .into();
    let end_height = get_latest_height(&mut client).await?;

    let start = Instant::now();
    let cbs = download_chain(&mut client, start_height, end_height, None).await?;
    eprintln!("Download chain: {} ms", start.elapsed().as_millis());

    let start = Instant::now();
    let blocks = decrypter.decrypt_blocks(&cbs);
    eprintln!("Decrypt Notes: {} ms", start.elapsed().as_millis());

    let start = Instant::now();
    let witnesses = calculate_tree_state_v2(&cbs, &blocks);
    eprintln!("Tree State & Witnesses: {} ms", start.elapsed().as_millis());

    eprintln!("# Witnesses {}", witnesses.len());
    for w in witnesses.iter() {
        let mut bb: Vec<u8> = vec![];
        w.write(&mut bb).unwrap();
        log::info!("{}", hex::encode(&bb));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::LWD_URL;
    #[allow(unused_imports)]
    use crate::chain::{
        calculate_tree_state_v1, calculate_tree_state_v2, download_chain, get_latest_height,
        get_tree_state, DecryptNode,
    };
    use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
    use crate::NETWORK;
    use dotenv;
    use std::time::Instant;
    use zcash_client_backend::encoding::decode_extended_full_viewing_key;
    use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
    use zcash_primitives::zip32::ExtendedFullViewingKey;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_get_latest_height() -> anyhow::Result<()> {
        let mut client = CompactTxStreamerClient::connect(LWD_URL).await?;
        let height = get_latest_height(&mut client).await?;
        assert!(height > 1288000);
        Ok(())
    }

    #[tokio::test]
    async fn test_download_chain() -> anyhow::Result<()> {
        dotenv::dotenv().unwrap();
        let fvk = dotenv::var("FVK").unwrap();

        let mut fvks: HashMap<u32, ExtendedFullViewingKey> = HashMap::new();
        let fvk =
            decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap()
                .unwrap();
        fvks.insert(1, fvk);
        let decrypter = DecryptNode::new(fvks);
        let mut client = CompactTxStreamerClient::connect(LWD_URL).await?;
        let start_height: u32 = crate::NETWORK
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into();
        let end_height = get_latest_height(&mut client).await?;

        let start = Instant::now();
        let cbs = download_chain(&mut client, start_height, end_height, None).await?;
        eprintln!("Download chain: {} ms", start.elapsed().as_millis());

        let start = Instant::now();
        let blocks = decrypter.decrypt_blocks(&cbs);
        eprintln!("Decrypt Notes: {} ms", start.elapsed().as_millis());

        // no need to calculate tree before the first note if we can
        // get it from the server
        // disabled because I want to see the performance of a complete scan

        // let first_block = blocks.iter().find(|b| !b.notes.is_empty()).unwrap();
        // let height = first_block.height - 1;
        // let tree_state = get_tree_state(&mut client, height).await;
        // let tree_state = hex::decode(tree_state).unwrap();
        // let tree_state = CommitmentTree::<Node>::read(&*tree_state).unwrap();

        // let witnesses = calculate_tree_state(&cbs, &blocks, 0, tree_state);

        let start = Instant::now();
        let witnesses = calculate_tree_state_v2(&cbs, &blocks);
        eprintln!("Tree State & Witnesses: {} ms", start.elapsed().as_millis());

        eprintln!("# Witnesses {}", witnesses.len());
        for w in witnesses.iter() {
            let mut bb: Vec<u8> = vec![];
            w.write(&mut bb).unwrap();
            log::info!("{}", hex::encode(&bb));
        }

        Ok(())
    }
}
