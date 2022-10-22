//! Access to server mempool

use anyhow::anyhow;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::Parameters;
use crate::api::sync::get_latest_height;

use crate::coinconfig::CoinConfig;
use crate::db::AccountData;

/// Scan the mempool and return the unconfirmed balance
pub async fn scan() -> anyhow::Result<i64> {
    let c = CoinConfig::get_active();
    let AccountData { fvk, .. } = c.db()?.get_account_info(c.id_account)?;
    let height = get_latest_height().await?;
    let mut mempool = c.mempool.lock().unwrap();
    let current_height = c.height;
    if height != current_height {
        CoinConfig::set_height(height);
        mempool.clear()?;
    }
    let fvk = decode_extended_full_viewing_key(
        c.chain.network().hrp_sapling_extended_full_viewing_key(),
        &fvk,
    ).map_err(|_| anyhow!("Decode error"))?;
    let mut client = c.connect_lwd().await?;
    mempool
        .update(&mut client, height, &fvk.fvk.vk.ivk())
        .await?;

    Ok(mempool.get_unconfirmed_balance())
}
