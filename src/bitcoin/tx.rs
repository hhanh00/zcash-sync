use anyhow::Result;
use base58check::FromBase58Check;
use bitcoin::{
    hashes::Hash, psbt::Psbt, LockTime, OutPoint, PubkeyHash, Script, Sequence, Transaction, TxIn,
    TxOut, Witness,
};
use electrum_client::ElectrumApi;

use crate::{
    bitcoin::{get_client, get_script},
    db::data_generated::fb::RecipientT,
};

pub fn prepare_tx(
    coin: u8,
    id_account: u32,
    recipients: &[RecipientT],
    num_blocks: u32,
    url: &str,
) -> Result<String> {
    let mut amount = 0;
    let client = get_client(url)?;
    let fee_rate = (client.estimate_fee(num_blocks as usize)? * 100_000f64) as u64;
    let mut fee = 0;

    let script = get_script(coin, id_account)?;
    let utxos = client.script_list_unspent(&script)?;

    let mut psbt = None;

    for _ in 0..2 {
        let target_amount = amount + fee;
        let mut input = vec![];
        let mut value_inputs = 0;
        for utxo in utxos.iter() {
            let tx_in = TxIn {
                previous_output: OutPoint {
                    txid: utxo.tx_hash,
                    vout: utxo.tx_pos as u32,
                },
                script_sig: Script::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            };
            value_inputs += utxo.value;
            input.push(tx_in);
            if value_inputs >= target_amount {
                break;
            }
        }
        if value_inputs < target_amount {
            anyhow::bail!("Not Enough Funds");
        }
        let change = value_inputs - target_amount;

        let mut output = vec![];
        for recipient in recipients.iter() {
            amount += recipient.amount;
            let address = recipient.address.as_deref().unwrap();
            let (_version, hash) = address
                .from_base58check()
                .map_err(|_| anyhow::anyhow!("Invalid address"))?;
            let pkh = PubkeyHash::from_slice(&hash)?;
            let tx_out = TxOut {
                value: recipient.amount,
                script_pubkey: Script::new_p2pkh(&pkh),
            };
            output.push(tx_out);
        }
        if change > 0 {
            let tx_out = TxOut {
                value: change,
                script_pubkey: script.clone(),
            };
            output.push(tx_out);
        }

        let tx = Transaction {
            version: 2,
            lock_time: LockTime::ZERO.into(),
            input,
            output,
        };

        let psbt2 = Psbt::from_unsigned_tx(tx).unwrap();
        fee = psbt2.unsigned_tx.vsize() as u64 * fee_rate;
        psbt = Some(psbt2);
    }

    let psbt = psbt.unwrap();
    let json = serde_json::to_string(&psbt)?;
    Ok(json)
}
