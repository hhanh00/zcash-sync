use zcash_address::unified::{Address, Container, Receiver};
use zcash_address::{FromAddress, Network, ToAddress, UnsupportedAddress, ZcashAddress};

#[derive(Debug, Clone)]
pub struct MyReceiver {
    pub net: Network,
    pub receiver: Receiver,
}

impl FromAddress for MyReceiver {
    fn from_sapling(net: Network, data: [u8; 43]) -> Result<Self, UnsupportedAddress> {
        Ok(MyReceiver {
            net,
            receiver: Receiver::Sapling(data),
        })
    }

    fn from_unified(net: Network, data: Address) -> Result<Self, UnsupportedAddress> {
        for r in data.items_as_parsed().iter() {
            match r {
                Receiver::Sapling(data) => {
                    return Ok(MyReceiver {
                        net,
                        receiver: Receiver::Sapling(*data),
                    });
                }
                _ => (),
            }
        }
        FromAddress::from_unified(net, data)
    }

    fn from_transparent_p2pkh(net: Network, data: [u8; 20]) -> Result<Self, UnsupportedAddress> {
        Ok(MyReceiver {
            net,
            receiver: Receiver::P2pkh(data),
        })
    }
}

pub fn get_ua(_sapling_addr: &str, _transparent_addr: &str) -> anyhow::Result<ZcashAddress> {
    todo!()
    // let sapling_addr = ZcashAddress::try_from_encoded(sapling_addr)?;
    // let transparent_addr = ZcashAddress::try_from_encoded(transparent_addr)?;
    // let receivers: Vec<_> = vec![sapling_addr, transparent_addr]
    //     .iter()
    //     .map(|r| r.clone().convert::<MyReceiver>().unwrap())
    //     .collect();
    // let net = receivers.first().unwrap().net.clone();
    // let receivers: Vec<_> = receivers.iter().map(|r| r.receiver.clone()).collect();
    // let ua: Address = Address::from_inner(receivers)?;
    // let ua_address = ZcashAddress::from_unified(net, ua);
    // Ok(ua_address)
}

pub fn get_sapling(ua_addr: &str) -> anyhow::Result<ZcashAddress> {
    let ua_addr = ZcashAddress::try_from_encoded(ua_addr)?;
    let r = ua_addr.convert::<MyReceiver>()?;
    if let Receiver::Sapling(data) = r.receiver {
        return Ok(ZcashAddress::from_sapling(r.net, data));
    }
    anyhow::bail!("Invalid UA");
}

#[cfg(test)]
mod tests {
    use crate::ua::{get_sapling, get_ua};

    #[test]
    fn test_ua() -> anyhow::Result<()> {
        let ua = get_ua(
            "zs1lvzgfzzwl9n85446j292zg0valw2p47hmxnw42wnqsehsmyuvjk0mhxktcs0pqrplacm2vchh35",
            "t1UWSSWaojmV5dgDhrSfZC6MAfCwVQ9LLoo",
        )?;
        let ua_str = ua.to_string();
        println!("{}", ua);
        let za = get_sapling(&ua_str)?;
        println!("{}", za);

        Ok(())
    }
}

// t1UWSSWaojmV5dgDhrSfZC6MAfCwVQ9LLoo
// zs1lvzgfzzwl9n85446j292zg0valw2p47hmxnw42wnqsehsmyuvjk0mhxktcs0pqrplacm2vchh35
// u16cdcqfguv574pnntjx7dfh78u8m5cu3myxyvs9gedkymstj60u366vpn9qhkcch77e26rzyecyhem7qnzrl7ws2huraj8se8tgek4t3ngn4lfs95l4774mhvgyea4jj93gm92jhg3z7
