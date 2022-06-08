use crate::coinconfig::CoinConfig;
use bech32::{ToBase32, Variant};
use bip39::{Language, Mnemonic, Seed};
use rand::rngs::OsRng;
use rand::RngCore;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key,
    encode_extended_full_viewing_key, encode_extended_spending_key, encode_payment_address,
};
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::zip32::{ChildIndex, ExtendedFullViewingKey, ExtendedSpendingKey};

pub fn decode_key(
    coin: u8,
    key: &str,
    index: u32,
) -> anyhow::Result<(Option<String>, Option<String>, String, String)> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let res = if let Ok(mnemonic) = Mnemonic::from_phrase(key, Language::English) {
        let (sk, ivk, pa) = derive_secret_key(network, &mnemonic, index)?;
        Ok((Some(key.to_string()), Some(sk), ivk, pa))
    } else if let Ok(Some(sk)) =
        decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), key)
    {
        let (ivk, pa) = derive_viewing_key(network, &sk)?;
        Ok((None, Some(key.to_string()), ivk, pa))
    } else if let Ok(Some(fvk)) =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), key)
    {
        let pa = derive_address(network, &fvk)?;
        Ok((None, None, key.to_string(), pa))
    } else {
        Err(anyhow::anyhow!("Not a valid key"))
    };
    res
}

pub fn is_valid_key(coin: u8, key: &str) -> i8 {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    if Mnemonic::from_phrase(key, Language::English).is_ok() {
        return 0;
    }
    if let Ok(Some(_)) =
        decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), key)
    {
        return 1;
    }
    if let Ok(Some(_)) =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), key)
    {
        return 2;
    }
    -1
}

pub fn is_valid_address(coin: u8, address: &str) -> bool {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let recipient = RecipientAddress::decode(network, address);
    recipient.is_some()
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

pub fn generate_random_enc_key() -> anyhow::Result<String> {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let key = bech32::encode("zwk", key.to_base32(), Variant::Bech32)?;
    Ok(key)
}
