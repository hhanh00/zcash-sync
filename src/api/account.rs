//! Account related API

// Account creation

use crate::coinconfig::CoinConfig;
use crate::db::data_generated::fb::{
    AddressBalance, AddressBalanceArgs, AddressBalanceVec, AddressBalanceVecArgs, Backup,
    BackupArgs, BackupT,
};
use crate::db::AccountData;
use crate::key2::decode_key;
use crate::orchard::OrchardKeyBytes;
use crate::taddr::{derive_taddr, derive_tkeys};
use crate::unified::UnifiedAddressType;
use crate::zip32::derive_zip32;
use crate::KeyPack;
use anyhow::anyhow;
use bip39::{Language, Mnemonic};
use orchard::keys::{FullViewingKey, Scope};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::Serialize;
use zcash_address::unified::{Address as UA, Receiver};
use zcash_address::{ToAddress, ZcashAddress};
use zcash_client_backend::encoding::{decode_extended_full_viewing_key, encode_payment_address};
use zcash_client_backend::keys::UnifiedFullViewingKey;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::zip32::DiversifierIndex;

/// Create a new account
/// # Arguments
///
/// * `coin`: 0 for zcash, 1 for ycash
/// * `name`: account name
/// * `key`: `Some(key)` where key is either a passphrase,
/// a secret key or a viewing key for an existing account,
/// or `None` for a new randomly generated account
/// * `index`: `Some(x)` for account at index `x` or
/// `None` for main account (same as x = 0)
///
/// # Returns
/// `account id`
pub fn new_account(
    coin: u8,
    name: &str,
    key: Option<String>,
    index: Option<u32>,
) -> anyhow::Result<u32> {
    let key = match key {
        Some(key) => key,
        None => {
            let mut entropy = [0u8; 32];
            OsRng.fill_bytes(&mut entropy);
            let mnemonic = Mnemonic::from_entropy(&entropy, Language::English)?;
            mnemonic.phrase().to_string()
        }
    };
    let id_account = new_account_with_key(coin, name, &key, index.unwrap_or(0))?;
    Ok(id_account)
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
pub fn new_sub_account(name: &str, index: Option<u32>, count: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let AccountData { seed, .. } = db.get_account_info(c.id_account)?;
    let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
    let index = index.unwrap_or_else(|| db.next_account_id(&seed).unwrap());
    drop(db);
    for i in 0..count {
        new_account_with_key(c.coin, name, &seed, index + i)?;
    }
    Ok(())
}

fn new_account_with_key(coin: u8, name: &str, key: &str, index: u32) -> anyhow::Result<u32> {
    let c = CoinConfig::get(coin);
    let (seed, sk, ivk, pa, ofvk) = decode_key(coin, key, index)?;
    let db = c.db()?;
    let account = db.get_account_id(&ivk)?;
    let account = match account {
        Some(account) => account,
        None => {
            let account =
                db.store_account(name, seed.as_deref(), index, sk.as_deref(), &ivk, &pa)?;
            if c.chain.has_transparent() {
                db.create_taddr(account)?;
            }
            if c.chain.has_unified() {
                match ofvk {
                    Some(fvk) => {
                        db.store_orchard_fvk(account, &fvk.to_bytes())?;
                    }
                    None => {
                        db.create_orchard(account)?;
                    }
                }
            }
            db.store_ua_settings(account, false, true, c.chain.has_unified())?;
            account
        }
    };
    Ok(account)
}

pub fn convert_to_watchonly(coin: u8, id_account: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.convert_to_watchonly(id_account)?;
    Ok(())
}

pub fn get_backup_package(coin: u8, id_account: u32) -> anyhow::Result<BackupT> {
    let c = CoinConfig::get(coin);
    let network = c.chain.network();
    let db = c.db()?;
    let AccountData {
        name,
        seed,
        sk,
        fvk,
        aindex,
        ..
    } = db.get_account_info(id_account)?;
    let orchard_keys = db.get_orchard(id_account)?;
    let uvk = orchard_keys.map(|OrchardKeyBytes { fvk: ofvk, .. }| {
        // orchard sk is not serializable and must derived from seed
        let sapling_efvk =
            decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk)
                .unwrap();
        let sapling_dfvk = sapling_efvk.to_diversifiable_full_viewing_key();
        let orchard_fvk = orchard::keys::FullViewingKey::from_bytes(&ofvk);
        let ufvk = UnifiedFullViewingKey::new(Some(sapling_dfvk), orchard_fvk).unwrap();
        ufvk.encode(network)
    });
    let backup = BackupT {
        name: Some(name),
        seed,
        index: aindex,
        sk,
        fvk: Some(fvk),
        uvk,
    };
    Ok(backup)
}

