use anyhow::Result;
use base58check::FromBase58Check;
use bitcoin::{hashes::Hash, Address, PubkeyHash, Script};

use crate::db::with_coin;

use super::get_address;

pub fn get_script(coin: u8, id_account: u32) -> Result<Script> {
    let address = with_coin(coin, |c| get_address(c, id_account))?;
    let (_version, hash) = address
        .from_base58check()
        .map_err(|_| anyhow::anyhow!("Invalid address"))?;
    let pkh = PubkeyHash::from_slice(&hash)?;
    let script = Script::new_p2pkh(&pkh);
    Ok(script)
}

pub fn is_valid_address(address: &str) -> bool {
    address.parse::<Address>().is_ok()
}
