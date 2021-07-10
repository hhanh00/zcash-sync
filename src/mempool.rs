use crate::chain::to_output_description;
use crate::{
    connect_lightwalletd, get_latest_height, CompactTx, CompactTxStreamerClient, DbAdapter,
    Exclude, NETWORK,
};
use std::collections::HashMap;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_primitives::sapling::note_encryption::try_sapling_compact_note_decryption;
use zcash_primitives::sapling::SaplingIvk;

const DEFAULT_EXCLUDE_LEN: u8 = 1;

struct MemPoolTransacton {
    #[allow(dead_code)]
    balance: i64, // negative if spent
    exclude_len: u8,
}

pub struct MemPool {
    db_path: String,
    account: u32,
    ivk: Option<SaplingIvk>,
    height: BlockHeight,
    transactions: HashMap<Vec<u8>, MemPoolTransacton>,
    nfs: HashMap<Vec<u8>, u64>,
    balance: i64,
    ld_url: String,
}

impl MemPool {
    pub fn new(db_path: &str, ld_url: &str) -> MemPool {
        MemPool {
            db_path: db_path.to_string(),
            account: 0,
            ivk: None,
            height: BlockHeight::from(0),
            transactions: HashMap::new(),
            nfs: HashMap::new(),
            balance: 0,
            ld_url: ld_url.to_string(),
        }
    }

    pub fn set_account(&mut self, account: u32) -> anyhow::Result<()> {
        let db = DbAdapter::new(&self.db_path)?;
        let ivk = db.get_ivk(account)?;
        self.account = account;
        self.set_ivk(&ivk);
        self.clear()?;
        Ok(())
    }

    fn set_ivk(&mut self, ivk: &str) {
        let fvk =
            decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
                .unwrap()
                .unwrap();
        let ivk = fvk.fvk.vk.ivk();
        self.ivk = Some(ivk);
    }

    pub async fn scan(&mut self) -> anyhow::Result<i64> {
        if self.ivk.is_some() {
            let ivk = self.ivk.as_ref().unwrap().clone();
            let mut client = connect_lightwalletd(&self.ld_url).await?;
            let height = BlockHeight::from(get_latest_height(&mut client).await?);
            if self.height != height {
                // New blocks invalidate the mempool
                self.clear()?;
            }
            self.height = height;
            self.update(&mut client, &ivk).await?;
        }

        Ok(self.balance)
    }

    pub fn get_unconfirmed_balance(&self) -> i64 {
        self.balance
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        let db = DbAdapter::new(&self.db_path)?;
        self.height = BlockHeight::from_u32(0);
        self.nfs = db.get_nullifier_amounts(self.account, true)?;
        self.transactions.clear();
        self.balance = 0;
        Ok(())
    }

    async fn update(
        &mut self,
        client: &mut CompactTxStreamerClient<Channel>,
        ivk: &SaplingIvk,
    ) -> anyhow::Result<()> {
        let filter: Vec<_> = self
            .transactions
            .iter()
            .map(|(hash, tx)| {
                let mut hash = hash.clone();
                hash.truncate(tx.exclude_len as usize);
                hash
            })
            .collect();

        let exclude = Exclude { txid: filter };
        let mut txs = client
            .get_mempool_tx(Request::new(exclude))
            .await?
            .into_inner();
        while let Some(tx) = txs.message().await? {
            match self.transactions.get_mut(&*tx.hash) {
                Some(tx) => {
                    tx.exclude_len += 1; // server sent us the same tx: make the filter more specific
                }
                None => {
                    let balance = self.scan_transaction(&tx, ivk);
                    let mempool_tx = MemPoolTransacton {
                        balance,
                        exclude_len: DEFAULT_EXCLUDE_LEN,
                    };
                    self.balance += balance;
                    self.transactions.insert(tx.hash.clone(), mempool_tx);
                }
            }
        }

        Ok(())
    }

    fn scan_transaction(&self, tx: &CompactTx, ivk: &SaplingIvk) -> i64 {
        let mut balance = 0i64;
        for cs in tx.spends.iter() {
            if let Some(&value) = self.nfs.get(&*cs.nf) {
                // nf recognized -> value is spent
                balance -= value as i64;
            }
        }
        for co in tx.outputs.iter() {
            let od = to_output_description(co);
            if let Some((note, _)) =
                try_sapling_compact_note_decryption(&NETWORK, self.height, ivk, &od)
            {
                balance += note.value as i64; // value is incoming
            }
        }

        balance
    }
}

#[cfg(test)]
mod tests {
    use crate::db::DEFAULT_DB_PATH;
    use crate::mempool::MemPool;
    use crate::{DbAdapter, LWD_URL};
    use std::time::Duration;

    #[tokio::test]
    async fn test_mempool() {
        let db = DbAdapter::new(DEFAULT_DB_PATH).unwrap();
        let ivk = db.get_ivk(1).unwrap();
        let mut mempool = MemPool::new("zec.db", LWD_URL);
        mempool.set_ivk(&ivk);
        loop {
            mempool.scan().await.unwrap();
            let unconfirmed = mempool.get_unconfirmed_balance();
            println!("{}", unconfirmed);
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}
