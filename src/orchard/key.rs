use bip39::{Language, Mnemonic};
use orchard::keys::{FullViewingKey, SpendingKey};

pub struct OrchardKeyBytes {
    pub sk: [u8; 32],
    pub fvk: [u8; 96],
}

pub fn derive_orchard_keys(coin_type: u32, seed: &str, account_index: u32) -> OrchardKeyBytes {
    let mnemonic = Mnemonic::from_phrase(seed, Language::English).unwrap();
    let sk = SpendingKey::from_zip32_seed(
        mnemonic.entropy(),
        coin_type,
        account_index,
    ).unwrap();
    let fvk = FullViewingKey::from(&sk);
    OrchardKeyBytes {
        sk: sk.to_bytes().clone(),
        fvk: fvk.to_bytes()
    }
}
