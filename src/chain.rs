use crate::commitment::{CTree, Witness};
use crate::db::AccountViewKey;
use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::lw_rpc::*;
use crate::scan::Blocks;
use crate::{advance_tree, has_cuda};
use ff::PrimeField;
use futures::{future, FutureExt};
use lazy_static::lazy_static;
use log::info;
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use rayon::prelude::*;
use std::collections::HashMap;
use std::convert::TryInto;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use sysinfo::{System, SystemExt};
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tonic::Request;
use zcash_note_encryption::batch::try_compact_note_decryption;
use zcash_note_encryption::{Domain, EphemeralKeyBytes, ShieldedOutput, COMPACT_NOTE_SIZE};
use zcash_primitives::consensus::{BlockHeight, Network, NetworkUpgrade, Parameters};
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use zcash_primitives::sapling::{Node, Note, PaymentAddress};
use zcash_primitives::transaction::components::sapling::CompactOutputDescription;
use zcash_primitives::zip32::ExtendedFullViewingKey;

#[cfg(feature = "cuda")]
use crate::gpu::cuda::{CudaProcessor, CUDA_CONTEXT};
#[cfg(feature = "apple_metal")]
use crate::gpu::metal::MetalProcessor;
use crate::gpu::{trial_decrypt, USE_GPU};

lazy_static! {
    pub static ref DOWNLOADED_BYTES: Mutex<usize> = Mutex::new(0);
    pub static ref TRIAL_DECRYPTIONS: Mutex<usize> = Mutex::new(0);
}

