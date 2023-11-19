pub use crate::note_selection::types::{
    Destination, Order, RecipientShort, Source, TransactionBuilderConfig, TransactionPlan,
    TransactionReport, UTXO,
};
pub use crate::note_selection::TransactionBuilderError::TxTooComplex;
pub use builder::{build_tx, get_secret_keys, SecretKeys, TxBuilderContext};
pub use fee::{FeeCalculator, FeeFlat, FeeRule, FeeZIP327};
pub use optimize::build_tx_plan;
use std::str::FromStr;
pub use utxo::fetch_utxos;

use crate::db::data_generated::fb::Recipient;
use thiserror::Error;
use ua::decode;
use zcash_primitives::consensus::Network;
use zcash_primitives::memo::Memo;

#[derive(Error, Debug)]
pub enum TransactionBuilderError {
    #[error("Not enough funds: Missing {0} zats")]
    NotEnoughFunds(u64),
    #[error("Only one recipient can pay for the fees")]
    DuplicateRecipientFee,
    #[error("Not enough funds to pay for the fees")]
    RecipientCannotPayFee,
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
pub fn recipients_to_orders(network: &Network, recipients: &[Recipient]) -> Result<Vec<Order>> {
    let orders: Result<Vec<_>> = recipients
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let destinations = decode(network, r.address().unwrap())?;
            Ok::<_, TransactionBuilderError>(Order {
                id: i as u32,
                destinations,
                raw_amount: r.amount(),
                take_fee: r.fee_included(), // Caller must make sure that at most one recipient pays for the fees
                memo: Memo::from_str(r.memo().unwrap()).unwrap().into(),
            })
        })
        .collect();
    Ok(orders?)
}

#[cfg(test)]
mod tests;