/// Update the transparent secret key for the given account from a derivation path
///
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]
/// * `path`: derivation path
///
/// Account must have a seed phrase
pub fn import_transparent_key(coin: u8, id_account: u32, path: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let AccountData { seed, .. } = db.get_account_info(c.id_account)?;
    let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
    let (sk, addr) = derive_tkeys(c.chain.network(), &seed, path)?;
    db.store_transparent_key(id_account, &sk, &addr)?;
    Ok(())
}

/// Update the transparent secret key for the given account
///
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]
/// * `sk`: secret key
pub fn import_transparent_secret_key(coin: u8, id_account: u32, sk: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let (_, addr) = derive_taddr(c.chain.network(), sk)?;
    db.store_transparent_key(id_account, &sk, &addr)?;
    Ok(())
}

/// Generate a new diversified address
pub fn get_diversified_address(ua_type: u8, time: u32) -> anyhow::Result<String> {
    let ua_type = ua_type & 6; // don't include transparent component
    if ua_type == 0 {
        anyhow::bail!("Must include a shielded receiver");
    }
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let AccountData { fvk, .. } = db.get_account_info(c.id_account)?;
    let fvk = decode_extended_full_viewing_key(
        c.chain.network().hrp_sapling_extended_full_viewing_key(),
        &fvk,
    )
    .map_err(|_| anyhow!("Bech32 Decode Error"))?;
    let mut di = [0u8; 11];
    di[4..8].copy_from_slice(&time.to_le_bytes());
    let diversifier_index = DiversifierIndex(di);
    let (_, pa) = fvk
        .find_address(diversifier_index)
        .ok_or_else(|| anyhow::anyhow!("Cannot generate new address"))?;

    let orchard_keys = db.get_orchard(c.id_account)?;
    if ua_type == 2 || orchard_keys.is_none() {
        // sapling only
        return Ok(encode_payment_address(
            c.chain.network().hrp_sapling_payment_address(),
            &pa,
        ));
    }

    let orchard_keys = orchard_keys.unwrap();
    let mut receivers = vec![];
    if ua_type & 2 != 0 {
        receivers.push(Receiver::Sapling(pa.to_bytes()));
    }
    if ua_type & 4 != 0 {
        let orchard_fvk = FullViewingKey::from_bytes(&orchard_keys.fvk).unwrap();
        let index = diversifier_index.0; // any sapling index is fine for orchard
        let orchard_address = orchard_fvk.address_at(index, Scope::External);
        receivers.push(Receiver::Orchard(orchard_address.to_raw_address_bytes()));
    }

    let unified_address = UA(receivers);
    let address = ZcashAddress::from_unified(
        c.chain.network().address_network().unwrap(),
        unified_address,
    );
    let address = address.encode();
    Ok(address)
}

/// Retrieve the transparent balance for the current account from the LWD server
pub async fn get_taddr_balance_default() -> anyhow::Result<u64> {
    let c = CoinConfig::get_active();
    get_taddr_balance(c.coin, c.id_account).await
}

/// Retrieve the transparent balance from the LWD server
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]
pub async fn get_taddr_balance(coin: u8, id_account: u32) -> anyhow::Result<u64> {
    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let address = c.db()?.get_taddr(id_account)?;
    let balance = match address {
        None => 0u64,
        Some(address) => crate::taddr::get_taddr_balance(&mut client, &address).await?,
    };
    Ok(balance)
}

