use crate::btc::BTCHandler;
use crate::coin::{Database, CoinApi, ZcashApi};
use crate::db::data_generated::fb;
use crate::db::FullEncryptedBackup;
use crate::eth::ETHHandler;
use std::collections::HashSet;
// use crate::mempool::MemPool;
use crate::pay::Tx;
use crate::ton::{init_ton_db, TonHandler};
use crate::tron::{init_tron_db, TronHandler};
use crate::zcash::ZcashHandler;
use crate::CoinHandler;
use crate::{db, init_btc_db, init_eth_db, TransactionPlan, TransactionReport};
use allo_isolate::{ffi, IntoDart};
use android_logger::Config;
use anyhow::anyhow;
use flatbuffers::FlatBufferBuilder;
use lazy_static::lazy_static;
use log::Level;
use parking_lot::{Mutex, MutexGuard};
use rusqlite::Connection;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::str::FromStr;
use zcash_primitives::consensus::Network::{MainNetwork, YCashMainNetwork};
use zcash_primitives::transaction::builder::Progress;

pub static mut POST_COBJ: Option<ffi::DartPostCObjectFnType> = None;

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

macro_rules! from_c_str {
    ($v: ident) => {
        let $v = CStr::from_ptr($v).to_string_lossy();
    };
}

#[no_mangle]
pub unsafe extern "C" fn dart_post_cobject(ptr: ffi::DartPostCObjectFnType) {
    POST_COBJ = Some(ptr);
}

#[repr(C)]
pub struct CResult<T> {
    pub value: T,
    error: *mut c_char,
    pub len: u32,
}

