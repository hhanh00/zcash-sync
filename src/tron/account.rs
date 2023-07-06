use anyhow::Result;
use base58check::{FromBase58Check, ToBase58Check};
use bip39::{Language, Mnemonic, MnemonicType, Seed};
use rusqlite::Connection;
use secp256k1::{All, PublicKey, Secp256k1};
use sha3::Digest;
use tiny_hderive::bip32::ExtendedPrivKey;
use tiny_hderive::bip44::DerivationPath;

const TRON_B58_VERSION: u8 = 0x41;

pub fn derive_key(connection: &Connection, name: &str, key: &str) -> Result<u32> {
    let phrase = if key.is_empty() {
        let mnemonic = Mnemonic::new(MnemonicType::Words24, Language::English);
        mnemonic.phrase().to_string()
    } else {
        key.to_string()
    };
    let mnemonic = Mnemonic::from_phrase(&phrase, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let path: DerivationPath = "m/44'/195'/0'/0/0".parse().unwrap();
    let esk = ExtendedPrivKey::derive(seed.as_bytes(), path).unwrap();
    let secp = Secp256k1::<All>::new();
    let sk = secp256k1::SecretKey::from_slice(&esk.secret())?;
    let pk = PublicKey::from_secret_key(&secp, &sk);
    let mut hasher = sha3::Keccak256::new();
    hasher.update(&pk.serialize_uncompressed()[1..]);
    let hash = hasher.finalize().to_vec();
    let address = hash[hash.len() - 20..].to_base58check(TRON_B58_VERSION);
    let id = super::db::store_keys(connection, name, &phrase, sk.as_ref(), &address)?;
    Ok(id)
}

pub fn is_valid_key(key: &str) -> bool {
    Mnemonic::from_phrase(&key, Language::English).is_ok()
}

pub fn is_valid_address(address: &str) -> bool {
    let address = address.to_owned();
    match address.from_base58check() {
        Ok((TRON_B58_VERSION, _)) => true,
        _ => false,
    }
}
