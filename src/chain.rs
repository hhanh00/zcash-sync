use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::lw_rpc::*;
use crate::{NETWORK, advance_tree};
use ff::PrimeField;
use group::GroupEncoding;
use rayon::prelude::*;
use tonic::transport::{Channel, Certificate, ClientTlsConfig};
use tonic::Request;
use zcash_primitives::consensus::{BlockHeight, Parameters, NetworkUpgrade};
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::note_encryption::try_sapling_compact_note_decryption;
use zcash_primitives::sapling::{Node, Note, PaymentAddress};
use zcash_primitives::transaction::components::sapling::CompactOutputDescription;
use crate::commitment::{CTree, Witness};
use std::time::Instant;
use log::info;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::zip32::ExtendedFullViewingKey;

const MAX_CHUNK: u32 = 50000;
// pub const LWD_URL: &str = "https://mainnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "https://lwdv3.zecwallet.co";
pub const LWD_URL: &str = "http://127.0.0.1:9067";

pub async fn get_latest_height(
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<u32> {
    let chainspec = ChainSpec {};
    let rep = client.get_latest_block(Request::new(chainspec)).await?;
    let block_id = rep.into_inner();
    Ok(block_id.height as u32)
}

/* download [start_height+1, end_height] inclusive */
pub async fn download_chain(
    client: &mut CompactTxStreamerClient<Channel>,
    start_height: u32,
    end_height: u32,
) -> anyhow::Result<Vec<CompactBlock>> {
    let mut cbs: Vec<CompactBlock> = Vec::new();
    let mut s = start_height + 1;
    while s < end_height {
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
            cbs.push(block);
        }
        s = e + 1;
    }
    Ok(cbs)
}

pub struct DecryptNode {
    fvks: Vec<ExtendedFullViewingKey>,
}

pub struct DecryptedBlock {
    pub height: u32,
    pub notes: Vec<DecryptedNote>,
    pub count_outputs: u32,
}

#[derive(Clone)]
pub struct DecryptedNote {
    pub ivk: ExtendedFullViewingKey,
    pub note: Note,
    pub pa: PaymentAddress,
    pub position: usize,

    pub height: u32,
    pub txid: Vec<u8>,
    pub tx_index: usize,
    pub output_index: usize,
}

fn decrypt_notes(block: &CompactBlock, fvks: &[ExtendedFullViewingKey]) -> DecryptedBlock {
    let height = BlockHeight::from_u32(block.height as u32);
    let mut count_outputs = 0u32;
    let mut notes: Vec<DecryptedNote> = vec![];
    for (tx_index, vtx) in block.vtx.iter().enumerate() {
        for (output_index, co) in vtx.outputs.iter().enumerate() {
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
            for fvk in fvks.iter() {
                let ivk = &fvk.fvk.vk.ivk();
                if let Some((note, pa)) =
                    try_sapling_compact_note_decryption(&NETWORK, height, ivk, &od)
                {
                    notes.push(DecryptedNote {
                        ivk: fvk.clone(),
                        note,
                        pa,
                        position: count_outputs as usize,
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
        notes,
        count_outputs,
    }
}

impl DecryptNode {
    pub fn new(fvks: Vec<ExtendedFullViewingKey>) -> DecryptNode {
        DecryptNode { fvks }
    }

    pub fn decrypt_blocks(&self, blocks: &[CompactBlock]) -> Vec<DecryptedBlock> {
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
                    if i == nn.position {
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
                    if i == nn.position {
                        positions.push(p);
                        n = notes.next();
                    }
                }
                i += 1;
                p += 1;
            }
        }
    }
    info!("Build CMU list: {} ms - {} nodes", start.elapsed().as_millis(), nodes.len());

    let witnesses: Vec<_> = positions.iter().map(|p| Witness::new(*p, 0, None)).collect();
    let (_, new_witnesses) = advance_tree(CTree::new(), &witnesses, &mut nodes);
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

pub async fn sync(ivk: &str) -> anyhow::Result<()> {
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
            .unwrap()
            .unwrap();
    let decrypter = DecryptNode::new(vec![fvk]);
    let mut client = connect_lightwalletd().await?;
    let start_height: u32 = crate::NETWORK
        .activation_height(NetworkUpgrade::Sapling)
        .unwrap()
        .into();
    let end_height = get_latest_height(&mut client).await?;

    let start = Instant::now();
    let cbs = download_chain(&mut client, start_height, end_height).await?;
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
    #[allow(unused_imports)]
    use crate::chain::{download_chain, get_latest_height, get_tree_state, calculate_tree_state_v1, calculate_tree_state_v2, DecryptNode};
    use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
    use crate::NETWORK;
    use dotenv;
    use std::time::Instant;
    use zcash_client_backend::encoding::decode_extended_full_viewing_key;
    use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
    use crate::chain::LWD_URL;

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

        let fvk =
            decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap()
                .unwrap();
        let decrypter = DecryptNode::new(vec![fvk]);
        let mut client = CompactTxStreamerClient::connect(LWD_URL).await?;
        let start_height: u32 = crate::NETWORK
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into();
        let end_height = get_latest_height(&mut client).await?;

        let start = Instant::now();
        let cbs = download_chain(&mut client, start_height, end_height).await?;
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
