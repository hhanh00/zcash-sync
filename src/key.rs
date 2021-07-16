use crate::NETWORK;
use bip39::{Language, Mnemonic, Seed};
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key,
    encode_extended_full_viewing_key, encode_extended_spending_key, encode_payment_address,
};
use zcash_primitives::consensus::Parameters;
use zcash_primitives::zip32::{ChildIndex, ExtendedFullViewingKey, ExtendedSpendingKey};

pub fn decode_key(key: &str) -> anyhow::Result<(Option<String>, Option<String>, String, String)> {
    let res = if let Ok(mnemonic) = Mnemonic::from_phrase(&key, Language::English) {
        let (sk, ivk, pa) = derive_secret_key(&mnemonic)?;
        Ok((Some(key.to_string()), Some(sk), ivk, pa))
    } else if let Ok(Some(sk)) =
        decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &key)
    {
        let (ivk, pa) = derive_viewing_key(&sk)?;
        Ok((None, Some(key.to_string()), ivk, pa))
    } else if let Ok(Some(fvk)) =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &key)
    {
        let pa = derive_address(&fvk)?;
        Ok((None, None, key.to_string(), pa))
    } else {
        Err(anyhow::anyhow!("Not a valid key"))
    };
    res
}

pub fn is_valid_key(key: &str) -> bool {
    if Mnemonic::from_phrase(&key, Language::English).is_ok() {
        return true;
    }
    if let Ok(Some(_)) =
        decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &key)
    {
        return true;
    }
    if let Ok(Some(_)) =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &key)
    {
        return true;
    }
    false
}

pub fn derive_secret_key(mnemonic: &Mnemonic) -> anyhow::Result<(String, String, String)> {
    let seed = Seed::new(&mnemonic, "");
    let master = ExtendedSpendingKey::master(seed.as_bytes());
    let path = [
        ChildIndex::Hardened(32),
        ChildIndex::Hardened(NETWORK.coin_type()),
        ChildIndex::Hardened(0),
    ];
    let extsk = ExtendedSpendingKey::from_path(&master, &path);
    let sk = encode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &extsk);

    let (fvk, pa) = derive_viewing_key(&extsk)?;
    Ok((sk, fvk, pa))
}

pub fn derive_viewing_key(extsk: &ExtendedSpendingKey) -> anyhow::Result<(String, String)> {
    let fvk = ExtendedFullViewingKey::from(extsk);
    let pa = derive_address(&fvk)?;
    let fvk =
        encode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &fvk);
    Ok((fvk, pa))
}

pub fn derive_address(fvk: &ExtendedFullViewingKey) -> anyhow::Result<String> {
    let (_, payment_address) = fvk.default_address().unwrap();
    let address = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &payment_address);
    Ok(address)
}
