use std::thread;
use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::runtime::Runtime;

pub async fn sync(connection: &Connection, url: &str, account: u32) -> Result<()> {
    let height = latest_height(url).await?;
    let address = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, String>(0),
    )?;
    let get_balance = format!("{url}/api/v2/getAddressBalance?address={address}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let rep = reqwest::get(&get_balance).await?;
    let rep_json = rep.json::<Value>().await?;
    let ok = rep_json["ok"].as_bool().unwrap_or(false);
    if !ok {
        anyhow::bail!("Request failed");
    }
    let balance = rep_json["result"].as_str().unwrap_or("0");
    let balance = balance.parse::<u64>().unwrap() / 10;
    connection.execute(
        "UPDATE accounts SET balance = ?1, height = ?2 WHERE id_account = ?3",
        params![balance, height, account],
    )?;
    Ok(())
}

pub async fn latest_height(url: &str) -> Result<u32> {
    let get_consensus_block = format!("{url}/api/v2/getConsensusBlock");
    let rep = reqwest::get(&get_consensus_block).await?;
    let rep_json = rep.json::<Value>().await?;
    let ok = rep_json["ok"].as_bool().unwrap_or(false);
    if !ok {
        anyhow::bail!("Request failed");
    }
    let height = rep_json["result"]["consensus_block"].as_u64().unwrap_or(0);
    Ok(height as u32)
}

pub fn broadcast(url: &str, raw_tx: &[u8]) -> Result<String> {
    let url = url.to_owned();
    let body = json!({
        "boc": base64::encode(raw_tx),
    });
    let rep_json = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let mut client = reqwest::Client::new();
            let post_boc = format!("{url}/api/v2/sendBoc");
            let req = client.post(&post_boc).json(&body).build()?;
            let rep = client.execute(req).await?;
            let body: Value = rep.json().await?;
            println!("{:?}", body);
            Ok::<_, anyhow::Error>(body)
        })
    }).join().unwrap()?;
    let ok = rep_json["ok"].as_bool().unwrap_or(false);
    if !ok {
        anyhow::bail!("Request failed");
    }
    let extra = rep_json["result"]["@extra"].as_str().unwrap().to_owned();
    Ok(extra)
}
