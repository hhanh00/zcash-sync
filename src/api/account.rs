// Account creation

use crate::coinconfig::CoinConfig;
use crate::key2::decode_key;
use crate::taddr::{derive_taddr, derive_tkeys};
use crate::{derive_zip32, KeyPack};
use anyhow::anyhow;
use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use rand::RngCore;
use zcash_client_backend::encoding::{decode_extended_full_viewing_key, encode_payment_address};
use zcash_primitives::consensus::Parameters;

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

pub fn new_sub_account(name: &str, index: Option<u32>, count: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let (seed, _) = db.get_seed(c.id_account)?;
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
    let (seed, sk, ivk, pa) = decode_key(coin, key, index)?;
    let db = c.db()?;
    let (account, exists) =
        db.store_account(name, seed.as_deref(), index, sk.as_deref(), &ivk, &pa)?;
    if !exists {
        db.create_taddr(account)?;
    }
    Ok(account)
}

pub fn import_transparent_key(coin: u8, id_account: u32, path: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let (seed, _) = db.get_seed(c.id_account)?;
    let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
    let (sk, addr) = derive_tkeys(c.chain.network(), &seed, path)?;
    db.store_transparent_key(id_account, &sk, &addr)?;
    Ok(())
}

pub fn import_transparent_secret_key(coin: u8, id_account: u32, sk: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let (sk, addr) = derive_taddr(c.chain.network(), sk)?;
    db.store_transparent_key(id_account, &sk, &addr)?;
    Ok(())
}

pub fn new_diversified_address() -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    let ivk = db.get_ivk(c.id_account)?;
    let fvk = decode_extended_full_viewing_key(
        c.chain.network().hrp_sapling_extended_full_viewing_key(),
        &ivk,
    )?
    .unwrap();
    let mut diversifier_index = db.get_diversifier(c.id_account)?;
    diversifier_index.increment().unwrap();
    let (new_diversifier_index, pa) = fvk
        .find_address(diversifier_index)
        .ok_or_else(|| anyhow::anyhow!("Cannot generate new address"))?;
    db.store_diversifier(c.id_account, &new_diversifier_index)?;
    let pa = encode_payment_address(c.chain.network().hrp_sapling_payment_address(), &pa);
    Ok(pa)
}

pub async fn get_taddr_balance_default() -> anyhow::Result<u64> {
    let c = CoinConfig::get_active();
    get_taddr_balance(c.coin, c.id_account).await
}

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

pub async fn scan_transparent_accounts(gap_limit: usize) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let mut client = c.connect_lwd().await?;
    crate::taddr::scan_transparent_accounts(c.chain.network(), &mut client, gap_limit).await?;
    Ok(())
}

// Account backup

pub fn get_backup(account: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let (seed, sk, ivk) = c.db()?.get_backup(account)?;
    if let Some(seed) = seed {
        return Ok(seed);
    }
    if let Some(sk) = sk {
        return Ok(sk);
    }
    Ok(ivk)
}

pub fn get_sk(account: u32) -> anyhow::Result<String> {
    let c = CoinConfig::get_active();
    let sk = c.db()?.get_sk(account)?;
    Ok(sk)
}

pub fn reset_db(coin: u8) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.reset_db()
}

pub fn truncate_data() -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let db = c.db()?;
    db.truncate_data()
}

pub fn delete_account(coin: u8, account: u32) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.delete_account(account)?;
    Ok(())
}

pub fn import_from_zwl(coin: u8, name: &str, data: &str) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let sks = crate::read_zwl(data)?;
    let db = c.db()?;
    for (i, key) in sks.iter().enumerate() {
        let name = format!("{}-{}", name, i + 1);
        let (seed, sk, ivk, pa) = decode_key(coin, key, 0)?;
        db.store_account(&name, seed.as_deref(), 0, sk.as_deref(), &ivk, &pa)?;
    }
    Ok(())
}

pub fn derive_keys(
    coin: u8,
    id_account: u32,
    account: u32,
    external: u32,
    address: Option<u32>,
) -> anyhow::Result<KeyPack> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    let (seed, _) = db.get_seed(id_account)?;
    let seed = seed.unwrap();
    derive_zip32(c.chain.network(), &seed, account, external, address)
}
