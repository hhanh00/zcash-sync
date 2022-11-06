use std::collections::HashMap;
use crate::contact::{Contact, ContactDecoder};
use crate::{AccountData, CoinConfig, CompactTxStreamerClient, DbAdapter, Hash, TxFilter};
use std::convert::TryFrom;
use serde::Serialize;
use orchard::keys::{FullViewingKey, IncomingViewingKey, OutgoingViewingKey, Scope};
use orchard::note_encryption::OrchardDomain;
use orchard::value::ValueCommitment;
use tonic::transport::Channel;
use tonic::Request;
use zcash_address::{ToAddress, ZcashAddress};
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_payment_address, encode_transparent_address,
};
use zcash_note_encryption::{try_note_decryption, try_output_recovery_with_ovk};
use zcash_params::coin::get_branch;
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::memo::{Memo, MemoBytes};
use zcash_primitives::sapling::note_encryption::{PreparedIncomingViewingKey, try_sapling_note_decryption, try_sapling_output_recovery};
use zcash_primitives::sapling::SaplingIvk;
use zcash_primitives::transaction::Transaction;
use crate::unified::orchard_as_unified;

#[derive(Debug)]
pub struct ContactRef {
    pub height: u32,
    pub index: u32,
    pub contact: Contact,
}

pub async fn get_transaction_details(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let mut client = c.connect_lwd().await?;
    let mut keys = HashMap::new();

    let reqs = {
        let db = c.db.as_ref().unwrap();
        let db = db.lock().unwrap();
        let reqs = db.get_txid_without_memo()?;
        for req in reqs.iter() {
            let decryption_keys = get_decryption_keys(network, req.account, &db)?;
            keys.insert(req.account, decryption_keys);
        }
        reqs
        // Make sure we don't hold a mutex across await
    };

    let mut details = vec![];
    for req in reqs.iter() {
        let tx_details = retrieve_tx_info(network, &mut client, req.height, req.id_tx, &req.txid, &keys[&req.account]).await?;
        log::info!("{:?}", tx_details);
        details.push(tx_details);
    }

    let db = c.db.as_ref().unwrap();
    let db = db.lock().unwrap();
    for tx_details in details.iter() {
        db.update_transaction_with_memo(tx_details)?;
        for c in tx_details.contacts.iter() {
            db.store_contact(c, false)?;
        }
    }

    Ok(())
}

async fn fetch_raw_transaction(network: &Network, client: &mut CompactTxStreamerClient<Channel>, height: u32, txid: &Hash) -> anyhow::Result<Transaction> {
    let consensus_branch_id = get_branch(network, height);
    let tx_filter = TxFilter {
        block: None,
        index: 0,
        hash: txid.to_vec(), // only hash is supported
    };
    let raw_tx = client
        .get_transaction(Request::new(tx_filter))
        .await?
        .into_inner();
    let tx = Transaction::read(&*raw_tx.data, consensus_branch_id)?;
    Ok(tx)
}

#[derive(Clone)]
pub struct DecryptionKeys {
    sapling_keys: (SaplingIvk, zcash_primitives::keys::OutgoingViewingKey),
    orchard_keys: Option<(IncomingViewingKey, OutgoingViewingKey)>
}

