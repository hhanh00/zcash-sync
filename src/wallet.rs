use crate::{NETWORK, get_latest_height, connect_lightwalletd, DbAdapter, get_secret_key, get_viewing_key, get_address};
use zcash_client_backend::address::RecipientAddress;
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::transaction::builder::Builder;
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_primitives::consensus::{Parameters, BlockHeight, BranchId};
use zcash_primitives::zip32::ExtendedFullViewingKey;
use zcash_client_backend::data_api::wallet::ANCHOR_OFFSET;
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use zcash_proofs::prover::LocalTxProver;
use crate::chain::send_transaction;
use zcash_params::{SPEND_PARAMS, OUTPUT_PARAMS};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::scan::ProgressCallback;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;

pub const DEFAULT_ACCOUNT: u32 = 1;
const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub struct Wallet {
    db_path: String,
    db: DbAdapter,
    prover: LocalTxProver,
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

    pub fn new_account_with_seed(&self, seed: &str) -> anyhow::Result<()> {
        let sk = get_secret_key(&seed).unwrap();
        let vk = get_viewing_key(&sk).unwrap();
        let pa = get_address(&vk).unwrap();
        self.db.store_account(&sk, &vk, &pa)?;
        Ok(())
    }

    async fn scan_async(&self, ivk: &str, chunk_size: u32, target_height_offset: u32, progress_callback: ProgressCallback) -> anyhow::Result<()> {
        crate::scan::sync_async(ivk, chunk_size, &self.db_path, target_height_offset, progress_callback).await
    }

    pub async fn get_latest_height() -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd().await?;
        let last_height = get_latest_height(&mut client).await?;
        Ok(last_height)
    }

    pub async fn sync(&self, account: u32, progress_callback: impl Fn(u32) + Send + 'static) -> anyhow::Result<()> {
        let ivk = self.db.get_ivk(account)?;
        let cb = Arc::new(Mutex::new(progress_callback));
        self.scan_async(&ivk, DEFAULT_CHUNK_SIZE, 10, cb.clone()).await?;
        self.scan_async(&ivk, DEFAULT_CHUNK_SIZE, 0, cb.clone()).await?;
        Ok(())
    }

    pub fn get_balance(&self) -> anyhow::Result<u64> {
        self.db.get_balance()
    }

    pub async fn send_payment(&self, account: u32, to_address: &str, amount: u64) -> anyhow::Result<String> {
        let secret_key = self.db.get_sk(account)?;
        let to_addr = RecipientAddress::decode(&NETWORK, to_address)
            .ok_or(anyhow::anyhow!("Invalid address"))?;
        let target_amount = Amount::from_u64(amount).unwrap();
        let skey = decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &secret_key)?.unwrap();
        let extfvk = ExtendedFullViewingKey::from(&skey);
        let (_, change_address) = extfvk.default_address().unwrap();
        let ovk = extfvk.fvk.ovk;
        let last_height = Self::get_latest_height().await?;
        let mut builder = Builder::new(NETWORK, BlockHeight::from_u32(last_height));
        let anchor_height = self.db.get_last_sync_height()?.ok_or_else(|| anyhow::anyhow!("No spendable notes"))?;
        let anchor_height = anchor_height.min(last_height - ANCHOR_OFFSET);
        log::info!("Anchor = {}", anchor_height);
        let mut notes = self.db.get_spendable_notes(anchor_height, &extfvk)?;
        notes.shuffle(&mut OsRng);
        log::info!("Spendable notes = {}", notes.len());

        let mut amount = target_amount;
        amount += DEFAULT_FEE;
        for n in notes {
            if amount.is_positive() {
                let a = amount.min(Amount::from_u64(n.note.value).unwrap());
                amount -= a;
                let merkle_path = n.witness.path().unwrap();
                builder.add_sapling_spend(skey.clone(), n.diversifier, n.note.clone(), merkle_path)?;
            }
        }
        if amount.is_positive() {
            anyhow::bail!("Not enough balance")
        }

        builder.send_change_to(Some(ovk), change_address);
        match to_addr {
            RecipientAddress::Shielded(pa) => builder.add_sapling_output(Some(ovk), pa, target_amount, None),
            RecipientAddress::Transparent(t_address) => builder.add_transparent_output(&t_address, target_amount),
        }?;

        let consensus_branch_id = BranchId::for_height(&NETWORK, BlockHeight::from_u32(last_height));
        let (tx, _) = builder.build(consensus_branch_id, &self.prover)?;
        let mut raw_tx: Vec<u8> = vec![];
        tx.write(&mut raw_tx)?;

        let mut client = connect_lightwalletd().await?;
        let tx_id = send_transaction(&mut client, &raw_tx, last_height).await?;
        Ok(tx_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::{get_secret_key, get_viewing_key, get_address};
    use crate::wallet::Wallet;

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