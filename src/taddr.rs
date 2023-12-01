use crate::api::payment_v2::build_tx_plan_with_utxos;
use crate::api::recipient::RecipientMemo;
use crate::chain::{get_latest_height, EXPIRY_HEIGHT_OFFSET};
use crate::coinconfig::CoinConfig;
use crate::db::data_generated::fb::FeeT;
use crate::db::AccountData;
use crate::key2::split_key;
use crate::note_selection::{Source, UTXO};
use crate::unified::orchard_as_unified;
use crate::zip32::derive_zip32;
use crate::{
    AddressList, BlockId, BlockRange, CompactTxStreamerClient, Connection,
    GetAddressUtxosArg, GetAddressUtxosReply, Hash, TransparentAddressBlockFilter, TxFilter,
};
use anyhow::anyhow;
use base58check::{FromBase58Check, ToBase58Check};
use bip39::{Language, Mnemonic, Seed};
use core::slice;
use futures::StreamExt;
use ripemd::{Digest, Ripemd160};
use rusqlite::OptionalExtension;
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use std::collections::HashMap;
use tiny_hderive::bip32::ExtendedPrivKey;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::encode_transparent_address;
use zcash_params::coin::get_branch;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::memo::Memo;
use zcash_primitives::transaction::components::OutPoint;
use zcash_primitives::transaction::Transaction;

pub async fn get_taddr_balance(
    client: &mut CompactTxStreamerClient<Channel>,
    address: &str,
) -> anyhow::Result<u64> {
    let req = AddressList {
        addresses: vec![address.to_string()],
    };
    let rep = client
        .get_taddress_balance(Request::new(req))
        .await?
        .into_inner();
    Ok(rep.value_zat as u64)
}

pub struct TransparentTxInfo {
    pub txid: Hash,
    pub height: u32,
    pub timestamp: u32,
    pub inputs: Vec<OutPoint>,
    pub in_value: u64,
    pub out_value: u64,
}

/* With the current LWD API, this function performs poorly because
the server does not return tx timestamp, height and value
 */
#[allow(unused)]
pub async fn get_ttx_history(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    address: &str,
) -> anyhow::Result<Vec<TransparentTxInfo>> {
    let mut rep = client
        .get_taddress_txids(Request::new(TransparentAddressBlockFilter {
            address: address.to_string(),
            range: None,
        }))
        .await?
        .into_inner();
    let mut heights: HashMap<u32, u32> = HashMap::new();
    let mut txs = vec![];
    while let Some(raw_tx) = rep.message().await? {
        let height = raw_tx.height as u32;
        heights.insert(height, 0);
        let consensus_branch_id = get_branch(network, height);
        let tx = Transaction::read(&*raw_tx.data, consensus_branch_id)?;
        let txid = tx.txid();
        let tx_data = tx.into_data();
        let mut inputs = vec![];
        if let Some(transparent_bundle) = tx_data.transparent_bundle() {
            for vin in transparent_bundle.vin.iter() {
                inputs.push(vin.prevout.clone());
            }
            let out_value = transparent_bundle
                .vout
                .iter()
                .map(|vout| i64::from(vout.value))
                .sum::<i64>() as u64;
            txs.push(TransparentTxInfo {
                txid: txid.as_ref().clone(),
                height,
                timestamp: 0,
                inputs,
                in_value: 0,
                out_value,
            });
        }
    }

    for (h, timestamp) in heights.iter_mut() {
        let block = client
            .get_block(Request::new(BlockId {
                height: *h as u64,
                hash: vec![],
            }))
            .await?
            .into_inner();
        *timestamp = block.time;
    }

    for tx in txs.iter_mut() {
        let mut in_value = 0;
        for input in tx.inputs.iter() {
            let raw_tx = client
                .get_transaction(Request::new(TxFilter {
                    block: None,
                    index: 0,
                    hash: input.hash().to_vec(),
                }))
                .await?
                .into_inner();
            let consensus_branch_id = get_branch(network, raw_tx.height as u32);
            let tx = Transaction::read(&*raw_tx.data, consensus_branch_id)?;
            let tx_data = tx.into_data();
            let transparent_bundle = tx_data
                .transparent_bundle()
                .ok_or(anyhow!("No transparent bundle"))?;
            let value = i64::from(transparent_bundle.vout[input.n() as usize].value);
            in_value += value;
        }
        tx.timestamp = heights[&tx.height];
        tx.in_value = in_value as u64;
        tx.inputs.clear();
    }
    Ok(txs)
}

