use bip39::{Language, Mnemonic, Seed};
use zcash_primitives::zip32::{ExtendedSpendingKey, ExtendedFullViewingKey, ChildIndex};
use crate::NETWORK;
use zcash_primitives::consensus::Parameters;
use zcash_client_backend::encoding::{encode_extended_spending_key, encode_extended_full_viewing_key, encode_payment_address, decode_extended_spending_key, decode_extended_full_viewing_key};
use anyhow::anyhow;

pub fn get_secret_key(seed: &str) -> anyhow::Result<String> {
    let mnemonic = Mnemonic::from_phrase(&seed, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let master = ExtendedSpendingKey::master(seed.as_bytes());
    let path = [
        ChildIndex::Hardened(32),
        ChildIndex::Hardened(NETWORK.coin_type()),
        ChildIndex::Hardened(0),
    ];
    let extsk = ExtendedSpendingKey::from_path(&master, &path);
    let spending_key = encode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &extsk);

    Ok(spending_key)
}

pub fn get_viewing_key(secret_key: &str) -> anyhow::Result<String> {
    let extsk = decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), secret_key)?.ok_or(anyhow!("Invalid Secret Key"))?;
    let fvk = ExtendedFullViewingKey::from(&extsk);
    let viewing_key = encode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk);
    Ok(viewing_key)
}

pub fn get_address(viewing_key: &str) -> anyhow::Result<String> {
    let fvk = decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &viewing_key)?.ok_or(anyhow!("Invalid Viewing Key"))?;
    let (_, payment_address) = fvk.default_address().unwrap();
    let address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &payment_address);
    Ok(address)
}
