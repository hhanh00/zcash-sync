// #![allow(dead_code)]
// #![allow(unused_imports)]
#[path = "generated/cash.z.wallet.sdk.rpc.rs"]
pub mod lw_rpc;

pub use zcash_params::coin::{get_branch, get_coin_type, CoinType};

// Mainnet
pub const LWD_URL: &str = "https://mainnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "https://lwdv3.zecwallet.co";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// Testnet
// pub const LWD_URL: &str = "https://testnet.lightwalletd.com:9067";
// pub const LWD_URL: &str = "http://lwd.hanh.me:9067";
// pub const LWD_URL: &str = "http://127.0.0.1:9067";

// YCash
// pub const LWD_URL: &str = "https://lite.ycash.xyz:9067";

mod builder;
mod chain;
mod coinconfig;
mod commitment;
mod contact;
mod db;
mod fountain;
mod hash;
mod key;
mod key2;
mod mempool;
mod misc;
mod pay;
mod prices;
mod print;
mod scan;
mod taddr;
mod transaction;
mod ua;
mod zip32;
// mod wallet;
pub mod api;

#[cfg(feature = "ledger")]
mod ledger;

#[cfg(not(feature = "ledger"))]
#[allow(dead_code)]
mod ledger {
    pub async fn build_tx_ledger(
        _tx: &mut super::pay::Tx,
        _prover: &impl zcash_primitives::sapling::prover::TxProver,
    ) -> anyhow::Result<Vec<u8>> {
        unreachable!()
    }
}

pub fn hex_to_hash(hex: &str) -> anyhow::Result<[u8; 32]> {
    let mut hash = [0u8; 32];
    hex::decode_to_slice(hex, &mut hash)?;
    Ok(hash)
}

pub use crate::builder::advance_tree;
pub use crate::chain::{
    calculate_tree_state_v2, connect_lightwalletd, download_chain, get_best_server,
    get_latest_height, ChainError, DecryptNode,
};
pub use crate::coinconfig::{
    init_coin, set_active, set_active_account, set_coin_lwd_url, CoinConfig,
};
pub use crate::commitment::{CTree, Witness};
pub use crate::db::{AccountData, AccountInfo, AccountRec, DbAdapter, TxRec};
pub use crate::fountain::{put_drop, FountainCodes, RaptorQDrops};
pub use crate::hash::{pedersen_hash, Hash, GENERATORS_EXP};
pub use crate::key::{generate_random_enc_key, KeyHelpers};
pub use crate::lw_rpc::compact_tx_streamer_client::CompactTxStreamerClient;
pub use crate::lw_rpc::*;
pub use crate::mempool::MemPool;
pub use crate::misc::read_zwl;
pub use crate::pay::{broadcast_tx, get_tx_summary, Tx, TxIn, TxOut};
pub use crate::print::*;
pub use crate::scan::{latest_height, sync_async};
pub use crate::ua::{get_sapling, get_ua};
pub use zip32::{derive_zip32, KeyPack};
// pub use crate::wallet::{decrypt_backup, encrypt_backup, RecipientMemo, Wallet, WalletBalance};

#[cfg(feature = "ledger_sapling")]
pub use crate::ledger::sapling::build_tx_ledger;

#[cfg(feature = "ledger")]
pub use crate::ledger::sweep_ledger;

#[cfg(feature = "nodejs")]
pub mod nodejs;

mod gpu;

pub use gpu::{has_cuda, has_gpu, has_metal, use_gpu};
