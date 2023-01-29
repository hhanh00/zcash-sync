use crate::coinconfig::CoinConfig;
use anyhow::anyhow;
use bip39::{Language, Mnemonic, Seed};
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key,
    encode_extended_full_viewing_key, encode_extended_spending_key, encode_payment_address,
};
use zcash_client_backend::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::zip32::{ChildIndex, ExtendedFullViewingKey, ExtendedSpendingKey};

pub fn decode_key(
    coin: u8,
    key: &str,
    index: u32,
) -> anyhow::Result<(
    Option<String>,
    Option<String>,
    String,
    String,
    Option<orchard::keys::FullViewingKey>,
)> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let res = if let Ok(mnemonic) = Mnemonic::from_phrase(key, Language::English) {
        let (sk, ivk, pa) = derive_secret_key(network, &mnemonic, index)?;
        Ok((Some(key.to_string()), Some(sk), ivk, pa, None))
    } else if let Ok(sk) =
        decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), key)
    {
        let (ivk, pa) = derive_viewing_key(network, &sk)?;
        Ok((None, Some(key.to_string()), ivk, pa, None))
    } else if let Ok(fvk) =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), key)
    {
        let pa = derive_address(network, &fvk)?;
        Ok((None, None, key.to_string(), pa, None))
    } else if let Ok(ufvk) = UnifiedFullViewingKey::decode(network, key) {
        let sapling_dfvk = ufvk
            .sapling()
            .ok_or(anyhow!("UFVK must contain a sapling key"))?;
        let sapling_efvk =
            ExtendedFullViewingKey::from_diversifiable_full_viewing_key(&sapling_dfvk);
        let key = encode_extended_full_viewing_key(
            network.hrp_sapling_extended_full_viewing_key(),
            &sapling_efvk,
        );
        let pa = derive_address(network, &sapling_efvk)?;
        let orchard_key = ufvk.orchard().cloned();
        Ok((None, None, key, pa, orchard_key))
    } else {
        Err(anyhow::anyhow!("Not a valid key"))
    };
    res
}

#[allow(dead_code)] // Used by C FFI
pub fn is_valid_key(coin: u8, key: &str) -> i8 {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    if Mnemonic::from_phrase(key, Language::English).is_ok() {
        return 0;
    }
    if decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), key).is_ok() {
        return 1;
    }
    if decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), key)
        .is_ok()
    {
        return 2;
    }
    if UnifiedFullViewingKey::decode(network, key).is_ok() {
        return 3;
    }
    // TODO: Accept UA viewing key
    -1
}

pub fn decode_address(coin: u8, address: &str) -> Option<RecipientAddress> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    zcash_client_backend::address::RecipientAddress::decode(network, address)
}

fn derive_secret_key(
    network: &Network,
    mnemonic: &Mnemonic,
    index: u32,
) -> anyhow::Result<(String, String, String)> {
    let seed = Seed::new(mnemonic, "");
    let master = ExtendedSpendingKey::master(seed.as_bytes());
    let path = [
        ChildIndex::Hardened(32),
        ChildIndex::Hardened(network.coin_type()),
        ChildIndex::Hardened(index),
    ];
    let extsk = ExtendedSpendingKey::from_path(&master, &path);
    let sk = encode_extended_spending_key(network.hrp_sapling_extended_spending_key(), &extsk);

    let (fvk, pa) = derive_viewing_key(network, &extsk)?;
    Ok((sk, fvk, pa))
}

fn derive_viewing_key(
    network: &Network,
    extsk: &ExtendedSpendingKey,
) -> anyhow::Result<(String, String)> {
    let fvk = ExtendedFullViewingKey::from(extsk);
    let pa = derive_address(network, &fvk)?;
    let fvk =
        encode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk);
    Ok((fvk, pa))
}

fn derive_address(network: &Network, fvk: &ExtendedFullViewingKey) -> anyhow::Result<String> {
    let (_, payment_address) = fvk.default_address();
    let address = encode_payment_address(network.hrp_sapling_payment_address(), &payment_address);
    Ok(address)
}
