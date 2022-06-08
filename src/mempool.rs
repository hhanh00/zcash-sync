use crate::chain::to_output_description;
use crate::{CompactTx, CompactTxStreamerClient, Exclude};
use std::collections::HashMap;
use tonic::transport::Channel;
use tonic::Request;

use crate::coinconfig::CoinConfig;
use zcash_params::coin::CoinChain;
use zcash_primitives::consensus::BlockHeight;
use zcash_primitives::sapling::note_encryption::try_sapling_compact_note_decryption;
use zcash_primitives::sapling::SaplingIvk;

const DEFAULT_EXCLUDE_LEN: u8 = 1;

struct MemPoolTransacton {
    #[allow(dead_code)]
    balance: i64, // negative if spent
    exclude_len: u8,
}

pub struct MemPool {
    coin: u8,
    transactions: HashMap<Vec<u8>, MemPoolTransacton>,
    nfs: HashMap<Vec<u8>, u64>,
    balance: i64,
}

impl MemPool {
    pub fn new(coin: u8) -> MemPool {
        MemPool {
            coin,
            transactions: HashMap::new(),
            nfs: HashMap::new(),
            balance: 0,
        }
    }

    pub fn get_unconfirmed_balance(&self) -> i64 {
        self.balance
    }

    pub fn clear(&mut self) -> anyhow::Result<()> {
        let c = CoinConfig::get(self.coin);
        self.nfs = c.db()?.get_nullifier_amounts(c.id_account, true)?;
        self.transactions.clear();
        self.balance = 0;
        Ok(())
    }

    pub async fn update(
        &mut self,
        client: &mut CompactTxStreamerClient<Channel>,
        height: u32,
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
                    let balance = self.scan_transaction(height, &tx, ivk);
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

    fn scan_transaction(&self, height: u32, tx: &CompactTx, ivk: &SaplingIvk) -> i64 {
        let c = CoinConfig::get_active();
        let mut balance = 0i64;
        for cs in tx.spends.iter() {
            if let Some(&value) = self.nfs.get(&*cs.nf) {
                // nf recognized -> value is spent
                balance -= value as i64;
            }
        }
        for co in tx.outputs.iter() {
            let od = to_output_description(co);
            if let Some((note, _)) = try_sapling_compact_note_decryption(
                c.chain.network(),
                BlockHeight::from_u32(height),
                ivk,
                &od,
            ) {
                balance += note.value as i64; // value is incoming
            }
        }

        balance
    }
}
