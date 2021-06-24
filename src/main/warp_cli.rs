use sync::{DbAdapter, Wallet, DEFAULT_ACCOUNT};
use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::RngCore;

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
    wallet.sync(DEFAULT_ACCOUNT, progress).await.unwrap();
    let tx_id = wallet.send_payment(DEFAULT_ACCOUNT, &address, 1000).await.unwrap();
    println!("TXID = {}", tx_id);

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
    db.trim_to_height(1460000).unwrap();
}

fn test_get_balance() {
    let db = DbAdapter::new(DB_NAME).unwrap();
    let balance = db.get_balance().unwrap();
    println!("Balance = {}", (balance as f64) / 100_000_000.0);
}

fn main() {
    init();
    // test_rewind();
    test().unwrap();
    test_get_balance();
}