/// Look for accounts that have some transparent balance. Stop when the gap limit
/// is exceeded and no balance was found
/// # Arguments
/// * `gap_limit`: number of accounts with 0 balance before the scan stops
pub async fn scan_transparent_accounts(gap_limit: usize) -> anyhow::Result<Vec<u8>> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    let addresses =
        crate::taddr::scan_transparent_accounts(c.chain.network(), &mut client, gap_limit).await?;
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut addrs = vec![];
    for a in addresses {
        let address = builder.create_string(&a.address);
        let ab = AddressBalance::create(
            &mut builder,
            &AddressBalanceArgs {
                index: a.index,
                address: Some(address),
                balance: a.balance,
            },
        );
        addrs.push(ab);
    }
    let addrs = builder.create_vector(&addrs);
    let addrs = AddressBalanceVec::create(
        &mut builder,
        &AddressBalanceVecArgs {
            values: Some(addrs),
        },
    );
    builder.finish(addrs, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

/// Get the backup string. It is either the passphrase, the secret key or the viewing key
/// depending on how the account was created
/// # Arguments
/// * `id_account`: account id as returned from [new_account]
///
/// Use the current active coin
pub fn get_backup(account: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let AccountData { seed, sk, fvk, .. } = c.db()?.get_account_info(account)?;
    if let Some(seed) = seed {
        return Ok(seed);
    }
    if let Some(sk) = sk {
        return Ok(sk);
    }
    Ok(fvk)
}

/// Get the secret key. Returns empty string if the account has no secret key
/// # Arguments
/// * `id_account`: account id as returned from [new_account]
///
/// Use the current active coin
pub fn get_sk(account: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let AccountData { sk, .. } = c.db()?.get_account_info(account)?;
    Ok(sk.unwrap_or(String::new()))
}

/// Reset the database
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
pub fn reset_db(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.reset_db()
}

/// Truncate all non account data for the current active coin
pub fn truncate_data() -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    db.truncate_data()
}

/// Truncate all synchronization data for the current active coin
pub fn truncate_sync_data() -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    db.truncate_sync_data()
}

/// Delete an account
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]
pub fn delete_account(coin: u8, account: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.delete_account(account)?;
    Ok(())
}

/// Import a ZWL data file
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `name`: prefix for the imported accounts
/// * `data`: data file
pub fn import_from_zwl(coin: u8, name: &str, data: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let sks = crate::misc::read_zwl(data)?;
    let db = c.db()?;
    for (i, key) in sks.iter().enumerate() {
        let name = format!("{}-{}", name, i + 1);
        let (seed, sk, ivk, pa, _ufvk) = decode_key(coin, key, 0)?;
        db.store_account(&name, seed.as_deref(), 0, sk.as_deref(), &ivk, &pa)?;
    }
    Ok(())
}

/// Derive keys using Zip-32
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]. Must have a passphrase
/// * `account`: derived account index
/// * `external`: external/internal
/// * `address`: address index
pub fn derive_keys(
    coin: u8,
    id_account: u32,
    account: u32,
    external: u32,
    address: Option<u32>,
) -> anyhow::Result<KeyPack> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let AccountData { seed, .. } = db.get_account_info(id_account)?;
    let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
    derive_zip32(c.chain.network(), &seed, account, external, address)
}

/// Get the Unified address
/// # Arguments
/// * `coin`: 0 for zcash, 1 for ycash
/// * `id_account`: account id as returned from [new_account]
/// * t, s, o: include transparent, sapling, orchard receivers?
///
/// The address depends on the UA settings and may include transparent, sapling & orchard receivers
pub fn get_unified_address(coin: u8, id_account: u32, address_type: u8) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let tpe = UnifiedAddressType {
        transparent: address_type & 1 != 0,
        sapling: address_type & 2 != 0,
        orchard: address_type & 4 != 0,
    };
    let address = crate::get_unified_address(c.chain.network(), &db, id_account, Some(tpe))?; // use ua settings
    Ok(address)
}

fn get_sapling_address(coin: u8, id_account: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let AccountData { address, .. } = db.get_account_info(id_account)?;
    Ok(address)
}

pub fn get_address(coin: u8, id_account: u32, address_type: u8) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let address = if c.chain.has_unified() {
        get_unified_address(coin, id_account, address_type)?
    } else {
        get_sapling_address(coin, id_account)?
    };
    Ok(address)
}

/// Decode a unified address into its receivers
///
/// For testing only. The format of the returned value is subject to change
pub fn decode_unified_address(coin: u8, address: &str) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let res = crate::decode_unified_address(c.chain.network(), address)?;
    Ok(res.to_string())
}
