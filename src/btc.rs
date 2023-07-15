mod db;
mod key;
mod pay;
mod sync;

use ambassador::Delegate;
use crate::coin::{CoinApi, Database, EncryptedDatabase};
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
use anyhow::Result;
use async_trait::async_trait;
pub use db::{
    delete_account, delete_secrets, get_account, get_address, get_height as get_db_height,
    get_property, has_account, init_db, list_accounts, set_property, update_name,
};
use electrum_client::bitcoin::secp256k1::SecretKey;
use electrum_client::bitcoin::{Network, PrivateKey};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub const BTCNET: Network = Network::Bitcoin;

#[derive(Delegate)]
#[delegate(Database, target = "db")]
pub struct BTCHandler {
    pub db: EncryptedDatabase,
    pub url: String,
}

impl BTCHandler {
    pub fn new(db_path: PathBuf, passwd: &str,
    ) -> Result<Self> {
        // TODO: InitDB
        Ok(Self {
            db: EncryptedDatabase::new(db_path, passwd, |c| Ok(()))?,
            url: "".to_string(),
        })
    }
}

#[async_trait(?Send)]
impl CoinApi for BTCHandler {
    fn coingecko_id(&self) -> &'static str {
        "bitcoin"
    }

    fn url(&self) -> String {
        self.url.clone()
    }
    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn list_accounts(&self) -> Result<AccountVecT> {
        list_accounts(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str, _index: Option<u32>) -> Result<u32> {
        let keys = key::derive_key(key)?;
        let id_account = db::store_keys(&self.connection(), name, &keys)?;
        Ok(id_account)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        key::derive_key(key).is_ok()
    }

    fn is_valid_address(&self, key: &str) -> bool {
        key::derive_address(key).is_ok()
    }

    fn get_backup(&self, account: u32) -> Result<BackupT> {
        super::db::read::get_backup(&self.connection(), account, |sk| {
            let sk = SecretKey::from_slice(&sk).unwrap();
            let privk = PrivateKey::new(sk, BTCNET);
            privk.to_wif()
        })
    }

    async fn sync(&mut self, _account: u32, _params: Vec<u8>) -> Result<u32> {
        sync::sync(&self.connection(), &self.url)
    }

    fn cancel_sync(&mut self) -> Result<()> {
        Ok(())
    }

    async fn get_latest_height(&self) -> Result<u32> {
        sync::get_height(&self.url)
    }

    fn reset_sync(&mut self, _height: u32) -> Result<()> {
        db::truncate(&self.connection())
    }

    fn get_balance(&self, account: u32) -> Result<u64> {
        db::get_balance(&self.connection(), account)
    }

    fn get_txs(&self, account: u32) -> Result<PlainTxVecT> {
        db::get_txs(&self.connection(), account)
    }

    fn get_notes(&self, account: u32) -> Result<PlainNoteVecT> {
        db::get_utxos(&self.connection(), account)
    }

    fn prepare_multi_payment(
        &self,
        account: u32,
        recipients: &RecipientsT,
        feeb: Option<u64>,
    ) -> Result<String> {
        let feeb = match feeb {
            Some(feeb) => feeb,
            None => sync::get_estimated_fee(&self.url)?,
        };
        pay::prepare(&self.connection(), account, recipients, feeb)
    }

    fn to_tx_report(&self, tx_plan: &str) -> Result<TxReportT> {
        pay::to_tx_report(tx_plan)
    }

    fn sign(&self, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
        pay::sign(&self.connection(), account, tx_plan)
    }

    fn broadcast(&self, raw_tx: &[u8]) -> Result<String> {
        sync::broadcast(&self.url, raw_tx)
    }
}
