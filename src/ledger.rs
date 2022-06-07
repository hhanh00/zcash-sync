use serde::{Deserialize, Serialize};

#[cfg(feature = "ledger_sapling")]
pub mod sapling;

mod transparent;

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct APDURequest {
    apduHex: String,
}

#[derive(Serialize, Deserialize)]
struct APDUReply {
    data: String,
    error: Option<String>,
}

pub use transparent::sweep_ledger;
