use crate::chain::send_transaction;
use crate::db::SpendableNote;
use crate::key::{decode_key, is_valid_key};
use crate::pay::prepare_tx;
use crate::pay::{ColdTxBuilder, Tx};
use crate::prices::fetch_historical_prices;
use crate::scan::ProgressCallback;
use crate::taddr::{get_taddr_balance, shield_taddr, add_shield_taddr};
use crate::{
    connect_lightwalletd, get_branch, get_latest_height, BlockId, CTree, DbAdapter, NETWORK,
};
use bip39::{Language, Mnemonic};
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::Deserialize;
use std::sync::{mpsc, Arc};
use tokio::sync::Mutex;
use tonic::Request;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key, encode_payment_address,
};
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_primitives::transaction::builder::{Builder, Progress};
use zcash_primitives::transaction::components::amount::{MAX_MONEY, DEFAULT_FEE};
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_proofs::prover::LocalTxProver;
use zcash_primitives::memo::Memo;
use std::str::FromStr;
use crate::contact::{Contact, serialize_contacts};

const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub struct Wallet {
    pub db_path: String,
    db: DbAdapter,
    prover: LocalTxProver,
    pub ld_url: String,
}

#[repr(C)]
pub struct WalletBalance {
    pub confirmed: u64,
    pub unconfirmed: i64,
    pub spendable: u64,
}

impl Default for WalletBalance {
    fn default() -> Self {
        WalletBalance {
            confirmed: 0,
            unconfirmed: 0,
            spendable: 0,
        }
    }
}

#[derive(Deserialize)]
pub struct Recipient {
    pub address: String,
    pub amount: u64,
    pub memo: String,
}

pub struct RecipientMemo {
    pub address: String,
    pub amount: u64,
    pub memo: Memo,
}

impl From<&Recipient> for RecipientMemo {
    fn from(r: &Recipient) -> Self {
        RecipientMemo {
            address: r.address.clone(),
            amount: r.amount,
            memo: Memo::from_str(&r.memo).unwrap(),
        }
    }
}

