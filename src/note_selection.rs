pub use crate::note_selection::types::{
    Destination, Order, RecipientShort, Source, TransactionBuilderConfig, TransactionPlan,
    TransactionReport, UTXO,
};
pub use crate::note_selection::TransactionBuilderError::TxTooComplex;
pub use builder::{build_tx, get_secret_keys, TxBuilderContext};
pub use fee::{FeeCalculator, FeeFlat, FeeZIP327};
pub use optimize::build_tx_plan;
use std::str::FromStr;
pub use utxo::fetch_utxos;

use crate::api::recipient::Recipient;
use thiserror::Error;
use ua::decode;
use zcash_primitives::memo::Memo;

#[derive(Error, Debug)]
pub enum TransactionBuilderError {
    #[error("Not enough funds: Missing {0} zats")]
    NotEnoughFunds(u64),
    #[error("Tx too complex")]
    TxTooComplex,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, TransactionBuilderError>;

mod builder;
mod fee;
mod optimize;
mod ser;
mod types;
mod ua;
mod utxo;

pub const MAX_ATTEMPTS: usize = 10;

#[allow(dead_code)]
pub fn recipients_to_orders(recipients: &[Recipient]) -> Result<Vec<Order>> {
    let orders: Result<Vec<_>> = recipients
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let destinations = decode(&r.address)?;
            Ok::<_, TransactionBuilderError>(Order {
                id: i as u32,
                destinations,
                amount: r.amount,
                memo: Memo::from_str(&r.memo).unwrap().into(),
            })
        })
        .collect();
    Ok(orders?)
}

#[cfg(test)]
mod tests;
