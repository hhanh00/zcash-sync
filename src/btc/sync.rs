use super::db;
use crate::btc::BTCNET;
use crate::db::data_generated::fb::PlainTxT;
use anyhow::Result;
use electrum_client::bitcoin::address::Payload;
use electrum_client::bitcoin::{Address, ScriptBuf, Txid};
use electrum_client::{Client, ElectrumApi};
use rusqlite::{params, Connection, OptionalExtension};

pub fn sync(connection: &Connection, url: &str) -> Result<()> {
    let client = Client::new(url)?;
    // check reorg
    let db_height = loop {
        let db_height = db::get_height(connection)?.unwrap_or_default().height;
        if db_height != 0 {
            let header = client.block_header(db_height as usize)?;
            let block_hash = header.block_hash();
            let block_hash: &[u8] = block_hash.as_ref();
            let db_block_hash = db::get_block_hash(connection, db_height)?;
            if block_hash == &db_block_hash {
                break db_height;
            }
            db::rewind_to(connection, db_height - 1)?;
        } else {
            break db_height;
        }
    };
    let header_notification = client.block_headers_subscribe()?;
    let new_height = header_notification.height as u32;
    if new_height > db_height {
        for account in db::get_accounts(connection)? {
            let wp = db::get_wp(connection, account)?;
            let address: Address = Address::new(BTCNET, Payload::WitnessProgram(wp.clone()));
            let pub_script = address.script_pubkey();
            let tx_history = client.script_get_history(pub_script.as_script())?;
            for tx in tx_history.iter() {
                if db::get_tx_txid(connection, account, &tx.tx_hash)?.is_none() {
                    let height = tx.height;
                    let header = client.block_header(height as usize)?;
                    let timestamp = header.time;
                    resolve_tx(
                        connection,
                        &client,
                        account,
                        height as u32,
                        timestamp,
                        &tx.tx_hash,
                        &pub_script,
                    )?;
                }
            }
        }
        db::store_header(connection, new_height, &header_notification.header)?;
    }
    Ok(())
}

fn resolve_tx(
    connection: &Connection,
    client: &Client,
    account: u32,
    height: u32,
    timestamp: u32,
    txid: &Txid,
    pub_script: &ScriptBuf,
) -> Result<PlainTxT> {
    let tx = client.transaction_get(txid)?;
    let mut total_spent = 0i64;
    for vin in tx.input.iter() {
        let prevout = &vin.previous_output;
        let prevout_hash: &[u8] = prevout.txid.as_ref();
        let value = connection
            .query_row(
                "SELECT value FROM utxos WHERE txid = ?1 AND vout = ?2",
                params![prevout_hash, prevout.vout],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(value) = value {
            // if we found it in db, that input is ours and now it is spent
            connection.execute(
                "UPDATE utxos SET spent = ?1 WHERE txid = ?2 AND vout = ?3",
                params![height, prevout_hash, prevout.vout],
            )?;
            total_spent += value;
        }
    }
    let mut total_received = 0i64;
    for (index, vout) in tx.output.iter().enumerate() {
        if &vout.script_pubkey == pub_script {
            db::store_utxo(
                connection,
                account,
                height,
                timestamp,
                txid,
                index as u32,
                vout.value,
            )?;
            total_received += vout.value as i64;
        }
    }
    let tx_value = total_received - total_spent;
    let id = db::store_tx(connection, account, height, timestamp, &tx, tx_value)?;

    let mut tx_hash = AsRef::<[u8]>::as_ref(txid).to_vec();
    tx_hash.reverse();
    let tx = PlainTxT {
        id,
        tx_id: Some(hex::encode(&tx_hash)),
        height,
        timestamp,
        value: tx_value,
        address: None,
    };
    Ok(tx)
}

pub fn get_height(url: &str) -> Result<u32> {
    let client = Client::new(url)?;
    let notification = client.block_headers_subscribe()?;
    let height = notification.height as u32;
    Ok(height)
}

pub fn broadcast(url: &str, txb: &[u8]) -> Result<String> {
    let client = Client::new(url)?;
    let txid = client.transaction_broadcast_raw(txb)?;
    Ok(txid.to_string())
}

pub fn get_estimated_fee(url: &str) -> Result<u64> {
    let client = Client::new(url)?;
    Ok((client.estimate_fee(1)? * 100_000f64) as u64)
}
