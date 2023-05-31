use anyhow::Result;
use orchard::keys::FullViewingKey;
use secp256k1::PublicKey;
use zcash_primitives::zip32::DiversifiableFullViewingKey;
use crate::ledger::transport::*;

pub fn ledger_get_fvks() -> Result<(PublicKey, DiversifiableFullViewingKey, Option<FullViewingKey>)> {
    ledger_init()?;
    let pubkey = ledger_get_pubkey()?;
    let pubkey = PublicKey::from_slice(&pubkey)?;
    let dfvk: DiversifiableFullViewingKey = ledger_get_dfvk()?;
    let o_dfvk = if ledger_has_orchard()? {
        let o_fvk = ledger_get_o_fvk()?;
        FullViewingKey::from_bytes(&o_fvk.try_into().unwrap())
    }
    else {
        None
    };
    Ok((pubkey, dfvk, o_dfvk))
}
