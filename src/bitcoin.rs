mod account;
mod db;

pub const COIN_BTC: u8 = 2u8;

pub use account::{get_address, get_balance, get_balances, get_notes, get_txs, new_account_with_key};
pub use db::migrate_db;
