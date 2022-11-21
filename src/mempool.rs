use crate::{AccountData, Empty, Hash, RawTransaction};
use orchard::keys::{FullViewingKey, IncomingViewingKey, Scope};
use orchard::note_encryption::OrchardDomain;
use std::collections::HashMap;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{Receiver, Sender};
use tonic::Request;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_note_encryption::try_note_decryption;

use crate::coinconfig::CoinConfig;
use zcash_primitives::consensus::{BlockHeight, Network, NetworkUpgrade, Parameters};
use zcash_primitives::sapling::note_encryption::{
    try_sapling_note_decryption, PreparedIncomingViewingKey,
};
use zcash_primitives::sapling::SaplingIvk;
use zcash_primitives::transaction::Transaction;

struct MemPoolImpl {
    network: Network,
    nfs: HashMap<Hash, u64>,
    balance: i64,
    pivk: PreparedIncomingViewingKey,
    oivk: Option<IncomingViewingKey>,
}

#[derive(Debug)]
pub enum MemPoolMsg {
    Active(u8, u32),
    Subscribe(u8, u32),
    Balance(u8, u32, i64),
    Close(u8, u32),
}

struct MemPoolHandler {
    coin: u8,
    id_account: u32,
    tx_mesg: Sender<MemPoolMsg>,
}

impl MemPoolHandler {
    pub fn new(coin: u8, id_account: u32, tx_mesg: Sender<MemPoolMsg>) -> Self {
        MemPoolHandler {
            coin,
            id_account,
            tx_mesg,
        }
    }

    pub async fn subscribe(&self) -> anyhow::Result<()> {
        let tx_mesg = self.tx_mesg.clone();
        let coin = self.coin;
        let id_account = self.id_account;
        let c = CoinConfig::get(self.coin);
        let mut client = c.connect_lwd().await?;
        let (nfs, sapling_ivk, orchard_ivk) = {
            let db = c.db()?;
            let nfs = db.get_nullifier_amounts(id_account, true)?;
            let network = c.chain.network();
            let AccountData { fvk, .. } = db.get_account_info(id_account)?;
            let fvk = decode_extended_full_viewing_key(
                network.hrp_sapling_extended_full_viewing_key(),
                &fvk,
            )
            .unwrap();
            let sapling_ivk = fvk.fvk.vk.ivk();
            let orchard_ivk = db.get_orchard(id_account)?.map(|k| {
                let fvk = FullViewingKey::from_bytes(&k.fvk).unwrap();
                fvk.to_ivk(Scope::External)
            });
            (nfs, sapling_ivk, orchard_ivk)
        };
        let mut mempool_impl = MemPoolImpl::new(c.chain.network(), nfs, sapling_ivk, orchard_ivk);
        let mut mempool_stream = client
            .get_mempool_stream(Request::new(Empty {}))
            .await?
            .into_inner();
        tokio::spawn(async move {
            while let Some(raw_tx) = mempool_stream.message().await? {
                let balance = mempool_impl.scan_transaction(&raw_tx)?;
                let _ = tx_mesg
                    .send(MemPoolMsg::Balance(coin, id_account, balance))
                    .await;
            }
            let _ = tx_mesg.send(MemPoolMsg::Close(coin, id_account)).await;
            Ok::<_, anyhow::Error>(())
        });
        Ok(())
    }
}

pub struct MemPool {
    tx_mesg: Sender<MemPoolMsg>,
}

impl MemPool {
    pub fn new(tx_mesg: Sender<MemPoolMsg>) -> Self {
        MemPool { tx_mesg }
    }

    pub async fn set_active(&self, coin: u8, id_account: u32) {
        let _ = self
            .tx_mesg
            .send(MemPoolMsg::Active(coin, id_account))
            .await;
    }
}

pub struct MemPoolRunner {
    runtime: Runtime,
}

impl MemPoolRunner {
    pub fn new() -> Self {
        MemPoolRunner {
            runtime: Runtime::new().unwrap(),
        }
    }

    pub async fn run<F: Fn(i64) + Send + Sync + 'static>(&mut self, f: F) -> MemPool {
        let (tx_mesg, rx_mesg) = tokio::sync::mpsc::channel::<MemPoolMsg>(8);
        let mempool = MemPool::new(tx_mesg.clone());
        self.runtime.spawn(async move {
            run_mempool_loop(tx_mesg, rx_mesg, f).await.unwrap();
        });
        mempool
    }
}

pub async fn run_mempool_loop<F: Fn(i64) + Send + Sync + 'static>(
    tx_mesg: Sender<MemPoolMsg>,
    mut rx_mesg: Receiver<MemPoolMsg>,
    f: F,
) -> anyhow::Result<()> {
    log::info!("MEMPOOL run");
    let mut active_coin = 0;
    let mut active_account = 0;
    let mut subscribed = false;
    while let Some(message) = rx_mesg.recv().await {
        match message {
            MemPoolMsg::Active(coin, id_account) => {
                if coin != active_coin || id_account != active_account {
                    active_coin = coin;
                    active_account = id_account;
                    subscribed = false;
                    let _ = tx_mesg.send(MemPoolMsg::Subscribe(active_coin, active_account)).await;
                }
            }
            MemPoolMsg::Subscribe(coin, id_account) => {
                if !subscribed {
                    let mempool_handler = MemPoolHandler::new(coin, id_account, tx_mesg.clone());
                    mempool_handler.subscribe().await?;
                    subscribed = true;
                }
            }
            MemPoolMsg::Balance(coin, id_account, balance) => {
                if coin == active_coin && id_account == active_account {
                    f(balance);
                }
            }
            MemPoolMsg::Close(coin, id_account) => {
                if coin == active_coin && id_account == active_account {
                    subscribed = false;
                    let _ = tx_mesg
                        .send(MemPoolMsg::Subscribe(active_coin, active_account))
                        .await;
                    f(0);
                }
            }
        }
    }
    Ok(())
}

impl MemPoolImpl {
    pub fn new(
        network: &Network,
        nfs: HashMap<Hash, u64>,
        sivk: SaplingIvk,
        oivk: Option<IncomingViewingKey>,
    ) -> Self {
        let pivk = PreparedIncomingViewingKey::new(&sivk);
        MemPoolImpl {
            network: network.clone(),
            nfs,
            balance: 0,
            pivk,
            oivk,
        }
    }

    fn scan_transaction(&mut self, tx: &RawTransaction) -> anyhow::Result<i64> {
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
                    balance += note.value as i64; // value is incoming
                }
            }
        }
        if let Some(orchard_bundle) = tx.orchard_bundle() {
            if let Some(ref oivk) = self.oivk {
                for a in orchard_bundle.actions().iter() {
                    let nf = a.nullifier().to_bytes();
                    if let Some(&value) = self.nfs.get(&nf) {
                        // nf recognized -> value is spent
                        balance -= value as i64;
                    }
                    let domain = OrchardDomain::for_action(a);
                    if let Some((note, _, _)) = try_note_decryption(&domain, oivk, a) {
                        balance += note.value().inner() as i64; // value is incoming
                    }
                }
            }
        }

        self.balance += balance;
        Ok(self.balance)
    }
}
