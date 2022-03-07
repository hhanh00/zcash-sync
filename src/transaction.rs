use crate::contact::{Contact, ContactDecoder};
use crate::{CompactTxStreamerClient, DbAdapter, TxFilter};
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::mpsc;
use std::sync::mpsc::SyncSender;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_payment_address, encode_transparent_address,
};
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::memo::Memo;
use zcash_primitives::sapling::note_encryption::{
    try_sapling_note_decryption, try_sapling_output_recovery,
};
use zcash_primitives::transaction::Transaction;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_params::coin::{CoinType, get_coin_chain};

#[derive(Debug)]
pub struct TransactionInfo {
    height: u32,
    index: u32, // index of tx in block
    id_tx: u32, // id of tx in db
    pub account: u32,
    pub address: String,
    pub memo: String,
    pub amount: i64,
    pub fee: u64,
    pub contacts: Vec<Contact>,
}

#[derive(Debug)]
pub struct ContactRef {
    pub height: u32,
    pub index: u32,
    pub contact: Contact,
}

pub async fn decode_transaction(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    nfs: &HashMap<(u32, Vec<u8>), u64>,
    id_tx: u32,
    account: u32,
    fvk: &ExtendedFullViewingKey,
    tx_hash: &[u8],
    height: u32,
    index: u32,
) -> anyhow::Result<TransactionInfo> {
    let ivk = fvk.fvk.vk.ivk();
    let ovk = fvk.fvk.ovk;

    let tx_filter = TxFilter {
        block: None,
        index: 0,
        hash: tx_hash.to_vec(), // only hash is supported
    };
    let raw_tx = client
        .get_transaction(Request::new(tx_filter))
        .await?
        .into_inner();
    let tx = Transaction::read(&*raw_tx.data)?;

    let height = BlockHeight::from_u32(height);
    let mut amount = 0i64;
    let mut address = String::new();
    for spend in tx.shielded_spends.iter() {
        let nf = spend.nullifier.to_vec();
        if let Some(&v) = nfs.get(&(account, nf)) {
            amount -= v as i64;
        }
    }

    let mut contact_decoder = ContactDecoder::new(tx.shielded_outputs.len());

    let mut tx_memo: Memo = Memo::Empty;

    for output in tx.vout.iter() {
        if let Some(taddr) = output.script_pubkey.address() {
            address = encode_transparent_address(
                &network.b58_pubkey_address_prefix(),
                &network.b58_script_address_prefix(),
                &taddr,
            );
        }
    }

    for output in tx.shielded_outputs.iter() {
        if let Some((note, pa, memo)) = try_sapling_note_decryption(network, height, &ivk, output)
        {
            amount += note.value as i64; // change or self transfer
            let _ = contact_decoder.add_memo(&memo); // ignore memo that is not for contacts
            let memo = Memo::try_from(memo)?;
            if address.is_empty() {
                address = encode_payment_address(network.hrp_sapling_payment_address(), &pa);
            }
            if memo != Memo::Empty {
                tx_memo = memo;
            }
        } else if let Some((_note, pa, memo)) =
            try_sapling_output_recovery(network, height, &ovk, &output)
        {
            address = encode_payment_address(network.hrp_sapling_payment_address(), &pa);
            let memo = Memo::try_from(memo)?;
            if memo != Memo::Empty {
                tx_memo = memo;
            }
        }
    }

    let fee = u64::from(tx.value_balance);

    let memo = match tx_memo {
        Memo::Empty => "".to_string(),
        Memo::Text(text) => text.to_string(),
        Memo::Future(_) => "Unrecognized".to_string(),
        Memo::Arbitrary(_) => "Unrecognized".to_string(),
    };
    let contacts = contact_decoder.finalize()?;
    let tx_info = TransactionInfo {
        height: u32::from(height),
        index,
        id_tx,
        account,
        address,
        memo,
        amount,
        fee,
        contacts,
    };

    Ok(tx_info)
}

struct DecodeTxParams<'a> {
    tx: SyncSender<TransactionInfo>,
    client: CompactTxStreamerClient<Channel>,
    nf_map: &'a HashMap<(u32, Vec<u8>), u64>,
    index: u32,
    id_tx: u32,
    account: u32,
    fvk: ExtendedFullViewingKey,
    tx_hash: Vec<u8>,
    height: u32,
}

