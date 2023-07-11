use std::str::FromStr;
use anyhow::Result;
use orchard::keys::SpendingKey;
use rusqlite::Connection;
use secp256k1::SecretKey;
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_primitives::zip32::ExtendedSpendingKey;

pub struct SecretKeys {
    pub transparent: Option<SecretKey>,
    pub sapling: Option<ExtendedSpendingKey>,
    pub orchard: Option<SpendingKey>,
}

pub fn get_secret_keys(connection: &Connection, account: u32) -> Result<SecretKeys> {
    let t_details = super::transparent::get_transparent(connection, account)?;
    let z_details = super::account::get_account(connection, account)?;
    let o_details = super::orchard::get_orchard(connection, account)?;

    let tsk = t_details.and_then(|d| d.sk).and_then(|sk|
        SecretKey::from_str(&sk).ok()
    );
    let zsk = z_details.and_then(|d| d.sk).and_then(|sk|
        decode_extended_spending_key(
            network.hrp_sapling_extended_spending_key(),
            &sk,
        ).ok());
    let osk = o_details.and_then(|d| d.sk).and_then(|sk|
        Some(SpendingKey::from_bytes(sk).unwrap()));

    let sk = SecretKeys {
        transparent: tsk,
        sapling: zsk,
        orchard: osk,
    };
    Ok(sk)
}
