use std::slice;
use std::str::FromStr;
use anyhow::Result;
use rusqlite::Connection;
use zcash_primitives::consensus::Network;
use zcash_primitives::memo::Memo;
use crate::api::recipient::RecipientMemo;
use crate::{connect_lightwalletd, TransactionPlan};
use crate::chain::get_latest_height;

pub async fn transfer_pools(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    from_pool: u8,
    to_pool: u8,
    amount: u64,
    fee_included: bool,
    memo: &str,
    split_amount: u64,
    confirmations: u32,
) -> Result<TransactionPlan> {
    let mut client = connect_lightwalletd(url).await?;
    let address = crate::account::get_unified_address(network, connection, account, to_pool)?; // get our own unified address
    let recipient = RecipientMemo {
        address,
        amount,
        fee_included,
        memo: Memo::from_str(memo)?,
        max_amount_per_note: split_amount,
    };
    let last_height = get_latest_height(&mut client).await?;
    let tx_plan = crate::build_tx_plan(
        coin,
        account,
        last_height,
        slice::from_ref(&recipient),
        !from_pool,
        confirmations,
    )
        .await?;
    Ok(tx_plan)
}

