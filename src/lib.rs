use zcash_primitives::consensus::Network;

#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

pub const NETWORK: Network = Network::MainNetwork;

mod print;
mod chain;
mod path;
mod commitment;
mod scan;
mod builder;

pub use crate::chain::{LWD_URL, get_latest_height, download_chain, calculate_tree_state_v2, DecryptNode};
pub use crate::commitment::NotePosition;
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::scan::scan_all;