impl Wallet {
    pub fn new(db_path: &str, ld_url: &str) -> Wallet {
        let prover = LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS);
        let db = DbAdapter::new(db_path).unwrap();
        db.init_db().unwrap();
        Wallet {
            db_path: db_path.to_string(),
            db,
            prover,
            ld_url: ld_url.to_string(),
        }
    }

    pub fn valid_key(key: &str) -> bool {
        is_valid_key(key)
    }

    pub fn valid_address(address: &str) -> bool {
        let recipient = RecipientAddress::decode(&NETWORK, address);
        recipient.is_some()
    }

    pub fn new_account(&self, name: &str, data: &str) -> anyhow::Result<i32> {
        if data.is_empty() {
            let mut entropy = [0u8; 32];
            OsRng.fill_bytes(&mut entropy);
            let mnemonic = Mnemonic::from_entropy(&entropy, Language::English)?;
            let seed = mnemonic.phrase();
            self.new_account_with_key(name, seed)
        } else {
            self.new_account_with_key(name, data)
        }
    }

    pub fn get_backup(&self, account: u32) -> anyhow::Result<String> {
        let (seed, sk, ivk) = self.db.get_backup(account)?;
        if let Some(seed) = seed {
            return Ok(seed);
        }
        if let Some(sk) = sk {
            return Ok(sk);
        }
        Ok(ivk)
    }

    pub fn new_account_with_key(&self, name: &str, key: &str) -> anyhow::Result<i32> {
        let (seed, sk, ivk, pa) = decode_key(key)?;
        let account = self
            .db
            .store_account(name, seed.as_deref(), sk.as_deref(), &ivk, &pa)?;
        if account > 0 {
            self.db.create_taddr(account as u32)?;
        }
        Ok(account)
    }

    async fn scan_async(
        get_tx: bool,
        db_path: &str,
        chunk_size: u32,
        target_height_offset: u32,
        progress_callback: ProgressCallback,
        ld_url: &str,
    ) -> anyhow::Result<()> {
        crate::scan::sync_async(
            chunk_size,
            get_tx,
            db_path,
            target_height_offset,
            progress_callback,
            ld_url,
        )
        .await
    }

    pub async fn get_latest_height(&self) -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;
        Ok(last_height)
    }

    // Not a method in order to avoid locking the instance
    pub async fn sync_ex(
        get_tx: bool,
        anchor_offset: u32,
        db_path: &str,
        progress_callback: impl Fn(u32) + Send + 'static,
        ld_url: &str,
    ) -> anyhow::Result<()> {
        let cb = Arc::new(Mutex::new(progress_callback));
        Self::scan_async(
            get_tx,
            db_path,
            DEFAULT_CHUNK_SIZE,
            anchor_offset,
            cb.clone(),
            ld_url,
        )
        .await?;
        Self::scan_async(get_tx, db_path, DEFAULT_CHUNK_SIZE, 0, cb.clone(), ld_url).await?;
        Ok(())
    }

    pub async fn sync(
        &self,
        get_tx: bool,
        anchor_offset: u32,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        Self::sync_ex(
            get_tx,
            anchor_offset,
            &self.db_path,
            progress_callback,
            &self.ld_url,
        )
        .await
    }

    pub async fn skip_to_last_height(&self) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;
        let block_id = BlockId {
            height: last_height as u64,
            hash: vec![],
        };
        let block = client.get_block(block_id.clone()).await?.into_inner();
        let tree_state = client
            .get_tree_state(Request::new(block_id))
            .await?
            .into_inner();
        let tree = CTree::read(&*hex::decode(&tree_state.tree)?)?;
        self.db
            .store_block(last_height, &block.hash, block.time, &tree)?;

        Ok(())
    }

    pub fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        self.db.trim_to_height(height)
    }

    pub async fn send_multi_payment(
        &mut self,
        account: u32,
        recipients_json: &str,
        anchor_offset: u32,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<String> {
        let recipients: Vec<Recipient> = serde_json::from_str(recipients_json)?;
        let recipients: Vec<_> = recipients.iter().map(|r| RecipientMemo::from(r)).collect();
        self._send_payment(account, &recipients, anchor_offset, false, progress_callback)
            .await
    }

    pub async fn prepare_payment(
        &self,
        account: u32,
        to_address: &str,
        amount: u64,
        memo: &str,
        max_amount_per_note: u64,
        anchor_offset: u32,
    ) -> anyhow::Result<String> {
        let last_height = self.get_latest_height().await?;
        let recipients = Self::_build_recipients(to_address, amount, max_amount_per_note, memo)?;
        let tx = self._prepare_payment(account, amount, last_height, &recipients, anchor_offset)?;
        let tx_str = serde_json::to_string(&tx)?;
        Ok(tx_str)
    }

    fn _prepare_payment(
        &self,
        account: u32,
        amount: u64,
        last_height: u32,
        recipients: &[RecipientMemo],
        anchor_offset: u32,
    ) -> anyhow::Result<Tx> {
        let amount = Amount::from_u64(amount).unwrap();
        let ivk = self.db.get_ivk(account)?;
        let extfvk = decode_extended_full_viewing_key(
            NETWORK.hrp_sapling_extended_full_viewing_key(),
            &ivk,
        )?
        .unwrap();
        let notes = self.get_spendable_notes(account, &extfvk, last_height, anchor_offset)?;
        let mut builder = ColdTxBuilder::new(last_height);
        prepare_tx(&mut builder, None, &notes, amount, &extfvk, recipients)?;
        Ok(builder.tx)
    }

    pub async fn send_payment(
        &mut self,
        account: u32,
        to_address: &str,
        amount: u64,
        memo: &str,
        max_amount_per_note: u64,
        anchor_offset: u32,
        shield_transparent_balance: bool,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<String> {
        let recipients = Self::_build_recipients(to_address, amount, max_amount_per_note, memo)?;
        self._send_payment(account, &recipients, anchor_offset, shield_transparent_balance, progress_callback)
            .await
    }

    async fn _send_payment(
        &mut self,
        account: u32,
        recipients: &[RecipientMemo],
        anchor_offset: u32,
        shield_transparent_balance: bool,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<String> {
        let secret_key = self.db.get_sk(account)?;
        let target_amount = Amount::from_u64(recipients.iter().map(|r| r.amount).sum()).unwrap();
        let skey =
            decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &secret_key)?
                .unwrap();
        let extfvk = ExtendedFullViewingKey::from(&skey);
        let last_height = self.get_latest_height().await?;
        let notes = self.get_spendable_notes(account, &extfvk, last_height, anchor_offset)?;
        log::info!("Spendable notes = {}", notes.len());

        let mut builder = Builder::new(NETWORK, BlockHeight::from_u32(last_height));
        log::info!("Preparing tx");
        let selected_notes = prepare_tx(
            &mut builder,
            Some(skey.clone()),
            &notes,
            target_amount,
            &extfvk,
            recipients,
        )?;

        let (progress_tx, progress_rx) = mpsc::channel::<Progress>();

        builder.with_progress_notifier(progress_tx);
        tokio::spawn(async move {
            while let Ok(progress) = progress_rx.recv() {
                log::info!("Progress: {}", progress.cur());
                progress_callback(progress);
            }
        });

        if shield_transparent_balance {
            add_shield_taddr(&mut builder,
                             &self.db,
                             account,
                             &self.ld_url,
                             Amount::zero()).await?;
        }

        let consensus_branch_id = get_branch(last_height);
        let (tx, _) = builder.build(consensus_branch_id, &self.prover)?;
        log::info!("Tx built");
        let mut raw_tx: Vec<u8> = vec![];
        tx.write(&mut raw_tx)?;

        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let tx_id = send_transaction(&mut client, &raw_tx, last_height).await?;
        log::info!("Tx ID = {}", tx_id);

        let db_tx = self.db.begin_transaction()?;
        for id_note in selected_notes.iter() {
            DbAdapter::mark_spent(*id_note, 0, &db_tx)?;
        }
        db_tx.commit()?;
        Ok(tx_id)
    }

    pub fn get_ivk(&self, account: u32) -> anyhow::Result<String> {
        self.db.get_ivk(account)
    }

    pub fn new_diversified_address(&self, account: u32) -> anyhow::Result<String> {
        let ivk = self.get_ivk(account)?;
        let fvk = decode_extended_full_viewing_key(
            NETWORK.hrp_sapling_extended_full_viewing_key(),
            &ivk,
        )?
        .unwrap();
        let mut diversifier_index = self.db.get_diversifier(account)?;
        diversifier_index.increment().unwrap();
        let (new_diversifier_index, pa) = fvk
            .address(diversifier_index)
            .map_err(|_| anyhow::anyhow!("Cannot generate new address"))?;
        self.db.store_diversifier(account, &new_diversifier_index)?;
        let pa = encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa);
        Ok(pa)
    }

    pub async fn get_taddr_balance(&self, account: u32) -> anyhow::Result<u64> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let address = self.db.get_taddr(account)?;
        let balance = match address {
            None => 0u64,
            Some(address) => get_taddr_balance(&mut client, &address).await?,
        };
        Ok(balance)
    }

    pub async fn shield_taddr(&self, account: u32) -> anyhow::Result<String> {
        shield_taddr(&self.db, account, &self.prover, &self.ld_url).await
    }

    pub fn store_contact(&self, id: u32, name: &str, address: &str, dirty: bool) -> anyhow::Result<()> {
        let contact = Contact {
            id,
            name: name.to_string(),
            address: address.to_string(),
        };
        self.db.store_contact(&contact, dirty)?;
        Ok(())
    }

    pub async fn commit_unsaved_contacts(&self, account: u32, anchor_offset: u32) -> anyhow::Result<String> {
        let contacts = self.db.get_unsaved_contacts()?;
        let memos = serialize_contacts(&contacts)?;
        let tx_id = self.save_contacts_tx(&memos, account, anchor_offset).await.unwrap();
        Ok(tx_id)
    }

    pub async fn save_contacts_tx(&self, memos: &[Memo], account: u32, anchor_offset: u32) -> anyhow::Result<String> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;

        let secret_key = self.db.get_sk(account)?;
        let address = self.db.get_address(account)?;
        let skey = decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &secret_key)?.unwrap();
        let extfvk = ExtendedFullViewingKey::from(&skey);
        let notes = self.get_spendable_notes(account, &extfvk, last_height, anchor_offset)?;

        let mut builder = Builder::new(NETWORK, BlockHeight::from_u32(last_height));

        let recipients: Vec<_> = memos.iter().map(|m| {
            RecipientMemo {
                address: address.clone(),
                amount: 0,
                memo: m.clone(),
            }
        }).collect();
        prepare_tx(&mut builder, Some(skey), &notes, DEFAULT_FEE, &extfvk, &recipients)?;

        let consensus_branch_id = get_branch(last_height);
        let (tx, _) = builder.build(consensus_branch_id, &self.prover)?;
        let mut raw_tx: Vec<u8> = vec![];
        tx.write(&mut raw_tx)?;

        let tx_id = send_transaction(&mut client, &raw_tx, last_height).await?;
        log::info!("Tx ID = {}", tx_id);
        Ok(tx_id)
    }

    pub fn set_lwd_url(&mut self, ld_url: &str) -> anyhow::Result<()> {
        self.ld_url = ld_url.to_string();
        Ok(())
    }

    pub fn get_spendable_notes(
        &self,
        account: u32,
        extfvk: &ExtendedFullViewingKey,
        last_height: u32,
        anchor_offset: u32,
    ) -> anyhow::Result<Vec<SpendableNote>> {
        let anchor_height = self.db
            .get_last_sync_height()?
            .ok_or_else(|| anyhow::anyhow!("No spendable notes"))?;
        let anchor_height = anchor_height.min(last_height - anchor_offset);
        log::info!("Anchor = {}", anchor_height);
        let mut notes = self.db
            .get_spendable_notes(account, anchor_height, extfvk)?;
        notes.shuffle(&mut OsRng);
        log::info!("Spendable notes = {}", notes.len());

        Ok(notes)
    }

    fn _build_recipients(
        to_address: &str,
        amount: u64,
        max_amount_per_note: u64,
        memo: &str,
    ) -> anyhow::Result<Vec<RecipientMemo>> {
        let mut recipients: Vec<RecipientMemo> = vec![];
        let target_amount = Amount::from_u64(amount).unwrap();
        let max_amount_per_note = if max_amount_per_note != 0 {
            Amount::from_u64(max_amount_per_note).unwrap()
        } else {
            Amount::from_i64(MAX_MONEY).unwrap()
        };
        let mut remaining_amount = target_amount;
        while remaining_amount.is_positive() {
            let note_amount = remaining_amount.min(max_amount_per_note);
            let recipient = RecipientMemo {
                address: to_address.to_string(),
                amount: u64::from(note_amount),
                memo: Memo::from_str(memo)?,
            };
            recipients.push(recipient);
            remaining_amount -= note_amount;
        }
        Ok(recipients)
    }

    pub async fn sync_historical_prices(
        &mut self,
        now: i64,
        days: u32,
        currency: &str,
    ) -> anyhow::Result<u32> {
        let quotes = fetch_historical_prices(now, days, currency, &self.db)
            .await?;
        self.db.store_historical_prices(&quotes, currency)?;
        Ok(quotes.len() as u32)
    }

    pub fn delete_account(&self, account: u32) -> anyhow::Result<()> {
        self.db.delete_account(account)?;
        Ok(())
    }

    pub fn truncate_data(&self) -> anyhow::Result<()> {
        self.db.truncate_data()
    }
}

#[cfg(test)]
mod tests {
    use crate::key::derive_secret_key;
    use crate::wallet::Wallet;
    use crate::LWD_URL;
    use bip39::{Language, Mnemonic};

    #[tokio::test]
    async fn test_wallet_seed() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let wallet = Wallet::new("zec.db", LWD_URL);
        wallet.new_account_with_key("test", &seed).unwrap();
    }

    #[tokio::test]
    async fn test_payment() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let (sk, vk, pa) =
            derive_secret_key(&Mnemonic::from_phrase(&seed, Language::English).unwrap()).unwrap();
        println!("{} {} {}", sk, vk, pa);
        // let wallet = Wallet::new("zec.db");
        //
        // let tx_id = wallet.send_payment(1, &pa, 1000).await.unwrap();
        // println!("TXID = {}", tx_id);
    }

    #[test]
    pub fn test_diversified_address() {
        let wallet = Wallet::new("zec.db", LWD_URL);
        let address = wallet.new_diversified_address(1).unwrap();
        println!("{}", address);
    }
}
