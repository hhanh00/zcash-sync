use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use bitcoin::{
    blockdata::script,
    consensus::encode,
    hashes::Hash,
    psbt::{serialize::Serialize, Input, PartiallySignedTransaction},
    secp256k1::{All, Message, Secp256k1},
    util::sighash::SighashCache,
    Address, EcdsaSig, EcdsaSighashType, OutPoint, PackedLockTime, Script, Sequence, Transaction,
    TxIn, TxOut, Txid, Witness,
};
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

use super::{get_address, get_backup, util::parse_seckey};

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

        let size = (inputs.len() * 148 + outputs.len() * 34 + 10) as u64;
        println!("fee/b {} vsize {}", fee_rate, size);
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

pub fn sign_plan(coin: u8, id_account: u32, tx: &TransactionPlan) -> Result<Vec<u8>> {
    let backup = with_coin(coin, |c| get_backup(c, id_account))?;
    let sk = backup.tsk.unwrap();
    let sk = parse_seckey(&sk)?;

    let mut inputs = vec![];
    for s in tx.spends.iter() {
        if let Source::Transparent { txid, index } = s.source {
            let txid = Txid::from_inner(txid.clone());
            let txin = TxIn {
                previous_output: OutPoint { txid, vout: index },
                script_sig: Script::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            };
            inputs.push(txin);
        }
    }

    let mut outputs = vec![];
    for o in tx.outputs.iter() {
        if let Destination::TransparentAddress(ref address) = o.destination {
            let address = address.parse::<Address>()?;
            let script = address.script_pubkey();
            let txout = TxOut {
                value: o.amount,
                script_pubkey: script,
            };
            outputs.push(txout);
        }
    }

    let transaction = Transaction {
        version: 2,
        lock_time: PackedLockTime::ZERO,
        input: inputs,
        output: outputs,
    };
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(transaction)?;

    let account_address = tx.account_address.parse::<Address>()?;
    let prev_output = account_address.script_pubkey();

    let mut inputs = vec![];
    let mut prev_outpoints: HashMap<OutPoint, TxOut> = HashMap::new();
    for s in tx.spends.iter() {
        if let Source::Transparent { txid, index } = s.source {
            let txid = Txid::from_inner(txid.clone());
            let op = OutPoint { txid, vout: index };

            let witness_utxo = TxOut {
                value: s.amount,
                script_pubkey: prev_output.clone(),
            };
            prev_outpoints.insert(op, witness_utxo.clone());
            let input = Input {
                witness_utxo: Some(witness_utxo),
                ..Default::default()
            };
            inputs.push(input);
        }
    }
    psbt.inputs = inputs;
    assert_eq!(psbt.inputs.len(), psbt.unsigned_tx.input.len());

    let secp = Secp256k1::<All>::new();

    let cache = SighashCache::new(&psbt.unsigned_tx);
    let sighash_all = EcdsaSighashType::All;
    let sighash_type = sighash_all.to_u32();
    for (i, input) in psbt.inputs.iter_mut().enumerate() {
        let spend_utxo = input.witness_utxo.clone().unwrap();
        let sighash = cache.legacy_signature_hash(i, &spend_utxo.script_pubkey, sighash_type)?;
        let message = Message::from_slice(&sighash)?;
        let signature = secp.sign_ecdsa(&message, &sk);
        let mut final_signature = Vec::with_capacity(75);
        final_signature.extend_from_slice(&signature.serialize_der());
        final_signature.push(sighash_type as u8);

        let pk = bitcoin::PublicKey::new(sk.public_key(&secp));

        let signature = EcdsaSig::from_slice(&final_signature)?;
        let sig_script = script::Builder::new()
            .push_slice(&signature.serialize())
            .push_slice(&pk.inner.serialize());
        input.final_script_sig = Some(sig_script.into_script());

        input.partial_sigs = BTreeMap::new();
        input.sighash_type = None;
        input.redeem_script = None;
        input.witness_script = None;
        input.bip32_derivation = BTreeMap::new();
    }

    let tx = psbt.extract_tx();

    let tx_bytes = encode::serialize(&tx);

    Ok(tx_bytes)
}

pub fn broadcast(tx: &[u8], url: &str) -> Result<String> {
    let client = get_client(url)?;
    let txid = client.transaction_broadcast_raw(tx)?;
    // let txid = Txid::all_zeros();
    Ok(txid.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{TransactionPlan, COIN_CONFIG};

    use super::sign_plan;

    pub const TX_PLAN: &str = r#"{"account_address":"1PxJA1euG6pqNUQ3H1GAqnror2Uq5HMxBC","anchor_height":0,"expiry_height":0,"orchard_anchor":"0000000000000000000000000000000000000000000000000000000000000000","spends":[{"id":0,"source":{"Transparent":{"txid":"8f368323623235820a63d7c2c0c9e7304e56a032108b99567f33835e6bb5c134","index":0}},"amount":10000}],"outputs":[{"id_order":0,"destination":{"TransparentAddress":"1PxJA1euG6pqNUQ3H1GAqnror2Uq5HMxBC"},"amount":4500,"memo":"f6"},{"id_order":null,"destination":{"TransparentAddress":"1PxJA1euG6pqNUQ3H1GAqnror2Uq5HMxBC"},"amount":2276,"memo":"f6"}],"fee":3224,"net_chg":[0,0]}"#;

    #[test]
    fn test() {
        {
            let mut c = COIN_CONFIG[2].lock().unwrap();
            c.set_db_path(
                "/Users/hanhhuynhhuu/Library/Containers/me.hanh.ywallet/Data/databases/btc.db",
            )
            .unwrap();
            c.open_db().unwrap();
        }

        let tx: TransactionPlan = serde_json::from_str(TX_PLAN).unwrap();
        sign_plan(2, 1, &tx).unwrap();
    }
}
