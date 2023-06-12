use crate::db::data_generated::fb::{
    AccountT, AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
use std::sync::MutexGuard;

use anyhow::Result;
use rusqlite::Connection;

pub trait CoinApi: Send {
    fn db_path(&self) -> &str;
    fn coingecko_id(&self) -> &'static str;
    fn get_url(&self) -> String;
    fn set_url(&mut self, url: &str);
    fn get_property(&self, name: &str) -> Result<String> {
        super::btc::get_property(&self.connection(), name)
    }
    fn set_property(&mut self, name: &str, value: &str) -> Result<()> {
        super::btc::set_property(&self.connection(), name, value)
    }

    fn list_accounts(&self) -> Result<AccountVecT>;
    fn get_account(&self, account: u32) -> Result<AccountT> {
        super::btc::get_account(&self.connection(), account)
    }
    fn get_address(&self, account: u32) -> Result<String> {
        super::btc::get_address(&self.connection(), account)
    }
    fn new_account(&self, name: &str, key: &str) -> Result<u32>;
    fn convert_to_view(&self, account: u32) -> Result<()> {
        super::btc::delete_secrets(&self.connection(), account)
    }
    fn has_account(&self, account: u32) -> Result<bool> {
        super::btc::has_account(&self.connection(), account)
    }
    fn update_name(&self, account: u32, name: &str) -> Result<()> {
        super::btc::update_name(&self.connection(), account, name)
    }
    fn delete_account(&self, account: u32) -> Result<()> {
        super::btc::delete_account(&self.connection(), account)
    }
    fn get_active_account(&self) -> Result<u32> {
        crate::db::read::get_active_account(&self.connection())
    }
    fn set_active_account(&self, account: u32) -> Result<()> {
        crate::db::read::set_active_account(&self.connection(), account)
    }

    fn is_valid_key(&self, key: &str) -> bool;
    fn is_valid_address(&self, key: &str) -> bool;
    fn get_backup(&self, account: u32) -> Result<BackupT> {
        super::btc::get_backup(&self.connection(), account)
    }

    fn sync(&mut self) -> Result<()>;
    fn cancel_sync(&mut self) -> Result<()>;
    fn get_latest_height(&self) -> Result<u32>;
    fn get_db_height(&self) -> Result<Option<HeightT>> {
        super::btc::get_db_height(&self.connection())
    }
    fn skip_to_last_height(&mut self) -> Result<()>;
    fn rewind_to_height(&mut self, height: u32) -> Result<()>;
    fn truncate(&mut self) -> Result<()>;

    fn get_balance(&self, account: u32) -> Result<u64>;
    fn get_txs(&self, account: u32) -> Result<PlainTxVecT>;
    fn get_notes(&self, account: u32) -> Result<PlainNoteVecT>;

    fn prepare_multi_payment(
        &self,
        account: u32,
        recipients: &RecipientsT,
        feeb: Option<u64>,
    ) -> Result<String>;
    fn to_tx_report(&self, tx_plan: &str) -> Result<TxReportT>;
    fn sign(&self, account: u32, tx_plan: &str) -> Result<Vec<u8>>;
    fn broadcast(&self, raw_tx: &[u8]) -> Result<String>;

    fn connection(&self) -> MutexGuard<Connection>;
}

pub struct NoCoin;

impl CoinApi for NoCoin {
    fn db_path(&self) -> &str {
        unimplemented!()
    }

    fn coingecko_id(&self) -> &'static str {
        unimplemented!()
    }

    fn get_url(&self) -> String {
        unimplemented!()
    }

    fn set_url(&mut self, _url: &str) {
        unimplemented!()
    }

    fn list_accounts(&self) -> Result<AccountVecT> {
        unimplemented!()
    }

    fn new_account(&self, _name: &str, _key: &str) -> Result<u32> {
        unimplemented!()
    }

    fn is_valid_key(&self, _key: &str) -> bool {
        unimplemented!()
    }

    fn is_valid_address(&self, _key: &str) -> bool {
        unimplemented!()
    }

    fn sync(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn cancel_sync(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn get_latest_height(&self) -> Result<u32> {
        unimplemented!()
    }

    fn skip_to_last_height(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn rewind_to_height(&mut self, _height: u32) -> Result<()> {
        unimplemented!()
    }

    fn truncate(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn get_balance(&self, _account: u32) -> Result<u64> {
        unimplemented!()
    }

    fn get_txs(&self, _account: u32) -> Result<PlainTxVecT> {
        unimplemented!()
    }

    fn get_notes(&self, _account: u32) -> Result<PlainNoteVecT> {
        unimplemented!()
    }

    fn prepare_multi_payment(
        &self,
        _account: u32,
        _recipients: &RecipientsT,
        _feeb: Option<u64>,
    ) -> Result<String> {
        unimplemented!()
    }

    fn to_tx_report(&self, _tx_plan: &str) -> Result<TxReportT> {
        unimplemented!()
    }

    fn sign(&self, _account: u32, _tx_plan: &str) -> Result<Vec<u8>> {
        unimplemented!()
    }

    fn broadcast(&self, _raw_tx: &[u8]) -> Result<String> {
        unimplemented!()
    }

    fn connection(&self) -> MutexGuard<Connection> {
        unimplemented!()
    }
}
