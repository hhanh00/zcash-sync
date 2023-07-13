use anyhow::Result;
use rusqlite::Connection;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use zcash_primitives::consensus::Network;

mod account;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct AccountId(u8, u32);

#[derive(Debug)]
pub enum MPCtl {
    Open(AccountId),
    NewBlock,
    Balance(AccountId, i64),
}

pub struct MemPool {
    runtime: Runtime,
    tx: mpsc::Sender<MPCtl>,
}

impl MemPool {
    pub fn spawn<F: Fn(i64) + Send + Sync + 'static>(notify: F) -> Result<MemPool> {
        let (tx_ctl, mut rx_ctl) = mpsc::channel::<MPCtl>(8);
        let tx_ctl2 = tx_ctl.clone();
        let mut worker = MPWorker::new();
        let runtime = Runtime::new().unwrap();
        runtime.spawn(async move {
            while let Some(m) = rx_ctl.recv().await {
                log::info!("{:?}", m);
                match m {
                    MPCtl::Open(account_id) => {
                        worker.set(account_id).await;
                        worker.open(tx_ctl2.clone()).await;
                    }
                    MPCtl::NewBlock => {
                        worker.open(tx_ctl2.clone()).await;
                    }
                    MPCtl::Balance(account_id, balance) => {
                        if worker.account_id == Some(account_id) {
                            notify(balance);
                        }
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        });
        Ok(MemPool {
            runtime,
            tx: tx_ctl,
        })
    }

    pub fn set_active(&self, coin: u8, id: u32) {
        if id != 0 {
            let _ = self.tx.blocking_send(MPCtl::Open(AccountId(coin, id)));
        }
    }

    pub async fn new_block(&self) {
        let _ = self.tx.send(MPCtl::NewBlock).await;
    }
}

struct MPWorker {
    account_id: Option<AccountId>,
    tx_close: Option<mpsc::Sender<()>>,
}

impl MPWorker {
    fn new() -> Self {
        MPWorker {
            account_id: None,
            tx_close: None,
        }
    }

    async fn set(&mut self, account_id: AccountId) {
        self.close().await;
        self.account_id = Some(account_id);
    }

    async fn open(&mut self, network: &Network, connection: &Connection, url: &str, tx_balance: mpsc::Sender<MPCtl>) {
        self.close().await;
        if let Some(account_id) = self.account_id {
            let (tx, rx) = mpsc::channel::<()>(1);
            self.tx_close = Some(tx);
            account::spawn(network, connection, url, account_id.0, account_id.1, rx, tx_balance).unwrap();
        }
    }

    async fn close(&mut self) {
        if let Some(tx) = self.tx_close.take() {
            let _ = tx.send(()).await;
        }
    }
}
