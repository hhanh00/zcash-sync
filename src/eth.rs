use crate::coin::CoinApi;
use crate::db::data_generated::fb::{AccountVecT, BackupT, PlainNoteVecT, PlainTxVecT, TxReportT};
use crate::RecipientsT;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

mod account;
mod db;
mod pay;
mod sync;

use crate::btc::get_address;
use async_trait::async_trait;
pub use db::init_db;

pub struct ETHHandler {
    connection: Mutex<Connection>,
    db_path: PathBuf,
    url: String,
}

impl ETHHandler {
    pub fn new(connection: Connection, db_path: PathBuf) -> Self {
        ETHHandler {
            connection: Mutex::new(connection),
            db_path,
            url: String::new(),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for ETHHandler {
    fn db_path(&self) -> &str {
        self.db_path.to_str().unwrap()
    }

    fn coingecko_id(&self) -> &'static str {
        "ethereum"
    }

    fn get_url(&self) -> String {
        self.url.clone()
    }

    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn list_accounts(&self) -> anyhow::Result<AccountVecT> {
        db::list_accounts(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str) -> anyhow::Result<u32> {
        account::derive_key(&self.connection(), name, key)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        account::is_valid_key(key)
    }

    fn is_valid_address(&self, address: &str) -> bool {
        account::is_valid_address(address)
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        super::db::read::get_backup(&self.connection(), account, |sk| {
            "0x".to_string() + &hex::encode_upper(&sk)
        })
    }

    async fn sync(&mut self, _account: u32) -> anyhow::Result<()> {
        sync::sync(&self.connection(), &self.url)
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_latest_height(&self) -> anyhow::Result<u32> {
        sync::get_latest_height(&self.url)
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn rewind_to_height(&mut self, _height: u32) -> anyhow::Result<()> {
        Ok(())
    }

    fn truncate(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        account::get_balance(&self.connection(), &self.url, account)
    }

    fn get_txs(&self, _account: u32) -> anyhow::Result<PlainTxVecT> {
        Ok(PlainTxVecT { txs: Some(vec![]) })
    }

    fn get_notes(&self, _account: u32) -> anyhow::Result<PlainNoteVecT> {
        Ok(PlainNoteVecT {
            notes: Some(vec![]),
        })
    }

    fn prepare_multi_payment(
        &self,
        account: u32,
        recipients: &RecipientsT,
        _feeb: Option<u64>,
    ) -> anyhow::Result<String> {
        pay::prepare(&self.connection(), &self.url, account, recipients)
    }

    fn to_tx_report(&self, tx_plan: &str) -> anyhow::Result<TxReportT> {
        pay::to_tx_report(tx_plan)
    }

    fn sign(&self, account: u32, tx_plan: &str) -> anyhow::Result<Vec<u8>> {
        pay::sign(&self.connection(), account, tx_plan)
    }

    fn broadcast(&self, raw_tx: &[u8]) -> anyhow::Result<String> {
        pay::broadcast(&self.url, raw_tx)
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}
