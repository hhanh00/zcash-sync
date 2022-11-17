use crate::api::account::get_unified_address;
use crate::api::recipient::RecipientMemo;
use crate::api::sync::get_latest_height;
pub use crate::broadcast_tx;
use crate::note_selection::{FeeFlat, Order};
use crate::{
    build_tx, fetch_utxos, get_secret_keys, note_selection, AccountData, CoinConfig, DbAdapter,
    TransactionBuilderConfig, TransactionBuilderError, TransactionPlan, TxBuilderContext,
    MAX_ATTEMPTS,
};
use rand::rngs::OsRng;
use std::cmp::min;
use std::slice;
use std::str::FromStr;
use zcash_primitives::memo::{Memo, MemoBytes};
use zcash_primitives::transaction::builder::Progress;

#[allow(dead_code)]
type PaymentProgressCallback = Box<dyn Fn(Progress) + Send + Sync>;

pub async fn build_tx_plan(
    coin: u8,
    account: u32,
    last_height: u32,
    recipients: &[RecipientMemo],
    excluded_flags: u8,
    confirmations: u32,
) -> note_selection::Result<TransactionPlan> {
    let c = CoinConfig::get(coin);
    let (fvk, checkpoint_height) = {
        let db = c.db()?;
        let AccountData { fvk, .. } = db.get_account_info(account)?;
        let checkpoint_height = get_checkpoint_height(&db, last_height, confirmations)?;
        (fvk, checkpoint_height)
    };
    let change_address = get_unified_address(coin, account, true, true, true)?;
    let context = TxBuilderContext::from_height(coin, checkpoint_height)?;

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
    let utxos = fetch_utxos(coin, account, checkpoint_height, excluded_flags).await?;

    let config = TransactionBuilderConfig::new(&change_address);
    let tx_plan = crate::note_selection::build_tx_plan::<FeeFlat>(
        &fvk,
        checkpoint_height,
        &context.orchard_anchor,
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
    let tx = build_tx(c.chain.network(), &keys, &tx_plan, OsRng)?;
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

pub async fn build_max_tx(
    coin: u8,
    account: u32,
    last_height: u32,
    recipient: &RecipientMemo, // amount & max_amount per note are ignored
    excluded_flags: u8,
    confirmations: u32,
) -> note_selection::Result<TransactionPlan> {
    let mut recipient = recipient.clone();
    let checkpoint_height = {
        let c = CoinConfig::get(coin);
        let db = c.db()?;
        get_checkpoint_height(&db, last_height, confirmations)?
    };
    let utxos = fetch_utxos(coin, account, checkpoint_height, excluded_flags).await?;
    let available_funds: u64 = utxos.iter().map(|n| n.amount).sum();
    recipient.amount = available_funds;
    for _ in 0..MAX_ATTEMPTS {
        // this will fail at least once because of the fees
        let result = build_tx_plan(
            coin,
            account,
            last_height,
            slice::from_ref(&recipient),
            excluded_flags,
            confirmations,
        )
        .await;
        match result {
            Err(TransactionBuilderError::NotEnoughFunds(missing)) => {
                recipient.amount -= missing; // reduce the amount and retry
            }
            _ => return result,
        }
    }
    Err(TransactionBuilderError::TxTooComplex)
}

/// Make a transaction that shields the transparent balance
pub async fn shield_taddr(coin: u8, account: u32, confirmations: u32) -> anyhow::Result<String> {
    let address = get_unified_address(coin, account, false, true, true)?; // get our own unified address
    let recipient = RecipientMemo {
        address,
        amount: 0,
        memo: Memo::from_str("Shield Transparent Balance")?,
        max_amount_per_note: 0,
    };
    let last_height = get_latest_height().await?;
    let tx_plan = build_max_tx(coin, account, last_height, &recipient, 2, confirmations).await?;
    let tx_id = sign_and_broadcast(coin, account, &tx_plan).await?;
    log::info!("TXID: {}", tx_id);
    Ok(tx_id)
}

fn mark_spent(coin: u8, ids: &[u32]) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let mut db = c.db()?;
    db.tx_mark_spend(ids)?;
    Ok(())
}

fn get_checkpoint_height(
    db: &DbAdapter,
    last_height: u32,
    confirmations: u32,
) -> anyhow::Result<u32> {
    let anchor_height = last_height.saturating_sub(confirmations);
    let checkpoint_height = db
        .get_checkpoint_height(anchor_height)?
        .unwrap_or_else(|| db.sapling_activation_height()); // get the latest checkpoint before the requested anchor height
    Ok(checkpoint_height)
}
