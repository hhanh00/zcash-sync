use anyhow::{anyhow, Result};
use bip39::{Language, Mnemonic, MnemonicType};
use ethers::prelude::coins_bip39::English;
use ethers::prelude::*;
use rusqlite::Connection;
use std::str::FromStr;
use std::thread;
use tokio::runtime::Runtime;

pub fn derive_key(connection: &Connection, name: &str, key: &str) -> Result<u32> {
    let key = if key.is_empty() {
        let mnemonic = Mnemonic::new(MnemonicType::Words24, Language::English);
        mnemonic.phrase().to_string()
    } else {
        key.to_string()
    };
    let wallet = MnemonicBuilder::<English>::default()
        .phrase(&*key)
        .build()?;
    let sk = wallet.signer();
    let skb: [u8; 32] = sk.to_bytes().into();
    let address = wallet.address();
    let address = ethers::utils::to_checksum(&address, None);
    super::db::store_keys(connection, name, &key, &skb, &address)
}

pub fn is_valid_key(key: &str) -> bool {
    MnemonicBuilder::<English>::default()
        .phrase(&*key)
        .build()
        .is_ok()
}

pub fn is_valid_address(address: &str) -> bool {
    Address::from_str(address).is_ok()
}

pub fn get_balance(connection: &Connection, url: &str, account: u32) -> Result<u64> {
    let address = super::get_address(connection, account)?;
    let address = Address::from_str(&address[2..])?;
    let provider = Provider::<Http>::try_from(url)?;
    let balance = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let wei = provider.get_balance(address, None).await?;
            Ok::<_, anyhow::Error>(wei / U256::exp10(10)) // rescale to sats
        })
    })
    .join()
    .map_err(|_| anyhow!("get_balance"))??;
    Ok(balance.as_u64())
}
