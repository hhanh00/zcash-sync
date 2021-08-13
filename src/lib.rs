#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

#[cfg(feature="ycash")]
mod coin {
    use zcash_primitives::consensus::{Network, BranchId};

    pub const NETWORK: Network = Network::YCashMainNetwork;
    pub const TICKER: &str = "ycash";
    pub fn get_branch(_height: u32) -> BranchId {
        BranchId::Ycash
    }
}

#[cfg(not(feature="ycash"))]
mod coin {
    use zcash_primitives::consensus::{Network, BranchId, BlockHeight};

    pub const NETWORK: Network = Network::MainNetwork;
    pub const TICKER: &str = "zcash";
    pub fn get_branch(height: u32) -> BranchId {
        BranchId::for_height(&NETWORK, BlockHeight::from_u32(height))
    }
}

pub use coin::{NETWORK, TICKER, get_branch};

// Mainnet
// pub const LWD_URL: &str = "https://mainnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "https://lwdv3.zecwallet.co";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
pub const LWD_URL: &str = "http://127.0.0.1:9067";

// Testnet
// pub const LWD_URL: &str = "https://testnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

mod builder;
mod chain;
mod commitment;
mod db;
mod hash;
mod key;
mod mempool;
mod print;
mod scan;
mod taddr;
mod transaction;
mod pay;
mod wallet;
mod prices;

pub use crate::builder::advance_tree;
pub use crate::chain::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_latest_height, sync,
    ChainError, DecryptNode,
};
pub use crate::commitment::{CTree, Witness};
pub use crate::db::DbAdapter;
pub use crate::hash::pedersen_hash;
pub use crate::key::{is_valid_key, decode_key};
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::mempool::MemPool;
pub use crate::print::*;
pub use crate::scan::{latest_height, scan_all, sync_async};
pub use crate::wallet::{Wallet, WalletBalance};
pub use crate::pay::{sign_offline_tx, broadcast_tx, Tx};
