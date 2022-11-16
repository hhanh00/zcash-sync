//! Contact Address book

use crate::api::payment_v2::{build_tx_plan, sign_and_broadcast};
use crate::api::recipient::RecipientMemo;
use crate::api::sync::get_latest_height;
use crate::coinconfig::CoinConfig;
use crate::contact::{serialize_contacts, Contact};
use crate::db::AccountData;
use zcash_primitives::memo::Memo;

/// Store contact in the database
/// # Arguments
/// * `id`: contact id
/// * `name`: contact name
/// * `address`: contact address
/// * `dirty`: true if the database hasn't been saved to the blockchain yet
pub fn store_contact(id: u32, name: &str, address: &str, dirty: bool) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let contact = Contact {
        id,
        name: name.to_string(),
        address: address.to_string(),
    };
    c.db()?.store_contact(&contact, dirty)?;
    Ok(())
}

/// Save the new/modified contacts to the blockchain
/// # Arguments
/// * `anchor_offset`: minimum confirmations required for note selection
pub async fn commit_unsaved_contacts(anchor_offset: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let contacts = c.db()?.get_unsaved_contacts()?;
    let memos = serialize_contacts(&contacts)?;
    let tx_id = save_contacts_tx(&memos, anchor_offset).await?;
    Ok(tx_id)
}

async fn save_contacts_tx(memos: &[Memo], anchor_offset: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let last_height = get_latest_height().await?;
    let AccountData { address, .. } = c.db()?.get_account_info(c.id_account)?;
    let recipients: Vec<_> = memos
        .iter()
        .map(|m| RecipientMemo {
            address: address.clone(),
            amount: 0,
            memo: m.clone(),
            max_amount_per_note: 0,
        })
        .collect();

    let tx_plan = build_tx_plan(
        c.coin,
        c.id_account,
        last_height,
        &recipients,
        1,
        anchor_offset,
    )
    .await?;
    let tx_id = sign_and_broadcast(c.coin, c.id_account, &tx_plan).await?;
    Ok(tx_id)
}
