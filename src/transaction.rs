use crate::{CompactTxStreamerClient, TxFilter, DbAdapter, NETWORK, connect_lightwalletd};
use tonic::transport::Channel;
use tonic::Request;
use zcash_primitives::transaction::Transaction;
use zcash_primitives::sapling::note_encryption::{try_sapling_note_decryption, try_sapling_output_recovery};
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_client_backend::encoding::{decode_extended_full_viewing_key, encode_payment_address};
use zcash_primitives::memo::{MemoBytes, Memo};
use std::convert::TryFrom;
use std::collections::HashMap;

#[derive(Debug)]
pub struct TransactionInfo {
    pub address: String,
    pub memo: Memo,
    amount: i64,
    pub fee: u64,
}

pub async fn decode_transaction(client: &mut CompactTxStreamerClient<Channel>,
                                nfs: &HashMap<Vec<u8>, u64>,
                                fvk: &str,
                                tx_hash: &[u8],
                                height: u32) -> anyhow::Result<TransactionInfo> {
    let fvk = decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk)?.unwrap();
    let ivk = fvk.fvk.vk.ivk();
    let ovk = fvk.fvk.ovk;

    let tx_filter = TxFilter {
        block: None,
        index: 0,
        hash: tx_hash.to_vec(), // only hash is supported
    };
    let raw_tx = client.get_transaction(Request::new(tx_filter)).await?.into_inner();
    let tx = Transaction::read(&*raw_tx.data)?;

    let height = BlockHeight::from_u32(height);
    let mut amount = 0i64;
    let mut address = String::new();
    for spend in tx.shielded_spends.iter() {
        let nf = spend.nullifier.to_vec();
        if let Some(&v) = nfs.get(&nf) {
            amount -= v as i64;
        }
    }

    let mut tx_memo = MemoBytes::empty();
    for output in tx.shielded_outputs.iter() {
        if let Some((note, pa, memo)) = try_sapling_note_decryption(&NETWORK, height, &ivk, output) {
            amount += note.value as i64; // change or self transfer
            if address.is_empty() {
                address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
                tx_memo = memo;
            }
        }
        else if let Some((_note, pa, memo)) = try_sapling_output_recovery(&NETWORK, height, &ovk, &output) {
            address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
            tx_memo = memo;
        }
    }

    let fee = u64::from(tx.value_balance);

    let tx_info = TransactionInfo {
        address,
        memo: Memo::try_from(tx_memo)?,
        amount,
        fee
    };

    Ok(tx_info)
}

pub async fn retrieve_tx_info(tx_ids: &[u32], ld_url: &str, db_path: &str) -> anyhow::Result<()> {
    let mut client = connect_lightwalletd(ld_url).await?;
    let db = DbAdapter::new(db_path)?;
    for &id_tx in tx_ids.iter() {
        let (account, height, tx_hash) = db.get_txhash(id_tx)?;
        let nfs = db.get_nullifier_amounts(account, false)?;
        let fvk = db.get_ivk(account)?;
        let tx_info = decode_transaction(&mut client, &nfs, &fvk, &tx_hash, height).await?;
        db.store_tx_metadata(id_tx, &tx_info)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{connect_lightwalletd, LWD_URL, DbAdapter};
    use crate::transaction::decode_transaction;

    #[tokio::test]
    async fn test_decode_transaction() {
        let tx_hash = hex::decode("b47da170329dc311b98892eac23e83025f8bb3ce10bb07535698c91fb37e1e54").unwrap();
        let mut client = connect_lightwalletd(LWD_URL).await.unwrap();
        let db = DbAdapter::new("./zec.db").unwrap();
        let account = 1;
        let nfs = db.get_nullifier_amounts(account, false).unwrap();
        let fvk = db.get_ivk(account).unwrap();
        let tx_info = decode_transaction(&mut client, &nfs, &fvk, &tx_hash, 1313212).await.unwrap();
        println!("{:?}", tx_info);
    }
}
