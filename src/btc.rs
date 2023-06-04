mod db;
mod key;
mod pay;
mod sync;

use crate::coin::CoinApi;
use crate::db::data_generated::fb::{
    AccountT, AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
pub use db::init_db;
use electrum_client::bitcoin::Network;
use rusqlite::Connection;
use std::sync::{Mutex, MutexGuard};

pub const BTCNET: Network = Network::Testnet;

pub struct BTCHandler {
    pub connection: Mutex<Connection>,
    pub url: String,
}

impl BTCHandler {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection: Mutex::new(connection),
            url: "".to_string(),
        }
    }
}

impl CoinApi for BTCHandler {
    fn coingecko_id(&self) -> &'static str {
        "bitcoin"
    }

    fn get_url(&self) -> String {
        self.url.clone()
    }
    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn get_property(&self, name: &str) -> anyhow::Result<String> {
        db::get_property(&self.connection.lock().unwrap(), name)
    }

    fn set_property(&mut self, name: &str, value: &str) -> anyhow::Result<()> {
        db::set_property(&self.connection.lock().unwrap(), name, value)
    }

    fn list_accounts(&self) -> anyhow::Result<AccountVecT> {
        db::list_accounts(&self.connection.lock().unwrap())
    }

    fn get_account(&self, account: u32) -> anyhow::Result<AccountT> {
        db::get_account(&self.connection.lock().unwrap(), account)
    }

    fn get_address(&self, account: u32) -> anyhow::Result<String> {
        db::get_address(&self.connection.lock().unwrap(), account)
    }

    fn new_account(&self, name: &str, key: &str) -> anyhow::Result<u32> {
        let keys = key::derive_key(key)?;
        println!("{keys:?}");
        let id_account = db::store_keys(&self.connection.lock().unwrap(), name, &keys)?;
        Ok(id_account)
    }

    fn convert_to_view(&self, account: u32) -> anyhow::Result<()> {
        db::delete_secrets(&self.connection.lock().unwrap(), account)
    }

    fn has_account(&self, account: u32) -> anyhow::Result<bool> {
        db::has_account(&self.connection.lock().unwrap(), account)
    }

    fn update_name(&self, account: u32, name: &str) -> anyhow::Result<()> {
        db::update_name(&self.connection.lock().unwrap(), account, name)
    }

    fn delete_account(&self, account: u32) -> anyhow::Result<()> {
        db::delete_account(&self.connection.lock().unwrap(), account)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        key::derive_key(key).is_ok()
    }

    fn is_valid_address(&self, key: &str) -> bool {
        key::derive_address(key).is_ok()
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        db::get_backup(&self.connection.lock().unwrap(), account)
    }

    fn sync(&mut self) -> anyhow::Result<()> {
        sync::sync(&self.connection.lock().unwrap(), &self.url)
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn get_latest_height(&self) -> anyhow::Result<u32> {
        sync::get_height(&self.url)
    }

    fn get_db_height(&self) -> anyhow::Result<Option<HeightT>> {
        db::get_height(&self.connection.lock().unwrap())
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        db::rewind_to(&self.connection.lock().unwrap(), height)
    }

    fn truncate(&mut self) -> anyhow::Result<()> {
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
