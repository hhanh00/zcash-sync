use crate::api::recipient::decode_memo;
use crate::contact::{Contact, ContactDecoder};
use crate::unified::orchard_as_unified;
use crate::{CompactTxStreamerClient, connect_lightwalletd, Hash, TxFilter};
use orchard::keys::{FullViewingKey, IncomingViewingKey, OutgoingViewingKey, Scope};
use orchard::note_encryption::OrchardDomain;
use serde::Serialize;
use std::collections::HashMap;
use std::convert::TryFrom;
use anyhow::anyhow;
use rusqlite::Connection;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_payment_address, encode_transparent_address,
};
use zcash_note_encryption::{try_note_decryption, try_output_recovery_with_ovk};
use zcash_params::coin::get_branch;
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::memo::{Memo, MemoBytes};
use zcash_primitives::sapling::note_encryption::{
    try_sapling_note_decryption, try_sapling_output_recovery, PreparedIncomingViewingKey,
};
use zcash_primitives::sapling::SaplingIvk;
use zcash_primitives::transaction::Transaction;
use crate::db::data_generated::fb::ContactT;

#[derive(Debug)]
pub struct ContactRef {
    pub height: u32,
    pub index: u32,
    pub contact: Contact,
}

pub async fn get_transaction_details(network: &Network, connection: &Connection, url: &str) -> anyhow::Result<()> {
    let mut client = connect_lightwalletd(url).await?;
    let mut keys = HashMap::new();

    let reqs = {
        let reqs = crate::db::transaction::list_txid_without_memo(connection)?;
        for req in reqs.iter() {
            if !keys.contains_key(&req.account) {
                let decryption_keys = get_decryption_keys(network, connection, req.account)?;
                keys.insert(req.account, decryption_keys);
            }
        }
        reqs
        // Make sure we don't hold a mutex across await
    };

    let mut details = vec![];
    for req in reqs.iter() {
        let tx_details = retrieve_tx_info(network, &mut client, req, &keys[&req.account]).await?;
        log::info!("{:?}", tx_details);
        details.push(tx_details);
    }

    for tx_details in details.iter() {
        crate::db::transaction::update_transaction_with_memo(connection, tx_details)?;
        for c in tx_details.contacts.iter() {
            crate::db::contact::store_contact(connection, c, false)?;
        }
        let z_msg = decode_memo(
            tx_details.id_tx,
            &tx_details.memo,
            &tx_details.address,
            tx_details.timestamp,
            tx_details.height,
            tx_details.incoming,
        );
        if !z_msg.is_empty() {
            crate::db::message::store_message(connection, tx_details.account, &z_msg)?;
        }
    }

    Ok(())
}

async fn fetch_raw_transaction(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
    txid: &Hash,
) -> anyhow::Result<Transaction> {
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
    orchard_keys: Option<(IncomingViewingKey, OutgoingViewingKey)>,
}

