//! Contact Address book

use crate::api::payment_v2::build_tx_plan;
use crate::api::recipient::RecipientMemo;
use crate::api::sync::get_latest_height;
use crate::coinconfig::CoinConfig;
use crate::contact::{serialize_contacts, Contact};
use crate::db::data_generated::fb::FeeT;
use crate::{get_ua_of, TransactionPlan};
use zcash_primitives::memo::Memo;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;

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
pub async fn commit_unsaved_contacts(
    coin: u8,
    account: u32,
    anchor_offset: u32,
    fee: &FeeT,
    z_factor: u32,
) -> anyhow::Result<TransactionPlan> {
    let c = CoinConfig::get(coin);
    let contacts = c.db()?.get_unsaved_contacts()?;
    let memos = serialize_contacts(&contacts)?;
    let tx_plan = save_contacts_tx(coin, account, &memos, anchor_offset, fee, z_factor).await?;
    Ok(tx_plan)
}

async fn save_contacts_tx(
    coin: u8,
    account: u32,
    memos: &[Memo],
    anchor_offset: u32,
    fee: &FeeT,
    z_factor: u32,
) -> anyhow::Result<TransactionPlan> {
    let c = CoinConfig::get(coin);
    let last_height = get_latest_height(coin).await?;
    let address = get_ua_of(c.chain.network(), &c.connection(), account, 7)?;
    let recipients: Vec<_> = memos
        .iter()
        .map(|m| RecipientMemo {
            address: address.clone(),
            amount: u64::from(DEFAULT_FEE),
            fee_included: false,
            memo: m.clone(),
            max_amount_per_note: 0,
        })
        .collect();

    let tx_plan = build_tx_plan(
        coin,
        account,
        last_height,
        &recipients,
        1,
        anchor_offset,
        fee,
        z_factor,
    )
    .await?;
    Ok(tx_plan)
}
