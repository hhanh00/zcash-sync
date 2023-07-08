use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

pub async fn latest_height(url: &str) -> Result<u32> {
    let url = format!("{url}/api/block?limit=1");
    let rep: Value = reqwest::get(&url).await?.json().await?;
    let res = &rep["data"].as_array().ok_or(anyhow!("Invalid response"))?[0];
    let height = res["number"].as_u64().ok_or(anyhow!("Invalid response"))?;
    Ok(height as u32)
}

pub async fn sync(connection: &Connection, url: &str, account: u32) -> Result<u32> {
    let height = latest_height(url).await?;
    connection.execute(
        "UPDATE accounts SET height = ?1 WHERE id_account = ?2",
        params![height, account],
    )?;
    Ok(height)
}
