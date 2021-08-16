use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::{thread_rng, RngCore};
use rusqlite::NO_PARAMS;
use sync::{pedersen_hash, print_witness2, ChainError, DbAdapter, Wallet, Witness, LWD_URL};
use zcash_client_backend::data_api::wallet::ANCHOR_OFFSET;
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;

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
    let seed2 = dotenv::var("SEED2").unwrap();
    let ivk = dotenv::var("IVK").unwrap();
    let address = dotenv::var("ADDRESS").unwrap();
    let progress = |height| {
        log::info!("Height = {}", height);
    };
    let wallet = Wallet::new(DB_NAME, LWD_URL);
    wallet.new_account_with_key("main", &seed).unwrap();
    // wallet.new_account_with_key("test", &seed2).unwrap();
    // wallet.new_account_with_key("zecpages", &ivk).unwrap();

    let res = wallet.sync(true, ANCHOR_OFFSET, progress).await;
    if let Err(err) = res {
        if let Some(_) = err.downcast_ref::<ChainError>() {
            println!("REORG");
        } else {
            panic!("{}", err);
        }
    }
    // let tx_id = wallet
    //     .send_payment(
    //         1,
    //         &address,
    //         50000,
    //         "test memo",
    //         0,
    //         2,
    //         move |progress| {
    //             println!("{}", progress.cur());
    //         },
    //     )
    //     .await
    //     .unwrap();
    // println!("TXID = {}", tx_id);

    let tx = wallet
        .prepare_payment(1, &address, 50000, "test memo", 0, 2)
        .await
        .unwrap();
    println!("TX = {}", tx);

    Ok(())
}

#[allow(dead_code)]
async fn test_sync() {
    let progress = |height| {
        log::info!("Height = {}", height);
    };

    let wallet = Wallet::new(DB_NAME, LWD_URL);
    wallet.sync(true, ANCHOR_OFFSET, progress).await.unwrap();
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
    db.trim_to_height(1314000).unwrap();
}

#[allow(dead_code)]
fn test_get_balance() {
    let db = DbAdapter::new(DB_NAME).unwrap();
    let balance = db.get_balance(1).unwrap();
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

#[allow(dead_code)]
fn w() {
    let db = DbAdapter::new("zec.db").unwrap();
    // let w_b: Vec<u8> = db.connection.query_row("SELECT witness FROM sapling_witnesses WHERE note = 66 AND height = 1466097", NO_PARAMS, |row| row.get(0)).unwrap();
    // let w = Witness::read(0, &*w_b).unwrap();
    // print_witness2(&w);
    //
    let w_b: Vec<u8> = db
        .connection
        .query_row(
            "SELECT witness FROM sapling_witnesses WHERE note = 66 AND height = 1466200",
            NO_PARAMS,
            |row| row.get(0),
        )
        .unwrap();
    let w = Witness::read(0, &*w_b).unwrap();
    print_witness2(&w);

    println!("GOOD");
    let witness = dotenv::var("WITNESS2").unwrap();
    let w = Witness::read(0, &*hex::decode(&witness).unwrap()).unwrap();
    print_witness2(&w);
}

#[allow(dead_code)]
fn test_hash() {
    let mut r = thread_rng();

    for _ in 0..100 {
        let mut a = [0u8; 32];
        r.fill_bytes(&mut a);
        let mut b = [0u8; 32];
        r.fill_bytes(&mut b);
        let depth = (r.next_u32() % 64) as u8;

        // let sa = "767a9a7e989289efdfa69c4c8e985c31f3c2c0353f20a80f572854206f077f86";
        // let sb = "944c46945a9e7a0a753850bd90f69d44ac884b60244a9f8eacf3a2aeddd08d6e";
        // a.copy_from_slice(&hex::decode(sa).unwrap());
        // b.copy_from_slice(&hex::decode(sb).unwrap());
        // println!("A: {}", hex::encode(a));
        // println!("B: {}", hex::encode(b));

        let node1 = Node::new(a);
        let node2 = Node::new(b);
        let hash = Node::combine(depth as usize, &node1, &node2);
        let hash2 = pedersen_hash(depth, &a, &b);
        // println!("Reference Hash: {}", hex::encode(hash.repr));
        // println!("This Hash:      {}", hex::encode(hash2));
        // need to expose repr for this check
        assert_eq!(hash.repr, hash2);
    }
}

fn main() {
    // test_hash();
    //
    init();
    // test_invalid_witness()
    // test_rewind();
    // test_sync().await;
    test().unwrap();
    // test_get_balance();
    // w();
}
