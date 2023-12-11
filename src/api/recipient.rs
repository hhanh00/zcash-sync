use crate::db::data_generated::fb::{Recipient, Recipients};
use crate::db::ZMessage;
use crate::{AccountData, CoinConfig};
use serde::Deserialize;
use std::str::FromStr;
use zcash_primitives::memo::Memo;

#[derive(Clone, Deserialize)]
pub struct RecipientShort {
    pub address: String,
    pub amount: u64,
}

#[derive(Clone, Debug)]
pub struct RecipientMemo {
    pub address: String,
    pub amount: u64,
    pub fee_included: bool,
    pub memo: Memo,
    pub max_amount_per_note: u64,
}

impl RecipientMemo {
    pub fn from_recipient(from: &str, r: &Recipient) -> anyhow::Result<Self> {
        let memo = if !r.reply_to() && r.subject().as_ref().unwrap().is_empty() {
            r.memo().unwrap().to_string()
        } else {
            encode_memo(from, r.reply_to(), r.subject().unwrap(), r.memo().unwrap())
        };
        Ok(RecipientMemo {
            address: r.address().unwrap().to_string(),
            amount: r.amount(),
            fee_included: r.fee_included(),
            memo: Memo::from_str(&memo)?,
            max_amount_per_note: r.max_amount_per_note(),
        })
    }
}

impl From<RecipientShort> for RecipientMemo {
    fn from(r: RecipientShort) -> Self {
        RecipientMemo {
            address: r.address,
            amount: r.amount,
            fee_included: false,
            memo: Memo::Empty,
            max_amount_per_note: 0,
        }
    }
}

/// Encode a message into a memo
pub fn encode_memo(from: &str, include_from: bool, subject: &str, body: &str) -> String {
    let from = if include_from { from } else { "" };
    let msg = format!("\u{1F6E1}MSG\n{}\n{}\n{}", from, subject, body);
    msg
}

/// Decode a memo into a message
pub fn decode_memo(
    id_tx: u32,
    memo: &str,
    recipient: &str,
    timestamp: u32,
    height: u32,
    incoming: bool,
) -> ZMessage {
    let memo_lines: Vec<_> = memo.splitn(4, '\n').collect();
    let msg = if memo_lines.len() == 4 && memo_lines[0] == "\u{1F6E1}MSG" {
        ZMessage {
            id_tx,
            sender: if memo_lines[1].is_empty() {
                None
            } else {
                Some(memo_lines[1].to_string())
            },
            recipient: recipient.to_string(),
            subject: memo_lines[2].to_string(),
            body: memo_lines[3].to_string(),
            timestamp,
            height,
            incoming,
        }
    } else {
        ZMessage {
            id_tx,
            sender: None,
            recipient: recipient.to_string(),
            subject: String::new(),
            body: memo.to_string(),
            timestamp,
            height,
            incoming,
        }
    };
    msg
}

/// Parse a json document that contains a list of recipients
pub fn parse_recipients(
    from_addr: &str,
    recipients: &Recipients,
) -> anyhow::Result<Vec<RecipientMemo>> {
    let recipients = recipients.values().unwrap();
    let recipient_memos: anyhow::Result<Vec<_>> = recipients
        .iter()
        .map(|r| RecipientMemo::from_recipient(&from_addr, &r))
        .collect();
    recipient_memos
}
