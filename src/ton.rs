use anyhow::Result;
use ambassador::Delegate;
use crate::coin::{CoinApi, Database, EncryptedDatabase};
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, TxReportT,
};
use crate::fb::RecipientsT;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

mod account;
mod db;
mod pay;
mod sync;

pub use account::db_height;
use async_trait::async_trait;
pub use db::init_db as init_ton_db;

#[derive(Delegate)]
#[delegate(Database, target = "db")]
pub struct TonHandler {
    pub db: EncryptedDatabase,
    url: String,
}

impl TonHandler {
    pub fn new(db_path: PathBuf, passwd: &str) -> Result<Self> {
        Ok(TonHandler {
            db: EncryptedDatabase::new(db_path, passwd, |c| Ok(()))?,
            url: String::new(),
        })
    }
}

#[async_trait(?Send)]
impl CoinApi for TonHandler {
    fn coingecko_id(&self) -> &'static str {
        "the-open-network"
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
        account::db_height(&self.connection(), account)
    }

    fn reset_sync(&mut self, _height: u32) -> anyhow::Result<()> {
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
}
