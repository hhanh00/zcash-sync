use crate::db::SpendableNote;
// use crate::wallet::RecipientMemo;
use crate::api::recipient::RecipientMemo;
use crate::chain::get_latest_height;
use crate::coinconfig::CoinConfig;
use crate::{GetAddressUtxosReply, RawTransaction};
use anyhow::anyhow;
use jubjub::Fr;
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use tonic::Request;
use zcash_keys::address::Address;
use zcash_keys::encoding::{
    decode_extended_full_viewing_key, encode_extended_full_viewing_key, encode_payment_address,
};
use crate::coin::{get_coin_chain, CoinChain, CoinType};
use zcash_protocol::consensus::{BlockHeight, NetworkConstants, Parameters};
use sapling::keys::OutgoingViewingKey;
use sapling::prover::{OutputProver, SpendProver};
use zcash_transparent::address::Script;
use zcash_protocol::memo::{Memo, MemoBytes};
use incrementalmerkletree::{witness::IncrementalWitness, Hashable};

use sapling::{Diversifier, Node, PaymentAddress, Rseed};
use zcash_primitives::transaction::builder::{BuildConfig, Builder, Progress};
use zcash_protocol::value::MAX_MONEY;
use zcash_protocol::value::Zatoshis;
use zcash_transparent::builder::TransparentSigningSet;
use zcash_transparent::bundle::{OutPoint, TxOut as ZTxOut};
use zcash_primitives::transaction::fees::fixed::FeeRule;
use sapling::zip32::{ExtendedFullViewingKey, ExtendedSpendingKey};

/// The fixed transaction fee, in zatoshis, charged by this wallet.
const DEFAULT_FEE: u64 = 1000;

#[derive(Serialize, Deserialize, Debug)]
pub struct Tx {
    pub coin_type: CoinType,
    pub height: u32,
    pub t_inputs: Vec<TTxIn>,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub change: String,
    pub ovk: String,
}

