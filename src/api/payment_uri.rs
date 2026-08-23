//! encode and decode Payment URI

use crate::coinconfig::CoinConfig;
use crate::key2::decode_address;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use zip321::{Payment, TransactionRequest};
use zcash_protocol::memo::{Memo, MemoBytes};
use zcash_protocol::value::Zatoshis;

/// Build a payment URI
/// # Arguments
/// * `address`: recipient address
/// * `amount`: amount in zats
/// * `memo`: memo text
pub fn make_payment_uri(
    coin: u8,
    address: &str,
    amount: u64,
    memo: &str,
) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let addr = decode_address(coin, address).ok_or_else(|| anyhow::anyhow!("Invalid address"))?;
    let payment = Payment::new(
        addr.to_zcash_address(c.chain.network()),
        Some(Zatoshis::from_u64(amount).map_err(|_| anyhow::anyhow!("Invalid amount"))?),
        Some(MemoBytes::from(Memo::from_str(memo)?)),
        None,
        None,
        vec![],
    )
    .map_err(|e| anyhow::anyhow!("Invalid payment: {:?}", e))?;
    let treq = TransactionRequest::new(vec![payment])
        .map_err(|e| anyhow::anyhow!("Invalid payment URI: {:?}", e))?;
    let uri = treq.to_uri();
    let uri = format!("{}{}", c.chain.ticker(), &uri[5..]); // hack to replace the URI scheme
    Ok(uri)
}

/// Decode a payment uri
/// # Arguments
/// * `uri`: payment uri
pub fn parse_payment_uri(coin: u8, uri: &str) -> anyhow::Result<PaymentURI> {
    let c = CoinConfig::get(coin);
    let scheme = c.chain.ticker();
    let scheme_len = scheme.len().min(uri.len());
    if uri[..scheme_len].ne(scheme) {
        anyhow::bail!("Invalid Payment URI: Invalid scheme");
    }
    let uri = format!("zcash{}", &uri[scheme_len..]); // hack to replace the URI scheme
    let treq = TransactionRequest::from_uri(&uri)
        .map_err(|e| anyhow::anyhow!("Invalid Payment URI: {:?}", e))?;
    let payments = treq.payments();
    if payments.len() != 1 {
        anyhow::bail!("Invalid Payment URI: Exactly one payee expected")
    }
    let payment = payments.values().next().unwrap();
    let memo = match payment.memo() {
        Some(memo) => match Memo::try_from(memo.clone()) {
            Ok(Memo::Text(text)) => Ok(text.to_string()),
            Ok(Memo::Empty) => Ok(String::new()),
            _ => Err(anyhow::anyhow!("Invalid Memo")),
        },
        None => Ok(String::new()),
    }?;
    let payment = PaymentURI {
        address: payment.recipient_address().to_string(),
        amount: u64::from(
            payment
                .amount()
                .ok_or_else(|| anyhow::anyhow!("Invalid Payment URI: Missing amount"))?,
        ),
        memo,
    };

    // let payment_json = serde_json::to_string(&payment)?;
    //
    Ok(payment)
}

#[derive(Serialize, Deserialize)]
pub struct PaymentURI {
    pub address: String,
    pub amount: u64,
    pub memo: String,
}
