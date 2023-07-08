mod db;
mod key;
mod pay;
mod sync;

use crate::coin::CoinApi;
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
use async_trait::async_trait;
pub use db::{
    delete_account, delete_secrets, get_account, get_address, get_height as get_db_height,
    get_property, has_account, init_db, list_accounts, set_property, update_name,
};
use electrum_client::bitcoin::secp256k1::SecretKey;
use electrum_client::bitcoin::{Network, PrivateKey};
use rusqlite::Connection;
use std::sync::{Mutex, MutexGuard};

pub const BTCNET: Network = Network::Bitcoin;

pub struct BTCHandler {
    pub db_path: String,
    pub connection: Mutex<Connection>,
    pub url: String,
}

impl BTCHandler {
    pub fn new(connection: Connection, db_path: &str) -> Self {
        Self {
            db_path: db_path.to_string(),
            connection: Mutex::new(connection),
            url: "".to_string(),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for BTCHandler {
    fn db_path(&self) -> &str {
        &self.db_path
    }

    fn coingecko_id(&self) -> &'static str {
        "bitcoin"
    }

    fn get_url(&self) -> String {
        self.url.clone()
    }
    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn list_accounts(&self) -> anyhow::Result<AccountVecT> {
        list_accounts(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str, _index: Option<u32>) -> anyhow::Result<u32> {
        let keys = key::derive_key(key)?;
        println!("{keys:?}");
        let id_account = db::store_keys(&self.connection.lock().unwrap(), name, &keys)?;
        Ok(id_account)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        key::derive_key(key).is_ok()
    }

    fn is_valid_address(&self, key: &str) -> bool {
        key::derive_address(key).is_ok()
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        super::db::read::get_backup(&self.connection(), account, |sk| {
            let sk = SecretKey::from_slice(&sk).unwrap();
            let privk = PrivateKey::new(sk, BTCNET);
            privk.to_wif()
        })
    }

    async fn sync(&mut self, _account: u32, _params: Vec<u8>) -> anyhow::Result<u32> {
        sync::sync(&self.connection.lock().unwrap(), &self.url)
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_latest_height(&self) -> anyhow::Result<u32> {
        sync::get_height(&self.url)
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<u32> {
        db::rewind_to(&self.connection.lock().unwrap(), height)
    }

    fn truncate(&mut self, _height: u32) -> anyhow::Result<()> {
        db::truncate(&self.connection.lock().unwrap())
    }

    fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        db::get_balance(&self.connection.lock().unwrap(), account)
    }

    fn get_txs(&self, account: u32) -> anyhow::Result<PlainTxVecT> {
        db::get_txs(&self.connection.lock().unwrap(), account)
    }

    fn get_notes(&self, account: u32) -> anyhow::Result<PlainNoteVecT> {
        db::get_utxos(&self.connection.lock().unwrap(), account)
    }

    fn prepare_multi_payment(
        &self,
        account: u32,
        recipients: &RecipientsT,
        feeb: Option<u64>,
    ) -> anyhow::Result<String> {
        let feeb = match feeb {
            Some(feeb) => feeb,
            None => sync::get_estimated_fee(&self.url)?,
        };
        pay::prepare(&self.connection.lock().unwrap(), account, recipients, feeb)
    }

    fn to_tx_report(&self, tx_plan: &str) -> anyhow::Result<TxReportT> {
        pay::to_tx_report(tx_plan)
    }

    fn sign(&self, account: u32, tx_plan: &str) -> anyhow::Result<Vec<u8>> {
        pay::sign(&self.connection.lock().unwrap(), account, tx_plan)
    }

    fn broadcast(&self, raw_tx: &[u8]) -> anyhow::Result<String> {
        sync::broadcast(&self.url, raw_tx)
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}
