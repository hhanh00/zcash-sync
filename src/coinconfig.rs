use crate::fountain::FountainCodes;
use crate::mempool::{MemPool, MemPoolRunner};
use crate::{connect_lightwalletd, CompactTxStreamerClient, DbAdapter};
use anyhow::anyhow;
use lazy_static::lazy_static;
use lazycell::AtomicLazyCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tonic::transport::Channel;
use zcash_params::coin::{get_coin_chain, CoinChain};
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_proofs::prover::LocalTxProver;

lazy_static! {
    pub static ref COIN_CONFIG: [Mutex<CoinConfig>; 3] = [
        Mutex::new(CoinConfig::new(0)),
        Mutex::new(CoinConfig::new(1)),
        Mutex::new(CoinConfig::new(2)),
    ];
    pub static ref PROVER: AtomicLazyCell<LocalTxProver> = AtomicLazyCell::new();
    pub static ref RAPTORQ: Mutex<FountainCodes> = Mutex::new(FountainCodes::new());
    pub static ref MEMPOOL: AtomicLazyCell<MemPool> = AtomicLazyCell::new();
    pub static ref MEMPOOL_RUNNER: Mutex<MemPoolRunner> = Mutex::new(MemPoolRunner::new());
}

pub static ACTIVE_COIN: AtomicU8 = AtomicU8::new(0);

/// Set the active coin
pub fn set_active(active: u8) {
    ACTIVE_COIN.store(active, Ordering::Release);
}

/// Set the active account for a given coin
pub fn set_active_account(coin: u8, id: u32) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.id_account = id;
    if let Some(mempool) = MEMPOOL.borrow() {
        mempool.set_active(coin, id);
    }
}

/// Set the lightwalletd url for a given coin
pub fn set_coin_lwd_url(coin: u8, lwd_url: &str) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.lwd_url = Some(lwd_url.to_string());
}

/// Get the URL of the lightwalletd server for a given coin
#[allow(dead_code)] // Used by C FFI
pub fn get_coin_lwd_url(coin: u8) -> String {
    let c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.lwd_url.clone().unwrap_or_default()
}

/// Set the db passwd
pub fn set_coin_passwd(coin: u8, passwd: &str) {
    let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
    c.passwd = passwd.to_string();
}

/// Initialize a coin with a database path
pub fn init_coin(coin: u8, db_path: &str) -> anyhow::Result<()> {
    {
        let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
        c.set_db_path(db_path)?;
    }
    migrate_db(coin, db_path)?;
    {
        let mut c = COIN_CONFIG[coin as usize].lock().unwrap();
        c.open_db()?;
    }
    Ok(())
}

/// Upgrade database schema for given coin and db path
/// Used from ywallet
pub fn migrate_db(coin: u8, db_path: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    match coin {
        2 => {
            let connection = DbAdapter::open_or_create(db_path, &c.passwd)?;
            crate::bitcoin::migrate_db(&connection)?;
        }
        _ => {
            let chain = c.chain;
            DbAdapter::migrate_db(chain.network(), db_path, &c.passwd, chain.has_unified())?;
        }
    }
    Ok(())
}

pub async fn migrate_data(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.migrate_data(coin).await?;
    Ok(())
}

#[derive(Clone)]
pub struct CoinConfig {
    pub coin: u8,
    pub id_account: u32,
    pub height: u32,
    pub lwd_url: Option<String>,
    pub passwd: String,
    pub db_path: Option<String>,
    pub db: Option<Arc<Mutex<DbAdapter>>>,
    pub chain: &'static (dyn CoinChain + Send),
}

impl CoinConfig {
    pub fn new(coin: u8) -> Self {
        let chain = get_coin_chain(coin);
        CoinConfig {
            coin,
            id_account: 0,
            height: 0,
            lwd_url: None,
            passwd: String::new(),
            db_path: None,
            db: None,
            chain,
        }
    }

    pub fn set_db_path(&mut self, db_path: &str) -> anyhow::Result<()> {
        self.db_path = Some(db_path.to_string());
        Ok(())
    }

    pub fn open_db(&mut self) -> anyhow::Result<()> {
        let mut db = DbAdapter::new(self.coin, self.db_path.as_ref().unwrap(), &self.passwd)?;
        if self.coin < 2 {
            db.init_db()?;
        }
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

    pub fn db(&self) -> anyhow::Result<MutexGuard<DbAdapter>> {
        let db = self.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        Ok(db)
    }

    pub async fn connect_lwd(&self) -> anyhow::Result<CompactTxStreamerClient<Channel>> {
        if let Some(lwd_url) = &self.lwd_url {
            connect_lightwalletd(lwd_url).await
        } else {
            Err(anyhow!("LWD URL Not set"))
        }
    }

    pub fn url(&self) -> &str {
        self.lwd_url.as_ref().expect("URL must be set")
    }
}

pub fn get_prover() -> &'static LocalTxProver {
    if !PROVER.filled() {
        let _ = PROVER.fill(LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS));
    }
    PROVER.borrow().unwrap()
}
