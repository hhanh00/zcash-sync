use crate::api::payment::{build_sign_send_multi_payment, RecipientMemo};
use crate::api::sync::get_latest_height;
use crate::coinconfig::CoinConfig;
use crate::contact::{serialize_contacts, Contact};
use zcash_primitives::memo::Memo;

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

pub async fn commit_unsaved_contacts(anchor_offset: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let contacts = c.db()?.get_unsaved_contacts()?;
    let memos = serialize_contacts(&contacts)?;
    let tx_id = save_contacts_tx(&memos, anchor_offset).await?;
    Ok(tx_id)
}

pub async fn save_contacts_tx(memos: &[Memo], anchor_offset: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let last_height = get_latest_height().await?;
    let address = c.db()?.get_address(c.id_account)?;
    let recipients: Vec<_> = memos
        .iter()
        .map(|m| RecipientMemo {
            address: address.clone(),
            amount: 0,
            memo: m.clone(),
            max_amount_per_note: 0,
        })
        .collect();

    let tx_id = build_sign_send_multi_payment(
        last_height,
        &recipients,
        false,
        anchor_offset,
        Box::new(|_| {}),
    )
    .await?;
    Ok(tx_id)
}
