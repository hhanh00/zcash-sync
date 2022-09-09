#[macro_use]
extern crate rocket;

use anyhow::anyhow;
use lazy_static::lazy_static;
use rocket::fairing::AdHoc;
use rocket::http::Status;
use rocket::response::Responder;
use rocket::serde::{json::Json, Deserialize, Serialize};
use rocket::{response, Request, Response, State};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::Mutex;
use thiserror::Error;
use warp_api_ffi::api::payment::{Recipient, RecipientMemo};
use warp_api_ffi::api::payment_uri::PaymentURI;
use warp_api_ffi::{
    derive_zip32, get_best_server, AccountData, AccountInfo, AccountRec, CoinConfig, KeyPack,
    RaptorQDrops, Tx, TxRec,
};

lazy_static! {
    static ref SYNC_CANCELED: Mutex<bool> = Mutex::new(false);
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Hex(#[from] hex::FromHexError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl<'r> Responder<'r, 'static> for Error {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let error = self.to_string();
        Response::build_from(error.respond_to(req)?)
            .status(Status::InternalServerError)
            .ok()
    }
}

fn init(coin: u8, config: HashMap<String, String>) -> anyhow::Result<()> {
    warp_api_ffi::init_coin(
        coin,
        config
            .get("db_path")
            .ok_or(anyhow!("Missing configuration value"))?,
    )?;
    warp_api_ffi::set_coin_lwd_url(
        coin,
        config
            .get("lwd_url")
            .ok_or(anyhow!("Missing configuration value"))?,
    );
    Ok(())
}

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let _ = dotenv::dotenv();

    let rocket = rocket::build();
    let figment = rocket.figment();
    let zec: HashMap<String, String> = figment.extract_inner("zec")?;
    init(0, zec)?;
    let yec: HashMap<String, String> = figment.extract_inner("yec")?;
    init(1, yec)?;
    let arrr: HashMap<String, String> = figment.extract_inner("arrr")?;
    init(2, arrr)?;

    warp_api_ffi::set_active_account(0, 1);
    warp_api_ffi::set_active(0);

    let _ = rocket
        .mount(
            "/",
            routes![
                set_active,
                new_account,
                list_accounts,
                sync,
                rewind,
                get_latest_height,
                get_backup,
                get_balance,
                get_address,
                get_tx_history,
                pay,
                mark_synced,
                create_offline_tx,
                sign_offline_tx,
                broadcast_tx,
                new_diversified_address,
                make_payment_uri,
                parse_payment_uri,
                split_data,
                merge_data,
                derive_keys,
                instant_sync,
                trial_decrypt,
            ],
        )
        .attach(AdHoc::config::<Config>())
        .launch()
        .await?;

    Ok(())
}

#[post("/set_active?<coin>&<id_account>")]
pub fn set_active(coin: u8, id_account: u32) {
    warp_api_ffi::set_active_account(coin, id_account);
    warp_api_ffi::set_active(coin);
}

#[post("/new_account", format = "application/json", data = "<seed>")]
pub fn new_account(seed: Json<AccountSeed>) -> Result<String, Error> {
    let id_account = warp_api_ffi::api::account::new_account(
        seed.coin,
        &seed.name,
        seed.key.clone(),
        seed.index,
    )?;
    warp_api_ffi::set_active_account(seed.coin, id_account);
    Ok(id_account.to_string())
}

#[get("/accounts")]
pub fn list_accounts() -> Result<Json<Vec<AccountRec>>, Error> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let accounts = db.get_accounts()?;
    Ok(Json(accounts))
}

#[post("/sync?<offset>")]
pub async fn sync(offset: Option<u32>) -> Result<(), Error> {
    let c = CoinConfig::get_active();
    warp_api_ffi::api::sync::coin_sync(
        c.coin,
        true,
        offset.unwrap_or(0),
        50,
        |_| {},
        &SYNC_CANCELED,
    )
    .await?;
    Ok(())
}

