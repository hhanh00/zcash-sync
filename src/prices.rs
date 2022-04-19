use crate::DbAdapter;
use chrono::NaiveDateTime;
use zcash_params::coin::get_coin_chain;

const DAY_SEC: i64 = 24 * 3600;

#[derive(Debug)]
pub struct Quote {
    pub timestamp: i64,
    pub price: f64,
}

pub async fn fetch_historical_prices(
    now: i64,
    days: u32,
    currency: &str,
    db: &DbAdapter,
) -> anyhow::Result<Vec<Quote>> {
    let chain = get_coin_chain(db.coin_type);
    let json_error = || anyhow::anyhow!("Invalid JSON");
    let today = now / DAY_SEC;
    let from_day = today - days as i64;
    let latest_quote = db.get_latest_quote(currency)?;
    let latest_day = if let Some(latest_quote) = latest_quote {
        latest_quote.timestamp / DAY_SEC
    } else {
        0
    };
    let latest_day = latest_day.max(from_day);

    let mut quotes: Vec<Quote> = vec![];
    if latest_day < today {
        let from = (latest_day + 1) * DAY_SEC;
        let to = today * DAY_SEC;
        let client = reqwest::Client::new();
        let url = format!(
            "https://api.coingecko.com/api/v3/coins/{}/market_chart/range",
            chain.ticker()
        );
        let params = [
            ("from", from.to_string()),
            ("to", to.to_string()),
            ("vs_currency", currency.to_string()),
        ];
        let req = client.get(url).query(&params);
        let res = req.send().await?;
        let r: serde_json::Value = res.json().await?;
        let prices = r["prices"].as_array().ok_or_else(json_error)?;
        let mut prev_timestamp = 0i64;
        for p in prices.iter() {
            let p = p.as_array().ok_or_else(json_error)?;
            let ts = p[0].as_i64().ok_or_else(json_error)? / 1000;
            let price = p[1].as_f64().ok_or_else(json_error)?;
            // rounded to daily
            let date = NaiveDateTime::from_timestamp(ts, 0).date().and_hms(0, 0, 0);
            let timestamp = date.timestamp();
            if timestamp != prev_timestamp {
                let quote = Quote { timestamp, price };
                quotes.push(quote);
            }
            prev_timestamp = timestamp;
        }
    }

    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use crate::db::DEFAULT_DB_PATH;
    use crate::prices::fetch_historical_prices;
    use crate::DbAdapter;
    use std::time::SystemTime;
    use zcash_params::coin::CoinType;

    #[tokio::test]
    async fn test_fetch_quotes() {
        let currency = "EUR";
        let mut db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let quotes = fetch_historical_prices(now, 365, currency, &db)
            .await
            .unwrap();
        for q in quotes.iter() {
            println!("{:?}", q);
        }
        db.store_historical_prices(&quotes, currency).unwrap();
    }
}
