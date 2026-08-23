use super::types::*;
use zcash_keys::address::Address;
use zcash_keys::encoding::{decode_payment_address, decode_transparent_address};
use zcash_protocol::consensus::{Network, NetworkConstants, Parameters};
use zcash_transparent::address::TransparentAddress;

pub fn decode(network: &Network, address: &str) -> anyhow::Result<[Option<Destination>; 3]> {
    let mut destinations: [Option<Destination>; 3] = [None; 3];
    if let Ok(data) = decode_payment_address(network.hrp_sapling_payment_address(), address) {
        let destination = Destination::Sapling(data.to_bytes());
        destinations[Pool::Sapling as usize] = Some(destination);
    } else if let Ok(Some(ta)) = decode_transparent_address(
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        address,
    ) {
        let destination = Destination::from_transparent(&ta);
        destinations[Pool::Transparent as usize] = Some(destination);
    } else if let Some(address) = Address::decode(network, address) {
        match address {
            Address::Sapling(data) => {
                let destination = Destination::Sapling(data.to_bytes());
                destinations[Pool::Sapling as usize] = Some(destination);
            }
            Address::Transparent(ta) => {
                let destination = Destination::from_transparent(&ta);
                destinations[Pool::Transparent as usize] = Some(destination);
            }
            Address::Unified(ua) => {
                if let Some(data) = ua.orchard() {
                    let destination = Destination::Orchard(data.to_raw_address_bytes());
                    destinations[Pool::Orchard as usize] = Some(destination);
                }
                if let Some(data) = ua.sapling() {
                    let destination = Destination::Sapling(data.to_bytes());
                    destinations[Pool::Sapling as usize] = Some(destination);
                }
                if let Some(ta) = ua.transparent() {
                    let destination = Destination::from_transparent(ta);
                    destinations[Pool::Transparent as usize] = Some(destination);
                }
            }
            Address::Tex(_) => {}
        }
    }

    Ok(destinations)
}
