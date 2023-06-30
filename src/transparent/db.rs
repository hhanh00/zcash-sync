use crate::{
    connect_lightwalletd, BlockId, BlockRange, ChainSpec, RawTransaction,
    TransparentAddressBlockFilter,
};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::mpsc;
use std::thread;
use tokio::runtime::Runtime;
use tonic::Request;
use zcash_client_backend::encoding::AddressCodec;
use zcash_params::coin::get_branch;
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};

use zcash_primitives::transaction::Transaction;

const FINALIZATION_HEIGHT: u64 = 90u64;

pub fn migrate_db(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS block_timestamps(
        height INTEGER PRIMARY KEY,
        hash BLOB NOT NULL,
        timestamp INTEGER)",
        [],
    )?;
    connection.execute(
        "CREATE TABLE IF NOT EXISTS t_blocks(
        id_block INTEGER PRIMARY KEY,
        account INTEGER NOT NULL,
        height INTEGER NOT NULL,
        UNIQUE (account, height))",
        [],
    )?;
    connection.execute(
        "CREATE TABLE IF NOT EXISTS t_txs(
        id_tx INTEGER PRIMARY KEY,
        account INTEGER NOT NULL,
        hash BLOB NOT NULL,
        height INTEGER NOT NULL,
        value INTEGER NOT NULL,
        address STRING NOT NULL)",
        [],
    )?;
    connection.execute(
        "CREATE TABLE IF NOT EXISTS t_utxos(
        id_utxo INTEGER PRIMARY KEY,
        account INTEGER NOT NULL,
        tx_hash BLOB NOT NULL,
        vout INTEGER NOT NULL,
        height INTEGER NOT NULL,
        value INTEGER NOT NULL,
        spent INTEGER)",
        [],
    )?;
    Ok(())
}

pub fn truncate_height(connection: &Connection, account: u32, height: u32) -> Result<()> {
    connection.execute(
        "DELETE FROM t_txs WHERE height > ?1 AND account = ?2",
        [height, account],
    )?;
    connection.execute(
        "DELETE FROM t_utxos WHERE height > ?1 AND account = ?2",
        [height, account],
    )?;
    connection.execute(
        "UPDATE t_utxos SET spent = NULL WHERE spent > ?1 AND account = ?2",
        [height, account],
    )?;
    connection.execute(
        "DELETE FROM t_blocks WHERE height > ?1 AND account = ?2",
        [height, account],
    )?;

    Ok(())
}

#[derive(Clone, Default, Debug)]
struct BlockHash {
    height: u64,
    hash: Vec<u8>,
    time: u32,
}

impl From<BlockHash> for BlockId {
    fn from(value: BlockHash) -> Self {
        BlockId {
            height: value.height,
            hash: value.hash.clone(),
        }
    }
}

impl BlockId {
    fn trim(self) -> Self {
        BlockId {
            height: self.height,
            hash: vec![],
        }
    }
}

enum GetTxData {
    StartHeight(u32),
    Tx(RawTransaction),
    LatestBlockId(BlockHash),
}

