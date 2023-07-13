use crate::db::data_generated::fb::{
    AccountT, AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
use std::sync::MutexGuard;

use crate::ton::TonHandler;
use crate::tron::TronHandler;
use crate::zcash::ZcashHandler;
use crate::{BTCHandler, ETHHandler};
use anyhow::Result;
use async_trait::async_trait;
use rusqlite::Connection;
use zcash_primitives::consensus::Network;

#[async_trait(?Send)]
#[enum_delegate::register]
pub trait CoinApi {
    fn is_private(&self) -> bool {
        false
    }
    fn db_path(&self) -> &str;
    fn coingecko_id(&self) -> &'static str;
    fn url(&self) -> String;
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
    fn new_account(&self, name: &str, key: &str, index: Option<u32>) -> Result<u32>;
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
    fn get_backup(&self, account: u32) -> Result<BackupT>;

    async fn sync(&mut self, account: u32, params: Vec<u8>) -> Result<u32>;
    fn cancel_sync(&mut self) -> Result<()>;
    async fn get_latest_height(&self) -> Result<u32>;
    fn get_db_height(&self, _account: u32) -> Result<Option<HeightT>> {
        super::btc::get_db_height(&self.connection())
    }
    fn skip_to_last_height(&mut self) -> Result<()>;
    fn rewind_to_height(&mut self, height: u32) -> Result<u32>;
    fn truncate(&mut self, height: u32) -> Result<()>;

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
    fn mark_inputs_spent(&self, _tx_plan: &str, _height: u32) -> Result<()> { Ok(()) }

    fn connection(&self) -> MutexGuard<Connection>;
}

#[async_trait(?Send)]
pub trait ZcashApi: Send {
    fn network(&self) -> Network;
    // fn new_sub_account(&self, name: &str, parent: u32, index: Option<u32>, count: u32) -> Result<()>;
    // fn get_available_addrs(&self, account: u32) -> Result<u8>;
    // fn get_ua(&self, account: u32, ua_type: u8) -> Result<String>;
    // async fn transparent_sync(&self, account: u32) -> Result<()>;
    // fn get_diversified_address(&self, account: u32, ua_type: u8, time: u32) -> Result<String>;
}

pub struct NoCoin;

#[async_trait(?Send)]
impl CoinApi for NoCoin {
    fn db_path(&self) -> &str {
        unimplemented!()
    }

    fn coingecko_id(&self) -> &'static str {
        unimplemented!()
    }

    fn url(&self) -> String {
        unimplemented!()
    }

    fn set_url(&mut self, _url: &str) {
        unimplemented!()
    }

    fn list_accounts(&self) -> Result<AccountVecT> {
        unimplemented!()
    }

    fn new_account(&self, _name: &str, _key: &str, _index: Option<u32>) -> Result<u32> {
        unimplemented!()
    }

    fn is_valid_key(&self, _key: &str) -> bool {
        unimplemented!()
    }

    fn is_valid_address(&self, _key: &str) -> bool {
        unimplemented!()
    }

    fn get_backup(&self, _account: u32) -> Result<BackupT> {
        unimplemented!()
    }

    async fn sync(&mut self, _account: u32, _params: Vec<u8>) -> Result<u32> {
        unimplemented!()
    }

    fn cancel_sync(&mut self) -> Result<()> {
        unimplemented!()
    }

    async fn get_latest_height(&self) -> Result<u32> {
        unimplemented!()
    }

    fn skip_to_last_height(&mut self) -> Result<()> {
        unimplemented!()
    }

    fn rewind_to_height(&mut self, _height: u32) -> Result<u32> {
        unimplemented!()
    }

    fn truncate(&mut self, _height: u32) -> Result<()> {
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

// #[enum_delegate::implement(CoinApi)]
pub enum CoinHandler {
    NoCoin(NoCoin),
    Zcash(ZcashHandler),
    BTC(BTCHandler),
    ETH(ETHHandler),
    TON(TonHandler),
    TRON(TronHandler),
}

impl Default for CoinHandler {
    fn default() -> Self {
        CoinHandler::NoCoin(NoCoin {})
    }
}

impl ZcashApi for CoinHandler {
    fn network(&self) -> Network {
        match self {
            CoinHandler::Zcash(zcash) => zcash.network(),
            _ => unimplemented!(),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for CoinHandler {
    fn db_path(&self) -> &str {
        todo!()
    }

    fn coingecko_id(&self) -> &'static str {
        todo!()
    }

    fn url(&self) -> String {
        todo!()
    }

    fn set_url(&mut self, url: &str) {
        todo!()
    }

    fn list_accounts(&self) -> Result<AccountVecT> {
        todo!()
    }

    fn new_account(&self, name: &str, key: &str, index: Option<u32>) -> Result<u32> {
        todo!()
    }

    fn is_valid_key(&self, key: &str) -> bool {
        todo!()
    }

    fn is_valid_address(&self, key: &str) -> bool {
        todo!()
    }

    fn get_backup(&self, account: u32) -> Result<BackupT> {
        todo!()
    }

    async fn sync(&mut self, account: u32, params: Vec<u8>) -> Result<u32> {
        todo!()
    }

    fn cancel_sync(&mut self) -> Result<()> {
        todo!()
    }

    async fn get_latest_height(&self) -> Result<u32> {
        todo!()
    }

    fn skip_to_last_height(&mut self) -> Result<()> {
        todo!()
    }

    fn rewind_to_height(&mut self, height: u32) -> Result<u32> {
        todo!()
    }

    fn truncate(&mut self, height: u32) -> Result<()> {
        todo!()
    }

    fn get_balance(&self, account: u32) -> Result<u64> {
        todo!()
    }

    fn get_txs(&self, account: u32) -> Result<PlainTxVecT> {
        todo!()
    }

    fn get_notes(&self, account: u32) -> Result<PlainNoteVecT> {
        todo!()
    }

    fn prepare_multi_payment(&self, account: u32, recipients: &RecipientsT, feeb: Option<u64>) -> Result<String> {
        todo!()
    }

    fn to_tx_report(&self, tx_plan: &str) -> Result<TxReportT> {
        todo!()
    }

    fn sign(&self, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
        todo!()
    }

    fn broadcast(&self, raw_tx: &[u8]) -> Result<String> {
        todo!()
    }

    fn connection(&self) -> MutexGuard<Connection> {
        todo!()
    }
}
