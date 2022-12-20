use crate::coinconfig::{init_coin, CoinConfig, MEMPOOL, MEMPOOL_RUNNER};
use crate::db::FullEncryptedBackup;
use crate::note_selection::TransactionReport;
use crate::{ChainError, TransactionPlan, Tx};
use allo_isolate::{ffi, IntoDart};
use android_logger::Config;
use lazy_static::lazy_static;
use log::Level;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Mutex;
use tokio::sync::Semaphore;
use zcash_primitives::transaction::builder::Progress;

static mut POST_COBJ: Option<ffi::DartPostCObjectFnType> = None;

const MAX_COINS: u8 = 2;

#[no_mangle]
pub unsafe extern "C" fn dummy_export() {}

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
            error: std::ptr::null_mut::<c_char>(),
        },
        Err(e) => {
            log::error!("{}", e);
            CResult {
                value: unsafe { std::mem::zeroed() },
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

#[no_mangle]
pub unsafe extern "C" fn deallocate_str(s: *mut c_char) {
    let _ = CString::from_raw(s);
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
}

#[no_mangle]
pub unsafe extern "C" fn init_wallet(coin: u8, db_path: *mut c_char) -> CResult<u8> {
    try_init_logger();
    from_c_str!(db_path);
    to_cresult(init_coin(coin, &db_path).and_then(|()| Ok(0u8)))
}

#[no_mangle]
pub unsafe extern "C" fn migrate_db(coin: u8, db_path: *mut c_char) -> CResult<u8> {
    try_init_logger();
    from_c_str!(db_path);
    to_cresult(crate::coinconfig::migrate_db(coin, &db_path).and_then(|()| Ok(0u8)))
}

#[no_mangle]
#[tokio::main]
pub async unsafe extern "C" fn migrate_data_db(coin: u8) -> CResult<u8> {
    try_init_logger();
    to_cresult(
        crate::coinconfig::migrate_data(coin)
            .await
            .and_then(|()| Ok(0u8)),
    )
}

#[no_mangle]
pub unsafe extern "C" fn set_active(active: u8) {
    crate::coinconfig::set_active(active);
}

#[no_mangle]
pub unsafe extern "C" fn set_active_account(coin: u8, id: u32) {
    crate::coinconfig::set_active_account(coin, id);
}

#[no_mangle]
pub unsafe extern "C" fn set_coin_lwd_url(coin: u8, lwd_url: *mut c_char) {
    from_c_str!(lwd_url);
    crate::coinconfig::set_coin_lwd_url(coin, &lwd_url);
}

#[no_mangle]
pub unsafe extern "C" fn get_lwd_url(coin: u8) -> *mut c_char {
    let server = crate::coinconfig::get_coin_lwd_url(coin);
    to_c_str(server)
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
    let mut mempool_runner = MEMPOOL_RUNNER.lock().unwrap();
    let mempool = mempool_runner
        .run(move |balance: i64| {
            let mut balance = balance.into_dart();
            if port != 0 {
                if let Some(p) = POST_COBJ {
                    p(port, &mut balance);
                }
            }
        })
        .await;
    let _ = MEMPOOL.fill(mempool);
    log::info!("end mempool_start");
}

#[no_mangle]
pub unsafe extern "C" fn mempool_set_active(coin: u8, id_account: u32) {
    let mempool = MEMPOOL.borrow().unwrap();
    mempool.set_active(coin, id_account);
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
    let data = if !data.is_empty() {
        Some(data.to_string())
    } else {
        None
    };
    let index = if index >= 0 { Some(index as u32) } else { None };
    let res = crate::api::account::new_account(coin, &name, data, index);
    to_cresult(res)
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
pub unsafe extern "C" fn get_backup(coin: u8, id_account: u32) -> CResult<*mut c_char> {
    let res = || {
        let backup = crate::api::account::get_backup_package(coin, id_account)?;
        let backup_str = serde_json::to_string(&backup)?;
        Ok::<_, anyhow::Error>(backup_str)
    };

    to_cresult_str(res())
}

#[no_mangle]
pub unsafe extern "C" fn get_address(
    coin: u8,
    id_account: u32,
    ua_type: u8,
) -> CResult<*mut c_char> {
    let address = crate::api::account::get_address(coin, id_account, ua_type);
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
    static ref SYNC_CANCELED: Mutex<bool> = Mutex::new(false);
}

#[no_mangle]
pub unsafe extern "C" fn cancel_warp() {
    log::info!("Sync canceled");
    *SYNC_CANCELED.lock().unwrap() = true;
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
    let res = async {
        let _permit = SYNC_LOCK.acquire().await?;
        log::info!("Sync started");
        let result = crate::api::sync::coin_sync(
            coin,
            get_tx,
            anchor_offset,
            max_cost,
            move |progress| {
                let mut progress = serde_json::to_string(&progress).unwrap().into_dart();
                if port != 0 {
                    if let Some(p) = POST_COBJ {
                        p(port, &mut progress);
                    }
                }
            },
            &SYNC_CANCELED,
        )
        .await;
        log::info!("Sync finished");

        match result {
            Ok(_) => Ok(0),
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
    *SYNC_CANCELED.lock().unwrap() = false;
    to_cresult(r)
}

#[no_mangle]
pub unsafe extern "C" fn is_valid_key(coin: u8, key: *mut c_char) -> i8 {
    from_c_str!(key);
    crate::key2::is_valid_key(coin, &key)
}

#[no_mangle]
pub unsafe extern "C" fn valid_address(coin: u8, address: *mut c_char) -> bool {
    from_c_str!(address);
    crate::key2::is_valid_address(coin, &address)
}

#[no_mangle]
pub unsafe extern "C" fn new_diversified_address(ua_type: u8) -> CResult<*mut c_char> {
    let res = || crate::api::account::new_diversified_address(ua_type);
    to_cresult_str(res())
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_latest_height() -> CResult<u32> {
    let height = crate::api::sync::get_latest_height().await;
    to_cresult(height)
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
    let res = crate::api::sync::skip_to_last_height(coin).await;
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
pub async unsafe extern "C" fn rescan_from(height: u32) {
    let res = crate::api::sync::rescan_from(height).await;
    log_error(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_taddr_balance(coin: u8, id_account: u32) -> CResult<u64> {
    let res = if coin == 0xFF {
        crate::api::account::get_taddr_balance_default().await
    } else {
        crate::api::account::get_taddr_balance(coin, id_account).await
    };
    to_cresult(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn transfer_pools(
    coin: u8,
    account: u32,
    from_pool: u8, to_pool: u8,
    amount: u64,
    fee_included: bool,
    memo: *mut c_char,
    split_amount: u64,
    confirmations: u32,
) -> CResult<*mut c_char> {
    from_c_str!(memo);
    let res = async move {
        let tx_plan = crate::api::payment_v2::transfer_pools(coin, account, from_pool, to_pool,
             amount, fee_included,
             &memo, split_amount, confirmations).await?;
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
    let res = crate::api::payment_v2::shield_taddr(coin, account, amount, confirmations).await;
    to_cresult_str(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn scan_transparent_accounts(gap_limit: u32) {
    let res = crate::api::account::scan_transparent_accounts(gap_limit as usize).await;
    log_error(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn prepare_multi_payment(
    coin: u8,
    account: u32,
    recipients_json: *mut c_char,
    anchor_offset: u32,
) -> CResult<*mut c_char> {
    from_c_str!(recipients_json);
    let res = async {
        let last_height = crate::api::sync::get_latest_height().await?;
        let recipients = crate::api::recipient::parse_recipients(&recipients_json)?;
        let tx = crate::api::payment_v2::build_tx_plan(
            coin,
            account,
            last_height,
            &recipients,
            0,
            anchor_offset,
        )
        .await?;
        let tx_str = serde_json::to_string(&tx)?;
        Ok(tx_str)
    };
    to_cresult_str(res.await)
}

#[no_mangle]
pub unsafe extern "C" fn transaction_report(coin: u8, plan: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(plan);
    let c = CoinConfig::get(coin);
    let res = || {
        let plan: TransactionPlan = serde_json::from_str(&plan)?;
        let report = TransactionReport::from_plan(c.chain.network(), plan);
        let report = serde_json::to_string(&report)?;
        Ok(report)
    };
    to_cresult_str(res())
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
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        let raw_tx = crate::api::payment_v2::sign_plan(coin, account, &tx_plan)?;
        let tx_str = base64::encode(&raw_tx);
        Ok::<_, anyhow::Error>(tx_str)
    };
    let res = res.await;
    to_cresult_str(res)
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
        let tx_plan: TransactionPlan = serde_json::from_str(&tx_plan)?;
        let txid = crate::api::payment_v2::sign_and_broadcast(coin, account, &tx_plan).await?;
        Ok::<_, anyhow::Error>(txid)
    };
    let res = res.await;
    to_cresult_str(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn broadcast_tx(tx_str: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(tx_str);
    let res = async {
        let tx = base64::decode(&*tx_str)?;
        crate::broadcast_tx(&tx).await
    };
    to_cresult_str(res.await)
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
    now: i64,
    days: u32,
    currency: *mut c_char,
) -> CResult<u32> {
    from_c_str!(currency);
    let res = crate::api::historical_prices::sync_historical_prices(now, days, &currency).await;
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
    let res = crate::api::contact::commit_unsaved_contacts(anchor_offset).await;
    to_cresult_str(res)
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
pub unsafe extern "C" fn delete_account(coin: u8, account: u32) {
    let res = crate::api::account::delete_account(coin, account);
    log_error(res)
}

#[no_mangle]
pub unsafe extern "C" fn make_payment_uri(
    address: *mut c_char,
    amount: u64,
    memo: *mut c_char,
) -> CResult<*mut c_char> {
    from_c_str!(memo);
    from_c_str!(address);
    let res = crate::api::payment_uri::make_payment_uri(&address, amount, &memo);
    to_cresult_str(res)
}

#[no_mangle]
pub unsafe extern "C" fn parse_payment_uri(uri: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(uri);
    let payment_json = || {
        let payment = crate::api::payment_uri::parse_payment_uri(&uri)?;
        let payment_json = serde_json::to_string(&payment)?;
        Ok(payment_json)
    };
    to_cresult_str(payment_json())
}

#[no_mangle]
pub unsafe extern "C" fn generate_key() -> CResult<*mut c_char> {
    let res = || {
        let secret_key = FullEncryptedBackup::generate_key()?;
        let keys = serde_json::to_string(&secret_key)?;
        Ok(keys)
    };
    to_cresult_str(res())
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
) {
    from_c_str!(key);
    from_c_str!(data_path);
    from_c_str!(dst_dir);
    let res = || {
        let backup = FullEncryptedBackup::new(&dst_dir);
        backup.restore(&key, &data_path)?;
        Ok(())
    };
    log_error(res())
}

#[no_mangle]
pub unsafe extern "C" fn split_data(id: u32, data: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(data);
    let res = || {
        let res = crate::fountain::FountainCodes::encode_into_drops(id, &base64::decode(&*data)?)?;
        let output = serde_json::to_string(&res)?;
        Ok(output)
    };
    to_cresult_str(res())
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
pub async unsafe extern "C" fn get_best_server(servers: *mut c_char) -> CResult<*mut c_char> {
    from_c_str!(servers);
    let best_server = crate::get_best_server(&servers).await;
    to_cresult_str(best_server)
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
) -> CResult<*mut c_char> {
    let res = || {
        let address = if has_address { Some(address) } else { None };
        let kp = crate::api::account::derive_keys(coin, id_account, account, external, address)?;
        let result = serde_json::to_string(&kp)?;
        Ok(result)
    };
    to_cresult_str(res())
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
