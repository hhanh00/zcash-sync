mod account;
mod db;
mod sync;
mod util;

pub const COIN_BTC: u8 = 2u8;

pub use account::{
    get_account_list, get_address, get_balance, get_balances, get_notes, get_txs,
    new_account_with_key,
};
pub use db::migrate_db;
pub use sync::{get_client, get_height, sync};
pub use util::get_script;
