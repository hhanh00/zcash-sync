use crate::coin::CoinApi;
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, TxReportT,
};
use crate::RecipientsT;
use async_trait::async_trait;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

mod account;
mod db;
mod sync;

pub use db::init_db as init_tron_db;

pub struct TronHandler {
    connection: Mutex<Connection>,
    db_path: PathBuf,
    url: String,
}

impl TronHandler {
    pub fn new(connection: Connection, db_path: PathBuf) -> Self {
        TronHandler {
            connection: Mutex::new(connection),
            db_path,
            url: String::new(),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for TronHandler {
    fn db_path(&self) -> &str {
        self.db_path.to_str().unwrap()
    }

    fn coingecko_id(&self) -> &'static str {
        "tron"
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn list_accounts(&self) -> anyhow::Result<AccountVecT> {
        db::list_accounts(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str, _index: Option<u32>) -> anyhow::Result<u32> {
        account::derive_key(&self.connection(), name, key)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        account::is_valid_key(key)
    }

    fn is_valid_address(&self, key: &str) -> bool {
        account::is_valid_address(key)
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        super::db::read::get_backup(&self.connection(), account, |sk| hex::encode_upper(&sk))
    }

    async fn sync(&mut self, account: u32, _params: Vec<u8>) -> anyhow::Result<u32> {
        sync::sync(&self.connection(), &self.url, account).await
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_latest_height(&self) -> anyhow::Result<u32> {
        sync::latest_height(&self.url).await
    }

    fn get_db_height(&self, account: u32) -> anyhow::Result<Option<HeightT>> {
        crate::ton::db_height(&self.connection(), account)
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<u32> {
        Ok(height)
    }

    fn truncate(&mut self, _height: u32) -> anyhow::Result<()> {
        Ok(())
    }

    fn get_balance(&self, _account: u32) -> anyhow::Result<u64> {
        Ok(0)
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
        _account: u32,
        _recipients: &RecipientsT,
        _feeb: Option<u64>,
    ) -> anyhow::Result<String> {
        todo!()
    }

    fn to_tx_report(&self, _tx_plan: &str) -> anyhow::Result<TxReportT> {
        todo!()
    }

    fn sign(&self, _account: u32, _tx_plan: &str) -> anyhow::Result<Vec<u8>> {
        todo!()
    }

    fn broadcast(&self, _raw_tx: &[u8]) -> anyhow::Result<String> {
        todo!()
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}
