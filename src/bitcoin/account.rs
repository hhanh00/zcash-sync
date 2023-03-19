use anyhow::Result;
use base58check::ToBase58Check;
use bip39::{Language, Mnemonic, Seed};
use bitcoin::hashes::Hash;
use electrum_client::{Client, ElectrumApi, bitcoin::{Script, PubkeyHash}};
use ripemd::{Digest, Ripemd160};
use rusqlite::{params, Connection};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use tiny_hderive::bip32::ExtendedPrivKey;
use base58check::FromBase58Check;

use crate::{
    db::{
        data_generated::fb::{BalanceT, ShieldedNoteVecT, ShieldedTxVecT},
        with_coin,
    },
    key2::split_key,
    taddr::parse_seckey,
};

use super::COIN_BTC;

pub fn new_account_with_key(name: &str, key: &str, index: u32) -> Result<u32> {
    let (phrase, password) = split_key(key);
    let (seed, sk, address) =
        if let Ok(mnemonic) = Mnemonic::from_phrase(&phrase, Language::English) {
            let (sk, address) = derive_from_mnemonic(mnemonic, &password, index)?;
            (Some(key), sk, address)
        } else if let Ok(sk) = parse_seckey(key) {
            let (sk, address) = derive_from_sk(sk)?;
            (None, sk, address)
        } else {
            anyhow::bail!("Invalid key")
        };
    let id = with_coin(COIN_BTC, |c| {
        c.execute(
            "INSERT INTO accounts(name,seed,aindex,sk,address) VALUES (?1,?2,?3,?4,?5) \
        ON CONFLICT(address) DO NOTHING",
            params![name, seed, index, sk, address],
        )?;
        let id = c.query_row(
            "SELECT id_account FROM accounts WHERE address = ?1",
            params![address],
            |row| row.get::<_, u32>(0),
        )?;
        Ok(id)
    })?;
    Ok(id)
}

fn derive_from_mnemonic(
    mnemonic: Mnemonic,
    password: &str,
    index: u32,
) -> Result<(String, String)> {
    let seed = Seed::new(&mnemonic, &password);
    let path = format!("m/44'/0'/{}'/0/0", index);
    let ext = ExtendedPrivKey::derive(seed.as_bytes(), &*path).expect("Invalid derivation path");
    let sk = SecretKey::from_slice(&ext.secret())?;
    derive_from_sk(sk)
}

fn derive_from_sk(sk: SecretKey) -> Result<(String, String)> {
    let secp = Secp256k1::<All>::new();
    let pk = PublicKey::from_secret_key(&secp, &sk);
    let pub_key = pk.serialize();
    let hash = Ripemd160::digest(&Sha256::digest(&pub_key)).to_vec();
    let address = hash.to_base58check(0);

    let mut sk = sk.serialize_secret().to_vec();
    sk.push(0x01);
    let sk = sk.to_base58check(0);

    Ok((sk, address))
}

pub fn get_address(connection: &Connection, id_account: u32) -> Result<String, anyhow::Error> {
    let address = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        params![id_account],
        |row| row.get::<_, String>(0),
    )?;
    Ok(address)
}

pub fn get_balance(coin: u8, id_account: u32, url: &str) -> Result<u64> {
    let client = Client::new(url)?;
    let address = with_coin(coin, |c| get_address(c, id_account))?;
    let (_version, hash) = address.from_base58check().map_err(|_| anyhow::anyhow!("Invalid address"))?;
    let pkh = PubkeyHash::from_slice(&hash)?;
    let script = Script::new_p2pkh(&pkh);
    let balance = client.script_get_balance(&script)?;
    let balance = balance.confirmed;
    Ok(balance)
}

pub fn get_balances(_id: u32, _height: u32) -> Result<BalanceT> {
    let balance = BalanceT {
        shielded: 0,
        unconfirmed_spent: 0,
        balance: 0,
        under_confirmed: 0,
        excluded: 0,
        sapling: 0,
        orchard: 0,
    };
    Ok(balance)
}

pub fn get_notes(_id: u32) -> Result<ShieldedNoteVecT> {
    Ok(ShieldedNoteVecT {
        notes: Some(vec![]),
    })
}

pub fn get_txs(_id: u32) -> Result<ShieldedTxVecT> {
    Ok(ShieldedTxVecT { txs: Some(vec![]) })
}
