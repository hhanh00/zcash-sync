use crate::db::data_generated::fb::{TONTransaction, TONTransactionT, TxOutputT, TxReportT};
use crate::fb::RecipientT;
use anyhow::Result;
use flatbuffers::FlatBufferBuilder;
use num_bigint::BigUint;
use rusqlite::Connection;
use serde_json::Value;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;
use tonlib::address::TonAddress;
use tonlib::cell::{BagOfCells, CellBuilder};
use tonlib::crypto::KeyPair as TonKeyPair;
use tonlib::wallet::{TonWallet, WalletVersion};

pub fn prepare(
    connection: &Connection,
    url: &str,
    account: u32,
    recipients: &[RecipientT],
) -> Result<String> {
    if recipients.len() != 1 {
        anyhow::bail!("Exactly one recipient required");
    }
    let recipient = recipients[0].clone();
    let address = super::db::get_address(connection, account)?;
    let url = url.to_owned();
    let unsigned_tx = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let get_address_info = format!("{url}/getWalletInformation?address={address}");
            let rep = reqwest::get(&get_address_info).await?;
            let rep_json = rep.json::<Value>().await?;
            let ok = rep_json["ok"].as_bool().unwrap_or(false);
            if !ok {
                anyhow::bail!("Request failed");
            }
            let result = &rep_json["result"];
            let state = result["account_state"].as_str().unwrap_or("");
            let seqno = result["seqno"].as_u64().unwrap_or(0);
            let tx = TONTransactionT {
                to: recipient.address.clone(),
                value: recipient.amount,
                seqno: seqno as u32,
                state: Some(state.to_owned()),
            };

            Ok::<_, anyhow::Error>(tx)
        })
    })
    .join()
    .unwrap()?;
    let mut builder = FlatBufferBuilder::new();
    let root = unsigned_tx.pack(&mut builder);
    builder.finish(root, None);
    let tx_data = base64::encode(builder.finished_data());
    Ok(tx_data)
}

pub fn to_tx_report(tx_plan: &str) -> Result<TxReportT> {
    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<TONTransaction>(&tx_plan)?;
    let tx: TONTransactionT = root.unpack();
    Ok(TxReportT {
        outputs: Some(vec![TxOutputT {
            address: tx.to,
            amount: tx.value,
            ..TxOutputT::default()
        }]),
        transparent: tx.value,
        ..TxReportT::default()
    })
}

pub fn sign(connection: &Connection, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<TONTransaction>(&tx_plan)?;
    let tx: TONTransactionT = root.unpack();

    let wallet_address = super::db::get_address(connection, account)?.parse::<TonAddress>()?;
    let recipient_address = tx.to.unwrap().parse::<TonAddress>()?;
    let sk = super::db::get_sk(connection, account)?;
    let kp = nacl::sign::generate_keypair(&sk);
    let kp = TonKeyPair {
        secret_key: kp.skey.to_vec(),
        public_key: kp.pkey.to_vec(),
    };
    let wallet_version = WalletVersion::V3R2;
    let state_init = match tx.state.as_ref().unwrap().as_str() {
        "uninitialized" => {
            let data = wallet_version.initial_data(0, &kp)?;
            let code = wallet_version.code();
            Some(
                CellBuilder::new()
                    .store_bit(false)? //Split depth
                    .store_bit(false)? //Ticktock
                    .store_bit(true)? //Code
                    .store_bit(true)? //Data
                    .store_bit(false)? //Library
                    .store_reference(code.single_root()?)?
                    .store_reference(data.single_root()?)?
                    .build()?,
            )
        }
        "active" => None,
        _ => anyhow::bail!("Unknown address state {}", tx.state.unwrap()),
    };
    let internal_message = CellBuilder::new()
        .store_u8(6, 0x10)
        .unwrap()
        .store_address(&recipient_address)
        .unwrap()
        .store_coins(&BigUint::from(tx.value * 10))
        .unwrap()
        .store_u64(64, 0)
        .unwrap()
        .store_u64(1 + 4 + 4 + 32 + 1 + 1, 0)
        .unwrap()
        .build()
        .unwrap();
    let wallet = TonWallet::derive(0, wallet_version, &kp)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let unsigned_body = wallet.create_external_body(now + 60, tx.seqno, internal_message)?;
    let signed_body = wallet.sign_external_body(&unsigned_body)?;
    let mut external_message = CellBuilder::new();
    external_message
        .store_u8(2, 2)? // incoming external tx
        .store_u8(2, 0)? // src = addr_none
        .store_address(&wallet_address)? // dest
        .store_coins(&BigUint::default())?; // import fee
    match state_init {
        Some(state_init) => {
            external_message
                .store_bit(true)? // state
                .store_bit(true)? // state as ref
                .store_child(state_init)?;
        }
        None => {
            external_message.store_bit(false)?; // no state
        }
    }
    let external_message = external_message
        .store_bit(true)? // signed_body as ref
        .store_child(signed_body)?
        .build()?;

    let boc = BagOfCells::from_root(external_message);
    let raw = boc.serialize(true)?;
    Ok(raw)
}
