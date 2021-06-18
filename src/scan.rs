use zcash_primitives::sapling::SaplingIvk;
use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
use crate::{DecryptNode, LWD_URL, get_latest_height, download_chain, calculate_tree_state_v2};
use zcash_primitives::consensus::{NetworkUpgrade, Parameters};
use std::time::Instant;
use log::info;

pub async fn scan_all(ivks: &[SaplingIvk]) -> anyhow::Result<()> {
    let decrypter = DecryptNode::new(ivks.to_vec());

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

