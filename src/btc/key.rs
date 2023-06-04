use crate::btc::BTCNET;
use anyhow::Result;
use bip39::{Language, Mnemonic, MnemonicType, Seed};
use electrum_client::bitcoin::address::Payload;
use electrum_client::bitcoin::bip32::{DerivationPath, ExtendedPrivKey};
use electrum_client::bitcoin::secp256k1::{All, Secp256k1};
use electrum_client::bitcoin::{Address, PrivateKey};
use std::str::FromStr;

#[derive(Debug)]
pub struct AccountKey {
    pub passphrase: Option<String>,
    pub secret_key: Option<[u8; 32]>,
    pub pub_key: Option<[u8; 33]>,
    pub pkh: [u8; 20],
    pub address: String,
}

pub fn derive_key(key: &str) -> Result<AccountKey> {
    if key.is_empty() {
        let m = Mnemonic::new(MnemonicType::Words24, Language::English);
        return derive_key(m.phrase());
    }
    if key.contains(" ") {
        derive_passphrase(key)
    } else if key.starts_with("tb1") {
        derive_address(key)
    } else {
        derive_private_key(key)
    }
}

fn derive_passphrase(passphrase: &str) -> Result<AccountKey> {
    let mnemonic = Mnemonic::from_phrase(passphrase, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let secp = Secp256k1::<All>::new();
    let ext = ExtendedPrivKey::new_master(BTCNET, seed.as_bytes())?;
    let ext = ext.derive_priv(&secp, &DerivationPath::from_str("m/84'/1'/0'/0/0").unwrap())?;
    let sk = ext.to_priv();
    let AccountKey {
        secret_key,
        pub_key,
        pkh,
        address,
        ..
    } = derive_secret_key(&sk)?;
    Ok(AccountKey {
        passphrase: Some(passphrase.to_string()),
        secret_key,
        pub_key,
        pkh,
        address,
    })
}

fn derive_private_key(wif: &str) -> Result<AccountKey> {
    let sk = PrivateKey::from_wif(wif)?;
    derive_secret_key(&sk)
}

pub fn derive_address(address: &str) -> Result<AccountKey> {
    let address = Address::from_str(address)?;
    let address = address.require_network(BTCNET)?;
    derive_payload(&address.payload)
}

fn derive_secret_key(sk: &PrivateKey) -> Result<AccountKey> {
    let secp = Secp256k1::<All>::new();
    let skb: [u8; 32] = sk.to_bytes().try_into().unwrap();
    let pub_key = sk.public_key(&secp);
    let pub_keyb: [u8; 33] = pub_key.to_bytes().try_into().unwrap();
    let payload = Payload::p2wpkh(&pub_key)?;
    let AccountKey { pkh, address, .. } = derive_payload(&payload)?;

    Ok(AccountKey {
        passphrase: None,
        secret_key: Some(skb),
        pub_key: Some(pub_keyb),
        pkh,
        address,
    })
}

fn derive_payload(payload: &Payload) -> Result<AccountKey> {
    let address = Address::new(BTCNET, payload.clone());
    let wp = match payload {
        Payload::WitnessProgram(wp) => wp,
        _ => anyhow::bail!("Invalid address"),
    };
    let pkhb: &[u8] = wp.program().as_ref();
    let pkhb: [u8; 20] = pkhb.try_into()?;
    Ok(AccountKey {
        passphrase: None,
        secret_key: None,
        pub_key: None,
        pkh: pkhb.clone(),
        address: address.to_string(),
    })
}

/*
let mnemonic = Mnemonic::from_phrase(phrase, Language::English)?;
let seed = Seed::new(&mnemonic, "");
let secp = Secp256k1::<All>::new();
let ext = ExtendedPrivKey::new_master(Network::Bitcoin, seed.as_bytes()).unwrap();
let ext = ext
.derive_priv(&secp, &DerivationPath::from_str("m/44'/0'/0'/0/0").unwrap())
.unwrap();
let pub_key = ext.to_priv().public_key(&secp);
let address = electrum_client::bitcoin::address::Address::p2pkh(&pub_key, Network::Bitcoin);
*/
