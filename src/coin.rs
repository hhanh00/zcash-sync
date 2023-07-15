use ambassador::Delegate;
use crate::db::cipher::check_passwd;
use crate::db::data_generated::fb::{
    AccountT, AccountVecT, BackupT, HeightT, PlainNoteVecT, PlainTxVecT, RecipientsT, TxReportT,
};
use crate::ton::TonHandler;
use crate::tron::TronHandler;
use crate::zcash::ZcashHandler;
use crate::{BTCHandler, ETHHandler};
use ambassador::delegatable_trait;
use anyhow::Result;
use async_trait::async_trait;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use zcash_primitives::consensus::Network;

pub struct EncryptedDatabase {
    db_path: PathBuf,
    pub pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    passwd: String,
}

impl EncryptedDatabase {
    pub fn new<F: Fn(&Connection) -> Result<()>>(
        db_path: PathBuf,
        passwd: &str,
        init: F,
    ) -> Result<Self> {
        let manager = r2d2_sqlite::SqliteConnectionManager::file(&db_path);
        let pool = r2d2::Pool::new(manager).unwrap();
        let connection = pool.get().unwrap();
        if !check_passwd(&connection, passwd)? {
            anyhow::bail!("Invalid password");
        }
        init(&connection)?;
        Ok(EncryptedDatabase {
            db_path,
            pool,
            passwd: passwd.to_owned(),
        })
    }
}

#[delegatable_trait]
pub trait Database {
    fn db_path(&self) -> &Path;
    fn connection(&self) -> crate::Connection;
}

impl Database for EncryptedDatabase {
    fn db_path(&self) -> &Path {
        self.db_path.as_path()
    }

    fn connection(&self) -> crate::Connection {
        let connection = self.pool.get().unwrap();
        let _ = crate::db::cipher::set_db_passwd(&connection, &self.passwd);
        connection
    }
}

#[async_trait(?Send)]
#[delegatable_trait]
pub trait CoinApi: Database {
    fn is_private(&self) -> bool {
        false
    }
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
    fn reset_sync(&mut self, height: u32) -> Result<()>;

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
    fn mark_inputs_spent(&self, _tx_plan: &str, _height: u32) -> Result<()> {
        Ok(())
    }
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

impl Database for NoCoin {
    fn db_path(&self) -> &Path {
        unimplemented!()
    }

    fn connection(&self) -> crate::Connection {
        unimplemented!()
    }
}

#[async_trait(?Send)]
impl CoinApi for NoCoin {
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

    fn reset_sync(&mut self, _height: u32) -> Result<()> {
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
}

#[derive(Delegate)]
#[delegate(Database)]
#[delegate(CoinApi)]
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