pub async fn retrieve_tx_info(
    coin_type: CoinType,
    client: &mut CompactTxStreamerClient<Channel>,
    db_path: &str,
    tx_ids: &[u32],
) -> anyhow::Result<()> {
    let network = {
        let chain = get_coin_chain(coin_type);
        chain.network().clone()
    };
    let db = DbAdapter::new(coin_type, db_path)?;

    let nfs = db.get_nullifiers_raw()?;
    let mut nf_map: HashMap<(u32, Vec<u8>), u64> = HashMap::new();
    for nf in nfs.iter() {
        nf_map.insert((nf.0, nf.2.clone()), nf.1);
    }
    let mut fvk_cache: HashMap<u32, ExtendedFullViewingKey> = HashMap::new();
    let mut decode_tx_params: Vec<DecodeTxParams> = vec![];
    let (tx, rx) = mpsc::sync_channel::<TransactionInfo>(4);
    for (index, &id_tx) in tx_ids.iter().enumerate() {
        let (account, height, tx_hash, ivk) = db.get_txhash(id_tx)?;
        let fvk: &ExtendedFullViewingKey = fvk_cache.entry(account).or_insert_with(|| {
            decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &ivk)
                .unwrap()
                .unwrap()
        });
        let params = DecodeTxParams {
            tx: tx.clone(),
            client: client.clone(),
            nf_map: &nf_map,
            index: index as u32,
            id_tx,
            account,
            fvk: fvk.clone(),
            tx_hash: tx_hash.clone(),
            height,
        };
        decode_tx_params.push(params);
    }

    let res = tokio_stream::iter(decode_tx_params).for_each_concurrent(None, |mut p| async move {
        if let Ok(tx_info) = decode_transaction(
            &network,
            &mut p.client,
            p.nf_map,
            p.id_tx,
            p.account,
            &p.fvk,
            &p.tx_hash,
            p.height,
            p.index,
        )
        .await
        {
            p.tx.send(tx_info).unwrap();
            drop(p.tx);
        }
    });

    let f = tokio::spawn(async move {
        let mut contacts: Vec<ContactRef> = vec![];
        while let Ok(tx_info) = rx.recv() {
            for c in tx_info.contacts.iter() {
                contacts.push(ContactRef {
                    height: tx_info.height,
                    index: tx_info.index,
                    contact: c.clone(),
                });
            }
            db.store_tx_metadata(tx_info.id_tx, &tx_info)?;
        }
        contacts.sort_by(|a, b| a.index.cmp(&b.index));
        for cref in contacts.iter() {
            db.store_contact(&cref.contact, false)?;
        }

        Ok::<_, anyhow::Error>(())
    });

    res.await;
    drop(tx);
    f.await??;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::transaction::decode_transaction;
    use crate::{connect_lightwalletd, DbAdapter, LWD_URL};
    use std::collections::HashMap;
    use zcash_client_backend::encoding::decode_extended_full_viewing_key;
    use zcash_primitives::consensus::{Network, Parameters};
    use zcash_params::coin::CoinType;

    #[tokio::test]
    async fn test_decode_transaction() {
        let tx_hash =
            hex::decode("b47da170329dc311b98892eac23e83025f8bb3ce10bb07535698c91fb37e1e54")
                .unwrap();
        let mut client = connect_lightwalletd(LWD_URL).await.unwrap();
        let db = DbAdapter::new(CoinType::Zcash, "./zec.db").unwrap();
        let account = 1;
        let nfs = db.get_nullifiers_raw().unwrap();
        let mut nf_map: HashMap<(u32, Vec<u8>), u64> = HashMap::new();
        for nf in nfs.iter() {
            if nf.0 == account {
                nf_map.insert((nf.0, nf.2.clone()), nf.1);
            }
        }
        let fvk = db.get_ivk(account).unwrap();
        let fvk =
            decode_extended_full_viewing_key(Network::MainNetwork.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap()
                .unwrap();
        let tx_info =
            decode_transaction(&Network::MainNetwork, &mut client, &nf_map, 1, account, &fvk, &tx_hash, 1313212, 1)
                .await
                .unwrap();
        println!("{:?}", tx_info);
    }
}
