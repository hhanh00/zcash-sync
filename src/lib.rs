use zcash_primitives::consensus::Network;

#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

pub const NETWORK: Network = Network::MainNetwork;

mod chain;
