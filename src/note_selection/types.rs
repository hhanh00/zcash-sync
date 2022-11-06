use serde::{Serialize, Deserialize};
use serde_with::serde_as;
use serde_hex::{SerHex, Strict};
use zcash_primitives::memo::MemoBytes;
use crate::note_selection::ua::decode;
use super::ser::MemoBytesProxy;

pub struct TransactionBuilderConfig {
    pub change_address: String,
}

impl TransactionBuilderConfig {
    pub fn new(change_address: &str) -> Self {
        TransactionBuilderConfig {
            change_address: change_address.to_string(),
        }
    }
}

#[serde_as]
#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Source {
    Transparent {
        #[serde(with = "SerHex::<Strict>")]
        txid: [u8; 32],
        index: u32,
    },
    Sapling {
        id_note: u32,
        #[serde(with = "SerHex::<Strict>")]
        diversifier: [u8; 11],
        #[serde(with = "SerHex::<Strict>")]
        rseed: [u8; 32],
        #[serde_as(as = "serde_with::hex::Hex")]
        witness: Vec<u8>,
    },
    Orchard {
        id_note: u32,
        #[serde(with = "SerHex::<Strict>")]
        diversifier: [u8; 11],
        #[serde(with = "SerHex::<Strict>")]
        rseed: [u8; 32],
        #[serde(with = "SerHex::<Strict>")]
        rho: [u8; 32],
        #[serde_as(as = "serde_with::hex::Hex")]
        witness: Vec<u8>,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
#[serde_as]
pub enum Destination {
    Transparent(#[serde(with = "SerHex::<Strict>")] [u8; 20]), // MD5
    Sapling(#[serde(with = "SerHex::<Strict>")] [u8; 43]),     // Diversifier + Jubjub Point
    Orchard(#[serde(with = "SerHex::<Strict>")] [u8; 43]),     // Diviersifer + Pallas Point
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pool {
    Transparent = 0,
    Sapling = 1,
    Orchard = 2,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PoolAllocation(pub [u64; 3]);

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct UTXO {
    pub source: Source,
    pub amount: u64,
}

#[derive(Serialize, Debug)]
pub struct Order {
    pub id: u32,
    pub destinations: [Option<Destination>; 3],
    pub amount: u64,
    #[serde(with = "MemoBytesProxy")] pub memo: MemoBytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub id_order: Option<u32>,
    pub destination: Destination,
    pub amount: u64,
    #[serde(with = "MemoBytesProxy")] pub memo: MemoBytes,
}

#[derive(Clone, Deserialize)]
pub struct RecipientShort {
    pub address: String,
    pub amount: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct TransactionPlan {
    pub fvk: String,
    pub height: u32,
    pub spends: Vec<UTXO>,
    pub outputs: Vec<Fill>,
    pub fee: u64,
    pub net_chg: [i64; 2],
}

#[derive(PartialEq, Debug)]
pub struct OrderGroupAmounts {
    pub t0: u64,
    pub s0: u64,
    pub o0: u64,
    pub x: u64,
    pub fee: u64,
}

pub struct OrderInfo {
    pub group_type: usize,
    pub amount: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FundAllocation {
    pub s1: u64,
    pub o1: u64,
    pub t2: u64,
    pub s2: u64,
    pub o2: u64,
}

impl Source {
    pub fn pool(&self) -> usize {
        match self {
            Source::Transparent { .. } => 0,
            Source::Sapling { .. } => 1,
            Source::Orchard { .. } => 2,
        }
    }
}

impl Destination {
    pub fn pool(&self) -> usize {
        match self {
            Destination::Transparent { .. } => 0,
            Destination::Sapling { .. } => 1,
            Destination::Orchard { .. } => 2,
        }
    }
}

impl Order {
    pub fn new(id: u32, address: &str, amount: u64, memo: MemoBytes) -> Self {
        let destinations = decode(address).unwrap();
        Order {
            id,
            destinations,
            amount,
            memo,
        }
    }
}
