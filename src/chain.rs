use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::lw_rpc::*;
use tonic::transport::Channel;
use tonic::Request;
use zcash_primitives::sapling::note_encryption::try_sapling_compact_note_decryption;
use crate::NETWORK;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::sapling::SaplingIvk;
use zcash_primitives::transaction::components::OutputDescription;
use jubjub::Scalar;
use group::GroupEncoding;
use ff::PrimeField;
use zcash_primitives::transaction::components::sapling::CompactOutputDescription;
use tokio::runtime::Runtime;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use futures::future::JoinAll;
use rayon::prelude::*;

const MAX_CHUNK: u32 = 50000;

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
    end_height: u32
) -> anyhow::Result<Vec<CompactBlock>> {
    let mut cbs: Vec<CompactBlock> = Vec::new();
    let mut s = start_height + 1;
    while s < end_height {
        eprintln!("{}", s);
        let e = (s + MAX_CHUNK).min(end_height);
        let range = BlockRange {
            start: Some(BlockId { height: s as u64, hash: vec![] }),
            end: Some(BlockId { height: e as u64, hash: vec![] })
        };
        let mut block_stream = client.get_block_range(Request::new(range)).await?.into_inner();
        while let Some(block) = block_stream.message().await? {
            cbs.push(block);
        }
        s = e + 1;
    }
    Ok(cbs)
}

struct DecryptNode {
    ivks: Vec<SaplingIvk>,
}

fn decrypt_notes(block: &CompactBlock, ivks: &[SaplingIvk]) {
    let height = BlockHeight::from_u32(block.height as u32);
    for vtx in block.vtx.iter() {
        for co in vtx.outputs.iter() {
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
            for ivk in ivks.iter() {
                if let Some((note, pa)) = try_sapling_compact_note_decryption(&NETWORK, height, ivk, &od) {
                    println!("{:?} {:?}", note, pa);
                }
            }
        }
    }
}

impl DecryptNode {
    pub fn new(ivks: Vec<SaplingIvk>) -> DecryptNode {
        DecryptNode {
            ivks,
        }
    }
    pub fn decrypt_blocks(&self, blocks: &[CompactBlock]) {
        blocks.par_iter().for_each(|b| {
            decrypt_notes(b, &self.ivks);
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::chain::{get_latest_height, download_chain, DecryptNode};
    use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
    use zcash_primitives::consensus::{Parameters, NetworkUpgrade};
    use zcash_client_backend::encoding::decode_extended_full_viewing_key;
    use crate::NETWORK;
    use dotenv;
    use tokio::runtime::Runtime;
    use std::time::Instant;

    #[tokio::test]
    async fn test_get_latest_height() -> anyhow::Result<()> {
        let mut client = CompactTxStreamerClient::connect("http://127.0.0.1:9067").await?;
        let height = get_latest_height(&mut client).await?;
        assert!(height > 1288000);
        Ok(())
    }

    #[tokio::test]
    async fn test_download_chain() -> anyhow::Result<()> {
        dotenv::dotenv().unwrap();
        let ivk = dotenv::var("IVK").unwrap();

        let fvk = decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk).unwrap().unwrap();
        let ivk = fvk.fvk.vk.ivk();
        let decrypter = DecryptNode::new(vec![ivk]);
        let mut client = CompactTxStreamerClient::connect("http://127.0.0.1:9067").await?;
        let start_height: u32 = crate::NETWORK.activation_height(NetworkUpgrade::Sapling).unwrap().into();
        let end_height = get_latest_height(&mut client).await?;

        let start = Instant::now();
        let cbs = download_chain(&mut client, start_height, end_height).await?;
        eprintln!("Download chain: {} ms", start.elapsed().as_millis());

        let start = Instant::now();
        decrypter.decrypt_blocks(&cbs);
        eprintln!("Decrypt Notes: {} ms", start.elapsed().as_millis());

        Ok(())
    }
}
