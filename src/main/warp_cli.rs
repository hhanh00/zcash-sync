use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::RngCore;
use sync::{DbAdapter, Wallet, DEFAULT_ACCOUNT, ChainError, Witness, print_witness2};
use rusqlite::NO_PARAMS;

const DB_NAME: &str = "zec.db";

fn init() {
    let db = DbAdapter::new(DB_NAME).unwrap();
    db.init_db().unwrap();
}

#[tokio::main]
#[allow(dead_code)]
async fn test() -> anyhow::Result<()> {
    dotenv::dotenv().unwrap();
    env_logger::init();

    let seed = dotenv::var("SEED").unwrap();
    let address = dotenv::var("ADDRESS").unwrap();
    let progress = |height| {
        log::info!("Height = {}", height);
    };
    let wallet = Wallet::new(DB_NAME);
    wallet.new_account_with_seed(&seed).unwrap();
    let res = wallet.sync(DEFAULT_ACCOUNT, progress).await;
    if let Err(err) = res {
        if let Some(_) = err.downcast_ref::<ChainError>() {
            println!("REORG");
        }
    }
    // let tx_id = wallet
    //     .send_payment(DEFAULT_ACCOUNT, &address, 50000)
    //     .await
    //     .unwrap();
    // println!("TXID = {}", tx_id);

    Ok(())
}

#[allow(dead_code)]
fn test_make_wallet() {
    let mut entropy = [0u8; 32];
    OsRng.fill_bytes(&mut entropy);
    let mnemonic = Mnemonic::from_entropy(&entropy, Language::English).unwrap();
    let phrase = mnemonic.phrase();
    println!("Seed Phrase: {}", phrase);
}

#[allow(dead_code)]
fn test_rewind() {
    let mut db = DbAdapter::new(DB_NAME).unwrap();
    db.trim_to_height(1466520).unwrap();
}

#[allow(dead_code)]
fn test_get_balance() {
    let db = DbAdapter::new(DB_NAME).unwrap();
    let balance = db.get_balance().unwrap();
    println!("Balance = {}", (balance as f64) / 100_000_000.0);
}

#[allow(dead_code)]
fn test_invalid_witness() {
    dotenv::dotenv().unwrap();
    env_logger::init();

    println!("BAD");
    let witness = dotenv::var("WITNESS").unwrap();
    let w = Witness::read(0, &*hex::decode(&witness).unwrap()).unwrap();
    print_witness2(&w);

    println!("GOOD");
    let witness = dotenv::var("WITNESS2").unwrap();
    let w = Witness::read(0, &*hex::decode(&witness).unwrap()).unwrap();
    print_witness2(&w);
}

fn w() {
    let db = DbAdapter::new("zec.db").unwrap();
    // let w_b: Vec<u8> = db.connection.query_row("SELECT witness FROM sapling_witnesses WHERE note = 66 AND height = 1466097", NO_PARAMS, |row| row.get(0)).unwrap();
    // let w = Witness::read(0, &*w_b).unwrap();
    // print_witness2(&w);
    //
    let w_b: Vec<u8> = db.connection.query_row("SELECT witness FROM sapling_witnesses WHERE note = 66 AND height = 1466200", NO_PARAMS, |row| row.get(0)).unwrap();
    let w = Witness::read(0, &*w_b).unwrap();
    print_witness2(&w);

    println!("GOOD");
    let witness = dotenv::var("WITNESS2").unwrap();
    let w = Witness::read(0, &*hex::decode(&witness).unwrap()).unwrap();
    print_witness2(&w);
}

fn main() {
    init();
    // test_invalid_witness()
    // test_rewind();
    test().unwrap();
    // test_get_balance();
    // w();
}
