use anyhow::Result;
use rusqlite::Connection;
use zcash_primitives::consensus::Network;
use crate::unified::UnifiedAddressType;

pub fn get_unified_address(network: &Network, connection: &Connection, account: u32, address_type: u8) -> Result<String> {
    let tpe = UnifiedAddressType {
        transparent: address_type & 1 != 0,
        sapling: address_type & 2 != 0,
        orchard: address_type & 4 != 0,
    };
    let address = crate::get_unified_address(network, connection, account, Some(tpe))?; // use ua settings
    Ok(address)
}
