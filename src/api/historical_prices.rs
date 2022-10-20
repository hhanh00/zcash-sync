//! Retrieve Historical Prices from coingecko

use crate::coinconfig::CoinConfig;

/// Retrieve historical prices
/// # Arguments
/// * `now`: current timestamp
/// * `days`: how many days to fetch
/// * `currency`: base currency
pub async fn sync_historical_prices(now: i64, days: u32, currency: &str) -> anyhow::Result<u32> {
    let c = CoinConfig::get_active();
    let mut db = c.db()?;
    let quotes = crate::prices::fetch_historical_prices(now, days, currency, &db).await?;
    db.store_historical_prices(&quotes, currency)?;
    Ok(quotes.len() as u32)
}
