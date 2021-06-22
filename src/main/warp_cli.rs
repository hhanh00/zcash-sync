use sync::{sync_async, DbAdapter};
use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::RngCore;

const DB_NAME: &str = "zec.db";

#[tokio::main]
#[allow(dead_code)]
async fn test() -> anyhow::Result<()> {
    dotenv::dotenv().unwrap();
    env_logger::init();

    let ivk = dotenv::var("IVK").unwrap();
    {
        let db = DbAdapter::new(DB_NAME)?;
        db.init_db()?;
    }
    sync_async(&ivk, 50000, DB_NAME, |height| {
        log::info!("Height = {}", height);
    }).await?;

    Ok(())
}

#[allow(dead_code)]
fn test_rewind() {
    let mut db = DbAdapter::new(DB_NAME).unwrap();
    db.trim_to_height(1_250_000).unwrap();
}

fn main() {
    // test_rewind();
    test().unwrap();
    // let mut entropy = [0u8; 32];
    // OsRng.fill_bytes(&mut entropy);
    // let mnemonic = Mnemonic::from_entropy(&entropy, Language::English).unwrap();
    // let phrase = mnemonic.phrase();
    // println!("Seed Phrase: {}", phrase);
}
