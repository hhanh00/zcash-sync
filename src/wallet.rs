use crate::chain::send_transaction;
use crate::key::{decode_key, is_valid_key};
use crate::scan::ProgressCallback;
use crate::{connect_lightwalletd, get_latest_height, BlockId, CTree, DbAdapter, NETWORK};
use anyhow::Context;
use bip39::{Language, Mnemonic};
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use rand::RngCore;
use std::sync::{mpsc, Arc};
use tokio::sync::Mutex;
use tonic::Request;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::data_api::wallet::ANCHOR_OFFSET;
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_primitives::consensus::{BlockHeight, BranchId, Parameters};
use zcash_primitives::transaction::builder::{Builder, Progress};
use zcash_primitives::transaction::components::amount::{DEFAULT_FEE, MAX_MONEY};
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_proofs::prover::LocalTxProver;

const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub struct Wallet {
    pub db_path: String,
    db: DbAdapter,
    prover: LocalTxProver,
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

impl Wallet {
    pub fn new(db_path: &str) -> Wallet {
        let prover = LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS);
        let db = DbAdapter::new(db_path).unwrap();
        db.init_db().unwrap();
        Wallet {
            db_path: db_path.to_string(),
            db,
            prover,
        }
    }

    pub fn valid_key(key: &str) -> bool {
        is_valid_key(key)
    }

    pub fn valid_address(address: &str) -> bool {
        let recipient = RecipientAddress::decode(&NETWORK, address);
        recipient.is_some()
    }

    pub fn new_account(&self, name: &str, data: &str) -> anyhow::Result<u32> {
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

    pub fn new_account_with_key(&self, name: &str, key: &str) -> anyhow::Result<u32> {
        let (seed, sk, ivk, pa) = decode_key(key)?;
        let account = self
            .db
            .store_account(name, seed.as_deref(), sk.as_deref(), &ivk, &pa)?;
        Ok(account)
    }

    async fn scan_async(
        db_path: &str,
        chunk_size: u32,
        target_height_offset: u32,
        progress_callback: ProgressCallback,
    ) -> anyhow::Result<()> {
        crate::scan::sync_async(chunk_size, db_path, target_height_offset, progress_callback).await
    }

    pub async fn get_latest_height() -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd().await?;
        let last_height = get_latest_height(&mut client).await?;
        Ok(last_height)
    }

    // Not a method in order to avoid locking the instance
    pub async fn sync_ex(
        db_path: &str,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        let cb = Arc::new(Mutex::new(progress_callback));
        Self::scan_async(db_path, DEFAULT_CHUNK_SIZE, 10, cb.clone()).await?;
        Self::scan_async(db_path, DEFAULT_CHUNK_SIZE, 0, cb.clone()).await?;
        Ok(())
    }

    pub async fn sync(
        &self,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        Self::sync_ex(&self.db_path, progress_callback).await
    }

    pub async fn skip_to_last_height(&self) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd().await?;
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

    pub async fn send_payment(
        &self,
        account: u32,
        to_address: &str,
        amount: u64,
        max_amount_per_note: u64,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<String> {
        let secret_key = self.db.get_sk(account)?;
        let to_addr = RecipientAddress::decode(&NETWORK, to_address)
            .ok_or(anyhow::anyhow!("Invalid address"))?;
        let target_amount = Amount::from_u64(amount).unwrap();
        let skey =
            decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &secret_key)?
                .unwrap();
        let extfvk = ExtendedFullViewingKey::from(&skey);
        let (_, change_address) = extfvk.default_address().unwrap();
        let ovk = extfvk.fvk.ovk;
        let last_height = Self::get_latest_height().await?;
        let mut builder = Builder::new(NETWORK, BlockHeight::from_u32(last_height));
        let anchor_height = self
            .db
            .get_last_sync_height()?
            .ok_or_else(|| anyhow::anyhow!("No spendable notes"))?;
        let anchor_height = anchor_height.min(last_height - ANCHOR_OFFSET);
        log::info!("Anchor = {}", anchor_height);
        let mut notes = self
            .db
            .get_spendable_notes(account, anchor_height, &extfvk)?;
        notes.shuffle(&mut OsRng);
        log::info!("Spendable notes = {}", notes.len());

        let mut amount = target_amount;
        amount += DEFAULT_FEE;
        let mut selected_note: Vec<u32> = vec![];
        for n in notes.iter() {
            if amount.is_positive() {
                let a = amount.min(
                    Amount::from_u64(n.note.value)
                        .map_err(|_| anyhow::anyhow!("Invalid amount"))?,
                );
                amount -= a;
                let merkle_path = n.witness.path().context("Invalid Merkle Path")?;
                let mut witness_bytes: Vec<u8> = vec![];
                n.witness.write(&mut witness_bytes)?;
                builder.add_sapling_spend(
                    skey.clone(),
                    n.diversifier,
                    n.note.clone(),
                    merkle_path,
                )?;
                selected_note.push(n.id);
            }
        }
        if amount.is_positive() {
            log::info!("Not enough balance");
            anyhow::bail!("Not enough balance");
        }

        log::info!("Preparing tx");
        builder.send_change_to(ovk, change_address);

        let max_amount_per_note = if max_amount_per_note != 0 {
            Amount::from_u64(max_amount_per_note).unwrap()
        } else {
            Amount::from_i64(MAX_MONEY).unwrap()
        };
        let mut remaining_amount = target_amount;
        while remaining_amount.is_positive() {
            let note_amount = target_amount.min(max_amount_per_note);
            match &to_addr {
                RecipientAddress::Shielded(pa) => {
                    builder.add_sapling_output(Some(ovk), pa.clone(), note_amount, None)
                }
                RecipientAddress::Transparent(t_address) => {
                    builder.add_transparent_output(&t_address, note_amount)
                }
            }?;
            remaining_amount -= note_amount;
        }

        let (progress_tx, progress_rx) = mpsc::channel::<Progress>();

        builder.with_progress_notifier(progress_tx);
        tokio::spawn(async move {
            while let Ok(progress) = progress_rx.recv() {
                log::info!("Progress: {}", progress.cur());
                progress_callback(progress);
            }
        });

        let consensus_branch_id =
            BranchId::for_height(&NETWORK, BlockHeight::from_u32(last_height));
        let (tx, _) = builder.build(consensus_branch_id, &self.prover)?;
        log::info!("Tx built");
        let mut raw_tx: Vec<u8> = vec![];
        tx.write(&mut raw_tx)?;

        let mut client = connect_lightwalletd().await?;
        let tx_id = send_transaction(&mut client, &raw_tx, last_height).await?;
        log::info!("Tx ID = {}", tx_id);

        for id_note in selected_note.iter() {
            self.db.mark_spent(*id_note, 0)?;
        }
        Ok(tx_id)
    }

    pub fn get_ivk(&self, account: u32) -> anyhow::Result<String> {
        self.db.get_ivk(account)
    }
}

#[cfg(test)]
mod tests {
    use crate::key::derive_secret_key;
    use crate::wallet::Wallet;
    use bip39::{Language, Mnemonic};

    #[tokio::test]
    async fn test_wallet_seed() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let wallet = Wallet::new("zec.db");
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
}
