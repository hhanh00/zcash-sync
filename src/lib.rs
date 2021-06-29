use zcash_primitives::consensus::Network;

#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

pub const NETWORK: Network = Network::MainNetwork;

mod builder;
mod chain;
mod commitment;
mod db;
mod key;
mod mempool;
mod print;
mod scan;
mod wallet;

pub use crate::builder::advance_tree;
pub use crate::chain::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_latest_height, sync,
    DecryptNode, LWD_URL, ChainError
};
pub use crate::commitment::{CTree, Witness};
pub use crate::db::DbAdapter;
pub use crate::key::{get_address, get_secret_key, get_viewing_key};
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::mempool::MemPool;
pub use crate::scan::{latest_height, scan_all, sync_async};
pub use crate::wallet::{Wallet, WalletBalance, DEFAULT_ACCOUNT};
pub use crate::print::*;