pub fn decode_transaction(
    network: &Network,
    height: u32,
    id_tx: u32,
    tx: Transaction,
    decryption_keys: &DecryptionKeys,
) -> anyhow::Result<TransactionDetails> {
    let (sapling_ivk, sapling_ovk) = decryption_keys.sapling_keys.clone();

    let height = BlockHeight::from_u32(height);
    let mut taddress: Option<String> = None;
    let mut zaddress: Option<String> = None;
    let mut oaddress: Option<String> = None;

    let tx = tx.into_data();
    // log::info!("{:?}", tx);

    let mut tx_memo: Memo = Memo::Empty;
    let mut contacts = vec![];

    if let Some(transparent_bundle) = tx.transparent_bundle() {
        for output in transparent_bundle.vout.iter() {
            if let Some(taddr) = output.recipient_address() {
                taddress = Some(encode_transparent_address(
                    &network.b58_pubkey_address_prefix(),
                    &network.b58_script_address_prefix(),
                    &taddr,
                ));
            }
        }
    }

    if let Some(sapling_bundle) = tx.sapling_bundle() {
        let mut contact_decoder = ContactDecoder::new(sapling_bundle.shielded_outputs.len());
        for output in sapling_bundle.shielded_outputs.iter() {
            let pivk = PreparedIncomingViewingKey::new(&sapling_ivk);
            if let Some((_note, pa, memo)) = try_sapling_note_decryption(network, height, &pivk, output) {
                let memo = Memo::try_from(memo)?;
                if zaddress.is_none() {
                    zaddress = Some(encode_payment_address(network.hrp_sapling_payment_address(), &pa));
                }
                if memo != Memo::Empty {
                    tx_memo = memo;
                }
            }
            if let Some((_note, pa, memo, ..)) = try_sapling_output_recovery(network, height, &sapling_ovk, output) {
                let _ = contact_decoder.add_memo(&memo); // ignore memo that is not for contacts, if we cannot decode it with ovk, we didn't make create this memo
                zaddress = Some(encode_payment_address(network.hrp_sapling_payment_address(), &pa));
                let memo = Memo::try_from(memo)?;
                if memo != Memo::Empty {
                    tx_memo = memo;
                }
            }
        }
        contacts = contact_decoder.finalize()?;
    }

    if let Some(orchard_bundle) = tx.orchard_bundle() {
        if let Some((orchard_ivk, orchard_ovk)) = decryption_keys.orchard_keys.clone() {
            for action in orchard_bundle.actions().iter() {
                let domain = OrchardDomain::for_action(action);
                if let Some((_note, pa, memo)) = try_note_decryption(&domain, &orchard_ivk, action) {
                    let memo = Memo::try_from(MemoBytes::from_bytes(&memo)?)?;
                    if oaddress.is_none() {
                        oaddress = Some(orchard_as_unified(network, &pa).encode());
                    }
                    if memo != Memo::Empty {
                        tx_memo = memo;
                    }
                }
                if let Some((_note, pa, memo, ..)) = try_output_recovery_with_ovk(&domain, &orchard_ovk, action,
                                                                                  action.cv_net(), &action.encrypted_note().out_ciphertext) {
                    let memo = Memo::try_from(MemoBytes::from_bytes(&memo)?)?;
                    oaddress = Some(orchard_as_unified(network, &pa).encode());
                    if memo != Memo::Empty {
                        tx_memo = memo;
                    }
                }
            }
        }
    }

    let address = zaddress.or(oaddress).or(taddress).unwrap_or(String::new());

    let memo = match tx_memo {
        Memo::Empty => "".to_string(),
        Memo::Text(text) => text.to_string(),
        Memo::Future(_) => "Unrecognized".to_string(),
        Memo::Arbitrary(_) => "Unrecognized".to_string(),
    };
    let tx_details = TransactionDetails {
        id_tx,
        address,
        memo,
        contacts,
    };

    Ok(tx_details)
}

fn get_decryption_keys(network: &Network, account: u32, db: &DbAdapter) -> anyhow::Result<DecryptionKeys> {
    let AccountData { fvk, .. } = db.get_account_info(account)?;
    let fvk = decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk).unwrap();
    let (sapling_ivk, sapling_ovk) = (fvk.fvk.vk.ivk(), fvk.fvk.ovk);

    let okey = db.get_orchard(account)?;
    let okey = okey.map(|okey| {
        let fvk = FullViewingKey::from_bytes(&okey.fvk).unwrap();
        (fvk.to_ivk(Scope::External), fvk.to_ovk(Scope::External))
    });
    let decryption_keys = DecryptionKeys {
        sapling_keys: (sapling_ivk, sapling_ovk),
        orchard_keys: okey,
    };
    Ok(decryption_keys)
}

pub async fn retrieve_tx_info(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
    id_tx: u32,
    txid: &Hash,
    decryption_keys: &DecryptionKeys
) -> anyhow::Result<TransactionDetails> {
    let transaction = fetch_raw_transaction(network, client, height, txid).await?;
    let tx_details = decode_transaction(network, height, id_tx, transaction, &decryption_keys)?;

    Ok(tx_details)
}

pub struct GetTransactionDetailRequest {
    pub account: u32,
    pub height: u32,
    pub id_tx: u32,
    pub txid: Hash,
}

#[derive(Serialize, Debug)]
pub struct TransactionDetails {
    pub id_tx: u32,
    pub address: String,
    pub memo: String,
    pub contacts: Vec<Contact>,
}

#[tokio::test]
async fn test_get_transaction_details() {
    crate::init_test();

    get_transaction_details(0).await.unwrap();
}
