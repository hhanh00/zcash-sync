//! Retrieve Historical Prices from coingecko

use crate::coinconfig::CoinConfig;

/// Retrieve historical prices
/// # Arguments
/// * `now`: current timestamp
/// * `days`: how many days to fetch
/// * `currency`: base currency
pub async fn sync_historical_prices(coin: u8, now: i64, days: u32, currency: &str) -> anyhow::Result<u32> {
    let c = CoinConfig::get(coin);
    let quotes = crate::prices::fetch_historical_prices(c.coin, now, days, currency).await?;
    let mut db = c.db()?;
    db.store_historical_prices(&quotes, currency)?;
    Ok(quotes.len() as u32)
}
