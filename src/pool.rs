use crate::api::recipient::RecipientMemo;
use crate::chain::get_latest_height;
use crate::{connect_lightwalletd, TransactionPlan};
use anyhow::Result;
use rusqlite::Connection;
use std::slice;
use std::str::FromStr;
use zcash_primitives::consensus::Network;
use zcash_primitives::memo::Memo;

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

    // let t_details = db::transparent::get_transparent(connection, account)?;
    // let z_details = db::account::get_account(connection, account)?;
    // let o_details = db::orchard::get_orchard(connection, account)?;
    // let taddr = t_details.map(|d| d.address).unwrap_or_default();
    // let fvk = z_details.and_then(|d| d.ivk).unwrap_or_default();
    // let orchard_fvk = o_details.map(|d| hex::encode(&d.fvk)).unwrap_or_default();

    let tx_plan = crate::pay::build_tx_plan(
        network,
        connection,
        url,
        account,
        last_height,
        slice::from_ref(&recipient),
        !from_pool,
        confirmations,
    )
    .await?;
    Ok(tx_plan)
}

/// Make a transaction that shields the transparent balance
pub async fn shield_taddr(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    amount: u64,
    confirmations: u32,
) -> anyhow::Result<TransactionPlan> {
    let tx_plan = transfer_pools(
        network,
        connection,
        url,
        account,
        1,
        6,
        amount,
        true,
        "Shield Transparent Balance",
        0,
        confirmations,
    )
    .await?;
    Ok(tx_plan)
}
