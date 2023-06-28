use crate::btc::BTCHandler;
use crate::coin::CoinApi;
use crate::coinconfig::{self, init_coin, CoinConfig, MEMPOOL};
use crate::db::data_generated::fb::{
    BalanceT, ContactVecT, MessageVecT, ProgressT, Recipients, SendTemplate, Servers,
    ShieldedNoteT, ShieldedNoteVecT, ShieldedTxT, ShieldedTxVecT, SpendingVecT, TxTimeValueVecT,
};
use crate::db::FullEncryptedBackup;
use crate::eth::ETHHandler;
use crate::mempool::MemPool;
use crate::note_selection::TransactionReport;
use crate::{init_btc_db, init_eth_db, ChainError, TransactionPlan, Tx};
use allo_isolate::{ffi, IntoDart};
use android_logger::Config;
use flatbuffers::FlatBufferBuilder;
use lazy_static::lazy_static;
use log::Level;
use rusqlite::Connection;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use tokio::sync::{oneshot, Semaphore};
use zcash_primitives::transaction::builder::Progress;

static mut POST_COBJ: Option<ffi::DartPostCObjectFnType> = None;

const MAX_COINS: u8 = 2;

fn with_coin<T, F: Fn(&Connection) -> anyhow::Result<T>>(coin: u8, f: F) -> anyhow::Result<T> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let connection = &db.connection;
    f(connection)
}

#[no_mangle]
pub unsafe extern "C" fn dummy_export() {}

macro_rules! fb_to_vec {
    ($v: ident) => {{
        let mut builder = FlatBufferBuilder::new();
        let root = $v.pack(&mut builder);
        builder.finish(root, None);
        builder.finished_data().to_vec()
    }};
}

#[no_mangle]
pub unsafe extern "C" fn dart_post_cobject(ptr: ffi::DartPostCObjectFnType) {
    POST_COBJ = Some(ptr);
}

macro_rules! from_c_str {
    ($v: ident) => {
        let $v = CStr::from_ptr($v).to_string_lossy();
    };
}

fn to_c_str(s: String) -> *mut c_char {
    CString::new(s).unwrap().into_raw()
}

fn to_cresult<T>(res: Result<T, anyhow::Error>) -> CResult<T> {
    let res = res.map_err(|e| e.to_string());
    match res {
        Ok(v) => CResult {
            value: v,
            len: 0,
            error: std::ptr::null_mut::<c_char>(),
        },
        Err(e) => {
            log::error!("{}", e);
            CResult {
                value: unsafe { std::mem::zeroed() },
                len: 0,
                error: to_c_str(e),
            }
        }
    }
}

fn to_cresult_str(res: Result<String, anyhow::Error>) -> CResult<*mut c_char> {
    let res = res.map(to_c_str);
    to_cresult(res)
}

fn log_error(res: Result<(), anyhow::Error>) {
    if let Err(e) = res {
        log::error!("{}", e.to_string());
    }
}

