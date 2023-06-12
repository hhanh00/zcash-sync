use crate::db::data_generated::fb::{ETHTransaction, ETHTransactionT, TxOutputT, TxReportT};
use crate::RecipientsT;
use anyhow::{anyhow, Result};
use ethers::prelude::*;
use ethers::types::transaction::eip2718::TypedTransaction;
use flatbuffers::FlatBufferBuilder;
use rusqlite::Connection;
use std::str::FromStr;
use std::thread;
use tokio::runtime::Runtime;

pub fn prepare(
    connection: &Connection,
    url: &str,
    account: u32,
    recipients: &RecipientsT,
) -> Result<String> {
    let recipients = recipients.values.as_ref().unwrap();
    if recipients.len() != 1 {
        anyhow::bail!("Must have exactly one recipient");
    }
    let recipient = recipients[0].clone();
    let to_address = recipient.address.unwrap();
    let address = super::db::get_address(connection, account)?;
    let from = Address::from_str(&address)?;
    let to = Address::from_str(&to_address)?;
    let provider = Provider::<Http>::try_from(url)?;
    let unsigned_tx = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let chain_id = provider.get_chainid().await?.as_u64();
            let tx_req = TypedTransaction::Eip1559(
                Eip1559TransactionRequest::default()
                    .chain_id(chain_id)
                    .from(from)
                    .to(to)
                    .data(vec![]),
            );
            let gas = provider.estimate_gas(&tx_req, None).await?;
            let nonce = provider.get_transaction_count(from, None).await?;
            let (max_fee_per_gas, max_priority_fee_per_gas) =
                provider.estimate_eip1559_fees(None).await?;
            Ok::<_, anyhow::Error>(ETHTransactionT {
                chain_id,
                to: Some(to_address),
                value: recipient.amount,
                nonce: nonce.as_u32(),
                gas: gas.as_u64(),
                max_fee_per_gas: max_fee_per_gas.as_u64(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.as_u64(),
            })
        })
    })
    .join()
    .map_err(|_| anyhow!("unsigned_tx"))??;

    let mut builder = FlatBufferBuilder::new();
    let root = unsigned_tx.pack(&mut builder);
    builder.finish(root, None);
    let tx_data = base64::encode(builder.finished_data());
    Ok(tx_data)
}

pub fn to_tx_report(tx_plan: &str) -> Result<TxReportT> {
    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<ETHTransaction>(&tx_plan)?;
    let tx: ETHTransactionT = root.unpack();
    let outputs = vec![TxOutputT {
        address: tx.to.clone(),
        amount: tx.value,
        ..TxOutputT::default()
    }];
    Ok(TxReportT {
        outputs: Some(outputs),
        transparent: tx.value,
        fee: tx.gas,
        ..TxReportT::default()
    })
}

pub fn sign(connection: &Connection, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<ETHTransaction>(&tx_plan)?;
    let tx: ETHTransactionT = root.unpack();
    let wei = U256::from(tx.value) * U256::exp10(10);
    let address = super::db::get_address(connection, account)?;
    let from = Address::from_str(&address)?;
    let to = Address::from_str(&tx.to.unwrap())?;

    let sk = super::db::get_sk(connection, account)?;
    let raw_tx = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let wallet = LocalWallet::from_bytes(&sk)?;
            let tx_req = TypedTransaction::Eip1559(
                Eip1559TransactionRequest::default()
                    .chain_id(tx.chain_id)
                    .from(from)
                    .to(to)
                    .value(wei)
                    .data(vec![])
                    .gas(tx.gas)
                    .max_fee_per_gas(tx.max_fee_per_gas)
                    .max_priority_fee_per_gas(tx.max_priority_fee_per_gas)
                    .nonce(tx.nonce),
            );
            let signature = wallet.sign_transaction(&tx_req).await?;
            let raw_tx = tx_req.rlp_signed(&signature).to_vec();
            Ok::<_, anyhow::Error>(raw_tx)
        })
    })
    .join()
    .map_err(|_| anyhow!("sign"))??;
    Ok(raw_tx)
}

pub fn broadcast(url: &str, raw_tx: &[u8]) -> Result<String> {
    let provider = Provider::<Http>::try_from(url)?;
    let raw_tx = raw_tx.to_vec();
    let id = thread::spawn(move || {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(async move {
            let pending_tx = provider.send_raw_transaction(Bytes::from(raw_tx)).await?;
            let tx_hash = pending_tx.tx_hash();
            let id = hex::encode(tx_hash.as_bytes());
            Ok::<_, anyhow::Error>(id)
        })
    })
    .join()
    .map_err(|_| anyhow!("broadcast"))??;
    Ok(id)
}