pub async fn get_latest_height(
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<u32> {
    let chainspec = ChainSpec {};
    let rep = client.get_latest_block(Request::new(chainspec)).await?;
    let block_id = rep.into_inner();
    Ok(block_id.height as u32)
}

pub async fn get_activation_date(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<u32> {
    let height = network.activation_height(NetworkUpgrade::Sapling).unwrap();
    let time = get_block_date(client, u32::from(height)).await?;
    Ok(time)
}

pub async fn get_block_date(
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
) -> anyhow::Result<u32> {
    let block = client
        .get_block(Request::new(BlockId {
            height: height as u64,
            hash: vec![],
        }))
        .await?
        .into_inner();
    Ok(block.time)
}

pub async fn get_block_by_time(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    time: u32,
) -> anyhow::Result<u32> {
    let mut start = u32::from(network.activation_height(NetworkUpgrade::Sapling).unwrap());
    let mut end = get_latest_height(client).await?;
    if time <= get_block_date(client, start).await? {
        return Ok(0);
    }
    if time >= get_block_date(client, end).await? {
        return Ok(end);
    }
    let mut block_mid;
    while end - start >= 1000 {
        block_mid = (start + end) / 2;
        let mid = get_block_date(client, block_mid).await?;
        if time < mid {
            end = block_mid - 1;
        } else if time > mid {
            start = block_mid + 1;
        } else {
            return Ok(block_mid);
        }
    }
    Ok(start)
}

#[derive(Error, Debug)]
pub enum ChainError {
    #[error("Blockchain reorganization")]
    Reorg,
    #[error("Synchronizer busy")]
    Busy,
}

fn get_mem_per_output() -> usize {
    if cfg!(feature = "cuda") {
        250
    } else {
        5
    }
}

#[cfg(feature = "cuda")]
fn get_available_memory() -> anyhow::Result<usize> {
    let cuda = CUDA_CONTEXT.lock().unwrap();
    if let Some(cuda) = cuda.as_ref() {
        cuda.total_memory()
    } else {
        get_system_available_memory()
    }
}

#[cfg(not(feature = "cuda"))]
fn get_available_memory() -> anyhow::Result<usize> {
    get_system_available_memory()
}

fn get_system_available_memory() -> anyhow::Result<usize> {
    let mut sys = System::new();
    sys.refresh_memory();
    let mem_available = sys.available_memory() as usize;
    Ok(mem_available)
}

const MAX_OUTPUTS_PER_CHUNKS: usize = 200_000;

/* download [start_height+1, end_height] inclusive */
#[allow(unused_variables)]
pub async fn download_chain(
    client: &mut CompactTxStreamerClient<Channel>,
    n_ivks: usize,
    start_height: u32,
    end_height: u32,
    mut prev_hash: Option<[u8; 32]>,
    max_cost: u32,
    blocks_tx: Sender<Blocks>,
    cancel: &'static Mutex<bool>,
) -> anyhow::Result<()> {
    let outputs_per_chunk = get_available_memory()? / get_mem_per_output();
    let outputs_per_chunk = outputs_per_chunk.min(MAX_OUTPUTS_PER_CHUNKS);
    log::info!("Outputs per chunk = {}", outputs_per_chunk);
    log::info!("max_cost = {}", max_cost);

    let mut output_count = 0;
    let mut cbs: Vec<CompactBlock> = Vec::new();
    let range = BlockRange {
        start: Some(BlockId {
            height: (start_height + 1) as u64,
            hash: vec![],
        }),
        end: Some(BlockId {
            height: end_height as u64,
            hash: vec![],
        }),
        spam_filter_threshold: max_cost as u64,
    };
    *DOWNLOADED_BYTES.lock().unwrap() = 0;
    *TRIAL_DECRYPTIONS.lock().unwrap() = 0;
    let mut block_stream = client
        .get_block_range(Request::new(range))
        .await?
        .into_inner();
    while let Some(mut block) = block_stream.message().await? {
        let block_size = get_block_size(&block);
        {
            let mut downloaded = DOWNLOADED_BYTES.lock().unwrap();
            *downloaded += block_size;
        }
        let c = *cancel.lock().unwrap();
        if c {
            log::info!("Canceling download");
            break;
        }
        if prev_hash.is_some() && block.prev_hash.as_slice() != prev_hash.unwrap() {
            log::warn!(
                "Reorg: {} != {}",
                hex::encode(block.prev_hash.as_slice()),
                hex::encode(prev_hash.unwrap())
            );
            anyhow::bail!(ChainError::Reorg);
        }
        let mut ph = [0u8; 32];
        ph.copy_from_slice(&block.hash);
        prev_hash = Some(ph);
        for tx in block.vtx.iter_mut() {
            tx.actions.clear(); // don't need Orchard actions
            let mut skipped = false;
            if tx.outputs.len() > max_cost as usize {
                for co in tx.outputs.iter_mut() {
                    co.epk.clear();
                    co.ciphertext.clear();
                    skipped = true;
                }
            }
            if skipped {
                log::debug!("Output skipped {}", tx.outputs.len());
            }
        }

        let block_output_count: usize = block.vtx.iter().map(|tx| tx.outputs.len()).sum();
        if output_count + block_output_count > outputs_per_chunk {
            // output
            let out = cbs;
            cbs = Vec::new();
            blocks_tx.send(Blocks(out)).await.unwrap();
            output_count = 0;
        }

        cbs.push(block);
        output_count += block_output_count;
    }
    let _ = blocks_tx.send(Blocks(cbs)).await;
    Ok(())
}

fn get_block_size(block: &CompactBlock) -> usize {
    block
        .vtx
        .iter()
        .map(|tx| {
            tx.spends.len() * 32
                + tx.outputs.len() * (32 * 2 + 52)
                + tx.actions.len() * (32 * 3 + 52)
                + 8
                + 32
                + 4
        })
        .sum::<usize>()
        + 16
        + 32 * 2
}

pub struct DecryptNode {
    vks: HashMap<u32, AccountViewKey>,
}

#[derive(Eq, Hash, PartialEq, Copy, Clone)]
pub struct Nf(pub [u8; 32]);

#[derive(Copy, Clone)]
pub struct NfRef {
    pub id_note: u32,
    pub account: u32,
}

pub struct DecryptedBlock {
    pub height: u32,
    pub notes: Vec<DecryptedNote>,
    pub count_outputs: u32,
    pub spends: Vec<Nf>,
    pub compact_block: CompactBlock,
    pub elapsed: usize,
}

#[derive(Clone)]
pub struct DecryptedNote {
    pub account: u32,
    pub ivk: ExtendedFullViewingKey,
    pub note: Note,
    pub pa: PaymentAddress,
    pub position_in_block: usize,
    pub viewonly: bool,

    pub height: u32,
    pub txid: Vec<u8>,
    pub tx_index: usize,
    pub output_index: usize,
}

pub fn to_output_description(co: &CompactSaplingOutput) -> CompactOutputDescription {
    let cmu: [u8; 32] = co.cmu.clone().try_into().unwrap();
    let cmu = bls12_381::Scalar::from_repr(cmu).unwrap();
    let epk: [u8; 32] = co.epk.clone().try_into().unwrap();
    let enc_ciphertext: [u8; 52] = co.ciphertext.clone().try_into().unwrap();

    CompactOutputDescription {
        ephemeral_key: EphemeralKeyBytes::from(epk),
        cmu,
        enc_ciphertext,
    }
}

struct AccountOutput<'a, N: Parameters> {
    epk: EphemeralKeyBytes,
    cmu: <SaplingDomain<N> as Domain>::ExtractedCommitmentBytes,
    ciphertext: [u8; COMPACT_NOTE_SIZE],
    tx_index: usize,
    output_index: usize,
    block_output_index: usize,
    vtx: &'a CompactTx,
    _phantom: PhantomData<N>,
}

impl<'a, N: Parameters> AccountOutput<'a, N> {
    fn new(
        tx_index: usize,
        output_index: usize,
        block_output_index: usize,
        vtx: &'a CompactTx,
        co: &CompactSaplingOutput,
    ) -> Self {
        let mut epk_bytes = [0u8; 32];
        epk_bytes.copy_from_slice(&co.epk);
        let epk = EphemeralKeyBytes::from(epk_bytes);
        let mut cmu_bytes = [0u8; 32];
        cmu_bytes.copy_from_slice(&co.cmu);
        let cmu = cmu_bytes;
        let mut ciphertext_bytes = [0u8; COMPACT_NOTE_SIZE];
        ciphertext_bytes.copy_from_slice(&co.ciphertext);

        AccountOutput {
            tx_index,
            output_index,
            block_output_index,
            vtx,
            epk,
            cmu,
            ciphertext: ciphertext_bytes,
            _phantom: PhantomData::default(),
        }
    }
}

impl<'a, N: Parameters> ShieldedOutput<SaplingDomain<N>, COMPACT_NOTE_SIZE>
    for AccountOutput<'a, N>
{
    fn ephemeral_key(&self) -> EphemeralKeyBytes {
        self.epk.clone()
    }
    fn cmstar_bytes(&self) -> <SaplingDomain<N> as Domain>::ExtractedCommitmentBytes {
        self.cmu
    }
    fn enc_ciphertext(&self) -> &[u8; COMPACT_NOTE_SIZE] {
        &self.ciphertext
    }
}

fn decrypt_notes<'a, N: Parameters>(
    network: &N,
    block: CompactBlock,
    vks: &[(&u32, &AccountViewKey)],
) -> DecryptedBlock {
    let height = BlockHeight::from_u32(block.height as u32);
    let mut count_outputs = 0u32;
    let mut spends: Vec<Nf> = vec![];
    let mut notes: Vec<DecryptedNote> = vec![];
    let vvks: Vec<_> = vks.iter().map(|vk| vk.1.ivk.clone()).collect();
    let mut outputs: Vec<(SaplingDomain<N>, AccountOutput<N>)> = vec![];
    for (tx_index, vtx) in block.vtx.iter().enumerate() {
        for cs in vtx.spends.iter() {
            let mut nf = [0u8; 32];
            nf.copy_from_slice(&cs.nf);
            spends.push(Nf(nf));
        }

        if let Some(fco) = vtx.outputs.first() {
            if !fco.epk.is_empty() {
                for (output_index, co) in vtx.outputs.iter().enumerate() {
                    let domain = SaplingDomain::<N>::for_height(network.clone(), height);
                    let output = AccountOutput::<N>::new(
                        tx_index,
                        output_index,
                        count_outputs as usize,
                        vtx,
                        co,
                    );
                    outputs.push((domain, output));

                    count_outputs += 1;
                }
            } else {
                // we filter by transaction, therefore if one epk is empty, every epk is empty
                // log::info!("Spam Filter tx {}", hex::encode(&vtx.hash));
                count_outputs += vtx.outputs.len() as u32;
            }
        }
    }

    let start = Instant::now();
    let notes_decrypted =
        try_compact_note_decryption::<SaplingDomain<N>, AccountOutput<N>>(&vvks, &outputs);
    let elapsed = start.elapsed().as_millis() as usize;

    for (pos, opt_note) in notes_decrypted.iter().enumerate() {
        if let Some((note, pa)) = opt_note {
            let vk = &vks[pos / outputs.len()];
            let output = &outputs[pos % outputs.len()];
            notes.push(DecryptedNote {
                account: *vk.0,
                ivk: vk.1.fvk.clone(),
                note: note.clone(),
                pa: pa.clone(),
                viewonly: vk.1.viewonly,
                position_in_block: output.1.block_output_index,
                height: block.height as u32,
                tx_index: output.1.tx_index,
                txid: output.1.vtx.hash.clone(),
                output_index: output.1.output_index,
            });
        }
    }

    DecryptedBlock {
        height: block.height as u32,
        spends,
        notes,
        count_outputs,
        compact_block: block,
        elapsed,
    }
}

