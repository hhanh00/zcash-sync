use bech32::{ToBase32, Variant};
use bip39::{Language, Mnemonic, Seed};
use rand::RngCore;
use rand::rngs::OsRng;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key,
    encode_extended_full_viewing_key, encode_extended_spending_key, encode_payment_address,
};
use zcash_primitives::consensus::Parameters;
use zcash_primitives::zip32::{ChildIndex, ExtendedFullViewingKey, ExtendedSpendingKey};
use zcash_params::coin::{CoinChain, CoinType, get_coin_chain};

pub struct KeyHelpers {
    coin_type: CoinType,
}

impl KeyHelpers {
    pub fn new(coin_type: CoinType) -> Self {
        KeyHelpers {
            coin_type
        }
    }

    fn chain(&self) -> &dyn CoinChain { get_coin_chain(self.coin_type) }

    pub fn decode_key(&self, key: &str, index: u32) -> anyhow::Result<(Option<String>, Option<String>, String, String)> {
        let network = self.chain().network();
        let res = if let Ok(mnemonic) = Mnemonic::from_phrase(&key, Language::English) {
            let (sk, ivk, pa) = self.derive_secret_key(&mnemonic, index)?;
            Ok((Some(key.to_string()), Some(sk), ivk, pa))
        } else if let Ok(Some(sk)) =
        decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), &key)
        {
            let (ivk, pa) = self.derive_viewing_key(&sk)?;
            Ok((None, Some(key.to_string()), ivk, pa))
        } else if let Ok(Some(fvk)) =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &key)
        {
            let pa = self.derive_address(&fvk)?;
            Ok((None, None, key.to_string(), pa))
        } else {
            Err(anyhow::anyhow!("Not a valid key"))
        };
        res
    }

    pub fn is_valid_key(&self, key: &str) -> i8 {
        let network = self.chain().network();
        if Mnemonic::from_phrase(&key, Language::English).is_ok() {
            return 0;
        }
        if let Ok(Some(_)) =
        decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), &key)
        {
            return 1;
        }
        if let Ok(Some(_)) =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &key)
        {
            return 2;
        }
        -1
    }

    pub fn derive_secret_key(&self, mnemonic: &Mnemonic, index: u32) -> anyhow::Result<(String, String, String)> {
        let network = self.chain().network();
        let seed = Seed::new(&mnemonic, "");
        let master = ExtendedSpendingKey::master(seed.as_bytes());
        let path = [
            ChildIndex::Hardened(32),
            ChildIndex::Hardened(network.coin_type()),
            ChildIndex::Hardened(index),
        ];
        let extsk = ExtendedSpendingKey::from_path(&master, &path);
        let sk = encode_extended_spending_key(network.hrp_sapling_extended_spending_key(), &extsk);

        let (fvk, pa) = self.derive_viewing_key(&extsk)?;
        Ok((sk, fvk, pa))
    }

    pub fn derive_viewing_key(&self, extsk: &ExtendedSpendingKey) -> anyhow::Result<(String, String)> {
        let network = self.chain().network();
        let fvk = ExtendedFullViewingKey::from(extsk);
        let pa = self.derive_address(&fvk)?;
        let fvk =
            encode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk);
        Ok((fvk, pa))
    }

    pub fn derive_address(&self, fvk: &ExtendedFullViewingKey) -> anyhow::Result<String> {
        let network = self.chain().network();
        let (_, payment_address) = fvk.default_address().unwrap();
        let address = encode_payment_address(network.hrp_sapling_payment_address(), &payment_address);
        Ok(address)
    }

    pub fn valid_address(&self, address: &str) -> bool {
        let recipient = RecipientAddress::decode(self.chain().network(), address);
        recipient.is_some()
    }
}

pub fn generate_random_enc_key() -> anyhow::Result<String> {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let key = bech32::encode("zwk", key.to_base32(), Variant::Bech32)?;
    Ok(key)
}