impl Tx {
    pub fn new(coin_type: CoinType, height: u32) -> Self {
        Tx {
            coin_type,
            height,
            t_inputs: vec![],
            inputs: vec![],
            outputs: vec![],
            change: "".to_string(),
            ovk: "".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxIn {
    pub diversifier: String,
    pub fvk: String,
    pub amount: u64,
    pub rseed: String,
    pub witness: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TTxIn {
    pub op: String,
    pub n: u32,
    pub amount: u64,
    pub script: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TxOut {
    pub addr: String,
    pub amount: u64,
    pub ovk: String,
    pub memo: String,
}

#[derive(Serialize, Debug)]
pub struct TxSummary {
    pub recipients: Vec<RecipientSummary>,
}

#[derive(Serialize, Debug)]
pub struct RecipientSummary {
    pub address: String,
    pub amount: u64,
}

#[allow(dead_code)]
pub struct TxBuilder {
    pub tx: Tx,
    coin_type: CoinType,
}

#[allow(dead_code)]
impl TxBuilder {
    pub fn new(coin_type: CoinType, height: u32) -> Self {
        TxBuilder {
            coin_type,
            tx: Tx::new(coin_type, height),
        }
    }

    fn add_t_input(&mut self, op: OutPoint, amount: u64, script: &[u8]) {
        self.tx.t_inputs.push(TTxIn {
            op: hex::encode(op.hash()),
            n: op.n(),
            amount,
            script: hex::encode(script),
        });
    }

    fn add_z_input(
        &mut self,
        diversifier: &Diversifier,
        fvk: &ExtendedFullViewingKey,
        amount: Zatoshis,
        rseed: &[u8],
        witness: &[u8],
    ) -> anyhow::Result<()> {
        let tx_in = TxIn {
            diversifier: hex::encode(diversifier.0),
            fvk: encode_extended_full_viewing_key(
                self.chain()
                    .network()
                    .hrp_sapling_extended_full_viewing_key(),
                fvk,
            ),
            amount: u64::from(amount),
            rseed: hex::encode(rseed),
            witness: hex::encode(witness),
        };
        self.tx.inputs.push(tx_in);
        Ok(())
    }

    fn add_t_output(&mut self, address: &str, amount: Zatoshis) -> anyhow::Result<()> {
        let tx_out = TxOut {
            addr: address.to_string(),
            amount: u64::from(amount),
            ovk: String::new(),
            memo: String::new(),
        };
        self.tx.outputs.push(tx_out);
        Ok(())
    }

    fn add_z_output(
        &mut self,
        address: &str,
        ovk: &OutgoingViewingKey,
        amount: Zatoshis,
        memo: &Memo,
    ) -> anyhow::Result<()> {
        let tx_out = TxOut {
            addr: address.to_string(),
            amount: u64::from(amount),
            ovk: hex::encode(ovk.0),
            memo: hex::encode(MemoBytes::from(memo).as_slice()),
        };
        self.tx.outputs.push(tx_out);
        Ok(())
    }

    fn set_change(
        &mut self,
        ovk: &OutgoingViewingKey,
        address: &PaymentAddress,
    ) -> anyhow::Result<()> {
        self.tx.change = encode_payment_address(
            self.chain().network().hrp_sapling_payment_address(),
            address,
        );
        self.tx.ovk = hex::encode(ovk.0);
        Ok(())
    }

    /// Add inputs to the transaction
    ///
    /// Select utxos and shielded notes and add them to
    /// the transaction
    ///
    /// Returns an array of received note ids
    pub fn select_inputs(
        &mut self,
        fvk: &ExtendedFullViewingKey,
        notes: &[SpendableNote],
        utxos: &[GetAddressUtxosReply],
        target_amount: u64,
    ) -> anyhow::Result<Vec<u32>> {
        let mut selected_notes: Vec<u32> = vec![];
        let target_amount = Zatoshis::from_u64(target_amount).unwrap();
        let mut t_amount = Zatoshis::ZERO;
        // If we use the transparent address, we use all the utxos
        if !utxos.is_empty() {
            for utxo in utxos.iter() {
                let mut tx_hash = [0u8; 32];
                tx_hash.copy_from_slice(&utxo.txid);
                let op = OutPoint::new(tx_hash, utxo.index as u32);
                self.add_t_input(op, utxo.value_zat as u64, &utxo.script);
                t_amount = (t_amount + Zatoshis::from_nonnegative_i64(utxo.value_zat).unwrap()).unwrap();
            }
        }
        let target_amount_with_fee = (target_amount
            + Zatoshis::from_u64(DEFAULT_FEE).map_err(|_| anyhow!("Invalid amount"))?)
        .ok_or(anyhow!("Invalid amount"))?;
        if target_amount_with_fee > t_amount {
            // We need to use some shielded notes because the transparent balance is not enough
            let mut amount = (target_amount_with_fee - t_amount).unwrap();

            // Pick spendable notes until we exceed the target_amount_with_fee or we ran out of notes
            let mut notes = notes.to_vec();
            notes.shuffle(&mut OsRng);

            for n in notes.iter() {
                if amount.is_positive() {
                    let a = amount.min(
                        Zatoshis::from_u64(n.note.value().inner())
                            .map_err(|_| anyhow::anyhow!("Invalid amount"))?,
                    );
                    amount = (amount - a).unwrap();
                    let mut witness_bytes: Vec<u8> = vec![];
                    zcash_primitives::merkle_tree::write_incremental_witness(
                        &n.witness,
                        &mut witness_bytes,
                    )?;
                    if let Rseed::BeforeZip212(rseed) = n.note.rseed() {
                        // rseed are stored as pre-zip212
                        self.add_z_input(
                            &n.diversifier,
                            fvk,
                            Zatoshis::from_u64(n.note.value().inner()).unwrap(),
                            &rseed.to_bytes(),
                            &witness_bytes,
                        )?;
                        selected_notes.push(n.id);
                    }
                }
            }

            if amount.is_positive() {
                log::info!("Not enough balance");
                anyhow::bail!(
                    "Not enough balance, need {} zats, missing {} zats",
                    u64::from(target_amount_with_fee),
                    u64::from(amount)
                );
            }
        }

        Ok(selected_notes)
    }

    /// Add outputs
    ///
    /// Expand the recipients if their amount exceeds the max amount per note
    /// Set the change
    pub fn select_outputs(
        &mut self,
        fvk: &ExtendedFullViewingKey,
        recipients: &[RecipientMemo],
    ) -> anyhow::Result<()> {
        let ovk = &fvk.fvk.ovk;
        let (_, change) = fvk.default_address();
        self.set_change(ovk, &change)?;

        for r in recipients.iter() {
            let to_addr = Address::decode(self.chain().network(), &r.address)
                .ok_or(anyhow::anyhow!("Invalid address"))?;
            let memo = &r.memo;

            let amount = Zatoshis::from_u64(r.amount).unwrap();
            let max_amount_per_note = r.max_amount_per_note;
            let max_amount_per_note = if max_amount_per_note != 0 {
                Zatoshis::from_u64(max_amount_per_note).unwrap()
            } else {
                Zatoshis::from_u64(MAX_MONEY).unwrap()
            };

            let mut is_first = true; // make at least an output note
            let mut remaining_amount = amount;
            while remaining_amount.is_positive() || is_first {
                is_first = false;
                let note_amount = remaining_amount.min(max_amount_per_note);
                remaining_amount = (remaining_amount - note_amount).unwrap();

                match &to_addr {
                    Address::Sapling(_pa) => {
                        log::info!("Sapling output: {}", r.amount);
                        self.add_z_output(&r.address, ovk, note_amount, memo)
                    }
                    Address::Transparent(_address) => {
                        self.add_t_output(&r.address, note_amount)
                    }
                    Address::Unified(_ua) => {
                        todo!() // TODO
                    }
                    Address::Tex(_) => Err(anyhow!("TEX addresses are not supported")),
                }?;
            }
        }

        Ok(())
    }

    fn chain(&self) -> &dyn CoinChain {
        get_coin_chain(self.coin_type)
    }
}

impl Tx {
    /// Sign the transaction with the transparent and shielded secret keys
    ///
    /// Returns the raw transaction bytes
    pub fn sign(
        &self,
        tsk: Option<SecretKey>,
        zsk: &ExtendedSpendingKey,
        prover: &(impl SpendProver + OutputProver),
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<Vec<u8>> {
        let chain = get_coin_chain(self.coin_type);
        let last_height = BlockHeight::from_u32(self.height as u32);
        let efvk = zsk.to_extended_full_viewing_key();
        let mut signing_set = TransparentSigningSet::new();

        let transparent_pubkey = if let Some(tsk) = tsk {
            Some(signing_set.add_key(tsk))
        } else if !self.t_inputs.is_empty() {
            anyhow::bail!("Missing secret key of transparent account");
        } else {
            None
        };

        // Collect the Sapling spends and compute the anchor from the first
        // spend's merkle path.
        let mut sapling_anchor = None;
        let mut spends: Vec<(sapling::keys::FullViewingKey, sapling::Note, sapling::MerklePath)> =
            vec![];
        for txin in self.inputs.iter() {
            let mut diversifier = [0u8; 11];
            hex::decode_to_slice(&txin.diversifier, &mut diversifier)?;
            let diversifier = Diversifier(diversifier);
            let fvk = decode_extended_full_viewing_key(
                chain.network().hrp_sapling_extended_full_viewing_key(),
                &txin.fvk,
            )
            .map_err(|_| anyhow!("Bech32 Decode Error"))?;
            if fvk != efvk {
                anyhow::bail!("Incorrect account - Secret key mismatch")
            }
            let pa = fvk.fvk.vk.to_payment_address(diversifier).unwrap();
            let mut rseed_bytes = [0u8; 32];
            hex::decode_to_slice(&txin.rseed, &mut rseed_bytes)?;
            let rseed = Fr::from_bytes(&rseed_bytes).unwrap();
            let note = pa.create_note(
                sapling::value::NoteValue::from_raw(txin.amount),
                Rseed::BeforeZip212(rseed),
            );
            let w = hex::decode(&txin.witness)?;
            let witness =
                zcash_primitives::merkle_tree::read_incremental_witness::<Node, _, 32>(
                    &*w,
                )?;
            let merkle_path = witness.path().unwrap();
            if sapling_anchor.is_none() {
                sapling_anchor = Some(sapling::Anchor::from(merkle_path.root(Node::empty_leaf())));
            }
            spends.push((fvk.fvk, note, merkle_path));
        }

        let build_config = BuildConfig::Standard {
            sapling_anchor,
            orchard_anchor: None,
            ironwood_anchor: None,
            orchard_padding: zcash_primitives::transaction::builder::BundlePadding::DEFAULT,
            ironwood_padding: zcash_primitives::transaction::builder::BundlePadding::DEFAULT,
        };
        let mut builder = Builder::new(*chain.network(), last_height, build_config);

        if let Some(pubkey) = transparent_pubkey {
            for txin in self.t_inputs.iter() {
                let mut txid = [0u8; 32];
                hex::decode_to_slice(&txin.op, &mut txid)?;
                builder
                    .add_transparent_p2pkh_input(
                        pubkey,
                        OutPoint::new(txid, txin.n),
                        ZTxOut::new(
                            Zatoshis::from_u64(txin.amount).unwrap(),
                            Script(zcash_script::script::Code(hex::decode(&txin.script).unwrap())),
                        ),
                    )
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        }

        for (fvk, note, merkle_path) in spends {
            builder
                .add_sapling_spend::<anyhow::Error>(fvk, note, merkle_path)
                .map_err(|e| anyhow!(e.to_string()))?;
        }

        for txout in self.outputs.iter() {
            let recipient = Address::decode(chain.network(), &txout.addr).unwrap();
            let amount = Zatoshis::from_u64(txout.amount).unwrap();
            match recipient {
                Address::Transparent(ta) => {
                    builder
                        .add_transparent_output(&ta, amount)
                        .map_err(|e| anyhow!(e.to_string()))?;
                }
                Address::Sapling(pa) => {
                    let mut ovk = [0u8; 32];
                    hex::decode_to_slice(&txout.ovk, &mut ovk)?;
                    let ovk = OutgoingViewingKey(ovk);
                    let mut memo = vec![0; 512];
                    let m = hex::decode(&txout.memo)?;
                    memo[..m.len()].copy_from_slice(&m);
                    let memo = MemoBytes::from_bytes(&memo)?;
                    builder
                        .add_sapling_output::<anyhow::Error>(Some(ovk), pa, amount, memo)
                        .map_err(|e| anyhow!(e.to_string()))?;
                }
                Address::Unified(_ua) => {
                    todo!() // TODO
                }
                Address::Tex(_) => {}
            }
        }

        let (progress_tx, progress_rx) = mpsc::channel::<Progress>();

        let builder = builder.with_progress_notifier(progress_tx);
        tokio::spawn(async move {
            while let Ok(progress) = progress_rx.recv() {
                log::info!("Progress: {}", progress.cur());
                progress_callback(progress);
            }
        });
        let sapling_extsks = vec![zsk.clone(); self.inputs.len()];
        let fee_rule = FeeRule::non_standard(Zatoshis::from_u64(DEFAULT_FEE).unwrap());
        let build_result = builder.build(
            &signing_set,
            &sapling_extsks,
            &[],
            OsRng,
            prover,
            prover,
            &fee_rule,
        )?;
        let mut raw_tx = vec![];
        build_result.transaction().write(&mut raw_tx)?;

        Ok(raw_tx)
    }
}

/// Broadcast a raw signed transaction to the network
pub async fn broadcast_tx(coin: u8, tx: &[u8]) -> anyhow::Result<String> {
    let c = CoinConfig::get(coin);
    let mut client = c.connect_lwd().await?;
    let latest_height = get_latest_height(&mut client).await?;
    let raw_tx = RawTransaction {
        data: tx.to_vec(),
        height: latest_height as u64,
    };

    let rep = client
        .send_transaction(Request::new(raw_tx))
        .await?
        .into_inner();
    let code = rep.error_code;
    if code == 0 {
        Ok(rep.error_message)
    } else {
        Err(anyhow::anyhow!(rep.error_message))
    }
}

pub fn get_tx_summary(tx: &Tx) -> anyhow::Result<TxSummary> {
    let mut recipients = vec![];
    for tx_out in tx.outputs.iter() {
        recipients.push(RecipientSummary {
            address: tx_out.addr.clone(),
            amount: tx_out.amount,
        });
    }
    Ok(TxSummary { recipients })
}