impl DecryptNode {
    pub fn new(vks: HashMap<u32, AccountViewKey>) -> DecryptNode {
        DecryptNode { vks }
    }

    pub fn decrypt_blocks(
        &self,
        network: &Network,
        blocks: Vec<CompactBlock>,
    ) -> Vec<DecryptedBlock> {
        let use_gpu = { *USE_GPU.lock().unwrap() };
        if use_gpu {
            #[cfg(feature = "cuda")]
            return self.cuda_decrypt_blocks(network, blocks);

            #[cfg(feature = "apple_metal")]
            return self.metal_decrypt_blocks(network, blocks);

            #[allow(unreachable_code)]
            self.decrypt_blocks_soft(network, blocks)
        } else {
            self.decrypt_blocks_soft(network, blocks)
        }
    }

    pub fn decrypt_blocks_soft(
        &self,
        network: &Network,
        blocks: Vec<CompactBlock>,
    ) -> Vec<DecryptedBlock> {
        let vks: Vec<_> = self.vks.iter().collect();
        let mut decrypted_blocks: Vec<DecryptedBlock> = blocks
            .into_par_iter()
            .map(|b| decrypt_notes(network, b, &vks))
            .collect();
        decrypted_blocks.sort_by(|a, b| a.height.cmp(&b.height));
        decrypted_blocks
    }