#[repr(C)]
pub struct CResultUnit {
    error: *mut c_char,
    pub len: u32,
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

fn to_cresult_unit(res: Result<(), anyhow::Error>) -> CResult<u8> {
    let res = res.map(|_| 0u8);
    to_cresult(res)
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

lazy_static! {
    static ref ZCASH_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref YCASH_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref BTC_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref ETH_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref TON_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref TRON_HANDLER: Mutex<CoinHandler> = Mutex::new(CoinHandler::default());
    static ref REGISTERED_COINS: Mutex<HashSet<u8>> = Mutex::new(HashSet::new());
}

fn get_coin_handler(coin: u8) -> MutexGuard<'static, CoinHandler> {
    match coin {
        0 => ZCASH_HANDLER.lock(),
        1 => YCASH_HANDLER.lock(),
        2 => BTC_HANDLER.lock(),
        3 => ETH_HANDLER.lock(),
        4 => TON_HANDLER.lock(),
        5 => TRON_HANDLER.lock(),
        _ => unreachable!(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn init_wallet(coin: u8, db_path: *mut c_char, passwd: *mut c_char) -> CResult<u8> {
    try_init_logger();
    from_c_str!(db_path);
    from_c_str!(passwd);
    let res = || {
        let db_path = PathBuf::from_str(&db_path)?;
        {
            let mut coins = REGISTERED_COINS.lock();
            coins.insert(coin);
        }
        match coin {
            0 => {
                let handler = ZcashHandler::new(coin, MainNetwork, "zcash", db_path, &passwd)?;
                *ZCASH_HANDLER.lock() = CoinHandler::Zcash(handler);
            }
            1 => {
                // db::migration::init_db(&connection, &YCashMainNetwork, false)?;
                let handler = ZcashHandler::new(coin, YCashMainNetwork, "ycash", db_path, &passwd)?;
                *YCASH_HANDLER.lock() = CoinHandler::Zcash(handler);
            }
            2 => {
                // init_btc_db(&connection)?;
                let handler = BTCHandler::new(db_path, &passwd)?;
                *BTC_HANDLER.lock() = CoinHandler::BTC(handler);
            }
            3 => {
                // init_eth_db(&connection)?;
                let handler = ETHHandler::new(db_path, &passwd)?;
                *ETH_HANDLER.lock() = CoinHandler::ETH(handler);
            }
            4 => {
                // init_ton_db(&connection)?;
                let handler = TonHandler::new(db_path, &passwd)?;
                *TON_HANDLER.lock() = CoinHandler::TON(handler);
            }
            5 => {
                // init_tron_db(&connection)?;
                let handler = TronHandler::new(db_path, &passwd)?;
                *TRON_HANDLER.lock() = CoinHandler::TRON(handler);
            }
            _ => unreachable!(),
        }
        Ok::<_, anyhow::Error>(0)
    };
    to_cresult(res())
}

// #[no_mangle]
// pub unsafe extern "C" fn set_active(active: u8) {
//     crate::coinconfig::set_active(active);
// }

#[no_mangle]
pub unsafe extern "C" fn set_coin_lwd_url(coin: u8, lwd_url: *mut c_char) {
    from_c_str!(lwd_url);
    get_coin_handler(coin).set_url(&lwd_url);
}

#[no_mangle]
pub unsafe extern "C" fn get_lwd_url(coin: u8) -> *mut c_char {
    let server = get_coin_handler(coin).url();
    to_c_str(server)
}

#[no_mangle]
pub unsafe extern "C" fn set_coin_passwd(_coin: u8, passwd: *mut c_char) {
    from_c_str!(passwd);
    // TODO
}

// #[no_mangle]
// pub unsafe extern "C" fn reset_app() -> CResult<u8> {
//     let res = || {
//         for i in 0..MAX_COINS {
//             crate::api::account::reset_db(i)?;
//         }
//         Ok(())
//     };
//     to_cresult_unit(res())
// }

// #[no_mangle]
// #[tokio::main]
// pub async unsafe extern "C" fn mempool_run(port: i64) {
//     try_init_logger();
//     log::info!("Starting MP");
//     let mut mempool = crate::coinconfig::MEMPOOL.lock().unwrap();
//     let mp = MemPool::spawn(move |balance: i64| {
//         let mut balance = balance.into_dart();
//         if port != 0 {
//             if let Some(p) = POST_COBJ {
//                 p(port, &mut balance);
//             }
//         }
//     })
//     .unwrap();
//     *mempool = Some(mp);
//     log::info!("end mempool_start");
// }

// #[no_mangle]
// pub unsafe extern "C" fn mempool_set_active(coin: u8, account: u32) {
//     let mempool = crate::coinconfig::MEMPOOL.lock().unwrap();
//     if coin < 2 {
//         if let Some(mempool) = mempool.as_ref() {
//             log::info!("MP active {coin} {account}");
//             mempool.set_active(coin, account);
//         }
//     }
// }

#[no_mangle]
pub unsafe extern "C" fn new_account(
    coin: u8,
    name: *mut c_char,
    data: *mut c_char,
    index: i32,
) -> CResult<u32> {
    from_c_str!(name);
    from_c_str!(data);
    let index = if index >= 0 { Some(index as u32) } else { None };

    let res = || {
        let account = get_coin_handler(coin).new_account(&name, &data, index)?;
        Ok(account)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn new_sub_account(
    coin: u8,
    name: *mut c_char,
    account: u32,
    index: i32,
    count: u32,
) -> CResult<u8> {
    from_c_str!(name);
    let index = if index >= 0 { Some(index as u32) } else { None };
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::account::new_sub_account(
        &h.network(),
        &h.connection(),
        account,
        &name,
        index,
        count,
    );
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn convert_to_watchonly(coin: u8, account: u32) -> CResult<u8> {
    let res = get_coin_handler(coin).convert_to_view(account);
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn get_backup(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let backup = get_coin_handler(coin).get_backup(account)?;
        Ok::<_, anyhow::Error>(fb_to_vec!(backup))
    };

    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_available_addrs(coin: u8, account: u32) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        if h.is_private() {
            db::account::get_available_addrs(&h.connection(), account)
        } else {
            Ok(1)
        }
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_address(coin: u8, account: u32, ua_type: u8) -> CResult<*mut c_char> {
    let res = || {
        let h = get_coin_handler(coin);
        if h.is_private() {
            crate::account::get_unified_address(&h.network(), &h.connection(), account, ua_type)
        } else {
            h.get_address(account)
        }
    };
    to_cresult_str(res())
}

#[no_mangle]
pub async unsafe extern "C" fn cancel_warp(coin: u8) {
    log::info!("Sync canceled");
    let _ = get_coin_handler(coin).cancel_sync();
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn warp(
    coin: u8,
    anchor_offset: u32,
    max_cost: u32,
    port: i64,
) -> CResult<u32> {
    let sync_params = fb::ZcashSyncParamsT {
        anchor_offset,
        max_cost,
        port,
    };
    let sync_params = fb_to_vec!(sync_params);
    to_cresult(
        get_coin_handler(coin)
            .sync(0 /* all accounts are synced */, sync_params)
            .await,
    )
    // TODO: Mempool clear
    // Mempool is disabled atm
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn fetch_tx_details(coin: u8, account: u32) -> CResult<u8> {
    let res = async {
        let h = get_coin_handler(coin);
        crate::transaction::fetch_transaction_details(
            &h.network(),
            &h.connection(),
            &h.url(),
            account,
        )
        .await
    };
    to_cresult_unit(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_key(coin: u8, key: *mut c_char) -> bool {
    from_c_str!(key);
    get_coin_handler(coin).is_valid_key(&key)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_address(coin: u8, address: *mut c_char) -> bool {
    from_c_str!(address);
    get_coin_handler(coin).is_valid_address(&address)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn trp_sync(coin: u8, account: u32) -> CResult<u8> {
    let res = async {
        let h = get_coin_handler(coin);
        if h.is_private() {
            crate::transparent::sync(&h.network(), &h.connection(), &h.url(), account).await
        } else {
            Ok(())
        }
    };
    to_cresult_unit(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn get_trp_txs(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let txs = get_coin_handler(coin).get_txs(account)?;
        Ok(fb_to_vec!(txs))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_trp_notes(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let notes = get_coin_handler(coin).get_notes(account)?;
        Ok(fb_to_vec!(notes))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_diversified_address(
    coin: u8,
    account: u32,
    ua_type: u8,
    time: u32,
) -> CResult<*mut c_char> {
    let res = || {
        let h = get_coin_handler(coin);
        if h.is_private() {
            crate::unified::get_diversified_address(
                &h.network(),
                &h.connection(),
                account,
                ua_type,
                time,
            )
        } else {
            Ok("".to_owned())
        }
    };
    to_cresult_str(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_latest_height(coin: u8) -> CResult<u32> {
    let res = async { get_coin_handler(coin).get_latest_height().await };
    to_cresult(res.await)
}

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

#[no_mangle]
pub unsafe extern "C" fn rewind_to(coin: u8, height: u32) -> CResult<u32> {
    let res = || {
        let h = get_coin_handler(coin);
        crate::sync2::rewind_to(&h.network(), &mut h.connection(), height)
    };

    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn rescan_from(coin: u8, height: u32) -> CResult<u8> {
    let res = || {
        let mut h = get_coin_handler(coin);
        h.reset_sync(height)
    };
    to_cresult_unit(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_taddr_balance(coin: u8, account: u32) -> CResult<u64> {
    let res = || get_coin_handler(coin).get_balance(account);
    to_cresult(res())
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
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = async move {
        let tx_plan = crate::pool::transfer_pools(
            &h.network(),
            &h.connection(),
            &h.url(),
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
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        let tx_plan = crate::pool::shield_taddr(
            &h.network(),
            &h.connection(),
            &h.url(),
            account,
            amount,
            confirmations,
        )
        .await?;
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
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        let addresses = crate::account::scan_transparent_accounts(
            &h.network(),
            &h.connection(),
            &h.url(),
            account,
            gap_limit as usize,
        )
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
        let recipients = flatbuffers::root::<fb::Recipients>(&recipients_bytes)?;
        let h = get_coin_handler(coin);
        if h.is_private() {
            let last_height = crate::chain::latest_height(&h.url()).await?;
            let address =
                crate::db::account::get_account(&h.connection(), account)?.and_then(|d| d.address);
            let sender = address.ok_or(anyhow!("No account"))?;
            let recipients = crate::api::recipient::parse_recipients(&sender, &recipients)?;
            let tx = crate::pay::build_tx_plan(
                &h.network(),
                &h.connection(),
                &h.url(),
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
            h.prepare_multi_payment(account, &recipients.unpack(), None)
        }
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn transaction_report(coin: u8, plan: *mut c_char) -> CResult<*const u8> {
    from_c_str!(plan);
    let res = || {
        let h = get_coin_handler(coin);
        let report = if h.is_private() {
            let plan: TransactionPlan = serde_json::from_str(&plan)?;
            TransactionReport::from_plan(&h.network(), plan)
        } else {
            h.to_tx_report(&plan)?
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
) -> CResult<*mut c_char> {
    from_c_str!(tx_plan);
    let res = async {
        let h = get_coin_handler(coin);
        let raw_tx = h.sign(account, &tx_plan)?;
        let tx_str = base64::encode(&raw_tx);
        Ok(tx_str)
    };
    to_cresult_str(res.await)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn broadcast_tx(coin: u8, tx_str: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(tx_str);
    let res = async {
        let tx = base64::decode(&*tx_str)?;
        get_coin_handler(coin).broadcast(&tx)
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
        let h = get_coin_handler(coin);
        let raw_tx = h.sign(account, &tx_plan)?;
        let id = h.broadcast(&raw_tx)?;
        let height = h.get_latest_height().await?;
        h.mark_inputs_spent(&tx_plan, height)?;
        Ok(id)
    };
    let res = res.await;
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_tkey(sk: *mut c_char) -> bool {
    from_c_str!(sk);
    crate::taddr::parse_seckey(&sk).is_ok()
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sweep_tkey(
    coin: u8,
    account: u32,
    last_height: u32,
    sk: *mut c_char,
    pool: u8,
) -> CResult<*mut c_char> {
    from_c_str!(sk);
    let h = get_coin_handler(coin);
    let txid = crate::taddr::sweep_tkey(
        &h.network(),
        &h.connection(),
        &h.url(),
        account,
        last_height,
        &sk,
        pool,
    )
    .await;
    to_cresult_str(txid)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_activation_date(coin: u8) -> CResult<u32> {
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::chain::get_activation_date(&h.network(), &h.url()).await;
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_block_by_time(coin: u8, time: u32) -> CResult<u32> {
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::chain::get_block_by_time(&h.network(), &h.url(), time).await;
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sync_historical_prices(
    coin: u8,
    now: u32,
    days: u32,
    currency: *mut c_char,
) -> CResult<u32> {
    from_c_str!(currency);
    let res = async {
        let h = get_coin_handler(coin);
        let connection = &mut h.connection();
        crate::historical_prices::sync_historical_prices(
            connection,
            h.coingecko_id(),
            now,
            days,
            &currency,
        )
        .await
    };
    to_cresult(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn store_contact(
    coin: u8,
    id: u32,
    name: *mut c_char,
    address: *mut c_char,
    dirty: bool,
) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(address);
    let h = get_coin_handler(coin);
    let contact = fb::ContactT {
        id,
        name: Some(name.to_string()),
        address: Some(address.to_string()),
    };
    let res = db::contact::store_contact(&h.connection(), &contact, dirty);
    to_cresult_unit(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn commit_unsaved_contacts(
    coin: u8,
    account: u32,
    anchor_offset: u32,
) -> CResult<*mut c_char> {
    let res = async move {
        let h = get_coin_handler(coin);
        let tx_plan = crate::contact::commit_unsaved_contacts(
            &h.network(),
            &h.connection(),
            &h.url(),
            account,
            anchor_offset,
        )
        .await?;
        let tx_plan_json = serde_json::to_string(&tx_plan)?;
        Ok(tx_plan_json)
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn mark_message_read(coin: u8, message: u32, read: bool) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        db::message::mark_message_read(&h.connection(), message, read)?;
        Ok(())
    };
    to_cresult_unit(res())
}

#[no_mangle]
pub unsafe extern "C" fn mark_all_messages_read(coin: u8, account: u32, read: bool) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let connection = h.connection();
        db::message::mark_all_messages_read(&connection, account, read)
    };
    to_cresult_unit(res())
}

#[no_mangle]
pub unsafe extern "C" fn truncate_data(coin: u8) -> CResult<u8> {
    let res = || {
        let mut h = get_coin_handler(coin);
        h.reset_sync(0)
    };
    to_cresult_unit(res())
}

// #[no_mangle]
// pub unsafe extern "C" fn truncate_sync_data() -> CResult<u8> {
//     let res = crate::api::account::truncate_sync_data();
//     log_error(res)
// }

#[no_mangle]
pub unsafe extern "C" fn check_account(coin: u8, account: u32) -> bool {
    get_coin_handler(coin).has_account(account).unwrap_or(false)
}

#[no_mangle]
pub unsafe extern "C" fn delete_account(coin: u8, account: u32) -> CResult<u8> {
    let res = get_coin_handler(coin).delete_account(account);
    to_cresult_unit(res)
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
    let h = get_coin_handler(coin);
    let res = crate::pay::make_payment_uri(&h.network(), h.coingecko_id(), &address, amount, &memo);
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn parse_payment_uri(coin: u8, uri: *mut c_char) -> CResult<*const u8> {
    from_c_str!(uri);
    let payment_json = || {
        let h = get_coin_handler(coin);
        let payment = crate::pay::parse_payment_uri(&h.network(), h.coingecko_id(), &uri)?;
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
        let coins = REGISTERED_COINS.lock();
        for coin in coins.iter() {
            let h = get_coin_handler(*coin);
            backup.add(&h.connection(), h.db_path().file_name().unwrap())?;
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
        let servers = flatbuffers::root::<fb::Servers>(&servers)?;
        let best_server = crate::get_best_server(servers).await?;
        Ok(best_server)
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn import_from_zwl(
    coin: u8,
    name: *mut c_char,
    data: *mut c_char,
) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(data);
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::account::import_from_zwl(&h.network(), &h.connection(), &name, &data);
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn derive_zip32(
    coin: u8,
    account: u32,
    index: u32,
    external: u32,
    has_address: bool,
    address: u32,
) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let address = if has_address { Some(address) } else { None };
        let kp = crate::key::derive_keys(
            &h.network(),
            &h.connection(),
            account,
            index,
            external,
            address,
        )?;
        Ok(fb_to_vec!(kp))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn clear_tx_details(coin: u8, account: u32) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        db::purge::clear_tx_details(&h.connection(), account)?;
        Ok(())
    };
    to_cresult_unit(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_account_list(coin: u8) -> CResult<*const u8> {
    let res = || {
        let accounts = get_coin_handler(coin).list_accounts()?;
        Ok(fb_to_vec!(accounts))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_active_account(coin: u8) -> CResult<u32> {
    let res = || get_coin_handler(coin).get_active_account();
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn set_active_account(coin: u8, id: u32) -> CResult<u8> {
    let res = || {
        get_coin_handler(coin).set_active_account(id)?;
        Ok(0)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_t_addr(coin: u8, id: u32) -> CResult<*mut c_char> {
    let res = get_coin_handler(coin).get_address(id);
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn get_sk(coin: u8, id: u32) -> CResult<*mut c_char> {
    let res = || {
        let b = get_coin_handler(coin).get_backup(id)?;
        Ok(b.sk.unwrap_or_default())
    };
    to_cresult_str(res())
}

#[no_mangle]
pub unsafe extern "C" fn update_account_name(coin: u8, id: u32, name: *mut c_char) -> CResult<u8> {
    from_c_str!(name);
    let res = get_coin_handler(coin).update_name(id, &name);
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn get_balances(
    coin: u8,
    id: u32,
    confirmed_height: u32,
    filter_excluded: bool,
) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let balances = if h.is_private() {
            db::account::get_balances(&h.connection(), id, confirmed_height, filter_excluded)
        } else {
            let balance = get_coin_handler(coin).get_balance(id)?;
            Ok(fb::BalanceT {
                balance,
                ..fb::BalanceT::default()
            })
        }?;
        Ok(fb_to_vec!(balances))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_db_height(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let height = get_coin_handler(coin)
            .get_db_height(account)?
            .unwrap_or_default();
        Ok(fb_to_vec!(height))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_notes(coin: u8, id: u32) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let notes = if h.is_private() {
            db::transaction::list_notes(&h.connection(), id)
        } else {
            get_coin_handler(coin).get_notes(id).map(|ns| {
                let notes = ns.notes.unwrap();
                let notes: Vec<_> = notes
                    .into_iter()
                    .map(|n| fb::ShieldedNoteT {
                        id,
                        height: n.height,
                        value: n.value,
                        timestamp: n.timestamp,
                        ..fb::ShieldedNoteT::default()
                    })
                    .collect();
                fb::ShieldedNoteVecT { notes: Some(notes) }
            })
        }?;
        Ok(fb_to_vec!(notes))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_txs(coin: u8, id: u32) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let txs = if h.is_private() {
            db::transaction::list_txs(&h.network(), &h.connection(), id)
        } else {
            get_coin_handler(coin).get_txs(id).map(|txs| {
                let txs: Vec<_> = txs
                    .txs
                    .unwrap()
                    .into_iter()
                    .map(|tx| fb::ShieldedTxT {
                        id,
                        tx_id: tx.tx_id.clone(),
                        height: tx.height,
                        short_tx_id: tx.tx_id.clone().map(|id| id[0..4].to_string()),
                        timestamp: tx.timestamp,
                        value: tx.value,
                        ..fb::ShieldedTxT::default()
                    })
                    .collect();
                fb::ShieldedTxVecT { txs: Some(txs) }
            })
        }?;
        Ok(fb_to_vec!(txs))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_messages(coin: u8, account: u32) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let messages = if h.is_private() {
            db::message::get_messages(&h.connection(), &h.network(), account)
        } else {
            Ok(fb::MessageVecT {
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
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        let data =
            crate::db::message::get_prev_next_message(&h.connection(), &subject, height, id)?;
        Ok(fb_to_vec!(data))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_templates(coin: u8) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        let data = crate::db::payment_tpl::get_templates(&h.connection())?;
        Ok(fb_to_vec!(data))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn save_send_template(coin: u8, template: *mut u8, len: u64) -> CResult<u32> {
    let template: Vec<u8> = Vec::from_raw_parts(template, len as usize, len as usize);
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        let template = flatbuffers::root::<fb::SendTemplate>(&template)?.unpack();
        let id = crate::db::payment_tpl::store_template(&h.connection(), &template)?;
        Ok(id)
    };
    to_cresult(res())
}

#[no_mangle]
pub unsafe extern "C" fn delete_send_template(coin: u8, id: u32) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        crate::db::payment_tpl::delete_template(&h.connection(), id)?;
        Ok(())
    };
    to_cresult_unit(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_contacts(coin: u8) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let contacts = if h.is_private() {
            db::contact::get_contacts(&h.connection())
        } else {
            Ok(fb::ContactVecT {
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
        let h = get_coin_handler(coin);
        let timeseries = if h.is_private() {
            db::historical_prices::get_pnl_txs(&h.connection(), id, timestamp)
        } else {
            Ok(fb::TxTimeValueVecT {
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
        let h = get_coin_handler(coin);
        let quotes =
            db::historical_prices::get_historical_prices(&h.connection(), timestamp, &currency)?;
        Ok(fb_to_vec!(quotes))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_spendings(coin: u8, id: u32, timestamp: u32) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let spendings = if h.is_private() {
            crate::db::historical_prices::get_spendings(&h.connection(), id, timestamp)
        } else {
            Ok(fb::SpendingVecT {
                values: Some(vec![]),
            })
        }?;
        Ok(fb_to_vec!(spendings))
    };
    to_cresult_bytes(res())
}

#[no_mangle]
pub unsafe extern "C" fn update_excluded(coin: u8, id: u32, excluded: bool) -> CResult<u8> {
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::db::transaction::update_excluded(&h.connection(), id, excluded);
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn invert_excluded(coin: u8, id: u32) -> CResult<u8> {
    let h = get_coin_handler(coin);
    assert!(h.is_private());
    let res = crate::db::transaction::invert_excluded(&h.connection(), id);
    to_cresult_unit(res)
}

#[no_mangle]
pub unsafe extern "C" fn get_checkpoints(coin: u8) -> CResult<*const u8> {
    let res = || {
        let h = get_coin_handler(coin);
        let data = crate::db::checkpoint::list_checkpoints(&h.connection())?;
        Ok(fb_to_vec!(data))
    };
    to_cresult_bytes(res())
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
    let res = || {
        let h = get_coin_handler(coin);
        crate::db::cipher::clone_db_with_passwd(&h.connection(), &temp_path, &passwd)?;
        Ok(())
    };
    to_cresult_unit(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_property(coin: u8, name: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(name);
    to_cresult_str(get_coin_handler(coin).get_property(&name))
}

#[no_mangle]
pub unsafe extern "C" fn set_property(
    coin: u8,
    name: *mut c_char,
    value: *mut c_char,
) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(value);
    to_cresult_unit(get_coin_handler(coin).set_property(&name, &value))
}

#[no_mangle]
pub unsafe extern "C" fn import_uvk(coin: u8, name: *mut c_char, yfvk: *mut c_char) -> CResult<u8> {
    from_c_str!(name);
    from_c_str!(yfvk);
    let res = || {
        let h = get_coin_handler(coin);
        assert!(h.is_private());
        crate::key::import_uvk(&h.network(), &h.connection(), &name, &yfvk)?;
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
        let h = get_coin_handler(coin);
        let network = h.network();
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        let prover = crate::coinconfig::get_prover();
        let pk = crate::orchard::get_proving_key();
        let raw_tx = tokio::task::spawn_blocking(move || {
            let (pubkey, dfvk, ofvk) = crate::ledger::ledger_get_fvks()?;
            let raw_tx = crate::ledger::build_ledger_tx(
                &network, &tx_plan, &pubkey, &dfvk, ofvk, prover, &pk,
            )?;
            Ok::<_, anyhow::Error>(raw_tx)
        })
        .await??;
        let response = h.broadcast(&raw_tx)?;
        Ok::<_, anyhow::Error>(response)
    };
    to_cresult_str(res.await)
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_import_account(coin: u8, name: *mut c_char) -> CResult<u32> {
    from_c_str!(name);
    let h = get_coin_handler(coin);
    let account = crate::ledger::import_account(&h.network(), &mut h.connection(), &name);
    to_cresult(account)
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_has_account(coin: u8, account: u32) -> CResult<bool> {
    let h = get_coin_handler(coin);
    let res = crate::ledger::is_external(&h.connection(), account);
    to_cresult(res)
}

#[cfg(feature = "ledger")]
#[no_mangle]
pub unsafe extern "C" fn ledger_toggle_binding(coin: u8, account: u32) -> CResult<u8> {
    let res = || {
        let h = get_coin_handler(coin);
        crate::ledger::toggle_binding(&h.connection(), account)?;
        Ok(())
    };
    to_cresult_unit(res())
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
