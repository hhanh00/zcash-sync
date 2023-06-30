use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;

pub fn sync(connection: &Connection, url: &str, account: u32) -> Result<()> {
    let height = latest_height(url)?;
    let address = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        [account],
        |r|
            r.get::<_, String>(0)
    )?;
    let get_balance = format!("{url}/api/v2/getAddressBalance?address={address}");
    let balance = thread::spawn(|| {
        thread::sleep(Duration::from_secs(1));
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let rep = reqwest::get(&get_balance).await?;
            let rep_json = rep.json::<Value>().await?;
            let ok = rep_json["ok"].as_bool().unwrap_or(false);
            if !ok {
                anyhow::bail!("Request failed");
            }
            let balance = rep_json["result"].as_str().unwrap_or("0");
            Ok::<_, anyhow::Error>(balance.to_owned())
        })
    })
    .join()
    .map_err(|_e| anyhow::anyhow!("Error"))??;
    let balance = balance.parse::<u64>().unwrap() / 10;
    connection.execute(
        "UPDATE accounts SET balance = ?1, height = ?2 WHERE id_account = ?3",
        params![balance, height, account],
    )?;
    Ok(())
}

pub fn latest_height(url: &str) -> Result<u32> {
    let get_consensus_block = format!("{url}/api/v2/getConsensusBlock");
    let height = thread::spawn(|| {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let rep = reqwest::get(&get_consensus_block).await?;
            let rep_json = rep.json::<Value>().await?;
            let height = rep_json["result"]["consensus_block"].as_u64().unwrap_or(0);
            Ok::<_, anyhow::Error>(height)
        })
    })
    .join()
    .map_err(|_e| anyhow::anyhow!("Error"))??;
    Ok(height as u32)
}
