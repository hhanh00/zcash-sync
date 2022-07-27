use crate::coinconfig::{init_coin, CoinConfig};
use crate::{ChainError, Tx};
use allo_isolate::{ffi, IntoDart};
use android_logger::Config;
use lazy_static::lazy_static;
use log::Level;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tokio::sync::Semaphore;
use zcash_primitives::transaction::builder::Progress;

static mut POST_COBJ: Option<ffi::DartPostCObjectFnType> = None;
static IS_ERROR: AtomicBool = AtomicBool::new(false);

lazy_static! {
    static ref LAST_ERROR: Mutex<RefCell<String>> = Mutex::new(RefCell::new(String::new()));
}

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
}

fn log_result<T: Default>(result: anyhow::Result<T>) -> T {
    match result {
        Err(err) => {
            log::error!("{}", err);
            let last_error = LAST_ERROR.lock().unwrap();
            last_error.replace(err.to_string());
            IS_ERROR.store(true, Ordering::Release);
            T::default()
        }
        Ok(v) => {
            IS_ERROR.store(false, Ordering::Release);
            v
        }
    }
}

fn log_string(result: anyhow::Result<String>) -> String {
    match result {
        Err(err) => {
            log::error!("{}", err);
            let last_error = LAST_ERROR.lock().unwrap();
            last_error.replace(err.to_string());
            IS_ERROR.store(true, Ordering::Release);
            format!("{}", err)
        }
        Ok(v) => {
            IS_ERROR.store(false, Ordering::Release);
            v
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_error() -> bool {
    IS_ERROR.load(Ordering::Acquire)
}

#[no_mangle]
pub unsafe extern "C" fn get_error_msg() -> *mut c_char {
    let error = LAST_ERROR.lock().unwrap();
    let e = error.take();
    to_c_str(e)
}

#[no_mangle]
pub unsafe extern "C" fn init_wallet(db_path: *mut c_char) {
    try_init_logger();
    from_c_str!(db_path);
    let _ = init_coin(0, &format!("{}/zec.db", &db_path));
    let _ = init_coin(1, &format!("{}/yec.db", &db_path));
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
        crate::api::account::reset_db(0)?;
        crate::api::account::reset_db(1)?;
        Ok(())
    };
    log_result(res())
}

#[no_mangle]
pub unsafe extern "C" fn new_account(
    coin: u8,
    name: *mut c_char,
    data: *mut c_char,
    index: i32,
) -> u32 {
    from_c_str!(name);
    from_c_str!(data);
    let data = if !data.is_empty() {
        Some(data.to_string())
    } else {
        None
    };
    let index = if index >= 0 { Some(index as u32) } else { None };
    let res = crate::api::account::new_account(coin, &name, data, index);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn new_sub_account(name: *mut c_char, index: i32, count: u32) {
    from_c_str!(name);
    let index = if index >= 0 { Some(index as u32) } else { None };
    let res = crate::api::account::new_sub_account(&name, index, count);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn import_transparent_key(coin: u8, id_account: u32, path: *mut c_char) {
    from_c_str!(path);
    let res = crate::api::account::import_transparent_key(coin, id_account, &path);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn import_transparent_secret_key(
    coin: u8,
    id_account: u32,
    secret_key: *mut c_char,
) {
    from_c_str!(secret_key);
    let res = crate::api::account::import_transparent_secret_key(coin, id_account, &secret_key);
    log_result(res)
}

lazy_static! {
    static ref SYNC_LOCK: Semaphore = Semaphore::new(1);
    static ref SYNC_CANCELED: AtomicBool = AtomicBool::new(false);
}

#[no_mangle]
pub unsafe extern "C" fn cancel_warp() {
    log::info!("Sync canceled");
    SYNC_CANCELED.store(true, Ordering::Release);
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn warp(coin: u8, get_tx: bool, anchor_offset: u32, port: i64) -> u8 {
    let res = async {
        let _permit = SYNC_LOCK.acquire().await?;
        log::info!("Sync started");
        let result = crate::api::sync::coin_sync(
            coin,
            get_tx,
            anchor_offset,
            move |height| {
                let mut height = height.into_dart();
                if port != 0 {
                    if let Some(p) = POST_COBJ {
                        p(port, &mut height);
                    }
                }
            },
            &SYNC_CANCELED,
        )
        .await;
        log::info!("Sync finished");

        crate::api::mempool::scan().await?;

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
    SYNC_CANCELED.store(false, Ordering::Release);
    log_result(r)
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
pub unsafe extern "C" fn new_diversified_address() -> *mut c_char {
    let res = || crate::api::account::new_diversified_address();
    to_c_str(log_string(res()))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_latest_height() -> u32 {
    let height = crate::api::sync::get_latest_height().await;
    log_result(height)
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

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn send_multi_payment(
    recipients_json: *mut c_char,
    use_transparent: bool,
    anchor_offset: u32,
    port: i64,
) -> *mut c_char {
    from_c_str!(recipients_json);
    let res = async move {
        let height = crate::api::sync::get_latest_height().await?;
        let recipients = crate::api::payment::parse_recipients(&recipients_json)?;
        let res = crate::api::payment::build_sign_send_multi_payment(
            height,
            &recipients,
            use_transparent,
            anchor_offset,
            Box::new(move |progress| {
                report_progress(progress, port);
            }),
        )
        .await?;
        Ok(res)
    };
    to_c_str(log_string(res.await))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn skip_to_last_height(coin: u8) {
    let res = crate::api::sync::skip_to_last_height(coin).await;
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn rewind_to_height(height: u32) {
    let res = crate::api::sync::rewind_to_height(height).await;
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn mempool_sync() -> i64 {
    let res = crate::api::mempool::scan().await;
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn mempool_reset() {
    let c = CoinConfig::get_active();
    let mut mempool = c.mempool.lock().unwrap();
    log_result(mempool.clear());
}

#[no_mangle]
pub unsafe extern "C" fn get_mempool_balance() -> i64 {
    let c = CoinConfig::get_active();
    let mempool = c.mempool.lock().unwrap();
    mempool.get_unconfirmed_balance()
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_taddr_balance(coin: u8, id_account: u32) -> u64 {
    let res = if coin == 0xFF {
        crate::api::account::get_taddr_balance_default().await
    } else {
        crate::api::account::get_taddr_balance(coin, id_account).await
    };
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn shield_taddr() -> *mut c_char {
    let res = crate::api::payment::shield_taddr().await;
    to_c_str(log_string(res))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn scan_transparent_accounts(gap_limit: u32) {
    let res = crate::api::account::scan_transparent_accounts(gap_limit as usize).await;
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn prepare_multi_payment(
    recipients_json: *mut c_char,
    use_transparent: bool,
    anchor_offset: u32,
) -> *mut c_char {
    from_c_str!(recipients_json);
    let res = async {
        let last_height = crate::api::sync::get_latest_height().await?;
        let recipients = crate::api::payment::parse_recipients(&recipients_json)?;
        let tx = crate::api::payment::build_only_multi_payment(
            last_height,
            &recipients,
            use_transparent,
            anchor_offset,
        )
        .await?;
        let tx_str = serde_json::to_string(&tx)?;
        Ok(tx_str)
    };
    to_c_str(log_string(res.await))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sign(tx: *mut c_char, port: i64) -> *mut c_char {
    from_c_str!(tx);
    let res = async {
        let tx: Tx = serde_json::from_str(&tx)?;
        let raw_tx = crate::api::payment::sign_only_multi_payment(
            &tx,
            Box::new(move |progress| {
                report_progress(progress, port);
            }),
        )
        .await?;
        let tx_str = base64::encode(&raw_tx);
        Ok(tx_str)
    };
    to_c_str(log_string(res.await))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn broadcast_tx(tx_str: *mut c_char) -> *mut c_char {
    from_c_str!(tx_str);
    let res = async {
        let tx = base64::decode(&*tx_str)?;
        crate::broadcast_tx(&tx).await
    };
    to_c_str(log_string(res.await))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_activation_date() -> u32 {
    let res = crate::api::sync::get_activation_date().await;
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_block_by_time(time: u32) -> u32 {
    let res = crate::api::sync::get_block_by_time(time).await;
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn sync_historical_prices(
    now: i64,
    days: u32,
    currency: *mut c_char,
) -> u32 {
    from_c_str!(currency);
    let res = crate::api::historical_prices::sync_historical_prices(now, days, &currency).await;
    log_result(res)
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
    log_result(res)
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn commit_unsaved_contacts(anchor_offset: u32) -> *mut c_char {
    let res = crate::api::contact::commit_unsaved_contacts(anchor_offset).await;
    to_c_str(log_string(res))
}

#[no_mangle]
pub unsafe extern "C" fn mark_message_read(message: u32, read: bool) {
    let res = crate::api::message::mark_message_read(message, read);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn mark_all_messages_read(read: bool) {
    let res = crate::api::message::mark_all_messages_read(read);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn truncate_data() {
    let res = crate::api::account::truncate_data();
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn delete_account(coin: u8, account: u32) {
    let res = crate::api::account::delete_account(coin, account);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn make_payment_uri(
    address: *mut c_char,
    amount: u64,
    memo: *mut c_char,
) -> *mut c_char {
    from_c_str!(memo);
    from_c_str!(address);
    let res = crate::api::payment_uri::make_payment_uri(&address, amount, &memo);
    to_c_str(log_string(res))
}

#[no_mangle]
pub unsafe extern "C" fn parse_payment_uri(uri: *mut c_char) -> *mut c_char {
    from_c_str!(uri);
    let payment_json = || {
        let payment = crate::api::payment_uri::parse_payment_uri(&uri)?;
        let payment_json = serde_json::to_string(&payment)?;
        Ok(payment_json)
    };
    to_c_str(log_string(payment_json()))
}

#[no_mangle]
pub unsafe extern "C" fn generate_random_enc_key() -> *mut c_char {
    to_c_str(log_string(crate::key2::generate_random_enc_key()))
}

#[no_mangle]
pub unsafe extern "C" fn get_full_backup(key: *mut c_char) -> *mut c_char {
    from_c_str!(key);
    let res = || {
        let mut accounts = vec![];
        for coin in [0, 1] {
            accounts.extend(crate::api::fullbackup::get_full_backup(coin)?);
        }

        let backup = crate::api::fullbackup::encrypt_backup(&accounts, &key)?;
        Ok(backup)
    };
    to_c_str(log_string(res()))
}

#[no_mangle]
pub unsafe extern "C" fn restore_full_backup(key: *mut c_char, backup: *mut c_char) {
    from_c_str!(key);
    from_c_str!(backup);
    let res = || {
        let accounts = crate::api::fullbackup::decrypt_backup(&key, &backup)?;
        for coin in [0, 1] {
            crate::api::fullbackup::restore_full_backup(coin, &accounts)?;
        }
        Ok(())
    };
    log_result(res())
}

#[no_mangle]
pub unsafe extern "C" fn split_data(id: u32, data: *mut c_char) -> *mut c_char {
    from_c_str!(data);
    let res = || {
        let res = crate::FountainCodes::encode_into_drops(id, &base64::decode(&*data)?)?;
        let output = serde_json::to_string(&res)?;
        Ok(output)
    };
    to_c_str(log_string(res()))
}

#[no_mangle]
pub unsafe extern "C" fn merge_data(drop: *mut c_char) -> *mut c_char {
    from_c_str!(drop);
    let res = || {
        let res = crate::put_drop(&*drop)?
            .map(|d| base64::encode(&d))
            .unwrap_or(String::new());
        Ok::<_, anyhow::Error>(res)
    };
    match res() {
        Ok(str) => to_c_str(str),
        Err(e) => {
            log::error!("{}", e);
            to_c_str(String::new()) // do not return error msg
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn get_tx_summary(tx: *mut c_char) -> *mut c_char {
    from_c_str!(tx);
    let res = || {
        let tx: Tx = serde_json::from_str(&tx)?;
        let summary = crate::get_tx_summary(&tx)?;
        let summary = serde_json::to_string(&summary)?;
        Ok::<_, anyhow::Error>(summary)
    };
    to_c_str(log_string(res()))
}

#[tokio::main]
#[no_mangle]
pub async unsafe extern "C" fn get_best_server(
    servers: *mut *mut c_char,
    count: u32,
) -> *mut c_char {
    let mut cservers = vec![];
    for i in 0..count {
        let ptr = *servers.offset(i as isize);
        let s = CStr::from_ptr(ptr).to_string_lossy();
        cservers.push(s.to_string());
    }
    let best_server = crate::get_best_server(&cservers)
        .await
        .unwrap_or(String::new());
    to_c_str(best_server)
}

#[no_mangle]
pub unsafe extern "C" fn import_from_zwl(coin: u8, name: *mut c_char, data: *mut c_char) {
    from_c_str!(name);
    from_c_str!(data);
    let res = crate::api::account::import_from_zwl(coin, &name, &data);
    log_result(res)
}

#[no_mangle]
pub unsafe extern "C" fn derive_zip32(
    coin: u8,
    id_account: u32,
    account: u32,
    external: u32,
    has_address: bool,
    address: u32,
) -> *mut c_char {
    let res = || {
        let address = if has_address { Some(address) } else { None };
        let kp = crate::api::account::derive_keys(coin, id_account, account, external, address)?;
        let result = serde_json::to_string(&kp)?;
        Ok(result)
    };
    to_c_str(log_string(res()))
}
