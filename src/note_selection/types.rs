use std::ops::{Add, Sub};
use zcash_primitives::memo::{Memo, MemoBytes};
use serde::Serialize;
use serde_with::serde_as;
use serde_hex::{SerHex,Strict};

#[derive(Clone, PartialEq, Debug)]
pub enum PrivacyPolicy {
    SamePoolOnly,
    SamePoolTypeOnly,
    AnyPool,
}

#[serde_as]
#[derive(Clone, Serialize, Debug)]
pub enum Source {
    Transparent {
        #[serde(with = "SerHex::<Strict>")] txid: [u8; 32],
        index: u32,
    },
    Sapling {
        id_note: u32,
        #[serde(with = "SerHex::<Strict>")] diversifier: [u8; 11],
        #[serde(with = "SerHex::<Strict>")] rseed: [u8; 32],
        #[serde_as(as = "serde_with::hex::Hex")] witness: Vec<u8>,
    },
    Orchard {
        id_note: u32,
        #[serde(with = "SerHex::<Strict>")] diversifier: [u8; 11],
        #[serde(with = "SerHex::<Strict>")] rseed: [u8; 32],
        #[serde(with = "SerHex::<Strict>")] rho: [u8; 32],
        #[serde_as(as = "serde_with::hex::Hex")] witness: Vec<u8>,
    },
}

#[derive(Clone, Copy, Serialize, Debug)]
#[serde_as]
pub enum Destination {
    Transparent(#[serde(with = "SerHex::<Strict>")] [u8; 20]), // MD5
    Sapling(#[serde(with = "SerHex::<Strict>")] [u8; 43]), // Diversifier + Jubjub Point
    Orchard(#[serde(with = "SerHex::<Strict>")] [u8; 43]), // Diviersifer + Pallas Point
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pool {
    Transparent = 0,
    Sapling = 1,
    Orchard = 2,
}

#[derive(Serialize)]
pub struct Order {
    pub id: u32,
    pub destinations: [Option<Destination>; 3],
    pub amount: u64,
    #[serde(with = "MemoBytesProxy")]
    pub memo: MemoBytes,
    pub no_fee: bool,

    pub filled: u64, // mutable
}

#[derive(Serialize)]
#[serde_as]
#[serde(remote = "MemoBytes")]
struct MemoBytesProxy(
    #[serde_as(as = "serde_with::hex::Hex")]
    #[serde(getter = "get_memo_bytes")]
    pub Vec<u8>
);

fn get_memo_bytes(memo: &MemoBytes) -> Vec<u8> {
    memo.as_slice().to_vec()
}

impl From<MemoBytesProxy> for MemoBytes {
    fn from(p: MemoBytesProxy) -> MemoBytes {
        MemoBytes::from_bytes(&p.0).unwrap()
    }
}

impl Default for Order {
    fn default() -> Self {
        Order {
            id: 0,
            destinations: [None; 3],
            amount: 0,
            memo: MemoBytes::empty(),
            no_fee: false,
            filled: 0
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Fill {
    pub id_order: u32,
    pub destination: Destination,
    pub amount: u64,
    pub is_fee: bool,
}

#[derive(Debug)]
pub struct Execution {
    pub allocation: PoolAllocation,
    pub fills: Vec<Fill>,
}

#[derive(Serialize)]
pub struct TransactionPlan {
    pub spends: Vec<UTXO>,
    pub outputs: Vec<Fill>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PoolAllocation(pub [u64; 3]);

pub type PoolPrecedence = [Pool; 3];

#[derive(Clone, Serialize, Debug)]
pub struct UTXO {
    pub source: Source,
    pub amount: u64,
}

impl PoolAllocation {
    pub fn total(&self) -> u64 {
        self.0.iter().sum()
    }
}

impl From<&[UTXO]> for PoolAllocation {
    fn from(utxos: &[UTXO]) -> Self {
        let mut allocation = PoolAllocation::default();
        for utxo in utxos {
            let pool = utxo.source.pool() as usize;
            allocation.0[pool] += utxo.amount;
        }
        allocation
    }
}

impl Add for PoolAllocation {
    type Output = PoolAllocation;

    fn add(self, rhs: Self) -> Self::Output {
        let mut res = PoolAllocation::default();
        for i in 0..3 {
            res.0[i] = self.0[i] + rhs.0[i];
        }
        res
    }
}

impl Sub for PoolAllocation {
    type Output = PoolAllocation;

    fn sub(self, rhs: Self) -> Self::Output {
        let mut res = PoolAllocation::default();
        for i in 0..3 {
            res.0[i] = self.0[i] - rhs.0[i];
        }
        res
    }
}

#[derive(Clone)]
pub struct NoteSelectConfig {
    pub privacy_policy: PrivacyPolicy,
    pub use_transparent: bool,
    pub precedence: PoolPrecedence,
    pub change_address: String,
}

impl Source {
    pub fn pool(&self) -> Pool {
        match self {
            Source::Transparent { .. } => Pool::Transparent,
            Source::Sapling { .. } => Pool::Sapling,
            Source::Orchard { .. } => Pool::Orchard,
        }
    }
}

impl Destination {
    pub fn pool(&self) -> Pool {
        match self {
            Destination::Transparent { .. } => Pool::Transparent,
            Destination::Sapling { .. } => Pool::Sapling,
            Destination::Orchard { .. } => Pool::Orchard,
        }
    }
}