#[post("/rewind?<height>")]
pub async fn rewind(height: u32) -> Result<(), Error> {
    warp_api_ffi::api::sync::rewind_to(height).await?;
    Ok(())
}

#[post("/mark_synced")]
pub async fn mark_synced() -> Result<(), Error> {
    let c = CoinConfig::get_active();
    warp_api_ffi::api::sync::skip_to_last_height(c.coin).await?;
    Ok(())
}

#[get("/latest_height")]
pub async fn get_latest_height() -> Result<Json<Heights>, Error> {
    let latest = warp_api_ffi::api::sync::get_latest_height().await?;
    let synced = warp_api_ffi::api::sync::get_synced_height()?;
    Ok(Json(Heights { latest, synced }))
}

#[get("/address")]
pub fn get_address() -> Result<String, Error> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let AccountData { address, .. } = db.get_account_info(c.id_account)?;
    Ok(address)
}

#[get("/backup")]
pub fn get_backup(config: &State<Config>) -> Result<Json<Backup>, Error> {
    if !config.allow_backup {
        Err(anyhow!("Backup API not enabled").into())
    } else {
        let c = CoinConfig::get_active();
        let db = c.db()?;
        let AccountData { seed, sk, fvk, .. } = db.get_account_info(c.id_account)?;
        Ok(Json(Backup { seed, sk, fvk }))
    }
}

#[get("/tx_history")]
pub fn get_tx_history() -> Result<Json<Vec<TxRec>>, Error> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let txs = db.get_txs(c.id_account)?;
    Ok(Json(txs))
}

#[get("/balance")]
pub fn get_balance() -> Result<String, Error> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let balance = db.get_balance(c.id_account)?;
    Ok(balance.to_string())
}

#[post("/create_offline_tx", data = "<payment>")]
pub async fn create_offline_tx(payment: Json<Payment>) -> Result<Json<Tx>, Error> {
    let c = CoinConfig::get_active();
    let latest = warp_api_ffi::api::sync::get_latest_height().await?;
    let from = {
        let db = c.db()?;
        let AccountData { address, .. } = db.get_account_info(c.id_account)?;
        address
    };
    let recipients: Vec<_> = payment
        .recipients
        .iter()
        .map(|p| RecipientMemo::from_recipient(&from, p))
        .collect();
    let tx = warp_api_ffi::api::payment::build_only_multi_payment(
        latest,
        &recipients,
        false,
        payment.confirmations,
    )
    .await?;
    Ok(Json(tx))
}

#[post("/sign_offline_tx", data = "<tx>")]
pub async fn sign_offline_tx(tx: Json<Tx>, config: &State<Config>) -> Result<String, Error> {
    if !config.allow_send {
        Err(anyhow!("Payment API not enabled").into())
    } else {
        let tx_hex =
            warp_api_ffi::api::payment::sign_only_multi_payment(&tx, Box::new(|_| {})).await?;
        Ok(hex::encode(tx_hex))
    }
}

#[post("/pay", data = "<payment>")]
pub async fn pay(payment: Json<Payment>, config: &State<Config>) -> Result<String, Error> {
    if !config.allow_send {
        Err(anyhow!("Payment API not enabled").into())
    } else {
        let c = CoinConfig::get_active();
        let latest = warp_api_ffi::api::sync::get_latest_height().await?;
        let from = {
            let db = c.db()?;
            let AccountData { address, .. } = db.get_account_info(c.id_account)?;
            address
        };
        let recipients: Vec<_> = payment
            .recipients
            .iter()
            .map(|p| RecipientMemo::from_recipient(&from, p))
            .collect();
        let txid = warp_api_ffi::api::payment::build_sign_send_multi_payment(
            latest,
            &recipients,
            false,
            payment.confirmations,
            Box::new(|_| {}),
        )
        .await?;
        Ok(txid)
    }
}

