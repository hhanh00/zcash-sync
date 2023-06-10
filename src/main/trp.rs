use rusqlite::Connection;
use std::path::Path;
use std::slice;
use warp_api_ffi::coin::CoinApi;
use warp_api_ffi::{btc, make_recipient, make_recipients};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME")?;
    let connection = Connection::open(Path::new(&home).join("databases/btc.db"))?;
    btc::init_db(&connection)?;
    let mut btc_handler = btc::BTCHandler::new(connection);
    btc_handler.set_url("tcp://blackie.c3-soft.com:57005");

    // Create test accounts
    btc_handler.new_account("From Seed", "defy source august paper parent monitor clerk drop myself meat knock artist manual faculty impose open submit pipe fluid trial ocean liquid earth swarm")?;
    // btc_handler.new_account(
    //     "From WIF",
    //     "Kzoc8sHEqqanrsnz7vzbuaACupw1Qkz9Y4xFedzG8o37E4mnkSxJ",
    // )?;
    // btc_handler.new_account("From Address", "bc1qg522q57j87md2h2d66fsyrzyraulvparkqcxea")?;
    // btc_handler.new_account("Main", "bc1qc6v36hk9lnud2llsyaaeqvc6jp8c4quq36py94")?;

    btc_handler.sync()?;
    let height = btc_handler.get_latest_height()?;
    println!("{height}");
    let account = 1;
    let balance = btc_handler.get_balance(account)?;
    println!("{balance}");
    let txs = btc_handler.get_txs(account)?;
    println!("{txs:?}");
    let notes = btc_handler.get_notes(account)?;
    println!("{notes:?}");

    let r = make_recipient("tb1qv7rve3kfp5f6ahhgukcqx9tz2wf3xfswpqhjwa", 1000);
    let rs = make_recipients(slice::from_ref(&r));
    let tx = btc_handler.prepare_multi_payment(account, &rs, Some(1))?;
    println!("tx: {tx}");

    let raw_tx = btc_handler.sign(account, &tx)?;
    let txid = btc_handler.broadcast(&raw_tx)?;
    println!("txid: {txid}");

    Ok(())
}