fn to_cresult_bytes(res: Result<Vec<u8>, anyhow::Error>) -> CResult<*const u8> {
    match res {
        Ok(v) => {
            let ptr = v.as_ptr();
            let len = v.len();
            std::mem::forget(v);
            CResult {
                value: ptr,
                len: len as u32,
                error: std::ptr::null_mut::<c_char>(),
            }
        }
        Err(e) => {
            log::error!("{}", e);
            CResult {
                value: unsafe { std::mem::zeroed() },
                len: 0,
                error: to_c_str(e.to_string()),
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn deallocate_str(s: *mut c_char) {
    let _ = CString::from_raw(s);
}

#[no_mangle]
pub unsafe extern "C" fn deallocate_bytes(ptr: *mut u8, len: u32) {
    drop(Vec::from_raw_parts(ptr, len as usize, len as usize));
}

fn try_init_logger() {
    android_logger::init_once(
        Config::default()
            // .format(|buf, record| {
            //     writeln!(
            //         buf,
            //         "{:?}-{:?}: {}",
            //         record.file(),
            //         record.line(),
            //         record.args()
            //     )
            // })
            .with_min_level(Level::Info),
    );
    let _ = env_logger::try_init();
}

#[repr(C)]
pub struct CResult<T> {
    value: T,
    error: *mut c_char,
    pub len: u32,
}

lazy_static! {
    static ref BTC_HANDLER: Mutex<Box<dyn CoinApi>> = Mutex::new(Box::new(crate::NoCoin {}));
    static ref ETH_HANDLER: Mutex<Box<dyn CoinApi>> = Mutex::new(Box::new(crate::NoCoin {}));
}

fn get_coin_handler(coin: u8) -> MutexGuard<'static, Box<dyn CoinApi>> {
    match coin {
        2 => BTC_HANDLER.lock().unwrap(),
        3 => ETH_HANDLER.lock().unwrap(),
        _ => unreachable!(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn init_wallet(coin: u8, db_path: *mut c_char) -> CResult<u8> {
    try_init_logger();
    from_c_str!(db_path);
    let res = || {
        match coin {
            2 => {
                let connection = Connection::open(&*db_path)?;
                init_btc_db(&connection)?;
                let handler = BTCHandler::new(connection, &*db_path);
                *BTC_HANDLER.lock().unwrap() = Box::new(handler);
            }
            3 => {
                let connection = Connection::open(&*db_path)?;
                init_eth_db(&connection)?;
                let handler = ETHHandler::new(connection, Path::new(&*db_path).to_path_buf());
                *ETH_HANDLER.lock().unwrap() = Box::new(handler);
            }
            _ => {
                init_coin(coin, &db_path)?;
            }
        }
        Ok::<_, anyhow::Error>(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn migrate_db(coin: u8, db_path: *mut c_char) -> CResult<u8> {
    try_init_logger();
    from_c_str!(db_path);
    let res = || {
        if coin < 2 {
            coinconfig::migrate_db(coin, &db_path)?;
        }
        Ok(0u8)
    };
    to_cresult(res())
}

#[no_mangle]
#[tokio::main]
pub async unsafe extern "C" fn migrate_data_db(coin: u8) -> CResult<u8> {
    try_init_logger();
    let res = async {
        if coin < 2 {
            coinconfig::migrate_data(coin).await?;
        }
        Ok(0u8)
    };
    to_cresult(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn set_active(active: u8) {
    crate::coinconfig::set_active(active);
}

#[no_mangle]
pub unsafe extern "C" fn set_coin_lwd_url(coin: u8, lwd_url: *mut c_char) {
    from_c_str!(lwd_url);
    if coin < 2 {
        coinconfig::set_coin_lwd_url(coin, &lwd_url);
    } else {
        get_coin_handler(coin).set_url(&lwd_url);
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_lwd_url(coin: u8) -> *mut c_char {
    let server = if coin < 2 {
        coinconfig::get_coin_lwd_url(coin)
    } else {
        get_coin_handler(coin).get_url()
    };
    to_c_str(server)
}

#[no_mangle]
pub unsafe extern "C" fn set_coin_passwd(coin: u8, passwd: *mut c_char) {
    from_c_str!(passwd);
    if coin < 2 {
        coinconfig::set_coin_passwd(coin, &passwd);
    }
}

#[no_mangle]
pub unsafe extern "C" fn reset_app() {
    let res = || {
        for i in 0..MAX_COINS {
            crate::api::account::reset_db(i)?;
        }
        Ok(())
    };
    log_error(res())
}

#[no_mangle]
#[tokio::main]
pub async unsafe extern "C" fn mempool_run(port: i64) {
    try_init_logger();
    log::info!("Starting MP");
    let mut mempool = MEMPOOL.lock().unwrap();
    let mp = MemPool::spawn(move |balance: i64| {
        let mut balance = balance.into_dart();
        if port != 0 {
            if let Some(p) = POST_COBJ {
                p(port, &mut balance);
            }
        }
    })
    .unwrap();
    *mempool = Some(mp);
    log::info!("end mempool_start");
}

#[no_mangle]
pub unsafe extern "C" fn mempool_set_active(coin: u8, id_account: u32) {
    let mempool = MEMPOOL.lock().unwrap();
    if coin < 2 {
        if let Some(mempool) = mempool.as_ref() {
            log::info!("MP active {coin} {id_account}");
            mempool.set_active(coin, id_account);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn new_account(
    coin: u8,
    name: *mut c_char,
    data: *mut c_char,
    index: i32,
) -> CResult<u32> {
    from_c_str!(name);
    from_c_str!(data);
    let key = if !data.is_empty() {
        Some(data.to_string())
    } else {
        None
    };
    let index = if index >= 0 { Some(index as u32) } else { None };

    let res = || {
        let id_account = if coin < 2 {
            crate::api::account::new_account(coin, &name, key, index)
        } else {
            get_coin_handler(coin).new_account(&name, &data)
        }?;
        Ok(id_account)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn new_sub_account(name: *mut c_char, index: i32, count: u32) {
    from_c_str!(name);
    let index = if index >= 0 { Some(index as u32) } else { None };
    let res = crate::api::account::new_sub_account(&name, index, count);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn convert_to_watchonly(coin: u8, id_account: u32) -> CResult<u8> {
    let res = crate::api::account::convert_to_watchonly(coin, id_account);
    to_cresult(res.and_then(|()| Ok(0u8)))
}

#[no_mangle]
pub unsafe extern "C" fn get_backup(coin: u8, id_account: u32) -> CResult<*const u8> {
    let res = || {
        let backup = if coin < 2 {
            crate::api::account::get_backup_package(coin, id_account)
        } else {
            get_coin_handler(coin).get_backup(id_account)
        }?;
        Ok::<_, anyhow::Error>(fb_to_vec!(backup))
    };

    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_available_addrs(coin: u8, account: u32) -> CResult<u8> {
    if coin >= 2 {
        return to_cresult(Ok(1)); // transparent only
    }
    let res = |connection: &Connection| {
        let res = crate::db::read::get_available_addrs(connection, account)?;
        Ok(res)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn get_address(
    coin: u8,
    id_account: u32,
    ua_type: u8,
) -> CResult<*mut c_char> {
    let address = if coin < 2 {
        crate::api::account::get_address(coin, id_account, ua_type)
    } else {
        get_coin_handler(coin).get_address(id_account)
    };
    to_cresult_str(address)
}

#[no_mangle]
pub unsafe extern "C" fn import_transparent_key(coin: u8, id_account: u32, path: *mut c_char) {
    from_c_str!(path);
    let res = crate::api::account::import_transparent_key(coin, id_account, &path);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn import_transparent_secret_key(
    coin: u8,
    id_account: u32,
    secret_key: *mut c_char,
) {
    from_c_str!(secret_key);
    let res = crate::api::account::import_transparent_secret_key(coin, id_account, &secret_key);
    log_error(res)
}

lazy_static! {
    static ref SYNC_LOCK: Semaphore = Semaphore::new(1);
    static ref TX_CANCEL: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);
}

#[no_mangle]
pub unsafe extern "C" fn cancel_warp() {
    log::info!("Sync canceled");
    let mut tx = TX_CANCEL.lock().unwrap();
    let tx = tx.take();
    tx.map(|tx| {
        log::info!("Cancellation sent");
        let _ = tx.send(());
    });
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn warp(
    coin: u8,
    get_tx: bool,
    anchor_offset: u32,
    max_cost: u32,
    port: i64,
) -> CResult<u8> {
    if coin >= 2 {
        return to_cresult(get_coin_handler(coin).sync().map(|_| 0));
    }

    let res = async {
        let _permit = SYNC_LOCK.acquire().await?;
        log::info!("Sync started");
        let (tx_cancel, rx_cancel) = oneshot::channel::<()>();
        *TX_CANCEL.lock().unwrap() = Some(tx_cancel);
        let result = crate::api::sync::coin_sync(
            coin,
            get_tx,
            anchor_offset,
            max_cost,
            move |progress| {
                let progress = ProgressT {
                    height: progress.height,
                    trial_decryptions: progress.trial_decryptions,
                    downloaded: progress.downloaded as u64,
                };
                let v = fb_to_vec!(progress);
                let mut progress = v.into_dart();
                if port != 0 {
                    if let Some(p) = POST_COBJ {
                        p(port, &mut progress);
                    }
                }
            },
            rx_cancel,
        )
        .await;
        log::info!("Sync finished");

        match result {
            Ok(new_blocks) => {
                log::info!("New blocks {new_blocks}");
                if new_blocks != 0 {
                    let mempool = MEMPOOL.lock().unwrap();
                    if let Some(mempool) = mempool.as_ref() {
                        mempool.new_block().await;
                    }
                }
                Ok(0)
            }
            Err(err) => {
                if let Some(e) = err.downcast_ref::<ChainError>() {
                    match e {
                        ChainError::Reorg => Ok(1),
                        ChainError::Busy => Ok(2),
                    }
                } else {
                    log::error!("{}", err);
                    Ok(0xFF)
                }
            }
        }
    };
    let r = res.await;
    *TX_CANCEL.lock().unwrap() = None;
    to_cresult(r)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_key(coin: u8, key: *mut c_char) -> i8 {
    from_c_str!(key);
    if coin < 2 {
        crate::key::is_valid_key(coin, &key)
    } else {
        if get_coin_handler(coin).is_valid_key(&key) {
            0
        } else {
            -1
        }
    }
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn transparent_sync(coin: u8, account: u32) -> CResult<u8> {
    if coin >= 2 {
        return to_cresult(Ok(0));
    }
    let res = async {
        crate::transparent::sync(coin, account).await?;
        Ok::<_, anyhow::Error>(0)
    };
    to_cresult(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn get_t_txs(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let txs = with_coin(coin, |connection: &Connection| {
            crate::transparent::get_txs(connection, account)
        })?;
        Ok(fb_to_vec!(txs))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_t_notes(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let utxos = with_coin(coin, |connection: &Connection| {
            crate::transparent::get_utxos(connection, account)
        })?;
        Ok(fb_to_vec!(utxos))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn valid_address(coin: u8, address: *mut c_char) -> bool {
    from_c_str!(address);
    if coin < 2 {
        crate::key::decode_address(coin, &address).is_some()
    } else {
        get_coin_handler(coin).is_valid_address(&address)
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_diversified_address(ua_type: u8, time: u32) -> CResult<*mut c_char> {
    let res = || crate::api::account::get_diversified_address(ua_type, time);
    to_cresult_str(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_latest_height(coin: u8) -> CResult<u32> {
    let res = async {
        if coin < 2 {
            let c = CoinConfig::get(coin);
            let mut client = c.connect_lwd().await?;
            let height = crate::chain::get_latest_height(&mut client).await?;
            Ok(height)
        } else {
            get_coin_handler(coin).get_latest_height()
        }
    };
    to_cresult(res.await)
}

#[allow(dead_code)]
fn report_progress(progress: Progress, port: i64) {
    if port != 0 {
        let progress = match progress.end() {
            Some(end) => (progress.cur() * 100 / end) as i32,
            None => -(progress.cur() as i32),
        };
        let mut progress = progress.into_dart();
        unsafe {
            if let Some(p) = POST_COBJ {
                p(port, &mut progress);
            }
        }
    }
}

// #[tokio::main]
// #[no_mangle]
// pub async unsafe extern "C" fn send_multi_payment(
//     coin: u8,
//     account: u32,
//     recipients_json: *mut c_char,
//     anchor_offset: u32,
//     port: i64,
// ) -> CResult<*mut c_char> {
//     from_c_str!(recipients_json);
//     let res = async move {
//         let height = crate::api::sync::get_latest_height().await?;
//         let recipients = crate::api::recipient::parse_recipients(&recipients_json)?;
//         let res = crate::api::payment_v2::build_sign_send_multi_payment(
//             coin,
//             account,
//             height,
//             &recipients,
//             anchor_offset,
//             Box::new(move |progress| {
//                 report_progress(progress, port);
//             }),
//         )
//         .await?;
//         Ok(res)
//     };
//     to_cresult_str(res.await)
// }

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn skip_to_last_height(coin: u8) {
    let res = if coin < 2 {
        crate::api::sync::skip_to_last_height(coin).await
    } else {
        Ok(())
    };
    log_error(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn rewind_to(height: u32) -> CResult<u32> {
    let res = crate::api::sync::rewind_to(height).await;
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn rescan_from(coin: u8, height: u32) {
    let res = async {
        if coin < 2 {
            crate::api::sync::rescan_from(coin, height).await
        } else {
            let mut h = get_coin_handler(coin);
            h.truncate()
        }
    };
    log_error(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_taddr_balance(coin: u8, id_account: u32) -> CResult<u64> {
    let res = async {
        if coin < 2 {
            crate::api::account::get_taddr_balance(coin, id_account).await
        } else {
            get_coin_handler(coin).get_balance(id_account)
        }
    };
    to_cresult(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn transfer_pools(
    coin: u8,
    account: u32,
    from_pool: u8,
    to_pool: u8,
    amount: u64,
    fee_included: bool,
    memo: *mut c_char,
    split_amount: u64,
    confirmations: u32,
) -> CResult<*mut c_char> {
    from_c_str!(memo);
    let res = async move {
        let tx_plan = crate::api::payment_v2::transfer_pools(
            coin,
            account,
            from_pool,
            to_pool,
            amount,
            fee_included,
            &memo,
            split_amount,
            confirmations,
        )
        .await?;
        let tx_plan = serde_json::to_string(&tx_plan)?;
        Ok::<_, anyhow::Error>(tx_plan)
    };
    to_cresult_str(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn shield_taddr(
    coin: u8,
    account: u32,
    amount: u64,
    confirmations: u32,
) -> CResult<*mut c_char> {
    let res = async move {
        let tx_plan =
            crate::api::payment_v2::shield_taddr(coin, account, amount, confirmations).await?;
        let tx_plan_json = serde_json::to_string(&tx_plan)?;
        Ok(tx_plan_json)
    };
    to_cresult_str(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn scan_transparent_accounts(
    coin: u8,
    account: u32,
    gap_limit: u32,
) -> CResult<*const u8> {
    let res = async {
        let addresses =
            crate::api::account::scan_transparent_accounts(coin, account, gap_limit as usize)
                .await?;
        Ok(fb_to_vec!(addresses))
    };
    to_cresult_bytes(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn prepare_multi_payment(
    coin: u8,
    account: u32,
    recipients_bytes: *mut u8,
    recipients_len: u64,
    anchor_offset: u32,
    excluded_pools: u8,
) -> CResult<*mut c_char> {
    let res = async {
        let recipients_bytes: Vec<u8> = Vec::from_raw_parts(
            recipients_bytes,
            recipients_len as usize,
            recipients_len as usize,
        );
        let recipients = flatbuffers::root::<Recipients>(&recipients_bytes)?;

        if coin < 2 {
            let c = CoinConfig::get(coin);
            let mut client = c.connect_lwd().await?;
            let last_height = crate::chain::get_latest_height(&mut client).await?;
            let recipients = crate::api::recipient::parse_recipients(&recipients)?;
            let tx = crate::api::payment_v2::build_tx_plan(
                coin,
                account,
                last_height,
                &recipients,
                excluded_pools,
                anchor_offset,
            )
            .await?;
            let tx_str = serde_json::to_string(&tx)?;
            Ok(tx_str)
        } else {
            get_coin_handler(coin).prepare_multi_payment(account, &recipients.unpack(), None)
        }
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn transaction_report(coin: u8, plan: *mut c_char) -> CResult<*const u8> {
    from_c_str!(plan);
    let res = || {
        let report = if coin < 2 {
            let c = CoinConfig::get(coin);
            let plan: TransactionPlan = serde_json::from_str(&plan)?;
            TransactionReport::from_plan(c.chain.network(), plan)
        } else {
            get_coin_handler(coin).to_tx_report(&plan)?
        };
        Ok(fb_to_vec!(report))
    };
    to_cresult_bytes(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sign(
    coin: u8,
    account: u32,
    tx_plan: *mut c_char,
    _port: i64,
) -> CResult<*mut c_char> {
    from_c_str!(tx_plan);
    let res = async {
        let raw_tx = if coin < 2 {
            let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
            crate::api::payment_v2::sign_plan(coin, account, &tx_plan)
        } else {
            get_coin_handler(coin).sign(account, &tx_plan)
        }?;
        let tx_str = base64::encode(&raw_tx);
        Ok(tx_str)
    };
    to_cresult_str(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sign_and_broadcast(
    coin: u8,
    account: u32,
    tx_plan: *mut c_char,
) -> CResult<*mut c_char> {
    from_c_str!(tx_plan);
    let res = async {
        if coin < 2 {
            let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
            let txid = crate::api::payment_v2::sign_and_broadcast(coin, account, &tx_plan).await?;
            Ok::<_, anyhow::Error>(txid)
        } else {
            let h = get_coin_handler(coin);
            let raw_tx = h.sign(account, &tx_plan)?;
            h.broadcast(&raw_tx)
        }
    };
    let res = res.await;
    to_cresult_str(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn broadcast_tx(coin: u8, tx_str: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(tx_str);
    let res = async {
        let tx = base64::decode(&*tx_str)?;
        if coin < 2 {
            crate::broadcast_tx(coin, &tx).await
        } else {
            get_coin_handler(coin).broadcast(&tx)
        }
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_tkey(sk: *mut c_char) -> bool {
    from_c_str!(sk);
    crate::taddr::parse_seckey(&sk).is_ok()
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sweep_tkey(
    last_height: u32,
    sk: *mut c_char,
    pool: u8,
    confirmations: u32,
) -> CResult<*mut c_char> {
    from_c_str!(sk);
    let txid = crate::taddr::sweep_tkey(last_height, &sk, pool, confirmations).await;
    to_cresult_str(txid)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_activation_date() -> CResult<u32> {
    let res = crate::api::sync::get_activation_date().await;
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_block_by_time(time: u32) -> CResult<u32> {
    let res = crate::api::sync::get_block_by_time(time).await;
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sync_historical_prices(
    coin: u8,
    now: i64,
    days: u32,
    currency: *mut c_char,
) -> CResult<u32> {
    from_c_str!(currency);
    let res = if coin < 2 {
        crate::api::historical_prices::sync_historical_prices(coin, now, days, &currency).await
    } else {
        let h = get_coin_handler(coin);
        let connection = &mut h.connection();
        crate::api::historical_prices::sync_historical_prices_inner(
            connection,
            h.coingecko_id(),
            now,
            days,
            &currency,
        )
        .await
    };
    to_cresult(res)
}

#[no_mangle]
pub unsafe extern "C" fn store_contact(
    id: u32,
    name: *mut c_char,
    address: *mut c_char,
    dirty: bool,
) {
    from_c_str!(name);
    from_c_str!(address);
    let res = crate::api::contact::store_contact(id, &name, &address, dirty);
    log_error(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn commit_unsaved_contacts(anchor_offset: u32) -> CResult<*mut c_char> {
    let res = async move {
        let tx_plan = crate::api::contact::commit_unsaved_contacts(anchor_offset).await?;
        let tx_plan_json = serde_json::to_string(&tx_plan)?;
        Ok(tx_plan_json)
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn mark_message_read(message: u32, read: bool) {
    let res = crate::api::message::mark_message_read(message, read);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn mark_all_messages_read(read: bool) {
    let res = crate::api::message::mark_all_messages_read(read);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn truncate_data() {
    let res = crate::api::account::truncate_data();
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn truncate_sync_data() {
    let res = crate::api::account::truncate_sync_data();
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn check_account(coin: u8, account: u32) -> bool {
    if coin < 2 {
        crate::api::account::check_account(coin, account)
    } else {
        get_coin_handler(coin).has_account(account).unwrap_or(false)
    }
}

#[no_mangle]
pub unsafe extern "C" fn delete_account(coin: u8, account: u32) {
    let res = if coin < 2 {
        crate::api::account::delete_account(coin, account)
    } else {
        get_coin_handler(coin).delete_account(account)
    };
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn make_payment_uri(
    coin: u8,
    address: *mut c_char,
    amount: u64,
    memo: *mut c_char,
) -> CResult<*mut c_char> {
    from_c_str!(memo);
    from_c_str!(address);
    let res = crate::api::payment_uri::make_payment_uri(coin, &address, amount, &memo);
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn parse_payment_uri(coin: u8, uri: *mut c_char) -> CResult<*const u8> {
    from_c_str!(uri);
    let payment_json = || {
        let payment = crate::api::payment_uri::parse_payment_uri(coin, &uri)?;
        Ok(fb_to_vec!(payment))
    };
    to_cresult_bytes(payment_json())
}

#[no_mangle]
pub unsafe extern "C" fn generate_key() -> CResult<*const u8> {
    let res = || {
        let secret_key = FullEncryptedBackup::generate_key()?;
        Ok(fb_to_vec!(secret_key))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn zip_backup(key: *mut c_char, dst_dir: *mut c_char) -> CResult<u8> {
    from_c_str!(key);
    from_c_str!(dst_dir);
    let res = || {
        let mut backup = FullEncryptedBackup::new(&dst_dir);
        for coin in 0..MAX_COINS {
            let c = CoinConfig::get(coin);
            let db = c.db().unwrap();
            let db_path = Path::new(&db.db_path);
            let db_name = db_path.file_name().unwrap().to_string_lossy();
            backup.add(&db.connection, &db_name)?;
        }
        let coin_handlers: &[&Mutex<Box<dyn CoinApi>>] = &[&BTC_HANDLER, &ETH_HANDLER];
        for h in coin_handlers {
            let h = h.lock().unwrap();
            let db_path = Path::new(h.db_path());
            backup.add(
                &h.connection(),
                &db_path.file_name().unwrap().to_string_lossy(),
            )?;
        }
        backup.close(&key)?;
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn unzip_backup(
    key: *mut c_char,
    data_path: *mut c_char,
    dst_dir: *mut c_char,
) -> CResult<u8> {
    from_c_str!(key);
    from_c_str!(data_path);
    from_c_str!(dst_dir);
    let res = || {
        let backup = FullEncryptedBackup::new(&dst_dir);
        backup.restore(&key, &data_path)?;
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn split_data(id: u32, data: *mut c_char) -> CResult<*const u8> {
    from_c_str!(data);
    let res = || {
        let qdrops =
            crate::fountain::FountainCodes::encode_into_drops(id, &base64::decode(&*data)?)?;
        Ok(fb_to_vec!(qdrops))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn merge_data(drop: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(drop);
    let res = || {
        let res = crate::fountain::RaptorQDrops::put_drop(&*drop)?
            .map(|d| base64::encode(&d))
            .unwrap_or(String::new());
        Ok::<_, anyhow::Error>(res)
    };
    let res = res().or(Ok(String::new()));
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn get_tx_summary(tx: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(tx);
    let res = || {
        let tx: Tx = serde_json::from_str(&tx)?;
        let summary = crate::pay::get_tx_summary(&tx)?;
        let summary = serde_json::to_string(&summary)?;
        Ok::<_, anyhow::Error>(summary)
    };
    to_cresult_str(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_best_server(servers: *mut u8, len: u64) -> CResult<*mut c_char> {
    let servers: Vec<u8> = Vec::from_raw_parts(servers, len as usize, len as usize);
    let res = async {
        let servers = flatbuffers::root::<Servers>(&servers)?;
        let best_server = crate::get_best_server(servers).await?;
        Ok(best_server)
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn import_from_zwl(coin: u8, name: *mut c_char, data: *mut c_char) {
    from_c_str!(name);
    from_c_str!(data);
    let res = crate::api::account::import_from_zwl(coin, &name, &data);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn derive_zip32(
    coin: u8,
    id_account: u32,
    account: u32,
    external: u32,
    has_address: bool,
    address: u32,
) -> CResult<*const u8> {
    let res = || {
        let address = if has_address { Some(address) } else { None };
        let kp = crate::api::account::derive_keys(coin, id_account, account, external, address)?;
        Ok(fb_to_vec!(kp))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn clear_tx_details(coin: u8, account: u32) -> CResult<u8> {
    if coin >= 2 {
        return to_cresult(Ok(0u8));
    }
    let res = |connection: &Connection| {
        crate::DbAdapter::clear_tx_details(connection, account)?;
        Ok(0)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn get_account_list(coin: u8) -> CResult<*const u8> {
    let res = || {
        let accounts = if coin >= 2 {
            get_coin_handler(coin).list_accounts()
        } else {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_account_list(connection)
            })
        }?;
        Ok(fb_to_vec!(accounts))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_active_account(coin: u8) -> CResult<u32> {
    let res = || {
        if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                let new_id = crate::db::read::get_active_account(connection)?;
                Ok(new_id)
            })
        } else {
            get_coin_handler(coin).get_active_account()
        }
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn set_active_account(coin: u8, id: u32) -> CResult<u8> {
    let res = || {
        if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                coinconfig::set_active_account(coin, id);
                crate::db::read::set_active_account(connection, id)?;
                Ok(())
            })?;
        } else {
            get_coin_handler(coin).set_active_account(id)?;
        }
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_t_addr(coin: u8, id: u32) -> CResult<*mut c_char> {
    let res = || {
        if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                let address = crate::db::read::get_t_addr(connection, id)?;
                Ok(address)
            })
        } else {
            get_coin_handler(coin).get_address(id)
        }
    };
    to_cresult_str(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_sk(coin: u8, id: u32) -> CResult<*mut c_char> {
    let res = || {
        if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_sk(connection, id)
            })
        } else {
            let b = get_coin_handler(coin).get_backup(id)?;
            Ok(b.sk.unwrap_or_default())
        }
    };
    to_cresult_str(res())
}

#[no_mangle]
pub unsafe extern "C" fn update_account_name(coin: u8, id: u32, name: *mut c_char) -> CResult<u8> {
    from_c_str!(name);
    let res = || {
        if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::update_account_name(connection, id, &name)
            })?;
        } else {
            get_coin_handler(coin).update_name(id, &name)?;
        }
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_balances(
    coin: u8,
    id: u32,
    confirmed_height: u32,
    filter_excluded: bool,
) -> CResult<*const u8> {
    let res = || {
        let balances = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_balances(connection, id, confirmed_height, filter_excluded)
            })
        } else {
            let balance = get_coin_handler(coin).get_balance(id)?;
            Ok(BalanceT {
                balance,
                ..BalanceT::default()
            })
        }?;
        Ok(fb_to_vec!(balances))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_db_height(coin: u8) -> CResult<*const u8> {
    let res = || {
        let height = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_db_height(connection)
            })
        } else {
            get_coin_handler(coin).get_db_height()
        }?
        .unwrap_or_default();
        Ok(fb_to_vec!(height))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_notes(coin: u8, id: u32) -> CResult<*const u8> {
    let res = || {
        let notes = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_notes(connection, id)
            })
        } else {
            get_coin_handler(coin).get_notes(id).map(|ns| {
                let notes = ns.notes.unwrap();
                let notes: Vec<_> = notes
                    .into_iter()
                    .map(|n| ShieldedNoteT {
                        id,
                        height: n.height,
                        value: n.value,
                        timestamp: n.timestamp,
                        ..ShieldedNoteT::default()
                    })
                    .collect();
                ShieldedNoteVecT { notes: Some(notes) }
            })
        }?;
        Ok(fb_to_vec!(notes))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_txs(coin: u8, id: u32) -> CResult<*const u8> {
    let res = || {
        let txs = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                let c = CoinConfig::get(coin);
                crate::db::read::get_txs(c.chain.network(), connection, id)
            })
        } else {
            get_coin_handler(coin).get_txs(id).map(|txs| {
                let txs: Vec<_> = txs
                    .txs
                    .unwrap()
                    .into_iter()
                    .map(|tx| ShieldedTxT {
                        id,
                        tx_id: tx.tx_id.clone(),
                        height: tx.height,
                        short_tx_id: tx.tx_id.clone().map(|id| id[0..4].to_string()),
                        timestamp: tx.timestamp,
                        value: tx.value,
                        ..ShieldedTxT::default()
                    })
                    .collect();
                ShieldedTxVecT { txs: Some(txs) }
            })
        }?;
        Ok(fb_to_vec!(txs))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_messages(coin: u8, id: u32) -> CResult<*const u8> {
    let res = || {
        let messages = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                let c = CoinConfig::get(coin);
                crate::db::read::get_messages(c.chain.network(), connection, id)
            })
        } else {
            Ok(MessageVecT {
                messages: Some(vec![]),
            })
        }?;
        Ok(fb_to_vec!(messages))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_prev_next_message(
    coin: u8,
    id: u32,
    subject: *mut c_char,
    height: u32,
) -> CResult<*const u8> {
    from_c_str!(subject);
    let res = |connection: &Connection| {
        let data = crate::db::read::get_prev_next_message(connection, &subject, height, id)?;
        Ok(data)
    };
    to_cresult_bytes(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn get_templates(coin: u8) -> CResult<*const u8> {
    let res = |connection: &Connection| {
        let data = crate::db::read::get_templates(connection)?;
        Ok(data)
    };
    to_cresult_bytes(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn save_send_template(coin: u8, template: *mut u8, len: u64) -> CResult<u32> {
    let template: Vec<u8> = Vec::from_raw_parts(template, len as usize, len as usize);
    let res = || {
        let c = CoinConfig::get(coin);
        let db = c.db()?;
        let template = flatbuffers::root::<SendTemplate>(&template)?;
        let id = db.store_template(&template)?;
        Ok(id)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn delete_send_template(coin: u8, id: u32) -> CResult<u8> {
    let res = || {
        let c = CoinConfig::get(coin);
        let db = c.db()?;
        db.delete_template(id)?;
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_contacts(coin: u8) -> CResult<*const u8> {
    let res = || {
        let contacts = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_contacts(connection)
            })
        } else {
            Ok(ContactVecT {
                contacts: Some(vec![]),
            })
        }?;
        Ok(fb_to_vec!(contacts))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_pnl_txs(coin: u8, id: u32, timestamp: u32) -> CResult<*const u8> {
    let res = || {
        let timeseries = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_pnl_txs(connection, id, timestamp)
            })
        } else {
            Ok(TxTimeValueVecT {
                values: Some(vec![]),
            })
        }?;
        Ok(fb_to_vec!(timeseries))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_historical_prices(
    coin: u8,
    timestamp: u32,
    currency: *mut c_char,
) -> CResult<*const u8> {
    from_c_str!(currency);
    let res = || {
        let quotes = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_historical_prices(connection, timestamp, &currency)
            })
        } else {
            let h = get_coin_handler(coin);
            let connection = h.connection();
            crate::db::read::get_historical_prices(&connection, timestamp, &currency)
        }?;
        Ok(fb_to_vec!(quotes))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_spendings(coin: u8, id: u32, timestamp: u32) -> CResult<*const u8> {
    let res = || {
        let spendings = if coin < 2 {
            with_coin(coin, |connection: &Connection| {
                crate::db::read::get_spendings(connection, id, timestamp)
            })
        } else {
            Ok(SpendingVecT {
                values: Some(vec![]),
            })
        }?;
        Ok(fb_to_vec!(spendings))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn update_excluded(coin: u8, id: u32, excluded: bool) -> CResult<u8> {
    let res = |connection: &Connection| {
        crate::db::read::update_excluded(connection, id, excluded)?;
        Ok(0)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn invert_excluded(coin: u8, id: u32) -> CResult<u8> {
    let res = |connection: &Connection| {
        crate::db::read::invert_excluded(connection, id)?;
        Ok(0)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn get_checkpoints(coin: u8) -> CResult<*const u8> {
    let res = |connection: &Connection| {
        let data = crate::db::read::get_checkpoints(connection)?;
        Ok(data)
    };
    to_cresult_bytes(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn decrypt_db(db_path: *mut c_char, passwd: *mut c_char) -> CResult<bool> {
    from_c_str!(passwd);
    from_c_str!(db_path);
    let res = || {
        let connection = Connection::open(&*db_path)?;
        let valid = crate::db::cipher::check_passwd(&connection, &passwd)?;
        Ok(valid)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn clone_db_with_passwd(
    coin: u8,
    temp_path: *mut c_char,
    passwd: *mut c_char,
) -> CResult<u8> {
    from_c_str!(passwd);
    from_c_str!(temp_path);
    let res = |connection: &Connection| {
        crate::db::cipher::clone_db_with_passwd(connection, &temp_path, &passwd)?;
        Ok(0)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn get_property(coin: u8, name: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(name);
    if coin >= 2 {
        return to_cresult_str(get_coin_handler(coin).get_property(&name));
    }
    let res = |connection: &Connection| {
        let value = crate::db::read::get_property(connection, &name)?;
        Ok(value)
    };
    to_cresult_str(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn set_property(
    coin: u8,
    name: *mut c_char,
    value: *mut c_char,
) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(value);
    if coin >= 2 {
        return to_cresult(
            get_coin_handler(coin)
                .set_property(&name, &value)
                .map(|_| 0),
        );
    }
    let res = |connection: &Connection| {
        crate::db::read::set_property(connection, &name, &value)?;
        Ok(0)
    };
    to_cresult(with_coin(coin, res))
}

#[no_mangle]
pub unsafe extern "C" fn import_uvk(coin: u8, name: *mut c_char, yfvk: *mut c_char) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(yfvk);
    let res = || {
        crate::key::import_uvk(coin, &name, &yfvk)?;
        Ok(0)
    };
    to_cresult(res())
}

#[cfg(feature = "ledger")]
#[no_mangle]
#[tokio::main]
pub async unsafe extern "C" fn ledger_send(coin: u8, tx_plan: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(tx_plan);
    let res = async {
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        let c = CoinConfig::get(coin);
        let prover = coinconfig::get_prover();
        let pk = crate::orchard::get_proving_key();
        let raw_tx = tokio::task::spawn_blocking(move || {
            let (pubkey, dfvk, ofvk) = crate::ledger::ledger_get_fvks()?;
            let raw_tx = crate::ledger::build_ledger_tx(
                c.chain.network(),
                &tx_plan,
                &pubkey,
                &dfvk,
                ofvk,
                prover,
                &pk,
            )?;
            Ok::<_, anyhow::Error>(raw_tx)
        })
        .await??;
        let response = crate::broadcast_tx(coin, &raw_tx).await?;
        Ok::<_, anyhow::Error>(response)
    };
    to_cresult_str(res.await)
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_import_account(coin: u8, name: *mut c_char) -> CResult<u32> {
    from_c_str!(name);
    let account = crate::ledger::import_account(coin, &name);
    to_cresult(account)
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_has_account(coin: u8, account: u32) -> CResult<bool> {
    let res = |connection: &Connection| crate::ledger::is_external(connection, account);
    to_cresult(with_coin(coin, res))
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_toggle_binding(coin: u8, account: u32) -> CResult<u8> {
    to_cresult(with_coin(coin, |connection: &Connection| {
        crate::ledger::toggle_binding(connection, account)?;
        Ok(0)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn has_cuda() -> bool {
    crate::gpu::has_cuda()
}

#[no_mangle]
pub unsafe extern "C" fn has_metal() -> bool {
    crate::gpu::has_metal()
}

#[no_mangle]
pub unsafe extern "C" fn has_gpu() -> bool {
    crate::gpu::has_gpu()
}

#[no_mangle]
pub unsafe extern "C" fn use_gpu(v: bool) {
    crate::gpu::use_gpu(v)
}
