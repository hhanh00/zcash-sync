use crate::chain::send_transaction;
use crate::mempool::MemPool;
use crate::scan::ProgressCallback;
use crate::{connect_lightwalletd, get_address, get_latest_height, get_secret_key, get_viewing_key, DbAdapter, NETWORK, BlockId, CTree};
use anyhow::Context;
use bip39::{Language, Mnemonic};
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use rand::RngCore;
use std::sync::Arc;
use tokio::sync::Mutex;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::data_api::wallet::ANCHOR_OFFSET;
use zcash_client_backend::encoding::{decode_extended_spending_key, decode_payment_address};
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_primitives::consensus::{BlockHeight, BranchId, Parameters};
use zcash_primitives::transaction::builder::Builder;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_proofs::prover::LocalTxProver;
use tonic::Request;

pub const DEFAULT_ACCOUNT: u32 = 1;
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

    pub fn valid_seed(seed: &str) -> bool {
        get_secret_key(&seed).is_ok()
    }

    pub fn valid_address(address: &str) -> bool {
        decode_payment_address(NETWORK.hrp_sapling_payment_address(), address).is_ok()
    }

    pub fn new_seed(&self) -> anyhow::Result<()> {
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        let mnemonic = Mnemonic::from_entropy(&entropy, Language::English)?;
        let seed = mnemonic.phrase();
        self.new_account_with_seed(seed)?;
        Ok(())
    }

    pub fn get_seed(&self, account: u32) -> anyhow::Result<String> {
        self.db.get_seed(account)
    }

    pub fn has_account(&self, account: u32) -> anyhow::Result<bool> {
        self.db.has_account(account)
    }

    pub fn new_account_with_seed(&self, seed: &str) -> anyhow::Result<()> {
        let sk = get_secret_key(&seed).unwrap();
        let vk = get_viewing_key(&sk).unwrap();
        let pa = get_address(&vk).unwrap();
        self.db.store_account(seed, &sk, &vk, &pa)?;
        Ok(())
    }

    async fn scan_async(
        ivk: &str,
        db_path: &str,
        chunk_size: u32,
        target_height_offset: u32,
        progress_callback: ProgressCallback,
    ) -> anyhow::Result<()> {
        crate::scan::sync_async(
            ivk,
            chunk_size,
            db_path,
            target_height_offset,
            progress_callback,
        )
        .await
    }

    pub async fn get_latest_height() -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd().await?;
        let last_height = get_latest_height(&mut client).await?;
        Ok(last_height)
    }

    // Not a method in order to avoid locking the instance
    pub async fn sync_ex(
        db_path: &str,
        ivk: &str,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        let cb = Arc::new(Mutex::new(progress_callback));
        Self::scan_async(&ivk, db_path, DEFAULT_CHUNK_SIZE, 10, cb.clone()).await?;
        Self::scan_async(&ivk, db_path, DEFAULT_CHUNK_SIZE, 0, cb.clone()).await?;
        Ok(())
    }

    pub async fn sync(
        &self,
        account: u32,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        let ivk = self.get_ivk(account)?;
        Self::sync_ex(&self.db_path, &ivk, progress_callback).await
    }

    pub async fn skip_to_last_height(&self) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd().await?;
        let last_height = get_latest_height(&mut client).await?;
        let block_id = BlockId {
            height: last_height as u64,
            hash: vec![],
        };
        let block = client.get_block(block_id.clone()).await?.into_inner();
        let tree_state = client.get_tree_state(Request::new(block_id)).await?.into_inner();
        let tree = CTree::read(&*hex::decode(&tree_state.tree)?)?;
        self.db.store_block(last_height, &block.hash, block.time, &tree)?;

        Ok(())
    }

    pub fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        self.db.trim_to_height(height)
    }

    pub async fn get_balance(&self, mempool: &MemPool) -> anyhow::Result<WalletBalance> {
        let last_height = Self::get_latest_height().await?;
        let anchor_height = last_height - ANCHOR_OFFSET;

        let confirmed = self.db.get_balance()?;
        let unconfirmed = mempool.get_unconfirmed_balance();
        let spendable = self.db.get_spendable_balance(anchor_height)?;
        Ok(WalletBalance {
            confirmed,
            unconfirmed,
            spendable,
        })
    }

    pub async fn send_payment(
        &self,
        account: u32,
        to_address: &str,
        amount: u64,
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
        let mut notes = self.db.get_spendable_notes(anchor_height, &extfvk)?;
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
            return Ok("".to_string());
        }

        log::info!("Preparing tx");
        builder.send_change_to(Some(ovk), change_address);
        match to_addr {
            RecipientAddress::Shielded(pa) => {
                builder.add_sapling_output(Some(ovk), pa, target_amount, None)
            }
            RecipientAddress::Transparent(t_address) => {
                builder.add_transparent_output(&t_address, target_amount)
            }
        }?;

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
    use crate::wallet::Wallet;
    use crate::{get_address, get_secret_key, get_viewing_key};

    #[tokio::test]
    async fn test_wallet_seed() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let wallet = Wallet::new("zec.db");
        wallet.new_account_with_seed(&seed).unwrap();
    }

    #[tokio::test]
    async fn test_payment() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let sk = get_secret_key(&seed).unwrap();
        let vk = get_viewing_key(&sk).unwrap();
        println!("{}", vk);
        let pa = get_address(&vk).unwrap();
        println!("{}", pa);
        let wallet = Wallet::new("zec.db");

        let tx_id = wallet.send_payment(1, &pa, 1000).await.unwrap();
        println!("TXID = {}", tx_id);
    }
}
