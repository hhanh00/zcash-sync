use crate::coinconfig::CoinConfig;
use crate::db::AccountBackup;
use bech32::FromBase32;
use chacha20poly1305::aead::{Aead, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

const NONCE: &[u8; 12] = b"unique nonce";

pub fn get_full_backup(coin: u8) -> anyhow::Result<Vec<AccountBackup>> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.get_full_backup(coin)
}

pub fn restore_full_backup(coin: u8, accounts: &[AccountBackup]) -> anyhow::Result<()> {
    let c = CoinConfig::get(coin);
    let db = c.db()?;
    db.restore_full_backup(accounts)
}

pub fn encrypt_backup(accounts: &[AccountBackup], key: &str) -> anyhow::Result<String> {
    let accounts_bin = bincode::serialize(&accounts)?;
    let backup = if !key.is_empty() {
        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" {
            anyhow::bail!("Invalid backup key")
        }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        // nonce is constant because we always use a different key!
        let cipher_text = cipher
            .encrypt(Nonce::from_slice(NONCE), &*accounts_bin)
            .map_err(|_e| anyhow::anyhow!("Failed to encrypt backup"))?;
        base64::encode(cipher_text)
    } else {
        base64::encode(accounts_bin)
    };
    Ok(backup)
}

pub fn decrypt_backup(key: &str, backup: &str) -> anyhow::Result<Vec<AccountBackup>> {
    let backup = if !key.is_empty() {
        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" {
            anyhow::bail!("Not a valid decryption key");
        }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        let backup = base64::decode(backup)?;
        cipher
            .decrypt(Nonce::from_slice(NONCE), &*backup)
            .map_err(|_e| anyhow::anyhow!("Failed to decrypt backup"))?
    } else {
        base64::decode(backup)?
    };

    let accounts: Vec<AccountBackup> = bincode::deserialize(&backup)?;
    Ok(accounts)
}
