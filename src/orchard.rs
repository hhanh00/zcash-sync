use lazy_static::lazy_static;
use lazycell::AtomicLazyCell;
use orchard::circuit::ProvingKey;

lazy_static! {
    /// Proving key for the fixed (post-NU6.2) circuit.
    pub static ref PROVING_KEY: AtomicLazyCell<ProvingKey> = AtomicLazyCell::new();
    /// Proving key for the insecure (pre-NU6.2) circuit.
    pub static ref PROVING_KEY_INSECURE: AtomicLazyCell<ProvingKey> = AtomicLazyCell::new();
}

mod hash;
mod key;
mod note;

pub use hash::{OrchardHasher, ORCHARD_ROOTS};
pub use key::{derive_orchard_keys, OrchardKeyBytes};
pub use note::{decode_merkle_path, DecryptedOrchardNote, OrchardDecrypter, OrchardViewKey};

/// Returns the proving key for the fixed (post-NU6.2) circuit.
pub fn get_proving_key() -> &'static ProvingKey {
    if !PROVING_KEY.filled() {
        log::info!("Building Orchard proving key (fixed, post-NU6.2)");
        let _ = PROVING_KEY.fill(ProvingKey::build());
    }
    PROVING_KEY.borrow().unwrap()
}

/// Returns the proving key for the insecure (pre-NU6.2) circuit.
pub fn get_proving_key_insecure() -> &'static ProvingKey {
    if !PROVING_KEY_INSECURE.filled() {
        log::info!("Building Orchard proving key (insecure, pre-NU6.2)");
        let _ = PROVING_KEY_INSECURE.fill(ProvingKey::build_for_version(
            halo2_gadgets::ecc::chip::CircuitVersion::InsecureUnanchoredBase,
        ));
    }
    PROVING_KEY_INSECURE.borrow().unwrap()
}

/// Returns the correct proving key based on whether NU6.2 is active at the given height.
pub fn get_proving_key_for_height(network: &zcash_primitives::consensus::Network, height: u32) -> &'static ProvingKey {
    use zcash_primitives::consensus::{BlockHeight, NetworkUpgrade, Parameters};
    let is_nu6_2 = network.is_nu_active(NetworkUpgrade::Nu6_2, BlockHeight::from_u32(height));
    let pk = if is_nu6_2 {
        // NU6.2 or later — use the fixed circuit
        get_proving_key()
    } else {
        // Pre-NU6.2 — use the insecure circuit
        get_proving_key_insecure()
    };
    pk
}
