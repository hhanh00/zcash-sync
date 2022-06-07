use prost::bytes::{Buf, BufMut};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use zcash_primitives::memo::{Memo, MemoBytes};

const CONTACT_COOKIE: u32 = 0x434E5440;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Contact {
    pub id: u32,
    pub name: String,
    pub address: String,
}

pub fn serialize_contacts(contacts: &[Contact]) -> anyhow::Result<Vec<Memo>> {
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

    pub fn finalize(&self) -> anyhow::Result<Vec<Contact>> {
        if !self.has_contacts {
            return Ok(Vec::new());
        }
        let data: Vec<_> = self.chunks.iter().cloned().flatten().collect();
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

#[cfg(test)]
mod tests {
    use crate::contact::{serialize_contacts, Contact};
    use crate::db::DEFAULT_DB_PATH;
    use crate::{DbAdapter, Wallet, LWD_URL};
    use zcash_params::coin::CoinType;

    #[test]
    fn test_contacts() {
        let db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        let contact = Contact {
            id: 0,
            name: "hanh".to_string(),
            address:
                "zs1lvzgfzzwl9n85446j292zg0valw2p47hmxnw42wnqsehsmyuvjk0mhxktcs0pqrplacm2vchh35"
                    .to_string(),
        };
        db.store_contact(&contact, true).unwrap();
    }

    #[tokio::test]
    async fn test_serialize() {
        let db = DbAdapter::new(CoinType::Zcash, DEFAULT_DB_PATH).unwrap();
        let contacts = db.get_unsaved_contacts().unwrap();
        let memos = serialize_contacts(&contacts).unwrap();
        for m in memos.iter() {
            println!("{:?}", m);
        }

        let mut wallet = Wallet::new(CoinType::Zcash, "zec.db");
        wallet.set_lwd_url(LWD_URL).unwrap();
        let tx_id = wallet.save_contacts_tx(&memos, 1, 3).await.unwrap();
        println!("{}", tx_id);
    }
}
