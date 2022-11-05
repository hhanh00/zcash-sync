use lazy_static::lazy_static;
use lazycell::AtomicLazyCell;
use orchard::circuit::ProvingKey;

lazy_static! {
    pub static ref PROVING_KEY: AtomicLazyCell<ProvingKey> = AtomicLazyCell::new();
}

mod hash;
mod note;
mod key;

pub use note::{OrchardDecrypter, OrchardViewKey, DecryptedOrchardNote};
pub use hash::{ORCHARD_ROOTS, OrchardHasher};
pub use key::{derive_orchard_keys, OrchardKeyBytes};

pub fn get_proving_key() -> &'static ProvingKey {
    if !PROVING_KEY.filled() {
        log::info!("Building Orchard proving key");
        let _ = PROVING_KEY.fill(ProvingKey::build());
    }
    PROVING_KEY.borrow().unwrap()
}
