use crate::{AccountData, db, DbAdapter};
use anyhow::anyhow;
use orchard::keys::{FullViewingKey, Scope};
use orchard::Address;
use rusqlite::Connection;
use zcash_address::unified::{Container, Encoding, Receiver};
use zcash_address::{unified, ToAddress, ZcashAddress};
use zcash_client_backend::encoding::{decode_payment_address, encode_payment_address, AddressCodec, decode_extended_full_viewing_key};
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::sapling::PaymentAddress;
use zcash_primitives::zip32::DiversifierIndex;
use crate::db::data_generated::fb::AccountDetailsT;

#[derive(Debug)]
pub struct UnifiedAddressType {
    pub transparent: bool,
    pub sapling: bool,
    pub orchard: bool,
}

pub struct DecodedUA {
    pub network: Network,
    pub transparent: Option<TransparentAddress>,
    pub sapling: Option<PaymentAddress>,
    pub orchard: Option<Address>,
}

impl std::fmt::Display for DecodedUA {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DecodedUA: {:?} {:?} {:?}",
            self.transparent.as_ref().map(|a| a.encode(&self.network)),
            self.sapling
                .as_ref()
                .map(|a| encode_payment_address(self.network.hrp_sapling_payment_address(), a)),
            self.orchard.as_ref().map(|a| {
                let ua = unified::Address(vec![Receiver::Orchard(a.to_raw_address_bytes())]);
                ua.encode(&self.network.address_network().unwrap())
            })
        )
    }
}

/*
 * It can also return a t-addr if there is no other selection
 */
pub fn get_unified_address(
    network: &Network,
    connection: &Connection,
    account: u32,
    tpe: Option<UnifiedAddressType>,
) -> anyhow::Result<String> {
    let mut tpe = match tpe {
        Some(tpe) => tpe,
        None => db::orchard::get_ua_settings(connection, account)?,
    };
    let transparent_details = db::transparent::get_transparent(connection, account)?;
    if transparent_details.is_none() {
        tpe.transparent = false;
    }
    let sapling_details = db::account::get_account(connection, account)?.ok_or(anyhow!("No zaddr"))?;
    let orchard_details = db::orchard::get_orchard(connection, account)?;
    if orchard_details.is_none() {
        tpe.orchard = false;
    }
    if !tpe.sapling && !tpe.orchard {
        // UA cannot be t-only
        let t_details = transparent_details.ok_or(anyhow!("No taddr"))?;
        return Ok(t_details.address.unwrap());
    }

    let address = match (tpe.transparent, tpe.sapling, tpe.orchard) {
        (false, true, false) => {
            let AccountDetailsT { address, .. } = sapling_details;
            return Ok(address.unwrap());
        }
        _ => {
            let mut rcvs = vec![];
            if tpe.transparent {
                let t_details = transparent_details.ok_or(anyhow!("No taddr"))?;
                let address = t_details.address.unwrap();
                let address = TransparentAddress::decode(network, &address)?;
                if let TransparentAddress::PublicKey(pkh) = address {
                    let rcv = Receiver::P2pkh(pkh);
                    rcvs.push(rcv);
                }
            }
            if tpe.sapling {
                let AccountDetailsT { address, .. } = sapling_details;
                let address = address.unwrap();
                let pa = decode_payment_address(network.hrp_sapling_payment_address(), &address)
                    .unwrap();
                let rcv = Receiver::Sapling(pa.to_bytes());
                rcvs.push(rcv);
            }
            if tpe.orchard {
                let okey = db::orchard::get_orchard(connection, account)?;
                if let Some(okey) = okey {
                    let fvk = FullViewingKey::from_bytes(&okey.fvk).unwrap();
                    let address = fvk.address_at(0usize, Scope::External);
                    let rcv = Receiver::Orchard(address.to_raw_address_bytes());
                    rcvs.push(rcv);
                }
            }

            assert!(!rcvs.is_empty());
            let addresses = unified::Address(rcvs);
            ZcashAddress::from_unified(network.address_network().unwrap(), addresses)
        }
    };
    Ok(address.encode())
}

pub fn decode_unified_address(network: &Network, ua: &str) -> anyhow::Result<DecodedUA> {
    let mut decoded_ua = DecodedUA {
        network: network.clone(),
        transparent: None,
        sapling: None,
        orchard: None,
    };
    let network = network.address_network().unwrap();
    let (a_network, ua) = unified::Address::decode(ua)?;
    if network != a_network {
        anyhow::bail!("Invalid network")
    }

    for recv in ua.items_as_parsed() {
        match recv {
            Receiver::Orchard(addr) => {
                decoded_ua.orchard = Address::from_raw_address_bytes(addr).into();
            }
            Receiver::Sapling(addr) => {
                decoded_ua.sapling = PaymentAddress::from_bytes(addr);
            }
            Receiver::P2pkh(addr) => {
                decoded_ua.transparent = Some(TransparentAddress::PublicKey(*addr));
            }
            Receiver::P2sh(_) => {}
            Receiver::Unknown { .. } => {}
        }
    }
    Ok(decoded_ua)
}

pub fn orchard_as_unified(network: &Network, address: &Address) -> ZcashAddress {
    let unified_address = unified::Address(vec![Receiver::Orchard(address.to_raw_address_bytes())]);
    ZcashAddress::from_unified(network.address_network().unwrap(), unified_address)
}

/// Generate a new diversified address
pub fn get_diversified_address(network: &Network, connection: &Connection, account: u32, ua_type: u8, time: u32) -> anyhow::Result<String> {
    let ua_type = ua_type & 6; // don't include transparent component
    if ua_type == 0 {
        anyhow::bail!("Must include a shielded receiver");
    }
    let AccountDetailsT { ivk, .. } = db::account::get_account_info(connection, account)?;
    let orchard_keys = db::orchard::get_orchard(connection, account)?;
    let mut receivers = vec![];
    if let Some(ivk) = ivk {
        let fvk = decode_extended_full_viewing_key(
            network.hrp_sapling_extended_full_viewing_key(),
            &ivk,
        ).unwrap();
        let mut di = [0u8; 11];
        di[4..8].copy_from_slice(&time.to_le_bytes());
        let diversifier_index = DiversifierIndex(di);
        let (_, pa) = fvk
            .find_address(diversifier_index)
            .ok_or_else(|| anyhow::anyhow!("Cannot generate new address"))?;

        if ua_type == 2 || orchard_keys.is_none() {
            // sapling only
            return Ok(encode_payment_address(
                c.chain.network().hrp_sapling_payment_address(),
                &pa,
            ));
        }
        if ua_type & 2 != 0 {
            receivers.push(Receiver::Sapling(pa.to_bytes()));
        }
    }

    if let Some(orchard_keys) = orchard_keys {
        if ua_type & 4 != 0 {
            let orchard_fvk = FullViewingKey::from_bytes(&orchard_keys.fvk).unwrap();
            let index = diversifier_index.0; // any sapling index is fine for orchard
            let orchard_address = orchard_fvk.address_at(index, Scope::External);
            receivers.push(Receiver::Orchard(orchard_address.to_raw_address_bytes()));
        }
    }

    let unified_address = UA(receivers);
    let address = ZcashAddress::from_unified(
        network.address_network().unwrap(),
        unified_address,
    );
    let address = address.encode();
    Ok(address)
}
