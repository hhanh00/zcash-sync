use zcash_primitives::consensus::{BranchId, Network};

pub const NETWORK: Network = Network::YCashMainNetwork;
pub const TICKER: &str = "ycash";
pub fn get_branch(_height: u32) -> BranchId {
    BranchId::Ycash
}
