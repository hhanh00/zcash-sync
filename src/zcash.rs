use crate::api::dart_ffi::POST_COBJ;
use crate::coin::{CoinApi, ZcashApi};
use crate::db::data_generated::fb::{
    AccountVecT, BackupT, PlainNoteVecT, PlainTxVecT, ProgressT, TxReportT, ZcashSyncParams,
};
use crate::{connect_lightwalletd, db, BTCHandler, ChainError, Progress, RecipientsT, TransactionPlan};
use allo_isolate::IntoDart;
use anyhow::Result;
use async_trait::async_trait;
use flatbuffers::FlatBufferBuilder;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;
use zcash_primitives::consensus::Network;

pub struct ZcashHandler {
    coin: u8,
    network: Network,
    coingecko_id: &'static str,
    connection: Mutex<Connection>,
    db_path: PathBuf,
    url: String,
    cancel: Mutex<Option<mpsc::Sender<()>>>,
}

fn progress(port: i64) -> impl Fn(Progress) {
    move |progress| unsafe {
        if let Some(p) = POST_COBJ {
            let progress = ProgressT {
                height: progress.height,
                trial_decryptions: progress.trial_decryptions,
                downloaded: progress.downloaded as u64,
            };
            let mut builder = FlatBufferBuilder::new();
            let root = progress.pack(&mut builder);
            builder.finish(root, None);
            let mut progress = builder.finished_data().to_vec().into_dart();
            p(port, &mut progress);
        }
    }
}

impl ZcashHandler {
    pub fn new(
        coin: u8,
        network: Network,
        coingecko_id: &'static str,
        connection: Connection,
        db_path: PathBuf,
    ) -> Self {
        ZcashHandler {
            coin,
            network,
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
    fn is_private(&self) -> bool {
        true
    }

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

    fn list_accounts(&self) -> Result<AccountVecT> {
        db::read::get_account_list(&self.connection())
    }

    fn new_account(&self, name: &str, key: &str, index: Option<u32>) -> Result<u32> {
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

    fn get_backup(&self, account: u32) -> Result<BackupT> {
        db::backup::get_backup_package(&self.network(), &self.connection(), account)
    }

    async fn sync(&mut self, _account: u32, params: Vec<u8>) -> Result<u32> {
        if self.cancel.lock().unwrap().is_some() {
            anyhow::bail!("Sync already in progress");
        }
        log::info!("Sync started");
        let root = flatbuffers::root::<ZcashSyncParams>(&params)?;
        let params = root.unpack();
        let progress_callback = progress(params.port);
        let (tx_cancel, rx_cancel) = mpsc::channel::<()>(1);
        {
            *self.cancel.lock().unwrap() = Some(tx_cancel);
        }
        let mut connection = Connection::open(&self.db_path)?;
        let new_height = crate::sync2::warp_sync_inner(
            self.network.clone(),
            &mut connection,
            &self.url,
            params.anchor_offset,
            params.max_cost,
            &progress_callback,
            self.coin == 0,
            rx_cancel,
        )
        .await;
        if let Err(err) = &new_height {
            if let Some(ChainError::Reorg(height)) = err.downcast_ref::<ChainError>() {
                let reorg_height = self.rewind_to_height(*height - 1)?;
                return Ok(reorg_height);
            }
        }
        {
            *self.cancel.lock().unwrap() = None;
        }
        log::info!("Sync finished");
        // TODO: Get tx details
        new_height
    }

    fn cancel_sync(&mut self) -> Result<()> {
        let cancel = self.cancel.lock().unwrap().take();
        if let Some(cancel) = cancel {
            let _ = cancel.send(());
        }
        Ok(())
    }

    async fn get_latest_height(&self) -> Result<u32> {
        let mut client = connect_lightwalletd(&self.url).await?;
        let height = crate::chain::get_latest_height(&mut client).await?;
        Ok(height)
    }

    fn skip_to_last_height(&mut self) -> Result<()> {
        let coin = self.coin;
        std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(crate::api::sync::skip_to_last_height(coin))
        })
        .join()
        .unwrap()
    }

    fn rewind_to_height(&mut self, height: u32) -> Result<u32> {
        crate::api::sync::rewind_to(height)
    }

    fn truncate(&mut self, height: u32) -> Result<()> {
        let coin = self.coin;
        std::thread::spawn(move || {
            let runtime = Runtime::new().unwrap();
            runtime.block_on(crate::api::sync::rescan_from(coin, height))
        })
        .join()
        .unwrap()
    }

    fn get_balance(&self, account: u32) -> Result<u64> {
        tokio::task::block_in_place(move || {
            Handle::current().block_on(crate::transparent::get_balance(
                &self.connection(),
                &self.url,
                account,
            ))
        })
    }

    // All these methods are specialized for zcash

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

    fn sign(&self, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        crate::pay::sign_plan(&self.network(), &self.connection(), account, &tx_plan)
    }

    fn broadcast(&self, _raw_tx: &[u8]) -> Result<String> {
        crate::pay::broadcast_tx(coin, &tx).await
    }

    fn mark_inputs_spent(&self, tx_plan: &str, height: u32) -> Result<()> {
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        crate::pay::mark_inputs_spent(&self.connection(), &tx_plan, height)
    }

    fn connection(&self) -> MutexGuard<Connection> {
        self.connection.lock().unwrap()
    }
}

#[async_trait]
impl ZcashApi for ZcashHandler {
    fn network(&self) -> Network {
        self.network.clone()
    }

    // fn new_sub_account(&self, name: &str, parent: u32, index: Option<u32>, count: u32) -> Result<()> {
    //     todo!()
    // }
}
