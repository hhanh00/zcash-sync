use chrono::NaiveDateTime;
use std::collections::HashMap;

const DAY_SEC: i64 = 24*3600;

pub async fn retrieve_historical_prices(timestamps: &[i64], currency: &str) -> anyhow::Result<Vec<(i64, f64)>> {
    if timestamps.is_empty() { return Ok(Vec::new()); }
    let mut timestamps_map: HashMap<i64, Option<f64>> = HashMap::new();
    for ts in timestamps {
        timestamps_map.insert(*ts, None);
    }
    let client = reqwest::Client::new();
    let start = timestamps.first().unwrap();
    let end = timestamps.last().unwrap() + DAY_SEC;
    println!("{}", end);
    let url = "https://api.coingecko.com/api/v3/coins/zcash/market_chart/range";
    let params = [("from", start.to_string()), ("to", end.to_string()), ("vs_currency", currency.to_string())];
    let req = client.get(url).query(&params);
    println!("{:?}", req);
    let res = req.send().await?;
    let r: serde_json::Value = res.json().await?;
    let prices = r["prices"].as_array().unwrap();
    for p in prices.iter() {
        let p = p.as_array().unwrap();
        let ts = p[0].as_i64().unwrap() / 1000;
        let px = p[1].as_f64().unwrap();
        // rounded to daily
        let date = NaiveDateTime::from_timestamp(ts, 0).date().and_hms(0, 0, 0);
        let ts = date.timestamp();
        println!("{} - {}", date, px);
        if let Some(None) = timestamps_map.get(&ts) {
            timestamps_map.insert(ts, Some(px));
        }
    }
    let prices: Vec<_> = timestamps_map.iter().map(|(k, v)| {
        (*k, v.expect(&format!("missing price for ts {}", *k)))
    }).collect();
    Ok(prices)
}

#[cfg(test)]
mod tests {
    use crate::DbAdapter;
    use crate::db::DEFAULT_DB_PATH;
    use crate::prices::retrieve_historical_prices;

    #[tokio::test]
    async fn test() {
        let currency = "EUR";
        let mut db = DbAdapter::new(DEFAULT_DB_PATH).unwrap();
        let ts = db.get_missing_prices_timestamp("USD").unwrap();
        let prices = retrieve_historical_prices(&ts, currency).await.unwrap();
        db.store_historical_prices(prices, currency).unwrap();
    }
}