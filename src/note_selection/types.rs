use super::ser::MemoBytesProxy;
use crate::note_selection::ua::decode;
use crate::unified::orchard_as_unified;
use crate::Hash;
use orchard::Address;
use serde::{Deserialize, Serialize};
use serde_hex::{SerHex, Strict};
use serde_with::serde_as;
use zcash_client_backend::encoding::{encode_payment_address, AddressCodec};
use zcash_params::coin::CoinType::Zcash;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::memo::MemoBytes;
use zcash_primitives::sapling::PaymentAddress;

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

impl Destination {
    pub fn address(&self, network: &Network) -> String {
        match self {
            Destination::Transparent(data) => {
                let ta = TransparentAddress::PublicKey(data.clone());
                ta.encode(network)
            }
            Destination::Sapling(data) => {
                let pa = PaymentAddress::from_bytes(data).unwrap();
                encode_payment_address(network.hrp_sapling_payment_address(), &pa)
            }
            Destination::Orchard(data) => {
                let address = Address::from_raw_address_bytes(data).unwrap();
                let zo = orchard_as_unified(network, &address);
                zo.encode()
            }
        }
    }
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
    pub id: u32,
    pub source: Source,
    pub amount: u64,
}

#[derive(Serialize, Debug)]
pub struct Order {
    pub id: u32,
    pub destinations: [Option<Destination>; 3],
    pub amount: u64,
    #[serde(with = "MemoBytesProxy")]
    pub memo: MemoBytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    pub id_order: Option<u32>,
    pub destination: Destination,
    pub amount: u64,
    #[serde(with = "MemoBytesProxy")]
    pub memo: MemoBytes,
}

#[derive(Clone, Deserialize)]
pub struct RecipientShort {
    pub address: String,
    pub amount: u64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde_as]
pub struct TransactionPlan {
    pub fvk: String,
    pub height: u32,
    #[serde(with = "SerHex::<Strict>")]
    pub orchard_anchor: Hash,
    pub spends: Vec<UTXO>,
    pub outputs: Vec<Fill>,
    pub fee: u64,
    pub net_chg: [i64; 2],
}

#[derive(Serialize)]
pub struct TransactionReport {
    pub outputs: Vec<TransactionOutput>,
    pub transparent: u64,
    pub sapling: u64,
    pub orchard: u64,
    pub net_sapling: i64,
    pub net_orchard: i64,
    pub fee: u64,
    pub privacy_level: u8,
}

#[derive(Serialize)]
pub struct TransactionOutput {
    pub id: u32,
    pub address: String,
    pub amount: u64,
    pub pool: u8,
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