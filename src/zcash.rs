use crate::coin::CoinApi;
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, PlainNoteVecT, PlainTxVecT, TxReportT, ZcashSyncParams,
};
use crate::{connect_lightwalletd, ChainError, RecipientsT};
use async_trait::async_trait;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

pub struct ZcashHandler {
    coin: u8,
    coingecko_id: &'static str,
    connection: Mutex<Connection>,
    db_path: PathBuf,
    url: String,
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

impl ZcashHandler {
    pub fn new(
        coin: u8,
        coingecko_id: &'static &str,
        connection: Connection,
        db_path: PathBuf,
    ) -> Self {
        ZcashHandler {
            coin,
            coingecko_id,
            connection: Mutex::new(connection),
            db_path,
            url: String::new(),
            cancel: Mutex::new(None),
        }
    }
}

#[async_trait(?Send)]
impl CoinApi for ZcashHandler {
    fn db_path(&self) -> &str {
        self.db_path.to_str().unwrap()
    }

    fn coingecko_id(&self) -> &'static str {
        self.coingecko_id
    }

    fn get_url(&self) -> String {
        self.url.clone()
    }

    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
    }

    fn list_accounts(&self) -> anyhow::Result<AccountVecT> {
        crate::db::read::get_account_list(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str, index: Option<u32>) -> anyhow::Result<u32> {
        let key = if !key.is_empty() {
            Some(key.to_owned())
        } else {
            None
        };
        crate::api::account::new_account(self.coin, &name, key, index)
    }

    fn is_valid_key(&self, key: &str) -> bool {
        crate::key::is_valid_key(self.coin, &key) >= 0
    }

    fn is_valid_address(&self, address: &str) -> bool {
        crate::key::decode_address(self.coin, &address).is_some()
    }

    fn get_backup(&self, account: u32) -> anyhow::Result<BackupT> {
        crate::api::account::get_backup_package(self.coin, account)
    }

    async fn sync(&mut self, _account: u32, params: Vec<u8>) -> anyhow::Result<()> {
        if self.cancel.lock().unwrap().is_some() {
            return Ok(());
        }
        log::info!("Sync started");
        let root = flatbuffers::root::<ZcashSyncParams>(&params)?;
        let params = root.unpack();
        let (tx_cancel, rx_cancel) = oneshot::channel::<()>();
        {
            *self.cancel.lock().unwrap() = Some(tx_cancel);
        }
        let coin = self.coin;
        let res = std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(async move {
                crate::sync::warp(
                    coin,
                    params.get_tx,
                    params.anchor_offset,
                    params.max_cost,
                    params.port,
                    rx_cancel,
                )
                .await
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .join()
        .unwrap();
        if let Err(err) = &res {
            if let Some(ChainError::Reorg(height)) = err.downcast_ref::<ChainError>() {
                self.rewind_to_height(*height - 1)?;
            }
        }
        {
            *self.cancel.lock().unwrap() = None;
        }
        log::info!("Sync finished");
        Ok(())
    }

    fn cancel_sync(&mut self) -> anyhow::Result<()> {
        let cancel = self.cancel.lock().unwrap().take();
        if let Some(cancel) = cancel {
            let _ = cancel.send(());
        }
        Ok(())
    }

    async fn get_latest_height(&self) -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd(&self.url).await?;
        let height = crate::chain::get_latest_height(&mut client).await?;
        Ok(height)
    }

    fn skip_to_last_height(&mut self) -> anyhow::Result<()> {
        let coin = self.coin;
        std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(crate::api::sync::skip_to_last_height(coin))
        })
        .join()
        .unwrap()
    }

    fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<u32> {
        crate::api::sync::rewind_to(height)
    }

    fn truncate(&mut self, height: u32) -> anyhow::Result<()> {
        let coin = self.coin;
        std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(crate::api::sync::rescan_from(coin, height))
        })
        .join()
        .unwrap()
    }

    fn get_balance(&self, account: u32) -> anyhow::Result<u64> {
        let coin = self.coin;
        std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(crate::api::account::get_taddr_balance(coin, account))
        })
        .join()
        .unwrap()
    }

    // All these methods are specialized for zcash

    fn get_txs(&self, _account: u32) -> anyhow::Result<PlainTxVecT> {
        unimplemented!()
    }

    fn get_notes(&self, _account: u32) -> anyhow::Result<PlainNoteVecT> {
        unimplemented!()
    }

    fn prepare_multi_payment(
        &self,
        _account: u32,
        _recipients: &RecipientsT,
        _feeb: Option<u64>,
    ) -> anyhow::Result<String> {
        unimplemented!()
    }

    fn to_tx_report(&self, _tx_plan: &str) -> anyhow::Result<TxReportT> {
        unimplemented!()
    }

    fn sign(&self, _account: u32, _tx_plan: &str) -> anyhow::Result<Vec<u8>> {
        unimplemented!()
    }

    fn broadcast(&self, _raw_tx: &[u8]) -> anyhow::Result<String> {
        unimplemented!()
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}
