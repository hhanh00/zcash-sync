use crate::db::ZMessage;
use crate::{AccountData, CoinConfig};
use serde::Deserialize;
use std::str::FromStr;
use zcash_primitives::memo::Memo;

#[derive(Deserialize)]
pub struct Recipient {
    pub address: String,
    pub amount: u64,
    pub fee_included: bool,
    pub reply_to: bool,
    pub subject: String,
    pub memo: String,
    pub max_amount_per_note: u64,
}

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
        let memo = if !r.reply_to && r.subject.is_empty() {
            r.memo.clone()
        } else {
            encode_memo(from, r.reply_to, &r.subject, &r.memo)
        };
        Ok(RecipientMemo {
            address: r.address.clone(),
            amount: r.amount,
            fee_included: r.fee_included,
            memo: Memo::from_str(&memo)?,
            max_amount_per_note: r.max_amount_per_note,
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
            subject: memo_lines[0].chars().take(20).collect(),
            body: memo.to_string(),
            timestamp,
            height,
            incoming,
        }
    };
    msg
}

/// Parse a json document that contains a list of recipients
pub fn parse_recipients(recipients: &str) -> anyhow::Result<Vec<RecipientMemo>> {
    let c = CoinConfig::get_active();
    let AccountData { address, .. } = c.db()?.get_account_info(c.id_account)?;
    let recipients: Vec<Recipient> = serde_json::from_str(recipients)?;
    let recipient_memos: anyhow::Result<Vec<_>> = recipients
        .iter()
        .map(|r| RecipientMemo::from_recipient(&address, r))
        .collect();
    recipient_memos
}
