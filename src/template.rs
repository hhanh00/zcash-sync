use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct ZMemo {
    pub include_reply_to: bool,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SendTemplate {
    pub id: u32,
    pub title: String,
    pub address: String,
    pub amount: u64,
    pub fiat_amount: f64,
    pub fee_included: bool,
    pub fiat: Option<String>,
    pub memo: ZMemo,
}
