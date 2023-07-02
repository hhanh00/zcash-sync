use crate::coin::CoinApi;
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, TxReportT,
};
use crate::RecipientsT;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

mod account;
mod db;
mod pay;
mod sync;

use async_trait::async_trait;
pub use db::init_db as init_ton_db;

pub struct TonHandler {
    connection: Mutex<Connection>,
    db_path: PathBuf,
    url: String,
}

impl TonHandler {
    pub fn new(connection: Connection, db_path: PathBuf) -> Self {
        TonHandler {
            connection: Mutex::new(connection),
            db_path,
            url: String::new(),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for TonHandler {
    fn db_path(&self) -> &str {
        self.db_path.to_str().unwrap()
    }

    fn coingecko_id(&self) -> &'static str {
        "the-open-network"
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

    fn is_valid_address(&self, key: &str) -> bool {
        account::is_valid_address(key)
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        super::db::read::get_backup(&self.connection(), account, |sk| hex::encode_upper(&sk))
    }

    async fn sync(&mut self, account: u32) -> anyhow::Result<()> {
        sync::sync(&self.connection(), &self.url, account).await
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_latest_height(&self) -> anyhow::Result<u32> {
        sync::latest_height(&self.url).await
    }

    fn get_db_height(&self, account: u32) -> anyhow::Result<Option<HeightT>> {
        account::db_height(&self.connection(), account)
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn rewind_to_height(&mut self, _height: u32) -> anyhow::Result<()> {
        Ok(())
    }

    fn truncate(&mut self) -> anyhow::Result<()> {
        todo!()
    }

    fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        account::balance(&self.connection(), account)
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
        pay::prepare(
            &self.connection(),
            &self.url,
            account,
            recipients.values.as_ref().unwrap(),
        )
    }

    fn to_tx_report(&self, tx_plan: &str) -> anyhow::Result<TxReportT> {
        pay::to_tx_report(tx_plan)
    }

    fn sign(&self, account: u32, tx_plan: &str) -> anyhow::Result<Vec<u8>> {
        pay::sign(&self.connection(), account, tx_plan)
    }

    fn broadcast(&self, raw_tx: &[u8]) -> anyhow::Result<String> {
        sync::broadcast(&self.url, raw_tx)
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}
