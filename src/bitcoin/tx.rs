use anyhow::Result;
use bitcoin::hashes::Hash;
use electrum_client::ElectrumApi;
use zcash_primitives::memo::MemoBytes;

use crate::{
    bitcoin::{get_client, get_script},
    db::{
        data_generated::fb::{RecipientT, TxOutputT, TxReportT},
        with_coin,
    },
    note_selection::{Destination, Fill, Source, UTXO},
    TransactionPlan,
};

use super::get_address;

pub fn prepare_tx(
    coin: u8,
    id_account: u32,
    recipients: &[RecipientT],
    num_blocks: u32,
    url: &str,
) -> Result<TransactionPlan> {
    let client = get_client(url)?;
    let fee_rate = (client.estimate_fee(num_blocks as usize)? * 100_000f64) as u64;
    let mut fee = 0;

    let account_address = with_coin(coin, |c| get_address(c, id_account))?;
    let script = get_script(coin, id_account)?;
    let utxos = client.script_list_unspent(&script)?;

    let mut tx = None;

    for _ in 0..2 {
        let mut amount = 0;
        let mut outputs = vec![];
        for (id, recipient) in recipients.iter().enumerate() {
            let address = recipient.address.as_ref().unwrap();
            let tx_out = Fill {
                id_order: Some(id as u32),
                destination: Destination::TransparentAddress(address.clone()),
                amount: recipient.amount,
                memo: MemoBytes::empty(),
            };
            amount += tx_out.amount;
            outputs.push(tx_out);
        }

        let target_amount = amount + fee;
        let mut inputs = vec![];
        let mut value_inputs = 0;
        for (id, utxo) in utxos.iter().enumerate() {
            let tx_in = UTXO {
                id: id as u32,
                source: Source::Transparent {
                    txid: utxo.tx_hash.into_inner(),
                    index: utxo.tx_pos as u32,
                },
                amount: utxo.value,
            };
            value_inputs += utxo.value;
            inputs.push(tx_in);
            if value_inputs >= target_amount {
                break;
            }
        }
        if value_inputs < target_amount {
            anyhow::bail!("Not Enough Funds");
        }
        let change = value_inputs - target_amount;

        if change > 0 {
            let tx_out = Fill {
                id_order: None,
                destination: Destination::TransparentAddress(account_address.clone()),
                amount: change,
                memo: MemoBytes::empty(),
            };
            outputs.push(tx_out);
        }

        let size = (inputs.len() * 180 + outputs.len() * 34) as u64;
        fee = size * fee_rate;

        let tx2 = TransactionPlan {
            account_address: account_address.clone(),
            anchor_height: 0,
            expiry_height: 0,
            orchard_anchor: [0u8; 32],
            spends: inputs,
            outputs,
            fee,
            net_chg: [0, 0],
        };
        tx = Some(tx2);
    }

    let tx = tx.unwrap();
    Ok(tx)
}

pub fn to_report(tx: &TransactionPlan) -> Result<TxReportT> {
    let mut value = 0;
    for i in tx.spends.iter() {
        value += i.amount;
    }
    let mut outputs = vec![];
    for o in tx.outputs.iter() {
        if o.id_order.is_none() {
            continue;
        }
        if let Destination::TransparentAddress(ref address) = o.destination {
            let output = TxOutputT {
                id: o.id_order.unwrap(),
                address: Some(address.clone()),
                amount: o.amount,
                pool: 0,
            };
            outputs.push(output);
        }
    }
    let report = TxReportT {
        outputs: Some(outputs),
        transparent: value,
        sapling: 0,
        orchard: 0,
        net_sapling: 0,
        net_orchard: 0,
        fee: tx.fee,
        privacy_level: 0,
    };
    Ok(report)
}
