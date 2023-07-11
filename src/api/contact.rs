//! Contact Address book

use crate::api::payment_v2::build_tx_plan;
use crate::api::recipient::RecipientMemo;
use crate::chain::get_latest_height;
use crate::coinconfig::CoinConfig;
use crate::contact::{serialize_contacts, Contact};
use crate::db::AccountData;
use crate::TransactionPlan;
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
pub async fn commit_unsaved_contacts(anchor_offset: u32) -> anyhow::Result<TransactionPlan> {
    let c = CoinConfig::get_active();
    let contacts = c.db()?.get_unsaved_contacts()?;
    let memos = serialize_contacts(&contacts)?;
    let tx_plan = save_contacts_tx(&memos, anchor_offset).await?;
    Ok(tx_plan)
}

