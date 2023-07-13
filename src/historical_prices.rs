//! Retrieve Historical Prices from coingecko

use crate::db::data_generated::fb::QuoteT;
use anyhow::{anyhow, Result};
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension};

pub async fn sync_historical_prices(
    connection: &mut Connection,
    ticker: &str,
    now: u32,
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
    latest_quote: Option<QuoteT>,
    now: u32,
    days: u32,
    currency: &str,
) -> Result<Vec<QuoteT>> {
    let json_error = || anyhow::anyhow!("Invalid JSON");
    let today = now / DAY_SEC;
    let from_day = today - days;
    let latest_day = if let Some(latest_quote) = latest_quote {
        latest_quote.timestamp / DAY_SEC
    } else {
        0
    };
    let latest_day = latest_day.max(from_day);

    let mut quotes: Vec<_> = vec![];
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
            let mut prev_timestamp = 0u32;
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
                let timestamp = date.timestamp() as u32;
                if timestamp != prev_timestamp {
                    let quote = QuoteT { timestamp, price };
                    quotes.push(quote);
                }
                prev_timestamp = timestamp;
            }
        }
    }

    Ok(quotes)
}

pub fn get_latest_quote(connection: &Connection, currency: &str) -> Result<Option<QuoteT>> {
    let quote = connection.query_row(
        "SELECT timestamp, price FROM historical_prices WHERE currency = ?1 ORDER BY timestamp DESC",
        [currency],
        |row| {
            Ok(QuoteT {
                timestamp: row.get(0)?,
                price: row.get(1)?,
            })
        }).optional()?;
    Ok(quote)
}

pub fn store_historical_prices(
    connection: &mut Connection,
    prices: &[QuoteT],
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

const DAY_SEC: u32 = 24 * 3600;
