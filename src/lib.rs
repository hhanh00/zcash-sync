use zcash_primitives::consensus::Network;

#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

pub const NETWORK: Network = Network::TestNetwork;

mod builder;
mod chain;
mod commitment;
mod scan;
mod key;
mod db;
mod wallet;
mod print;

pub use crate::builder::advance_tree;
pub use crate::chain::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_latest_height, sync,
    DecryptNode, LWD_URL,
};
pub use crate::commitment::{CTree, Witness};
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::scan::{scan_all, sync_async, latest_height};
pub use crate::key::{get_secret_key, get_address, get_viewing_key};
pub use crate::db::DbAdapter;
pub use crate::wallet::{Wallet, DEFAULT_ACCOUNT};
