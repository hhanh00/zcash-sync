use std::str::FromStr;
pub use crate::note_selection::TransactionBuilderError::TxTooComplex;
pub use crate::note_selection::types::{
    UTXO, Order, RecipientShort, TransactionBuilderConfig, TransactionPlan,
    Source, Destination };
pub use utxo::fetch_utxos;
pub use builder::{TxBuilderContext, get_secret_keys, build_tx};
pub use optimize::build_tx_plan;
pub use fee::{FeeCalculator, FeeZIP327};

use thiserror::Error;
use zcash_primitives::memo::Memo;
use ua::decode;
use optimize::{allocate_funds, fill, group_orders, outputs_for_change, select_inputs, sum_utxos};
use crate::api::payment::Recipient;

#[derive(Error, Debug)]
pub enum TransactionBuilderError {
    #[error("Not enough funds")]
    NotEnoughFunds,
    #[error("Tx too complex")]
    TxTooComplex,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, TransactionBuilderError>;

mod types;
mod ser;
mod ua;
mod utxo;
mod optimize;
mod fee;
mod builder;

const MAX_ATTEMPTS: usize = 10;

pub fn recipients_to_orders(recipients: &[Recipient]) -> Result<Vec<Order>> {
    let orders: Result<Vec<_>> = recipients.iter().enumerate().map(|(i, r)| {
        let destinations = decode(&r.address)?;
        Ok::<_, TransactionBuilderError>(Order {
            id: i as u32,
            destinations,
            amount: r.amount,
            memo: Memo::from_str(&r.memo).unwrap().into(),
        })
    }).collect();
    Ok(orders?)
}

#[cfg(test)]
mod tests;
