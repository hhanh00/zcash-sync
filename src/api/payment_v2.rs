use crate::api::account::get_unified_address;
use crate::api::recipient::{RecipientMemo, RecipientShort};
use crate::api::sync::get_latest_height;
pub use crate::broadcast_tx;
use crate::note_selection::{FeeZIP327, Order, TransactionReport};
use crate::{
    build_tx, fetch_utxos, get_secret_keys, AccountData, CoinConfig, TransactionBuilderConfig,
    TransactionPlan, TxBuilderContext,
};
use rand::rngs::OsRng;
use std::cmp::min;
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::transaction::builder::Progress;

type PaymentProgressCallback = Box<dyn Fn(Progress) + Send + Sync>;

pub async fn build_tx_plan(
    coin: u8,
    account: u32,
    last_height: u32,
    recipients: &[RecipientMemo],
    confirmations: u32,
) -> anyhow::Result<TransactionPlan> {
    let c = CoinConfig::get(coin);
    let fvk = {
        let db = c.db()?;
        let AccountData { fvk, .. } = db.get_account_info(account)?;
        fvk
    };
    let change_address = get_unified_address(coin, account, true, true, true)?;

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
    let utxos = fetch_utxos(coin, account, last_height, true, confirmations).await?;

    log::info!("UTXO: {:?}", utxos);

    let config = TransactionBuilderConfig::new(&change_address);
    let tx_plan = crate::note_selection::build_tx_plan::<FeeZIP327>(
        &fvk,
        last_height,
        &utxos,
        &orders,
        &config,
    )?;
    Ok(tx_plan)
}

pub fn sign_plan(coin: u8, account: u32, tx_plan: &TransactionPlan) -> anyhow::Result<Vec<u8>> {
    let c = CoinConfig::get(coin);
    let fvk = {
        let db = c.db()?;
        let AccountData { fvk, .. } = db.get_account_info(account)?;
        fvk
    };

    if fvk != tx_plan.fvk {
        return Err(anyhow::anyhow!("Account does not match transaction"));
    }

    let keys = get_secret_keys(coin, account)?;
    let context = TxBuilderContext::from_height(c.coin, tx_plan.height)?;
    let tx = build_tx(c.chain.network(), &keys, &tx_plan, context, OsRng).unwrap();
    Ok(tx)
}

pub async fn sign_and_broadcast(
    coin: u8,
    account: u32,
    tx_plan: &TransactionPlan,
) -> anyhow::Result<String> {
    let tx = sign_plan(coin, account, tx_plan)?;
    let txid = broadcast_tx(&tx).await?;
    let id_notes: Vec<_> = tx_plan
        .spends
        .iter()
        .filter_map(|n| if n.id != 0 { Some(n.id) } else { None })
        .collect();
    mark_spent(coin, &id_notes)?;
    Ok(txid)
}

/// Make a transaction that shields the transparent balance
pub async fn shield_taddr(coin: u8, account: u32) -> anyhow::Result<String> {
    // let last_height = get_latest_height().await?;
    // let tx_id = build_sign_send_multi_payment(coin, account, last_height, &[], 0, Box::new(|_| {})).await?;
    // Ok(tx_id)
    todo!()
}

fn mark_spent(coin: u8, ids: &[u32]) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let mut db = c.db()?;
    db.tx_mark_spend(ids)?;
    Ok(())
}
