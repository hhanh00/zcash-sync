use anyhow::anyhow;
use orchard::Address;
use orchard::keys::{FullViewingKey, Scope};
use zcash_address::{ToAddress, unified, ZcashAddress};
use zcash_address::unified::{Container, Encoding, Receiver};
use zcash_client_backend::encoding::{AddressCodec, decode_payment_address, encode_payment_address};
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;
use zcash_primitives::sapling::PaymentAddress;
use crate::{AccountData, DbAdapter};

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
        write!(f, "DecodedUA: {:?} {:?} {:?}",
            self.transparent.as_ref().map(|a| a.encode(&self.network)),
            self.sapling.as_ref().map(|a| encode_payment_address(self.network.hrp_sapling_payment_address(), a)),
            self.orchard.as_ref().map(|a| {
                let ua = unified::Address(vec![Receiver::Orchard(a.to_raw_address_bytes())]);
                ua.encode(&network2network(&self.network))
            })
        )
    }
}

pub fn get_unified_address(network: &Network, db: &DbAdapter, account: u32, tpe: Option<UnifiedAddressType>) -> anyhow::Result<String> {
    let tpe = tpe.ok_or(anyhow!("")).or_else(|_| db.get_ua_settings(account))?;

    let mut rcvs = vec![];
    if tpe.transparent {
        let address = db.get_taddr(account)?;
        if let Some(address) = address {
            let address = TransparentAddress::decode(network, &address)?;
            if let TransparentAddress::PublicKey(pkh) = address {
                let rcv = Receiver::P2pkh(pkh);
                rcvs.push(rcv);
            }
        }
    }
    if tpe.sapling {
        let AccountData { address , .. } = db.get_account_info(account)?;
        let pa = decode_payment_address(network.hrp_sapling_payment_address(), &address).unwrap();
        let rcv = Receiver::Sapling(pa.to_bytes());
        rcvs.push(rcv);
    }
    if tpe.orchard {
        let okey = db.get_orchard(account)?;
        if let Some(okey) = okey {
            let fvk = FullViewingKey::from_bytes(&okey.fvk).unwrap();
            let address = fvk.address_at(0usize, Scope::External);
            let rcv = Receiver::Orchard(address.to_raw_address_bytes());
            rcvs.push(rcv);
        }
    }

    let addresses = unified::Address(rcvs);
    let unified_address = ZcashAddress::from_unified(network2network(network), addresses);
    Ok(unified_address.encode())
}

pub fn decode_unified_address(network: &Network, ua: &str) -> anyhow::Result<DecodedUA> {
    let mut decoded_ua = DecodedUA {
        network: network.clone(),
        transparent: None,
        sapling: None,
        orchard: None
    };
    let network = network2network(network);
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

fn network2network(n: &Network) -> zcash_address::Network { n.address_network().unwrap() }

// u1pncsxa8jt7aq37r8uvhjrgt7sv8a665hdw44rqa28cd9t6qqmktzwktw772nlle6skkkxwmtzxaan3slntqev03g70tzpky3c58hfgvfjkcky255cwqgfuzdjcktfl7pjalt5sl33se75pmga09etn9dplr98eq2g8cgmvgvx6jx2a2xhy39x96c6rumvlyt35whml87r064qdzw30e