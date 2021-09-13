#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

mod coin;
pub use coin::{get_branch, NETWORK, TICKER};

// Mainnet
// pub const LWD_URL: &str = "https://mainnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "https://lwdv3.zecwallet.co";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// Testnet
// pub const LWD_URL: &str = "https://testnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// YCash
pub const LWD_URL: &str = "https://lite.ycash.xyz:9067";

mod builder;
mod chain;
mod commitment;
mod db;
mod hash;
mod key;
mod ua;
mod mempool;
mod pay;
mod prices;
mod print;
mod scan;
mod taddr;
mod transaction;
mod contact;
mod wallet;

pub use crate::builder::advance_tree;
pub use crate::chain::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_latest_height, sync,
    ChainError, DecryptNode,
};
pub use crate::commitment::{CTree, Witness};
pub use crate::db::DbAdapter;
pub use crate::hash::pedersen_hash;
pub use crate::key::{decode_key, is_valid_key};
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::mempool::MemPool;
pub use crate::pay::{broadcast_tx, sign_offline_tx, Tx};
pub use crate::print::*;
pub use crate::scan::{latest_height, scan_all, sync_async};
pub use crate::wallet::{Wallet, WalletBalance};
pub use crate::ua::{get_sapling, get_ua};