#[post("/broadcast_tx?<tx_hex>")]
pub async fn broadcast_tx(tx_hex: String) -> Result<String, Error> {
    let tx = hex::decode(tx_hex.trim_end()).map_err(|e| anyhow!(e.to_string()))?;
    let tx_id = warp_api_ffi::api::payment::broadcast_tx(&tx).await?;
    Ok(tx_id)
}

#[get("/new_diversified_address")]
pub fn new_diversified_address() -> Result<String, Error> {
    let address = warp_api_ffi::api::account::new_diversified_address()?;
    Ok(address)
}

#[post("/make_payment_uri", data = "<payment>")]
pub fn make_payment_uri(payment: Json<PaymentURI>) -> Result<String, Error> {
    let uri = warp_api_ffi::api::payment_uri::make_payment_uri(
        &payment.address,
        payment.amount,
        &payment.memo,
    )?;
    Ok(uri)
}

#[get("/parse_payment_uri?<uri>")]
pub fn parse_payment_uri(uri: String) -> Result<Json<PaymentURI>, Error> {
    let payment = warp_api_ffi::api::payment_uri::parse_payment_uri(&uri)?;
    Ok(Json(payment))
}

#[get("/split?<id>&<data>")]
pub fn split_data(id: u32, data: String) -> Result<Json<RaptorQDrops>, Error> {
    let result = warp_api_ffi::FountainCodes::encode_into_drops(id, &hex::decode(data).unwrap())?;
    Ok(Json(result))
}

#[post("/merge?<data>")]
pub fn merge_data(data: String) -> Result<String, Error> {
    let result = warp_api_ffi::put_drop(&data)?
        .map(|data| hex::encode(&data))
        .unwrap_or(String::new());
    Ok(result)
}

#[post("/zip32?<account>&<external>&<address>")]
pub fn derive_keys(
    account: u32,
    external: u32,
    address: Option<u32>,
) -> Result<Json<KeyPack>, Error> {
    let active = CoinConfig::get_active();
    let result = warp_api_ffi::api::account::derive_keys(
        active.coin,
        active.id_account,
        account,
        external,
        address,
    )?;
    Ok(Json(result))
}

#[post("/trial_decrypt?<height>&<cmu>&<epk>&<ciphertext>")]
pub async fn trial_decrypt(
    height: u32,
    cmu: String,
    epk: String,
    ciphertext: String,
) -> Result<String, Error> {
    let epk = hex::decode(&epk)?;
    let cmu = hex::decode(&cmu)?;
    let ciphertext = hex::decode(&ciphertext)?;
    let note = warp_api_ffi::api::sync::trial_decrypt(height, &cmu, &epk, &ciphertext)?;
    log::info!("{:?}", note);
    Ok(note.is_some().to_string())
}

#[post("/instant_sync")]
pub async fn instant_sync() -> Result<(), Error> {
    let c = CoinConfig::get_active();
    let fvk = {
        let db = c.db()?;
        let AccountData { fvk, .. } = db.get_account_info(c.id_account)?;
        fvk
    };
    let client = reqwest::Client::new();
    let response = client
        .post(format!("https://zec.hanh00.fun/api/scan_fvk?fvk={}", fvk))
        .send()
        .await?;
    // let r = response.text().await?;
    // println!("{}", r);
    let account_info: AccountInfo = response.json().await?;
    let mut db = c.db().unwrap();
    db.import_from_syncdata(&account_info)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct Config {
    allow_backup: bool,
    allow_send: bool,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct AccountSeed {
    coin: u8,
    name: String,
    key: Option<String>,
    index: Option<u32>,
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Heights {
    latest: u32,
    synced: u32,
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct Backup {
    seed: Option<String>,
    sk: Option<String>,
    fvk: String,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct Payment {
    recipients: Vec<Recipient>,
    confirmations: u32,
}
