use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::Parameters;

use crate::coinconfig::CoinConfig;
use crate::get_latest_height;

pub async fn scan() -> anyhow::Result<i64> {
    let c = CoinConfig::get_active();
    let ivk = c.db()?.get_ivk(c.id_account)?;
    let mut client = c.connect_lwd().await?;
    let height = get_latest_height(&mut client).await?;
    let mut mempool = c.mempool.lock().unwrap();
    let current_height = c.height;
    if height != current_height {
        CoinConfig::set_height(height);
        mempool.clear()?;
    }
    let fvk = decode_extended_full_viewing_key(
        c.chain.network().hrp_sapling_extended_full_viewing_key(),
        &ivk,
    )?
    .unwrap();
    mempool
        .update(&mut client, height, &fvk.fvk.vk.ivk())
        .await?;

    Ok(mempool.get_unconfirmed_balance())
}
