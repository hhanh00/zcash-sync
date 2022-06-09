#[macro_use]
extern crate rocket;

use rocket::serde::{Deserialize, json::Json};
use warp_api_ffi::CoinConfig;

#[rocket::main]
async fn main() -> anyhow::Result<()> {
    warp_api_ffi::init_coin(0, "/tmp/zec.db")?;

    let _ = rocket::build()
        .mount(
            "/",
            routes![
                set_lwd,
                new_account,
                sync,
                // get_address,
                // sync,
                // rewind,
                // balance,
                // pay,
                // tx_history
            ],
        )
        .launch()
        .await?;

    Ok(())
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct AccountSeed {
    coin: u8,
    name: String,
    key: Option<String>,
    index: Option<u32>,
}

#[post("/set_lwd?<coin>&<lwd_url>")]
pub fn set_lwd(coin: u8, lwd_url: String) {
    warp_api_ffi::set_coin_lwd_url(coin, &lwd_url);
}

#[post("/new_account", format = "application/json", data="<seed>")]
pub fn new_account(seed: Json<AccountSeed>) -> std::result::Result<String, String> {
    let id_account = warp_api_ffi::api::account::new_account(seed.coin, &seed.name, seed.key.clone(), seed.index);
    id_account.map(|v| v.to_string()).map_err(|e| e.to_string())
}

#[post("/sync?<offset>")]
pub async fn sync(offset: Option<u32>) {
    let coin = CoinConfig::get_active();
    let _ = warp_api_ffi::api::sync::coin_sync(coin.coin, true, offset.unwrap_or(0), |_| {}).await;
}


pub fn get_backup(id_account: u32) {

}