pub fn decode_transaction(
    network: &Network,
    account: u32,
    height: u32,
    timestamp: u32,
    id_tx: u32,
    tx: Transaction,
    incoming: bool,
    decryption_keys: &DecryptionKeys,
) -> anyhow::Result<TransactionDetails> {
    let (sapling_ivk, sapling_ovk) = decryption_keys.sapling_keys.clone();

    let block_height = BlockHeight::from_u32(height);
    let mut address: Option<String> = None;
    let mut change_address = None;

    let tx = tx.into_data();

    let mut tx_memo: Memo = Memo::Empty;
    let mut contacts = vec![];

    if let Some(sapling_bundle) = tx.sapling_bundle() {
        let mut contact_decoder = ContactDecoder::new(sapling_bundle.shielded_outputs.len());
        for output in sapling_bundle.shielded_outputs.iter() {
            let mut incoming = false;
            let mut outgoing = false;
            let mut temp_address = String::new();
            let pivk = PreparedIncomingViewingKey::new(&sapling_ivk);
            if let Some((_note, pa, memo)) =
                try_sapling_note_decryption(network, block_height, &pivk, output)
            {
                if let Ok(memo) = Memo::try_from(memo) {
                    if memo != Memo::Empty {
                        tx_memo = memo;
                    }
                }
                temp_address = encode_payment_address(network.hrp_sapling_payment_address(), &pa);
                incoming = true;
            }
            if let Some((_note, pa, memo, ..)) =
                try_sapling_output_recovery(network, block_height, &sapling_ovk, output)
            {
                let _ = contact_decoder.add_memo(&memo); // ignore memo that is not for contacts, if we cannot decode it with ovk, we didn't make create this memo
                if let Ok(memo) = Memo::try_from(memo) {
                    if memo != Memo::Empty {
                        tx_memo = memo;
                    }
                }
                temp_address = encode_payment_address(network.hrp_sapling_payment_address(), &pa);
                outgoing = true;
            }

            if incoming || outgoing {
                if incoming && outgoing {
                    // if we made incoming output, it is either a self-transfer or change
                    change_address = Some(temp_address);
                } else {
                    address.get_or_insert(temp_address);
                }
            }
        }
        contacts.extend(contact_decoder.finalize()?.into_iter());
    }

    if address.is_none() {
        if let Some(orchard_bundle) = tx.orchard_bundle() {
            let mut contact_decoder = ContactDecoder::new(orchard_bundle.actions().len());
            if let Some((orchard_ivk, orchard_ovk)) = decryption_keys.orchard_keys.clone() {
                let poivk = orchard::keys::PreparedIncomingViewingKey::new(&orchard_ivk);
                for action in orchard_bundle.actions().iter() {
                    let mut incoming = false;
                    let mut outgoing = false;
                    let mut temp_address = String::new();
                    let domain = OrchardDomain::for_action(action);
                    if let Some((_note, pa, memo)) = try_note_decryption(&domain, &poivk, action) {
                        if let Ok(memo) = Memo::try_from(MemoBytes::from_bytes(&memo)?) {
                            if memo != Memo::Empty {
                                tx_memo = memo;
                            }
                        }
                        temp_address = orchard_as_unified(network, &pa).encode();
                        incoming = true;
                    }
                    if let Some((_note, pa, memo, ..)) = try_output_recovery_with_ovk(
                        &domain,
                        &orchard_ovk,
                        action,
                        action.cv_net(),
                        &action.encrypted_note().out_ciphertext,
                    ) {
                        let memo_bytes = MemoBytes::from_bytes(&memo)?;
                        let _ = contact_decoder.add_memo(&memo_bytes); // ignore memo that is not for contacts, if we cannot decode it with ovk, we didn't make create this memo
                        if let Ok(memo) = Memo::try_from(memo_bytes) {
                            if memo != Memo::Empty {
                                tx_memo = memo;
                            }
                        }
                        temp_address = orchard_as_unified(network, &pa).encode();
                        outgoing = true;
                    }
                    if incoming || outgoing {
                        if incoming && outgoing {
                            change_address = Some(temp_address);
                        } else {
                            address.get_or_insert(temp_address);
                        }
                    }
                }
            }
            contacts.extend(contact_decoder.finalize()?);
        }
    }

    if address.is_none() {
        if let Some(transparent_bundle) = tx.transparent_bundle() {
            for output in transparent_bundle.vout.iter() {
                if let Some(taddr) = output.recipient_address() {
                    address = Some(encode_transparent_address(
                        &network.b58_pubkey_address_prefix(),
                        &network.b58_script_address_prefix(),
                        &taddr,
                    ));
                }
            }
        }
    }

    if address.is_none() {
        address = change_address; // use if we didn't find any other address
    }

    let address = address.unwrap_or(String::new());

    let memo = match tx_memo {
        Memo::Empty => "".to_string(),
        Memo::Text(text) => text.to_string(),
        Memo::Future(_) => "Unrecognized".to_string(),
        Memo::Arbitrary(_) => "Unrecognized".to_string(),
    };
    let tx_details = TransactionDetails {
        account,
        id_tx,
        address,
        height,
        timestamp,
        memo,
        incoming,
        contacts: contacts.into_iter().map(|c| ContactT {
            id: c.id,
            name: Some(c.name),
            address: Some(c.address),
        }).collect(),
    };

    Ok(tx_details)
}

fn get_decryption_keys(
    network: &Network,
    connection: &Connection,
    account: u32,
) -> anyhow::Result<DecryptionKeys> {
    let fvk = crate::db::account::get_account(connection, account)?.and_then(|d| d.ivk).ok_or(anyhow!("No zFVK"))?;
    let fvk =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk)
            .unwrap();
    let skey = (fvk.fvk.vk.ivk(), fvk.fvk.ovk);

    let okey = crate::db::orchard::get_orchard(connection, account)?;
    let okey = okey.map(|okey| {
        let fvk = FullViewingKey::from_bytes(&okey.fvk).unwrap();
        (fvk.to_ivk(Scope::External), fvk.to_ovk(Scope::External))
    });
    let decryption_keys = DecryptionKeys {
        sapling_keys: skey,
        orchard_keys: okey,
    };
    Ok(decryption_keys)
}

pub async fn retrieve_tx_info(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    req: &GetTransactionDetailRequest,
    decryption_keys: &DecryptionKeys,
) -> anyhow::Result<TransactionDetails> {
    let transaction = fetch_raw_transaction(network, client, req.height, &req.txid).await?;
    let tx_details = decode_transaction(
        network,
        req.account,
        req.height,
        req.timestamp,
        req.id_tx,
        transaction,
        req.value >= 0,
        &decryption_keys,
    )?;

    Ok(tx_details)
}

pub struct GetTransactionDetailRequest {
    pub account: u32,
    pub height: u32,
    pub timestamp: u32,
    pub id_tx: u32,
    pub txid: Hash,
    pub value: i64,
}

#[derive(Debug)]
pub struct TransactionDetails {
    pub account: u32,
    pub id_tx: u32,
    pub height: u32,
    pub timestamp: u32,
    pub address: String,
    pub memo: String,
    pub incoming: bool,
    pub contacts: Vec<ContactT>,
}
