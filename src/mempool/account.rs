use std::collections::HashMap;
use anyhow::Result;
use orchard::keys::{FullViewingKey, IncomingViewingKey, Scope};
use orchard::note_encryption::OrchardDomain;
use tokio::sync::mpsc;
use tonic::Request;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::consensus::{BlockHeight, Network, NetworkUpgrade, Parameters};
use zcash_primitives::sapling::keys::PreparedIncomingViewingKey;
use zcash_primitives::sapling::note_encryption::try_sapling_note_decryption;
use zcash_primitives::transaction::Transaction;
use crate::{AccountData, CoinConfig, Empty, Hash, RawTransaction};
use crate::mempool::{AccountId, MPCtl};

pub fn spawn(account_id: AccountId,
             mut rx_close: mpsc::Receiver<()>,
             tx_balance: mpsc::Sender<MPCtl>,
    ) -> Result<()> {
    let AccountId(coin, account) = account_id;
    log::info!("Start sub for {coin} {account}");
    tokio::spawn(async move {
        let c = CoinConfig::get(coin);
        let network = c.chain.network();
        let (nfs, sapling_ivk, orchard_ivk) = {
            let db = c.db()?;
            let nfs = db.get_nullifier_amounts(account, true)?;
            let AccountData { fvk, .. } = db.get_account_info(account)?;
            let fvk = decode_extended_full_viewing_key(
                network.hrp_sapling_extended_full_viewing_key(),
                &fvk,
            ).unwrap();
            let sapling_ivk = fvk.fvk.vk.ivk();
            let orchard_ivk = db.get_orchard(account)?.map(|k| {
                let fvk = FullViewingKey::from_bytes(&k.fvk).unwrap();
                fvk.to_ivk(Scope::External)
            });
            (nfs, sapling_ivk, orchard_ivk)
        };
        let pivk = PreparedIncomingViewingKey::new(&sapling_ivk);
        let mut handler = AccountHandler {
            network: network.clone(),
            nfs,
            balance: 0,
            pivk,
            oivk: orchard_ivk
        };

        let mut client = c.connect_lwd().await?;
        println!("get_mempool_stream");
        let mut mempool_stream = client
            .get_mempool_stream(Request::new(Empty {}))
            .await?
            .into_inner();

        let _ = tx_balance.send(MPCtl::Balance(account_id, 0)).await;
        loop {
            tokio::select! {
                _ = rx_close.recv() => {
                    break;
                }
                m = mempool_stream.message() => {
                    match m? {
                        None => { break; },
                        Some(raw_tx) => {
                            let balance = handler.scan_transaction(&raw_tx)?;
                            let _ = tx_balance.send(MPCtl::Balance(account_id, balance)).await;
                        }
                    }
                }
            }
        }
        println!("MP worker closing");
        Ok::<_, anyhow::Error>(())
    });
    Ok(())
}

struct AccountHandler {
    network: Network,
    nfs: HashMap<Hash, u64>,
    balance: i64,
    pivk: PreparedIncomingViewingKey,
    oivk: Option<IncomingViewingKey>,
}

impl AccountHandler {
    fn scan_transaction(&mut self, tx: &RawTransaction) -> Result<i64> {
        let height = tx.height as u32;
        let mut balance = 0i64;
        let consensus_branch_id = self.network.branch_id(NetworkUpgrade::Nu5);
        let tx = Transaction::read(&tx.data[..], consensus_branch_id)?;
        log::info!("Mempool TXID {}", tx.txid());
        if let Some(sapling_bundle) = tx.sapling_bundle() {
            for cs in sapling_bundle.shielded_spends.iter() {
                let nf = cs.nullifier.0;
                if let Some(&value) = self.nfs.get(&nf) {
                    // nf recognized -> value is spent
                    balance -= value as i64;
                }
            }
            for co in sapling_bundle.shielded_outputs.iter() {
                // let od = to_output_description(co);
                if let Some((note, _, _)) = try_sapling_note_decryption(
                    &self.network,
                    BlockHeight::from_u32(height),
                    &self.pivk,
                    co,
                ) {
                    balance += note.value().inner() as i64; // value is incoming
                }
            }
        }
        if let Some(orchard_bundle) = tx.orchard_bundle() {
            if let Some(ref oivk) = self.oivk {
                let poivk = orchard::keys::PreparedIncomingViewingKey::new(oivk);
                for a in orchard_bundle.actions().iter() {
                    let nf = a.nullifier().to_bytes();
                    if let Some(&value) = self.nfs.get(&nf) {
                        // nf recognized -> value is spent
                        balance -= value as i64;
                    }
                    let domain = OrchardDomain::for_action(a);
                    if let Some((note, _, _)) = try_note_decryption(&domain, &poivk, a) {
                        balance += note.value().inner() as i64; // value is incoming
                    }
                }
            }
        }

        self.balance += balance;
        Ok(self.balance)
    }
}
