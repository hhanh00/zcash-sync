#![allow(dead_code)]
// #![allow(unused_imports)]
// #![warn(missing_docs)]

//! A library for fast synchronization of y/zcash blockchain
//!
//! - Implements the warp sync algorithm for sapling
//! - Multi Account management

//! # Example
//! ```rust
//! use warp_api_ffi::api::account::{get_backup, new_account};
//! use warp_api_ffi::api::sync::coin_sync;
//! use warp_api_ffi::{CoinConfig, init_coin, set_coin_lwd_url};
//! use lazy_static::lazy_static;
//! use std::sync::Mutex;
//!
//! lazy_static! {
//!     static ref CANCEL: Mutex<bool> = Mutex::new(false);
//! }
//!
//! const FVK: &str = "zxviews1q0duytgcqqqqpqre26wkl45gvwwwd706xw608hucmvfalr759ejwf7qshjf5r9aa7323zulvz6plhttp5mltqcgs9t039cx2d09mgq05ts63n8u35hyv6h9nc9ctqqtue2u7cer2mqegunuulq2luhq3ywjcz35yyljewa4mgkgjzyfwh6fr6jd0dzd44ghk0nxdv2hnv4j5nxfwv24rwdmgllhe0p8568sgqt9ckt02v2kxf5ahtql6s0ltjpkckw8gtymxtxuu9gcr0swvz";
//!
//! #[tokio::main]
//! async fn main() {
//!     env_logger::init();
//!
//!     // Initialize the library for Zcash (coin = 0)
//!     init_coin(0, "./zec.db").unwrap();
//!     set_coin_lwd_url(0, "https://lwdv3.zecwallet.co:443"); // ZecWallet Lightwalletd URL
//!
//!     // Create a new account with the ZEC pages viewing key
//!     let id_account = new_account(0, "test_account", Some(FVK.to_string()),
//!                                  None).unwrap();
//!
//!     // Synchronize
//!     coin_sync(0 /* zcash */,
//!               true /* retrieve tx details */,
//!               0 /* sync to tip */,
//!               100 /* spam filter threshold */, |p| {
//!             log::info!("Progress: {}", p.height);
//!         }, &CANCEL).await.unwrap();
//!
//!     // Grab the database accessor
//!     let cc = &CoinConfig::get(0 /* zcash */);
//!     let db = cc.db.as_ref().unwrap().clone();
//!     let db = db.lock().unwrap();
//!
//!     // Query the account balance
//!     let balance = db.get_balance(id_account).unwrap();
//!
//!     println!("Balance = {}", balance)
//! }
//! ```

#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

// Mainnet
// const LWD_URL: &str = "https://mainnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "https://lwdv3.zecwallet.co";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// Testnet
// pub const LWD_URL: &str = "https://testnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// YCash
// pub const LWD_URL: &str = "https://lite.ycash.xyz:9067";

pub type Hash = [u8; 32];

pub mod api;
mod btc;
mod chain;
/// accounts, sync, payments, etc.
pub mod coin;
mod coinconfig;
mod contact;
mod db;
mod eth;
mod fountain;
mod hash;
mod key;
mod mempool;
mod misc;
mod note_selection;
mod orchard;
mod pay;
mod sapling;
mod scan;
mod sync;
mod taddr;
mod ton;
mod transaction;
pub mod transparent;
mod tron;
mod unified;
mod zcash;
mod zip32;

#[cfg(feature = "ledger")]
pub mod ledger;

pub use crate::chain::{connect_lightwalletd, get_best_server, ChainError};
pub use crate::coinconfig::{
    init_coin, set_active, set_active_account, set_coin_lwd_url, CoinConfig, COIN_CONFIG,
};
pub use crate::db::{AccountData, AccountRec, DbAdapter, DbAdapterBuilder, TxRec};
pub use crate::fountain::{FountainCodes, RaptorQDrops};
// pub use crate::key::KeyHelpers;
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::pay::{broadcast_tx, Tx, TxIn, TxOut};
// pub use crate::wallet::{decrypt_backup, encrypt_backup, RecipientMemo, Wallet, WalletBalance};

pub use crate::orchard::decode_merkle_path as decode_orchard_merkle_path;
pub use crate::unified::{decode_unified_address, get_unified_address};
pub use note_selection::{
    build_tx, build_tx_plan, fetch_utxos, get_secret_keys, Destination, Source,
    TransactionBuilderConfig, TransactionBuilderError, TransactionPlan, TxBuilderContext,
    MAX_ATTEMPTS,
};

pub use api::recipient::{make_recipient, make_recipients};
pub use btc::{init_db as init_btc_db, BTCHandler};
pub use coin::NoCoin;
pub use db::data_generated::fb::{RecipientT, RecipientsT};
pub use eth::{init_db as init_eth_db, ETHHandler};

#[cfg(feature = "nodejs")]
pub mod nodejs;

mod gpu;

pub fn init_test() {
    let _ = env_logger::try_init();
    init_coin(0, "./zec.db").unwrap();
    set_coin_lwd_url(0, "http://127.0.0.1:9067");
}

pub use taddr::derive_from_secretkey;
