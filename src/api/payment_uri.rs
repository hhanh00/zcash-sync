use crate::coinconfig::CoinConfig;
use serde::Serialize;
use std::convert::TryFrom;
use std::str::FromStr;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::zip321::{Payment, TransactionRequest};
use zcash_primitives::memo::Memo;
use zcash_primitives::transaction::components::Amount;

pub fn make_payment_uri(address: &str, amount: u64, memo: &str) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let addr = RecipientAddress::decode(c.chain.network(), address)
        .ok_or_else(|| anyhow::anyhow!("Invalid address"))?;
    let payment = Payment {
        recipient_address: addr,
        amount: Amount::from_u64(amount).map_err(|_| anyhow::anyhow!("Invalid amount"))?,
        memo: Some(Memo::from_str(memo)?.into()),
        label: None,
        message: None,
        other_params: vec![],
    };
    let treq = TransactionRequest {
        payments: vec![payment],
    };
    let uri = treq
        .to_uri(c.chain.network())
        .ok_or_else(|| anyhow::anyhow!("Cannot build Payment URI"))?;
    let uri = format!("{}{}", c.chain.ticker(), &uri[5..]); // hack to replace the URI scheme
    Ok(uri)
}

pub fn parse_payment_uri(uri: &str) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    if uri[..5].ne(c.chain.ticker()) {
        anyhow::bail!("Invalid Payment URI");
    }
    let uri = format!("zcash{}", &uri[5..]); // hack to replace the URI scheme
    let treq = TransactionRequest::from_uri(c.chain.network(), &uri)
        .map_err(|_| anyhow::anyhow!("Invalid Payment URI"))?;
    if treq.payments.len() != 1 {
        anyhow::bail!("Invalid Payment URI")
    }
    let payment = &treq.payments[0];
    let memo = match payment.memo {
        Some(ref memo) => {
            let memo = Memo::try_from(memo.clone())?;
            match memo {
                Memo::Text(text) => Ok(text.to_string()),
                Memo::Empty => Ok(String::new()),
                _ => Err(anyhow::anyhow!("Invalid Memo")),
            }
        }
        None => Ok(String::new()),
    }?;
    let payment = MyPayment {
        address: payment.recipient_address.encode(c.chain.network()),
        amount: u64::from(payment.amount),
        memo,
    };

    let payment_json = serde_json::to_string(&payment)?;

    Ok(payment_json)
}

#[derive(Serialize)]
struct MyPayment {
    address: String,
    amount: u64,
    memo: String,
}