    #[cfg(feature = "cuda")]
    pub fn cuda_decrypt_blocks(
        &self,
        network: &Network,
        blocks: Vec<CompactBlock>,
    ) -> Vec<DecryptedBlock> {
        if blocks.is_empty() {
            return vec![];
        }
        if has_cuda() {
            let processor = CudaProcessor::setup_decrypt(network, blocks).unwrap();
            return trial_decrypt(processor, self.vks.iter()).unwrap();
        }
        self.decrypt_blocks_soft(network, blocks)
    }

    #[cfg(feature = "apple_metal")]
    pub fn metal_decrypt_blocks(
        &self,
        network: &Network,
        blocks: Vec<CompactBlock>,
    ) -> Vec<DecryptedBlock> {
        if blocks.is_empty() {
            return vec![];
        }
        let processor = MetalProcessor::setup_decrypt(network, blocks).unwrap();
        trial_decrypt(processor, self.vks.iter()).unwrap()
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
    rep.sapling_tree
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

pub async fn connect_lightwalletd(url: &str) -> anyhow::Result<CompactTxStreamerClient<Channel>> {
    log::info!("LWD URL: {}", url);
    let mut channel = tonic::transport::Channel::from_shared(url.to_owned())?;
    if url.starts_with("https") {
        let pem = include_bytes!("ca.pem");
        let ca = Certificate::from_pem(pem);
        let tls = ClientTlsConfig::new().ca_certificate(ca);
        channel = channel.tls_config(tls)?;
    }
    let client = CompactTxStreamerClient::connect(channel).await?;
    Ok(client)
}

async fn get_height(server: String) -> Option<(String, u32)> {
    let mut client = connect_lightwalletd(&server).await.ok()?;
    let height = get_latest_height(&mut client).await.ok()?;
    log::info!("{} {}", server, height);
    Some((server, height))
}

pub async fn get_best_server(servers: &[String]) -> Option<String> {
    let mut server_heights = vec![];
    for s in servers.iter() {
        let server_height =
            tokio::spawn(timeout(Duration::from_secs(1), get_height(s.to_string()))).boxed();
        server_heights.push(server_height);
    }
    let mut server_heights = future::try_join_all(server_heights).await.ok()?;
    server_heights.shuffle(&mut OsRng);

    server_heights
        .into_iter()
        .filter_map(|h| h.unwrap_or(None))
        .max_by_key(|(_, h)| *h)
        .map(|x| x.0)
}
