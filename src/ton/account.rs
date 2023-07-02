use crate::db::data_generated::fb::HeightT;
use anyhow::Result;
use bip39::{Language, Mnemonic, MnemonicType, Seed};
use ed25519_dalek_bip32::{ChildIndex, DerivationPath, ExtendedSecretKey};
use rusqlite::Connection;
use tonlib::address::TonAddress;
use tonlib::crypto::KeyPair as TonKeyPair;
use tonlib::wallet::{TonWallet, WalletVersion};

pub fn derive_key(connection: &Connection, name: &str, key: &str) -> Result<u32> {
    let phrase = if key.is_empty() {
        let mnemonic = Mnemonic::new(MnemonicType::Words24, Language::English);
        mnemonic.phrase().to_string()
    } else {
        key.to_string()
    };
    let mnemonic = Mnemonic::from_phrase(&phrase, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let esk = ExtendedSecretKey::from_seed(seed.as_bytes())?;
    let esk = esk.derive(&DerivationPath::bip32([
        ChildIndex::hardened(44)?,
        ChildIndex::hardened(607)?,
        ChildIndex::hardened(0)?,
    ])?)?;
    let sk = esk.secret_key;
    let kp = nacl::sign::generate_keypair(sk.as_bytes());
    let kp = TonKeyPair {
        secret_key: kp.skey.to_vec(),
        public_key: kp.pkey.to_vec(),
    };
    let wallet = TonWallet::derive(0, WalletVersion::V3R2, &kp)?;
    let address = wallet.address.to_base64_url();
    let id = super::db::store_keys(connection, name, &phrase, sk.as_bytes(), &address)?;
    Ok(id)
}

pub fn is_valid_key(key: &str) -> bool {
    Mnemonic::from_phrase(&key, Language::English).is_ok()
}

pub fn is_valid_address(address: &str) -> bool {
    address.parse::<TonAddress>().is_ok()
}

pub fn balance(connection: &Connection, account: u32) -> Result<u64> {
    let balance = connection.query_row(
        "SELECT balance FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, u64>(0),
    )?;
    Ok(balance)
}

pub fn db_height(connection: &Connection, account: u32) -> Result<Option<HeightT>> {
    let height = connection.query_row(
        "SELECT height FROM accounts WHERE id_account = ?1",
        [account],
        |r| r.get::<_, u32>(0),
    )?;
    Ok(Some(HeightT {
        height,
        ..HeightT::default()
    }))
}
