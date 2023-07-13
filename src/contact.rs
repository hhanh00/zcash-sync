use crate::api::recipient::RecipientMemo;
use crate::db::data_generated::fb::AccountDetailsT;
use crate::{db, TransactionPlan};
use anyhow::{anyhow, Result};
use prost::bytes::{Buf, BufMut};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use zcash_primitives::consensus::Network;
use zcash_primitives::memo::{Memo, MemoBytes};

const CONTACT_COOKIE: u32 = 0x434E5440;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Contact {
    pub id: u32,
    pub name: String,
    pub address: String,
}

pub fn serialize_contacts(contacts: &[Contact]) -> Result<Vec<Memo>> {
    let cs_bin = bincode::serialize(&contacts)?;
    let chunks = cs_bin.chunks(500);
    let memos: Vec<_> = chunks
        .enumerate()
        .map(|(i, c)| {
            let n = i as u8;
            let mut bytes = [0u8; 511];
            let mut bb: Vec<u8> = vec![];
            bb.put_u32(CONTACT_COOKIE);
            bb.put_u8(n);
            bb.put_u16(c.len() as u16);
            bb.put_slice(c);
            bytes[0..bb.len()].copy_from_slice(&bb);
            Memo::Arbitrary(Box::new(bytes))
        })
        .collect();

    Ok(memos)
}

pub struct ContactDecoder {
    has_contacts: bool,
    chunks: Vec<Vec<u8>>,
}

impl ContactDecoder {
    pub fn new(n: usize) -> ContactDecoder {
        let mut chunks = vec![];
        chunks.resize(n, vec![]);
        ContactDecoder {
            has_contacts: false,
            chunks,
        }
    }

    pub fn add_memo(&mut self, memo: &MemoBytes) -> anyhow::Result<()> {
        let memo = Memo::try_from(memo.clone())?;
        if let Memo::Arbitrary(bytes) = memo {
            let (n, data) = ContactDecoder::_decode_box(&bytes)?;
            self.has_contacts = true;
            self.chunks[n as usize] = data;
        }

        Ok(())
    }

    pub fn finalize(&self) -> Result<Vec<Contact>> {
        if !self.has_contacts {
            return Ok(Vec::new());
        }
        let data: Vec<_> = self.chunks.iter().flatten().cloned().collect();
        let contacts = bincode::deserialize::<Vec<Contact>>(&data)?;
        Ok(contacts)
    }

    fn _decode_box(bb: &[u8; 511]) -> anyhow::Result<(u8, Vec<u8>)> {
        let mut bb: &[u8] = bb;
        let magic = bb.get_u32();
        if magic != CONTACT_COOKIE {
            anyhow::bail!("Not a contact record");
        }
        let n = bb.get_u8();
        let len = bb.get_u16() as usize;
        if len > bb.len() {
            anyhow::bail!("Buffer overflow");
        }

        let data = &bb[0..len];
        Ok((n, data.to_vec()))
    }
}

pub async fn commit_unsaved_contacts(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    anchor_offset: u32,
) -> Result<TransactionPlan> {
    let contacts = crate::db::contact::list_unsaved_contacts(connection)?;
    let memos = serialize_contacts(&contacts)?;
    let tx_plan =
        save_contacts_tx(network, connection, url, account, &memos, anchor_offset).await?;
    Ok(tx_plan)
}

async fn save_contacts_tx(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    memos: &[Memo],
    confirmations: u32,
) -> Result<TransactionPlan> {
    let last_height = crate::chain::latest_height(url).await?;
    let AccountDetailsT { address, .. } =
        db::account::get_account(connection, account)?.ok_or(anyhow!("No account"))?;
    let recipients: Vec<_> = memos
        .iter()
        .map(|m| RecipientMemo {
            address: address.clone().unwrap(),
            amount: 0,
            fee_included: false,
            memo: m.clone(),
            max_amount_per_note: 0,
        })
        .collect();

    let tx_plan = crate::pay::build_tx_plan(
        network,
        connection,
        url,
        account,
        last_height,
        &recipients,
        0,
        confirmations,
    )
    .await?;
    Ok(tx_plan)
}
