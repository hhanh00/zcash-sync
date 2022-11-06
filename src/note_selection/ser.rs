use serde::{Serialize, Deserialize};
use serde_with::serde_as;
use zcash_primitives::memo::MemoBytes;

#[derive(Serialize, Deserialize)]
#[serde_as]
#[serde(remote = "MemoBytes")]
pub struct MemoBytesProxy(
    #[serde_as(as = "serde_with::hex::Hex")]
    #[serde(getter = "get_memo_bytes")]
    pub Vec<u8>,
);

fn get_memo_bytes(memo: &MemoBytes) -> Vec<u8> {
    memo.as_slice().to_vec()
}

impl From<MemoBytesProxy> for MemoBytes {
    fn from(p: MemoBytesProxy) -> MemoBytes {
        MemoBytes::from_bytes(&p.0).unwrap()
    }
}

