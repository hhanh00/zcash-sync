use crate::{connect_lightwalletd, CompactTxStreamerClient, DbAdapter, MemPool};
use lazy_static::lazy_static;
use lazycell::AtomicLazyCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tonic::transport::Channel;
use zcash_params::coin::{get_coin_chain, CoinChain, CoinType};
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_proofs::prover::LocalTxProver;

lazy_static! {
    pub static ref COIN_CONFIG: [Mutex<CoinConfig>; 2] = [
        Mutex::new(CoinConfig::new(0, CoinType::Zcash)),
        Mutex::new(CoinConfig::new(1, CoinType::Ycash)),
    ];
    pub static ref PROVER: AtomicLazyCell<LocalTxProver> = AtomicLazyCell::new();
}

pub static ACTIVE_COIN: AtomicU8 = AtomicU8::new(0);

pub fn set_active(active: u8) {
    ACTIVE_COIN.store(active, Ordering::Release);
}

pub fn set_active_account(coin: u8, id: u32) {
    let mempool = {
        let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
        c.id_account = id;
        c.mempool.clone()
    };
    let mut mempool = mempool.lock().unwrap();
    let _ = mempool.clear();
}

pub fn set_coin_lwd_url(coin: u8, lwd_url: &str) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.lwd_url = lwd_url.to_string();
}

pub fn init_coin(coin: u8, db_path: &str) -> anyhow::Result<()> {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.set_db_path(db_path)?;
    Ok(())
}

#[derive(Clone)]
pub struct CoinConfig {
    pub coin: u8,
    pub coin_type: CoinType,
    pub id_account: u32,
    pub height: u32,
    pub lwd_url: String,
    pub db_path: String,
    pub mempool: Arc<Mutex<MemPool>>,
    pub db: Option<Arc<Mutex<DbAdapter>>>,
    pub chain: &'static (dyn CoinChain + Send),
}

impl CoinConfig {
    pub fn new(coin: u8, coin_type: CoinType) -> Self {
        let chain = get_coin_chain(coin_type);
        CoinConfig {
            coin,
            coin_type,
            id_account: 0,
            height: 0,
            lwd_url: String::new(),
            db_path: String::new(),
            db: None,
            mempool: Arc::new(Mutex::new(MemPool::new(coin))),
            chain,
        }
    }

    pub fn set_db_path(&mut self, db_path: &str) -> anyhow::Result<()> {
        self.db_path = db_path.to_string();
        let db = DbAdapter::new(self.coin_type, &self.db_path)?;
        db.init_db()?;
        self.db = Some(Arc::new(Mutex::new(db)));
        Ok(())
    }

    pub fn get(coin: u8) -> CoinConfig {
        let c = COIN_CONFIG[coin as usize].lock().unwrap();
        c.clone()
    }

    pub fn get_active() -> CoinConfig {
        let coin = ACTIVE_COIN.load(Ordering::Acquire) as usize;
        let c = COIN_CONFIG[coin].lock().unwrap();
        c.clone()
    }

    pub fn set_height(height: u32) {
        let coin = ACTIVE_COIN.load(Ordering::Acquire) as usize;
        let mut c = COIN_CONFIG[coin].lock().unwrap();
        c.height = height;
    }

    pub fn mempool(&self) -> MutexGuard<MemPool> {
        self.mempool.lock().unwrap()
    }

    pub fn db(&self) -> anyhow::Result<MutexGuard<DbAdapter>> {
        let db = self.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        Ok(db)
    }

    pub async fn connect_lwd(&self) -> anyhow::Result<CompactTxStreamerClient<Channel>> {
        connect_lightwalletd(&self.lwd_url).await
    }
}

pub fn get_prover() -> &'static LocalTxProver {
    if !PROVER.filled() {
        let _ = PROVER.fill(LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS));
    }
    PROVER.borrow().unwrap()
}