pub async fn get_taddr_tx_count(
    client: &mut CompactTxStreamerClient<Channel>,
    address: &str,
    range: &BlockRange,
) -> anyhow::Result<u32> {
    let req = TransparentAddressBlockFilter {
        address: address.to_string(),
        range: Some(range.clone()),
    };
    let rep = client
        .get_taddress_txids(Request::new(req))
        .await?
        .into_inner();
    let count = rep.count().await;
    Ok(count as u32)
}

pub async fn get_utxos(
    client: &mut CompactTxStreamerClient<Channel>,
    t_address: &str,
) -> anyhow::Result<Vec<GetAddressUtxosReply>> {
    let req = GetAddressUtxosArg {
        addresses: vec![t_address.to_string()],
        start_height: 0,
        max_entries: 0,
    };
    let utxo_rep = client
        .get_address_utxos(Request::new(req))
        .await?
        .into_inner();
    Ok(utxo_rep.address_utxos)
}

pub async fn scan_transparent_accounts(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    seed: &str,
    mut aindex: u32,
    gap_limit: usize,
) -> anyhow::Result<Vec<TBalance>> {
    let last_height = get_latest_height(client).await?;
    let range = BlockRange {
        start: Some(BlockId { height: 1, hash: vec![] }),
        end: Some(BlockId { height: last_height as u64, hash: vec![] }),
        ..BlockRange::default()
    };
    let mut addresses = vec![];
    let mut gap = 0;
    while gap < gap_limit {
        let bip44_path = format!("m/44'/{}'/0'/0/{}", network.coin_type(), aindex);
        log::info!("{} {}", aindex, bip44_path);
        let (_, address) = derive_tkeys(network, &seed, &bip44_path)?;
        let balance = get_taddr_balance(client, &address).await?;
        if balance > 0 {
            addresses.push(TBalance {
                index: aindex,
                address,
                balance,
            });
            gap = 0;
        } else {
            let tx_count = get_taddr_tx_count(client, &address, &range).await?;
            if tx_count != 0 {
                gap = 0;
            } else {
                gap += 1;
            }
        }
        aindex += 1;
    }
    Ok(addresses)
}

pub fn derive_tkeys(
    network: &Network,
    phrase: &str,
    path: &str,
) -> anyhow::Result<(String, String)> {
    let (phrase, password) = split_key(phrase);
    let mnemonic = Mnemonic::from_phrase(&phrase, Language::English)?;
    let seed = Seed::new(&mnemonic, &password);
    let ext = ExtendedPrivKey::derive(seed.as_bytes(), path)
        .map_err(|_| anyhow!("Invalid derivation path"))?;
    let secret_key = SecretKey::from_slice(&ext.secret())?;
    derive_from_secretkey(network, &secret_key)
}

pub fn parse_seckey(key: &str) -> anyhow::Result<SecretKey> {
    let (_, sk) = key.from_base58check().map_err(|_| anyhow!("Invalid key"))?;
    let sk = &sk[0..sk.len() - 1]; // remove compressed pub key marker
    let secret_key = SecretKey::from_slice(&sk)?;
    Ok(secret_key)
}

pub fn derive_taddr(network: &Network, key: &str) -> anyhow::Result<(SecretKey, String)> {
    let secret_key = parse_seckey(key)?;
    let (_, addr) = derive_from_secretkey(network, &secret_key)?;
    Ok((secret_key, addr))
}

pub fn derive_from_secretkey(
    network: &Network,
    sk: &SecretKey,
) -> anyhow::Result<(String, String)> {
    let secp = Secp256k1::<All>::new();
    let pub_key = PublicKey::from_secret_key(&secp, &sk);
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let address = TransparentAddress::PublicKey(pub_key.into());
    let address = encode_transparent_address(
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        &address,
    );
    let sk = sk.display_secret().to_string();
    Ok((sk, address))
}

pub fn derive_from_pubkey(network: &Network, pub_key: &[u8]) -> anyhow::Result<String> {
    let pub_key = PublicKey::from_slice(pub_key)?;
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let address = TransparentAddress::PublicKey(pub_key.into());
    let address = encode_transparent_address(
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        &address,
    );
    Ok(address)
}

