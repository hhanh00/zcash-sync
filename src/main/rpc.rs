#[macro_use]
extern crate rocket;

use rocket::fairing::AdHoc;
use rocket::serde::{Serialize, Deserialize, json::Json};
use rocket::State;
use warp_api_ffi::{CoinConfig, TxRec};
use warp_api_ffi::api::payment::{Recipient, RecipientMemo};

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    warp_api_ffi::init_coin(0, &dotenv::var("ZEC_DB_PATH").unwrap_or("/tmp/zec.db".to_string()))?;
    warp_api_ffi::set_coin_lwd_url(0, &dotenv::var("ZEC_LWD_URL").unwrap_or("https://mainnet.lightwalletd.com:9067".to_string()));

    let _ = rocket::build()
        .mount(
            "/",
            routes![
                set_lwd,
                set_active,
                new_account,
                sync,
                rewind,
                get_latest_height,
                get_backup,
                get_balance,
                get_address,
                get_tx_history,
                pay,
            ],
        )
        .attach(AdHoc::config::<Config>())
        .launch()
        .await?;

    Ok(())
}

#[post("/set_lwd?<coin>&<lwd_url>")]
pub fn set_lwd(coin: u8, lwd_url: String) {
    warp_api_ffi::set_coin_lwd_url(coin, &lwd_url);
}

#[post("/set_active?<coin>&<id_account>")]
pub fn set_active(coin: u8, id_account: u32) {
    warp_api_ffi::set_active_account(coin, id_account);
}

#[post("/new_account", format = "application/json", data="<seed>")]
pub fn new_account(seed: Json<AccountSeed>) -> String {
    let id_account = warp_api_ffi::api::account::new_account(seed.coin, &seed.name, seed.key.clone(), seed.index).unwrap();
    warp_api_ffi::set_active_account(seed.coin, id_account);
    id_account.to_string()
}

#[post("/sync?<offset>")]
pub async fn sync(offset: Option<u32>) {
    let coin = CoinConfig::get_active();
    let _ = warp_api_ffi::api::sync::coin_sync(coin.coin, true, offset.unwrap_or(0), |_| {}).await;
}

#[post("/rewind?<height>")]
pub async fn rewind(height: u32) {
    let _ = warp_api_ffi::api::sync::rewind_to_height(height).await;
}

#[get("/latest_height")]
pub async fn get_latest_height() -> Json<Heights> {
    let latest = warp_api_ffi::api::sync::get_latest_height().await.unwrap();
    let synced = warp_api_ffi::api::sync::get_synced_height().unwrap();
    Json(Heights { latest, synced })
}

#[get("/address")]
pub fn get_address() -> String {
    let c = CoinConfig::get_active();
    let db = c.db().unwrap();
    db.get_address(c.id_account).unwrap()
}

#[get("/backup")]
pub fn get_backup(config: &State<Config>) -> Result<Json<Backup>, String> {
    if !config.allow_backup {
        Err("Backup API not enabled".to_string())
    }
    else {
        let c = CoinConfig::get_active();
        let db = c.db().unwrap();
        let (seed, sk, fvk) = db.get_backup(c.id_account).unwrap();
        Ok(Json(Backup {
            seed,
            sk,
            fvk
        }))
    }
}

#[get("/tx_history")]
pub fn get_tx_history() -> Json<Vec<TxRec>> {
    let c = CoinConfig::get_active();
    let db = c.db().unwrap();
    let txs = db.get_txs(c.id_account).unwrap();
    Json(txs)
}

#[get("/balance")]
pub fn get_balance() -> String {
    let c = CoinConfig::get_active();
    let db = c.db().unwrap();
    let balance = db.get_balance(c.id_account).unwrap();
    balance.to_string()
}

#[post("/pay", data="<payment>")]
pub async fn pay(payment: Json<Payment>, config: &State<Config>) -> Result<String, String> {
    if !config.allow_send {
        Err("Backup API not enabled".to_string())
    }
    else {
        let c = CoinConfig::get_active();
        let latest = warp_api_ffi::api::sync::get_latest_height().await.unwrap();
        let from = {
            let db = c.db().unwrap();
            db.get_address(c.id_account).unwrap()
        };
        let recipients: Vec<_> = payment.recipients.iter().map(|p| RecipientMemo::from_recipient(&from, p)).collect();
        let txid = warp_api_ffi::api::payment::build_sign_send_multi_payment(
            latest,
            &recipients,
            false,
            payment.confirmations,
            Box::new(|_| {})
        ).await.unwrap();
        Ok(txid)
    }
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
