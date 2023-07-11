use crate::{connect_lightwalletd, has_unified};
use crate::db::data_generated::fb::{AccountDetailsT, AddressBalanceT, AddressBalanceVecT};
use crate::unified::UnifiedAddressType;
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use zcash_primitives::consensus::Network;
use crate::key::decode_key;

pub fn get_unified_address(
    network: &Network,
    connection: &Connection,
    account: u32,
    address_type: u8,
) -> Result<String> {
    let tpe = UnifiedAddressType {
        transparent: address_type & 1 != 0,
        sapling: address_type & 2 != 0,
        orchard: address_type & 4 != 0,
    };
    let address = crate::get_unified_address(network, connection, account, Some(tpe))?; // use ua settings
    Ok(address)
}

/// Look for accounts that have some transparent balance. Stop when the gap limit
/// is exceeded and no balance was found
/// # Arguments
/// * `gap_limit`: number of accounts with 0 balance before the scan stops
pub async fn scan_transparent_accounts(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    gap_limit: usize,
) -> Result<AddressBalanceVecT> {
    let seed = crate::db::account::get_account(connection, account)?
        .and_then(|d| d.seed)
        .ok_or(anyhow!("No seed"))?;
    let mut client = connect_lightwalletd(url).await?;
    let addresses =
        crate::taddr::scan_transparent_accounts(network, &mut client, &seed, aindex, gap_limit)
            .await?;
    let addresses: Vec<_> = addresses
        .into_iter()
        .map(|a| AddressBalanceT {
            index: a.index,
            address: Some(a.address),
            balance: a.balance,
        })
        .collect();
    let addresses = AddressBalanceVecT {
        values: Some(addresses),
    };
    Ok(addresses)
}

/// Import a ZWL data file
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `name`: prefix for the imported accounts
/// * `data`: data file
pub fn import_from_zwl(connection: &Connection, name: &str, data: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let sks = crate::misc::read_zwl(data)?;
    for (i, key) in sks.iter().enumerate() {
        let name = format!("{}-{}", name, i + 1);
        let (seed, sk, ivk, pa, _ufvk) = crate::key::decode_key(coin, key, 0)?;
        db.store_account(&name, seed.as_deref(), 0, sk.as_deref(), &ivk, &pa)?;
    }
    Ok(())
}

/// Create one or many sub accounts of the current account
///
/// # Example
/// ```rust
/// crate::api::account::new_sub_account("test", None, 5)
/// ```
///
/// # Arguments
/// * `name`: name of the sub accounts. Every sub account will have the same name
/// * `index`: Starting index. If `None`, use the index following the highest used index
/// * `count`: Number of subaccounts to create
pub fn new_sub_account(network: &Network, connection: &Connection, account: u32, name: &str, index: Option<u32>, count: u32) -> anyhow::Result<()> {
    let details = crate::db::account::get_account(connection, account)?.ok_or("No account")?;
    let seed = details.seed.ok_or_else(|| anyhow!("Account has no seed"))?;
    let index = index.unwrap_or_else(|| crate::db::account::get_next_aindex(connection, &seed)?);
    for i in 0..count {
        new_account_with_seed(network, connection, name, &seed, index + i)?;
    }
    Ok(())
}

fn new_account_with_seed(network: &Network, connection: &Connection, name: &str, seed: &str, index: u32) -> anyhow::Result<u32> {
    // derive the address for this seed at this index
    let (_, _, _, address, _) = decode_key(network, seed, index)?;
    let account = crate::db::account::get_account_by_address(connection, &address)?;
    let account = match account {
        Some(account) => account,
        None => {
            let account = crate::db::account::store_account(connection, name, seed.as_deref(), index, sk.as_deref(), &ivk, &pa)?;
            crate::db::transparent::create_taddr(network, connection, account)?;
            if has_unified(network) {
                crate::db::orchard::create_orchard(network, connection, account)?;
            }
            crate::db::orchard::store_ua_settings(connection, account, true, true, has_unified(network))?;
            account
        }
    };
    Ok(account)
}

