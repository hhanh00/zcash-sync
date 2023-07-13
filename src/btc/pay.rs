use crate::btc::{db, BTCNET};
use crate::db::data_generated::fb::{
    BTCInputT, BTCOutputT, BTCTx, BTCTxT, RecipientsT, TxOutputT, TxReportT,
};
use anyhow::{anyhow, Result};
use electrum_client::bitcoin::absolute::LockTime;
use electrum_client::bitcoin::consensus::encode::serialize;
use electrum_client::bitcoin::ecdsa::Signature;
use electrum_client::bitcoin::hashes::Hash;
use electrum_client::bitcoin::psbt::PartiallySignedTransaction;
use electrum_client::bitcoin::secp256k1::{All, Message, Secp256k1};
use electrum_client::bitcoin::sighash::{EcdsaSighashType, SighashCache};
use electrum_client::bitcoin::{
    Address, OutPoint, PublicKey, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness,
};
use flatbuffers::FlatBufferBuilder;
use rusqlite::Connection;
use std::str::FromStr;

pub fn sign(connection: &Connection, account: u32, tx_plan: &str) -> Result<Vec<u8>> {
    let sk = db::get_sk(connection, account)?.ok_or(anyhow!("No secret key"))?;
    let my_address = db::get_address(connection, account)?;
    let my_address = Address::from_str(&my_address)?.require_network(BTCNET)?;
    let spk = my_address.script_pubkey();

    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<BTCTx>(&tx_plan)?;
    let tx = root.unpack();
    println!("{tx:?}");

    let txins = tx.txins.unwrap();
    let input: Vec<_> = txins
        .iter()
        .map(|txin| TxIn {
            previous_output: OutPoint {
                txid: Txid::from_slice(&hex::decode(txin.tx_id.as_ref().unwrap()).unwrap())
                    .unwrap(),
                vout: txin.vout,
            },
            ..TxIn::default()
        })
        .collect();
    let txouts = tx.txouts.unwrap();
    let output: Vec<_> = txouts
        .iter()
        .map(|txout| TxOut {
            value: txout.value,
            script_pubkey: ScriptBuf::from_bytes(
                hex::decode(txout.script_pubkey.as_ref().unwrap()).unwrap(),
            ),
        })
        .collect();

    let tx = Transaction {
        version: 0,
        lock_time: LockTime::ZERO,
        input,
        output,
    };
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(tx)?;
    let secp = Secp256k1::<All>::new();
    let pk = sk.public_key(&secp);
    let pk = PublicKey::new(pk);
    let mut sighash_cache = SighashCache::new(&psbt.unsigned_tx);
    for ((index, vin), txin) in psbt.inputs.iter_mut().enumerate().zip(txins.iter()) {
        let script_code = spk.p2wpkh_script_code().unwrap();
        let sighash = sighash_cache.segwit_signature_hash(
            index,
            &script_code,
            txin.value,
            EcdsaSighashType::All,
        )?;
        let message = Message::from(sighash);
        let sig = secp.sign_ecdsa(&message, &sk);
        let sig = Signature::sighash_all(sig);
        let mut script_witness = Witness::new();
        script_witness.push(sig.to_vec());
        script_witness.push(pk.to_bytes());
        vin.final_script_witness = Some(script_witness);
    }
    let tx = psbt.extract_tx();
    let txb = serialize(&tx);

    Ok(txb)
}

pub fn prepare(
    connection: &Connection,
    account: u32,
    recipients: &RecipientsT,
    feeb: u64,
) -> Result<String> {
    let my_address = db::get_address(connection, account)?;
    let my_address = Address::from_str(&my_address)?.require_network(BTCNET)?;

    let recipients = recipients.values.as_ref().ok_or(anyhow!("No recipients"))?;
    let mut total_outs = 0u64;
    let mut tx_outs = vec![];
    let mut tx_out_fee = 0; // tx_out that can be used to pay for the fees
    let mut tx_out_fee_index = None;
    for (i, r) in recipients.iter().enumerate() {
        let address = r.address.as_ref().ok_or(anyhow!("Missing Address"))?;
        let address = Address::from_str(address)?.require_network(BTCNET)?;
        let value = r.amount;
        total_outs += value;
        let tx_out = BTCOutputT {
            value,
            script_pubkey: Some(hex::encode(&address.script_pubkey())),
        };
        if r.fee_included {
            tx_out_fee = tx_out.value;
            tx_out_fee_index = Some(i);
        }
        tx_outs.push(tx_out);
    }
    let vsize = 11 + 31 * (tx_outs.len() + 1); // reserve +1 output for change
    let mut fee = 0;

    // Take fee from output if allowed, return the remaining fee
    let mut add_fee = |fee| -> u64 {
        match tx_out_fee_index {
            Some(_) => {
                let f = tx_out_fee.min(fee);
                tx_out_fee -= f;
                fee - f
            }
            None => fee,
        }
    };

    fee += add_fee((vsize as u64) * feeb); // add base fee

    let mut total_ins = 0u64;
    let utxos = db::get_utxos(connection, account)?;
    let utxos = utxos.notes.as_ref().unwrap();
    let mut tx_ins = vec![];
    for utxo in utxos {
        let tx_in = BTCInputT {
            tx_id: utxo.tx_id.clone(),
            vout: utxo.vout,
            value: utxo.value,
        };
        tx_ins.push(tx_in);
        fee += add_fee(68 * feeb); // price per input
        total_ins += utxo.value;
        if total_ins >= total_outs + fee {
            break;
        }
    }
    if total_ins < total_outs + fee {
        anyhow::bail!("Not Enough Funds");
    }

    let change = total_ins - total_outs - fee;
    if change > 0 {
        tx_outs.push(BTCOutputT {
            value: change,
            script_pubkey: Some(hex::encode(&my_address.script_pubkey())),
        });
    }
    if let Some(tx_out_fee_index) = tx_out_fee_index {
        fee += tx_outs[tx_out_fee_index].value - tx_out_fee;
        tx_outs[tx_out_fee_index].value = tx_out_fee;
    }
    let tx = BTCTxT {
        txins: Some(tx_ins),
        txouts: Some(tx_outs),
        fee,
    };
    let mut builder = FlatBufferBuilder::new();
    let root = tx.pack(&mut builder);
    builder.finish(root, None);
    let tx_data = base64::encode(builder.finished_data());
    Ok(tx_data)
}

pub fn to_tx_report(tx_plan: &str) -> Result<TxReportT> {
    let tx_plan = base64::decode(tx_plan)?;
    let root = flatbuffers::root::<BTCTx>(&tx_plan)?;
    let tx = root.unpack();
    let mut transparent = tx.fee;
    let outputs: Vec<_> = tx
        .txouts
        .unwrap()
        .iter()
        .map(|o| {
            let script_pubkey = hex::decode(&o.script_pubkey.as_ref().unwrap()).unwrap();
            let script_pubkey = ScriptBuf::from_bytes(script_pubkey);
            let address = Address::from_script(&script_pubkey, BTCNET).unwrap();
            transparent += o.value;
            TxOutputT {
                address: Some(address.to_string()),
                amount: o.value,
                ..TxOutputT::default()
            }
        })
        .collect();
    Ok(TxReportT {
        outputs: Some(outputs),
        transparent,
        fee: tx.fee,
        ..TxReportT::default()
    })
}
