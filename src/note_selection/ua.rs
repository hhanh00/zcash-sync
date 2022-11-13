use super::types::*;
use zcash_address::unified::{Container, Receiver};
use zcash_address::{AddressKind, ZcashAddress};

pub fn decode(address: &str) -> anyhow::Result<[Option<Destination>; 3]> {
    let mut destinations: [Option<Destination>; 3] = [None; 3];
    let address = ZcashAddress::try_from_encoded(address)?;
    match address.kind {
        AddressKind::Sprout(_) => {}
        AddressKind::Sapling(data) => {
            let destination = Destination::Sapling(data);
            destinations[Pool::Sapling as usize] = Some(destination);
        }
        AddressKind::Unified(unified_address) => {
            for address in unified_address.items() {
                match address {
                    Receiver::Orchard(data) => {
                        let destination = Destination::Orchard(data);
                        destinations[Pool::Orchard as usize] = Some(destination);
                    }
                    Receiver::Sapling(data) => {
                        let destination = Destination::Sapling(data);
                        destinations[Pool::Sapling as usize] = Some(destination);
                    }
                    Receiver::P2pkh(data) => {
                        let destination = Destination::Transparent(data);
                        destinations[Pool::Transparent as usize] = Some(destination);
                    }
                    Receiver::P2sh(_) => {}
                    Receiver::Unknown { .. } => {}
                }
            }
        }
        AddressKind::P2pkh(data) => {
            let destination = Destination::Transparent(data);
            destinations[Pool::Transparent as usize] = Some(destination);
        }
        AddressKind::P2sh(_) => {}
    }

    Ok(destinations)
}
