use crate::api::historical_prices::Quote;
use crate::db::data_generated::fb::{
    QuoteT, QuoteVecT, SpendingT, SpendingVecT, TxTimeValueT, TxTimeValueVecT,
};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub fn store_historical_prices(
    connection: &mut Connection,
    prices: &[Quote],
    currency: &str,
) -> Result<()> {
    let db_tx = connection.transaction()?;
    {
        let mut statement = db_tx.prepare(
            "INSERT INTO historical_prices(timestamp, price, currency) VALUES (?1, ?2, ?3) \
            ON CONFLICT (currency, timestamp) DO NOTHING", // Ignore double insert due to async fetch price
        )?;
        for q in prices {
            statement.execute(params![q.timestamp, q.price, currency])?;
        }
    }
    db_tx.commit()?;
    Ok(())
}

pub fn get_latest_quote(connection: &Connection, currency: &str) -> Result<Option<Quote>> {
    let quote = connection.query_row(
        "SELECT timestamp, price FROM historical_prices WHERE currency = ?1 ORDER BY timestamp DESC",
        [currency],
        |r| {
            Ok(Quote { timestamp: r.get(0)?, price: r.get(1)? })
        }).optional()?;
    Ok(quote)
}

pub fn get_pnl_txs(connection: &Connection, id: u32, timestamp: u32) -> Result<TxTimeValueVecT> {
    let mut stmt = connection.prepare(
        "SELECT timestamp, value FROM transactions WHERE timestamp >= ?2 AND account = ?1 ORDER BY timestamp DESC")?;
    let rows = stmt.query_map([id, timestamp], |row| {
        let timestamp: u32 = row.get(0)?;
        let value: i64 = row.get(1)?;
        Ok(TxTimeValueT {
            timestamp,
            value: value as u64,
        })
    })?;
    let txs: Result<Vec<_>, _> = rows.collect();
    Ok(TxTimeValueVecT { values: txs.ok() })
}

pub fn get_historical_prices(
    connection: &Connection,
    timestamp: u32,
    currency: &str,
) -> Result<QuoteVecT> {
    let mut stmt = connection.prepare(
        "SELECT timestamp, price FROM historical_prices WHERE timestamp >= ?2 AND currency = ?1",
    )?;
    let rows = stmt.query_map(params![currency, timestamp], |row| {
        let timestamp: u32 = row.get(0)?;
        let price: f64 = row.get(1)?;
        Ok(QuoteT { timestamp, price })
    })?;
    let quotes: Result<Vec<_>, _> = rows.collect();
    Ok(QuoteVecT {
        values: quotes.ok(),
    })
}

pub fn get_spendings(connection: &Connection, id: u32, timestamp: u32) -> Result<SpendingVecT> {
    let mut stmt = connection.prepare(
        "SELECT SUM(value) as v, t.address, c.name FROM transactions t LEFT JOIN contacts c ON t.address = c.address \
        WHERE account = ?1 AND timestamp >= ?2 AND value < 0 GROUP BY t.address ORDER BY v ASC LIMIT 5")?;
    let rows = stmt.query_map([id, timestamp], |row| {
        let value: i64 = row.get(0)?;
        let address: Option<String> = row.get(1)?;
        let name: Option<String> = row.get(2)?;
        let recipient = name.or(address).or(Some(String::new()));

        Ok(SpendingT {
            recipient,
            amount: (-value) as u64,
        })
    })?;
    let spendings: Result<Vec<_>, _> = rows.collect();
    Ok(SpendingVecT {
        values: spendings.ok(),
    })
}
