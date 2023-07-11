//! Retrieve Historical Prices from coingecko

use crate::coinconfig::CoinConfig;
use anyhow::{anyhow, Result};
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension};

/// Retrieve historical prices
/// # Arguments
/// * `now`: current timestamp
/// * `days`: how many days to fetch
/// * `currency`: base currency
pub async fn sync_historical_prices(coin: u8, now: i64, days: u32, currency: &str) -> Result<u32> {
    let c = CoinConfig::get(coin);
    let mut db = c.db()?;
    let connection = &mut db.connection;
    let ticker = c.chain.ticker();
    sync_historical_prices_inner(connection, ticker, now, days, currency).await
}

pub async fn sync_historical_prices_inner(
    connection: &mut Connection,
    ticker: &str,
    now: i64,
    days: u32,
    currency: &str,
) -> Result<u32> {
    let latest_quote = crate::db::historical_prices::get_latest_quote(connection, currency)?;
    let quotes = fetch_historical_prices(ticker, latest_quote, now, days, currency).await?;
    crate::db::historical_prices::store_historical_prices(connection, &quotes, currency)?;
    Ok(quotes.len() as u32)
}

pub async fn fetch_historical_prices(
    ticker: &str,
    latest_quote: Option<Quote>,
    now: i64,
    days: u32,
    currency: &str,
) -> Result<Vec<Quote>> {
    let json_error = || anyhow::anyhow!("Invalid JSON");
    let today = now / DAY_SEC;
    let from_day = today - days as i64;
    let latest_day = if let Some(latest_quote) = latest_quote {
        latest_quote.timestamp / DAY_SEC
    } else {
        0
    };
    let latest_day = latest_day.max(from_day);

    let mut quotes: Vec<Quote> = vec![];
    let from = (latest_day + 1) * DAY_SEC;
    let to = today * DAY_SEC;
    if from != to {
        let client = reqwest::Client::new();
        let url = format!(
            "https://api.coingecko.com/api/v3/coins/{}/market_chart/range",
            ticker,
        );
        let params = [
            ("from", from.to_string()),
            ("to", to.to_string()),
            ("vs_currency", currency.to_string()),
        ];
        let req = client.get(url).query(&params);
        let res = req.send().await?;
        let t = res.text().await?;
        let r: serde_json::Value = serde_json::from_str(&t)?;
        let status = &r["status"]["error_code"];
        if status.is_null() {
            let prices = r["prices"].as_array().ok_or_else(json_error)?;
            let mut prev_timestamp = 0i64;
            for p in prices.iter() {
                let p = p.as_array().ok_or_else(json_error)?;
                let ts = p[0].as_i64().ok_or_else(json_error)? / 1000;
                let price = p[1].as_f64().ok_or_else(json_error)?;
                // rounded to daily
                let date = NaiveDateTime::from_timestamp_opt(ts, 0)
                    .ok_or(anyhow!("Invalid Date"))?
                    .date()
                    .and_hms_opt(0, 0, 0)
                    .ok_or(anyhow!("Invalid Date"))?;
                let timestamp = date.timestamp();
                if timestamp != prev_timestamp {
                    let quote = Quote { timestamp, price };
                    quotes.push(quote);
                }
                prev_timestamp = timestamp;
            }
        }
    }

    Ok(quotes)
}

pub fn get_latest_quote(connection: &Connection, currency: &str) -> Result<Option<Quote>> {
    let quote = connection.query_row(
        "SELECT timestamp, price FROM historical_prices WHERE currency = ?1 ORDER BY timestamp DESC",
        [currency],
        |row| {
            Ok(Quote {
                timestamp: row.get(0)?,
                price: row.get(1)?,
            })
        }).optional()?;
    Ok(quote)
}

pub fn store_historical_prices(
    connection: &mut Connection,
    prices: &[Quote],
    currency: &str,
) -> Result<()> {
    let db_transaction = connection.transaction()?;
    {
        let mut statement = db_transaction.prepare(
            "INSERT INTO historical_prices(timestamp, price, currency) VALUES (?1, ?2, ?3) \
                ON CONFLICT (currency, timestamp) DO NOTHING", // Ignore double insert due to async fetch price
        )?;
        for q in prices {
            statement.execute(params![q.timestamp, q.price, currency])?;
        }
    }
    db_transaction.commit()?;
    Ok(())
}

const DAY_SEC: i64 = 24 * 3600;

#[derive(Debug)]
pub struct Quote {
    pub timestamp: i64,
    pub price: f64,
}
