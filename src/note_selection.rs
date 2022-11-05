mod types;
mod fee;
mod utxo;
mod fill;
mod select;
mod builder;

#[cfg(test)]
mod tests;

use std::cmp::min;
use zcash_primitives::memo::MemoBytes;
pub use types::{Source, Destination, PrivacyPolicy, Pool, UTXO, NoteSelectConfig};
pub use utxo::fetch_utxos;
pub use fill::decode;
pub use select::note_select_with_fee;
pub use fee::{FeeZIP327, FeeFlat};
use crate::api::payment::RecipientMemo;
use crate::note_selection::types::TransactionPlan;

async fn prepare_multi_payment(
    coin: u8,
    account: u32,
    last_height: u32,
    recipients: &[RecipientMemo],
    config: &NoteSelectConfig,
    anchor_offset: u32) -> anyhow::Result<TransactionPlan>
{
    let mut orders = vec![];
    let mut id_order = 0;
    for r in recipients {
        let mut amount = r.amount;
        let max_amount_per_note = if r.max_amount_per_note == 0 { u64::MAX } else { r.max_amount_per_note };
        while amount > 0 {
            let a = min(amount, max_amount_per_note);
            let memo_bytes: MemoBytes = r.memo.clone().into();
            let order = decode(id_order, &r.address, a, memo_bytes)?;
            orders.push(order);
            amount -= a;
            id_order += 1;
        }
    }
    let utxos = fetch_utxos(coin, account, last_height, config.use_transparent, anchor_offset).await?;

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, config)?;

    Ok(tx_plan)
}