pub fn fetch_txs(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    my_address: &str,
) -> Result<()> {
    let (sender, recver) = mpsc::channel::<GetTxData>();
    let my_address = my_address.to_string();
    let my_address2 = my_address.clone();
    let db_block_id = get_db_height(network, connection, account)?;
    let url2 = url.to_string();
    let jh = thread::spawn(move || {
        let r = Runtime::new().unwrap();
        r.block_on(async move {
            let mut client = connect_lightwalletd(&url2).await?;
            let latest_block_id = client
                .get_latest_block(Request::new(ChainSpec {}))
                .await?
                .into_inner();
            if db_block_id.hash == latest_block_id.hash {
                // synced to latest_block
                return Ok::<_, anyhow::Error>(());
            }

            let db_block_id2: BlockId = db_block_id.clone().into();
            let block = client
                .get_block(Request::new(db_block_id2.trim()))
                .await?
                .into_inner();
            let db_block_hash = BlockHash {
                height: block.height,
                hash: block.hash,
                time: block.time,
            };

            let block = client
                .get_block(Request::new(latest_block_id.clone().trim()))
                .await?
                .into_inner();
            let latest_block_hash = BlockHash {
                height: block.height,
                hash: block.hash,
                time: block.time,
            };

            let start_height = {
                if db_block_id.hash == db_block_hash.hash {
                    // same hash, no re-org
                    db_block_id.height
                } else {
                    // hash has changed, reorg took place
                    db_block_id.height.saturating_sub(FINALIZATION_HEIGHT)
                }
            } as u32;

            sender.send(GetTxData::StartHeight(start_height))?;
            let mut tx_ids = client
                .get_taddress_txids(Request::new(TransparentAddressBlockFilter {
                    address: my_address2,
                    range: Some(BlockRange {
                        start: Some(BlockId {
                            height: (start_height + 1) as u64,
                            hash: vec![],
                        }),
                        end: Some(BlockId {
                            height: latest_block_id.height,
                            hash: vec![],
                        }),
                        spam_filter_threshold: 0,
                    }),
                }))
                .await?
                .into_inner();
            while let Some(tx) = tx_ids.message().await? {
                sender.send(GetTxData::Tx(tx))?;
            }
            sender.send(GetTxData::LatestBlockId(latest_block_hash))?;
            Ok::<_, anyhow::Error>(())
        })?;
        Ok::<_, anyhow::Error>(())
    });

    let mut get_prevout = connection.prepare(
        "SELECT id_utxo, value FROM t_utxos WHERE tx_hash = ?1 AND vout = ?2 AND account = ?3",
    )?;
    let mut mark_spent = connection.prepare("UPDATE t_utxos SET spent = ?1 WHERE id_utxo = ?2")?;
    let mut put_txout = connection.prepare(
        "INSERT INTO t_utxos(account, tx_hash, vout, height, value, spent) \
        VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
    )?;
    let mut put_tx = connection.prepare(
        "INSERT INTO t_txs(account, hash, height, value, address) \
        VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    while let Ok(data) = recver.recv() {
        match data {
            GetTxData::StartHeight(start_height) => {
                truncate_height(connection, account, start_height)?;
            }

            GetTxData::Tx(tx) => {
                let height = tx.height as u32;
                let branch_id = get_branch(network, height);
                let tx = Transaction::read(&*tx.data, branch_id)?;
                let mut has_io = false;
                if let Some(ref transparent_bundle) = tx.transparent_bundle {
                    let mut in_address = String::new();
                    let mut out_address = String::new();
                    let mut total_value = 0i64;
                    for vin in transparent_bundle.vin.iter() {
                        let res = get_prevout
                            .query_row(params![vin.prevout.hash(), vin.prevout.n(), account], |r| {
                                let id = r.get::<_, u32>(0)?;
                                let value = r.get::<_, i64>(1)?;
                                Ok((id, value))
                            })
                            .optional()?;
                        match res {
                            Some((id, v)) => {
                                has_io = true;
                                total_value -= v;
                                mark_spent.execute(params![height, id])?;
                            }
                            None => {
                                if let Some(ta) = vin.script_sig.address() {
                                    in_address = ta.encode(network);
                                }
                            }
                        }
                    }
                    for (i, vout) in transparent_bundle.vout.iter().enumerate() {
                        let address = vout.script_pubkey.address();
                        if let Some(ta) = address {
                            let address = ta.encode(network);
                            if address == my_address {
                                has_io = true;
                                total_value += i64::from(vout.value);
                                put_txout.execute(params![
                                    account,
                                    tx.txid().as_ref(),
                                    i,
                                    height,
                                    u64::from(vout.value)
                                ])?;
                            } else {
                                if let Some(ta) = vout.script_pubkey.address() {
                                    out_address = ta.encode(network);
                                }
                            }
                        }
                    }
                    if has_io {
                        let address = if total_value < 0 {
                            out_address
                        } else {
                            in_address
                        };
                        put_tx.execute(params![
                            account,
                            tx.txid().as_ref(),
                            height,
                            total_value,
                            address
                        ])?;
                    }
                }
            }

            GetTxData::LatestBlockId(block_hash) => {
                connection.execute(
                    "INSERT INTO t_blocks(height, account) \
                    VALUES (?1, ?2) ON CONFLICT (account, height) DO NOTHING",
                    params![account, block_hash.height],
                )?;
                connection.execute(
                    "INSERT INTO block_timestamps(height, hash, timestamp) \
                    VALUES (?1, ?2, ?3) ON CONFLICT (height) DO UPDATE SET \
                    hash = excluded.hash, timestamp = excluded.timestamp",
                    params![block_hash.height, block_hash.hash, block_hash.time],
                )?;
            }
        }
    }

    jh.join().unwrap()?;
    Ok(())
}

fn get_db_height(network: &Network, connection: &Connection, account: u32) -> Result<BlockHash> {
    let block_id = connection
        .query_row(
            "SELECT height, hash, timestamp FROM block_timestamps \
        WHERE height = (SELECT MAX(height) FROM t_blocks WHERE account = ?1)",
            [account],
            |r| {
                Ok(BlockHash {
                    height: r.get(0)?,
                    hash: r.get(1)?,
                    time: r.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(block_id.unwrap_or_else(|| BlockHash {
        height: network
            .activation_height(NetworkUpgrade::Sapling)
            .unwrap()
            .into(),
        ..BlockHash::default()
    }))
}

pub async fn update_timestamps(connection: &Connection, url: &str) -> Result<()> {
    let mut s = connection.prepare(
    "SELECT t_txs.height FROM t_txs LEFT JOIN block_timestamps b ON t_txs.height = b.height WHERE b.height IS NULL")?;
    let rows = s.query_map([], |r| r.get::<_, u32>(0))?;
    let heights: Result<Vec<_>, _> = rows.collect();
    s = connection.prepare(
        "INSERT INTO block_timestamps(height, hash, timestamp) \
    VALUES (?1, ?2, ?3) ON CONFLICT (height) DO NOTHING",
    )?;
    let mut client = connect_lightwalletd(url).await?;
    for h in heights? {
        let block = client
            .get_block(Request::new(BlockId {
                height: h as u64,
                hash: vec![],
            }))
            .await?
            .into_inner();
        s.execute(params![h, block.hash, block.time])?;
    }
    Ok(())
}
