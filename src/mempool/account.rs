use crate::mempool::{AccountId, MPCtl};
use crate::{connect_lightwalletd, Empty, Hash, RawTransaction};
use anyhow::{anyhow, Result};
use orchard::keys::{FullViewingKey, IncomingViewingKey, Scope};
use orchard::note_encryption::OrchardDomain;
use std::collections::HashMap;
use rusqlite::Connection;
use tokio::sync::mpsc;
use tonic::Request;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::consensus::{BlockHeight, Network, NetworkUpgrade, Parameters};
use zcash_primitives::sapling::keys::PreparedIncomingViewingKey;
use zcash_primitives::sapling::note_encryption::try_sapling_note_decryption;
use zcash_primitives::transaction::Transaction;

pub fn spawn(
    network: &Network,
    connection: &Connection,
    url: &str,
    coin: u8,
    account: u32,
    mut rx_close: mpsc::Receiver<()>,
    tx_balance: mpsc::Sender<MPCtl>,
) -> Result<()> {
    log::info!("Start sub for {account}");
    let nullifiers = crate::db::checkpoint::list_nullifier_amounts(connection, account, true)?;
    let fvk = crate::db::account::get_account(connection, account)?.and_then(|d| d.ivk).ok_or(anyhow!("No zFVK"))?;
    let ofvk = crate::db::orchard::get_orchard(connection, account)?.map(|d| FullViewingKey::from_bytes(&d.fvk).unwrap().to_ivk(Scope::External));

    tokio::spawn(async move {
        let fvk = decode_extended_full_viewing_key(
            network.hrp_sapling_extended_full_viewing_key(),
            &fvk,
        )
        .unwrap();
        let sapling_ivk = fvk.fvk.vk.ivk();
        let nfs = nullifiers.into_iter().collect();
        let pivk = PreparedIncomingViewingKey::new(&sapling_ivk);
        let mut handler = AccountHandler {
            network: network.clone(),
            nfs,
            balance: 0,
            pivk,
            oivk: ofvk,
        };

        let mut client = connect_lightwalletd(url).await?;
        let mut mempool_stream = client
            .get_mempool_stream(Request::new(Empty {}))
            .await?
            .into_inner();

        let account_id = AccountId(coin, account);
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