pub async fn sweep_tkey(
    coin: u8,
    account: u32,
    last_height: u32,
    sk: &str,
    pool: u8,
    address: &str,
    fee_rule: &FeeT,
) -> anyhow::Result<crate::TransactionPlan> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let (seckey, from_address) = derive_taddr(network, sk)?;

    let mut client = c.connect_lwd().await?;
    let utxos = get_utxos(&mut client, &from_address).await?;

    let utxos: Vec<_> = utxos
        .iter()
        .enumerate()
        .map(|(i, utxo)| UTXO {
            id: i as u32,
            source: Source::Transparent {
                txid: utxo.txid.clone().try_into().unwrap(),
                index: utxo.index as u32,
            },
            amount: utxo.value_zat as u64,
            key: Some(seckey.serialize_secret()),
        })
        .collect();

    let tx_plan = sweep_utxos(coin, account, pool, address, last_height,
        &utxos, fee_rule).await?;
    Ok(tx_plan)
}

pub async fn sweep_tseed(
    coin: u8,
    account: u32,
    last_height: u32,
    phrase: &str,
    pool: u8,
    address: &str,
    index: u32,
    limit: u32,
    fee_rule: &FeeT,
) -> anyhow::Result<crate::TransactionPlan> {
    let secp = Secp256k1::<All>::new();

    let range = BlockRange {
        start: Some(BlockId { height: 1, hash: vec![] }),
        end: Some(BlockId { height: last_height as u64, hash: vec![] }),
        ..BlockRange::default()
    };

    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let network = c.chain.network();

    let mut a = 0;
    let mut gap = 0;
    let mut external = 0;
    let mut inputs = vec![];
    loop {
        let kp = derive_zip32(network, phrase, index, external, Some(a))?;
        let tkey = kp.t_key.unwrap();
        let sk = parse_seckey(&tkey)?;
        let (_, taddr) = derive_from_secretkey(network, &sk)?;
        log::info!("sweep_tseed {}", taddr);
        let utxos = get_utxos(&mut client, &taddr).await?;
        for utxo in utxos.iter() {
            inputs.push(UTXO {
                source: Source::Transparent {
                    txid: utxo.txid.clone().try_into().unwrap(),
                    index: utxo.index as u32,
                },
                amount: utxo.value_zat as u64,
                key: Some(sk.serialize_secret()),
                id: 0,
            });
        }
        if utxos.is_empty() {
            let tx_count = get_taddr_tx_count(&mut client, &taddr, &range).await?;
            if tx_count == 0 {
                gap += 1;
            }
            else {
                gap = 0;
            }
        }
        else {
            gap = 0;
        }

        external += 1;
        if external == 2 {
            external = 0;
            a += 1;
        }

        if gap > limit {
            break;
        }
    }

    let tx_plan = sweep_utxos(coin, account, pool, address, last_height,
        &inputs, fee_rule).await?;
    Ok(tx_plan)
}

async fn sweep_utxos(coin: u8, account: u32, pool: u8,
    address: &str,
    last_height: u32,
    utxos: &[UTXO],
    fee_rule: &FeeT) -> anyhow::Result<crate::TransactionPlan> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let to_address = if address.is_empty() {
        let db = c.db().unwrap();

        let to_address = match pool {
            0 => db.get_taddr(account)?,
            1 => {
                let AccountData { address, .. } = db.get_account_info(account)?;
                Some(address)
            }
            2 => {
                let okeys = db.get_orchard(account)?;
                okeys.map(|okeys| {
                    let address = okeys.get_address(0);
                    orchard_as_unified(network, &address)
                })
            }
            _ => unreachable!(),
        };
        let to_address = to_address.ok_or(anyhow!("Account has no address of this type"))?;
        to_address
    }
    else {
        address.to_string()
    };

    let balance = utxos.iter().map(|utxo| utxo.amount).sum::<u64>();
    let recipient = RecipientMemo {
        address: to_address,
        amount: balance,
        fee_included: true,
        memo: Memo::default(),
        max_amount_per_note: 0,
    };
    let tx_plan = build_tx_plan_with_utxos(
        coin,
        account,
        last_height,
        last_height + EXPIRY_HEIGHT_OFFSET,
        slice::from_ref(&recipient),
        &utxos,
        fee_rule,
    )
    .await?;
    Ok(tx_plan)
}


pub fn get_base58_tsk(connection: &Connection, account: u32) -> anyhow::Result<Option<String>> {
    let tsk = connection
        .query_row("SELECT sk FROM taddrs WHERE account = ?1", [account], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?;
    let base58_tsk = tsk.flatten().map(|tsk| {
        let mut sk = hex::decode(tsk).unwrap();
        sk.push(0x01);
        sk.to_base58check(0x80)
    });
    Ok(base58_tsk)
}

pub struct TBalance {
    pub index: u32,
    pub address: String,
    pub balance: u64,
}
