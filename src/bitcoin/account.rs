use std::collections::HashMap;

use anyhow::Result;
use base58check::ToBase58Check;
use bip39::{Language, Mnemonic, Seed};
use bitcoin::hashes::Hash;
use bitcoin::{Address, Txid};
use electrum_client::ElectrumApi;
use ripemd::{Digest, Ripemd160};
use rusqlite::{params, Connection};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use tiny_hderive::bip32::ExtendedPrivKey;

use crate::db::data_generated::fb::{AccountVecT, ShieldedNoteT, ShieldedTxT, TrpTransactionT};
use crate::{
    db::{
        data_generated::fb::{BalanceT, ShieldedNoteVecT, ShieldedTxVecT},
        with_coin,
    },
    key2::split_key,
    taddr::parse_seckey,
};

use super::db::{fetch_accounts, fetch_txs, store_txs};
use super::{get_client, get_script, COIN_BTC};

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
    let sk = sk.to_base58check(0x80);

    Ok((sk, address))
}

pub fn get_address(connection: &Connection, id_account: u32) -> Result<String> {
    let address = connection.query_row(
        "SELECT address FROM accounts WHERE id_account = ?1",
        params![id_account],
        |row| row.get::<_, String>(0),
    )?;
    Ok(address)
}

pub fn get_account_list(coin: u8, url: &str) -> Result<AccountVecT> {
    let mut accounts = with_coin(coin, |c| fetch_accounts(c))?;
    let client = get_client(url)?;
    let scripts: Result<Vec<_>> = accounts.iter().map(|a| get_script(coin, a.id)).collect();
    let scripts = scripts?;
    let balances = client.batch_script_get_balance(scripts.iter())?;
    for (a, b) in accounts.iter_mut().zip(balances.iter()) {
        a.balance = b.confirmed;
    }
    let accounts = AccountVecT {
        accounts: Some(accounts),
    };
    Ok(accounts)
}

pub fn get_balance(coin: u8, id_account: u32, url: &str) -> Result<u64> {
    let balance = get_balances(coin, id_account, url)?;
    let balance = balance.balance;
    Ok(balance)
}

pub fn get_balances(coin: u8, id_account: u32, url: &str) -> Result<BalanceT> {
    let client = get_client(url)?;
    let script = get_script(coin, id_account)?;
    let balance = client.script_get_balance(&script)?;
    let balance = BalanceT {
        shielded: 0,
        unconfirmed_spent: 0,
        balance: balance.confirmed,
        under_confirmed: 0,
        excluded: 0,
        sapling: 0,
        orchard: 0,
    };
    Ok(balance)
}

pub fn get_notes(coin: u8, id_account: u32, url: &str) -> Result<ShieldedNoteVecT> {
    let txs = with_coin(coin, |c| fetch_txs(c, id_account))?;
    let tx_timestamps: HashMap<_, _> = txs
        .iter()
        .map(|tx| (tx.txid.as_deref().unwrap(), tx.timestamp))
        .collect();
    let client = get_client(url)?;
    let script = get_script(coin, id_account)?;
    let utxos = client.script_list_unspent(&script)?;
    let notes: Vec<_> = utxos
        .into_iter()
        .map(|utxo| {
            let timestamp = tx_timestamps[&*utxo.tx_hash];
            ShieldedNoteT {
                id: 0,
                height: utxo.height as u32,
                value: utxo.value,
                timestamp,
                orchard: false,
                excluded: false,
                spent: false,
            }
        })
        .collect();

    Ok(ShieldedNoteVecT { notes: Some(notes) })
}

pub fn get_txs(coin: u8, id_account: u32, url: &str) -> Result<ShieldedTxVecT> {
    let client = get_client(url)?;
    let script = get_script(coin, id_account)?;
    let history = client.script_get_history(&script)?;
    let mut new_txs = with_coin(coin, move |c| {
        let known_txs = fetch_txs(c, id_account)?;
        let known_txs_map: HashMap<_, _> = known_txs
            .into_iter()
            .map(|tx| (tx.txid.clone().unwrap(), tx))
            .collect();
        let mut new_txs = vec![];
        for h in history.iter() {
            let txid = h.tx_hash.to_vec();
            if !known_txs_map.contains_key(&txid) {
                let tx = TrpTransactionT {
                    id: 0,
                    txid: Some(txid.clone()),
                    height: h.height as u32,
                    timestamp: 0,
                    value: 0,
                    address: None,
                };
                new_txs.push(tx);
            }
        }
        Ok(new_txs)
    })?;

    let new_txids: Vec<_> = new_txs
        .iter()
        .map(|tx| {
            let txid = tx.txid.clone().unwrap();
            Txid::from_slice(&txid).unwrap()
        })
        .collect();
    let txs = client.batch_transaction_get(&new_txids)?;
    let mut timestamp_cache: HashMap<u32, u32> = HashMap::new();
    for (btc_tx, tx) in txs.iter().zip(new_txs.iter_mut()) {
        let timestamp = timestamp_cache.entry(tx.height).or_insert_with_key(|h| {
            let header = client.block_header(*h as usize).ok();
            header.map(|h| h.time).unwrap_or_default()
        });
        tx.timestamp = *timestamp;

        let mut tx_value = 0i64;
        let tx_input_ids: Vec<_> = btc_tx
            .input
            .iter()
            .map(|tx_in| tx_in.previous_output.txid)
            .collect();
        let tx_inputs = client.batch_transaction_get(&tx_input_ids)?;
        for (tx_in, tx) in btc_tx.input.iter().zip(tx_inputs.iter()) {
            let tx_out = &tx.output[tx_in.previous_output.vout as usize];
            let spend_script = &tx_out.script_pubkey;
            if &script == spend_script {
                // spending from our address
                let value = tx_out.value as i64;
                tx_value -= value;
            }
        }
        for tx_out in btc_tx.output.iter() {
            let output_script = &tx_out.script_pubkey;
            if &script == output_script {
                // output our address
                let value = tx_out.value as i64;
                tx_value += value;
            } else {
                let address = Address::from_script(&output_script, bitcoin::Network::Bitcoin)?;
                tx.address = Some(address.to_string());
            }
        }
        tx.value = tx_value;
    }

    let txs = with_coin(coin, |c| {
        store_txs(c, id_account, new_txs.iter())?;
        let txs = fetch_txs(c, id_account)?;
        let txs: Vec<_> = txs
            .into_iter()
            .map(|tx| {
                let txid = tx.txid.map(|txid| hex::encode(txid));
                ShieldedTxT {
                    id: tx.id,
                    tx_id: txid.clone(),
                    height: tx.height,
                    short_tx_id: txid.map(|s| s[0..8].to_string()),
                    timestamp: tx.timestamp,
                    name: None,
                    value: tx.value,
                    address: tx.address,
                    memo: None,
                }
            })
            .collect();
        Ok(txs)
    })?;

    Ok(ShieldedTxVecT { txs: Some(txs) })
}
