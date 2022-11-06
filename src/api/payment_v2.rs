use std::cmp::min;
use zcash_primitives::memo::MemoBytes;
use crate::api::payment::{RecipientMemo, RecipientShort};
use crate::{AccountData, CoinConfig, fetch_utxos, TransactionBuilderConfig, TransactionPlan};
use crate::note_selection::{FeeZIP327, Order};

async fn prepare_payment_v2(recipients: &[RecipientShort]) -> anyhow::Result<()> {
    todo!()
}

pub async fn build_tx_plan(
    coin: u8,
    account: u32,
    last_height: u32,
    recipients: &[RecipientMemo],
    config: &TransactionBuilderConfig,
    confirmations: u32,
) -> anyhow::Result<TransactionPlan> {
    let c = CoinConfig::get(coin);
    let fvk = {
        let db = c.db()?;
        let AccountData { fvk, .. } = db.get_account_info(account)?;
        fvk
    };

    let mut orders = vec![];
    let mut id_order = 0;
    for r in recipients {
        let mut amount = r.amount;
        let max_amount_per_note = if r.max_amount_per_note == 0 {
            u64::MAX
        } else {
            r.max_amount_per_note
        };
        while amount > 0 {
            let a = min(amount, max_amount_per_note);
            let memo_bytes: MemoBytes = r.memo.clone().into();
            let order = Order::new(id_order, &r.address, a, memo_bytes);
            orders.push(order);
            amount -= a;
            id_order += 1;
        }
    }
    let utxos = fetch_utxos(
        coin,
        account,
        last_height,
        true,
        confirmations,
    )
        .await?;

    log::info!("UTXO: {:?}", utxos);

    let tx_plan = crate::note_selection::build_tx_plan::<FeeZIP327>(&fvk, last_height, &utxos, &orders, config)?;
    Ok(tx_plan)
}
