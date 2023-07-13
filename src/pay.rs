use std::str::FromStr;
use crate::db::SpendableNote;
// use crate::wallet::RecipientMemo;
use crate::api::recipient::RecipientMemo;
use crate::chain::{get_checkpoint_height, get_latest_height, EXPIRY_HEIGHT_OFFSET};
use crate::{build_tx, connect_lightwalletd, db, GetAddressUtxosReply, RawTransaction, TransactionBuilderError, TransactionPlan};
use anyhow::{anyhow, Result};
use jubjub::Fr;
use rand::prelude::SliceRandom;
use rand::rngs::OsRng;
use rusqlite::Connection;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use tonic::Request;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_extended_full_viewing_key, encode_payment_address,
};
use zcash_client_backend::zip321::{Payment, TransactionRequest};
use zcash_params::coin::{get_coin_chain, CoinChain, CoinType};
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::keys::OutgoingViewingKey;
use zcash_primitives::legacy::Script;
use zcash_primitives::memo::{Memo, MemoBytes};
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::prover::TxProver;
use zcash_primitives::sapling::{Diversifier, Node, PaymentAddress, Rseed};
use zcash_primitives::transaction::builder::{Builder, Progress};
use zcash_primitives::transaction::components::amount::{DEFAULT_FEE, MAX_MONEY};
use zcash_primitives::transaction::components::{Amount, OutPoint, TxOut as ZTxOut};
use zcash_primitives::transaction::fees::fixed::FeeRule;
use zcash_primitives::zip32::{ExtendedFullViewingKey, ExtendedSpendingKey};
use crate::db::data_generated::fb::PaymentURIT;
use crate::unified::UnifiedAddressType;

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
        amount: Amount,
        rseed: &[u8],
        witness: &[u8],
    ) -> Result<()> {
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

    fn add_t_output(&mut self, address: &str, amount: Amount) -> Result<()> {
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
        amount: Amount,
        memo: &Memo,
    ) -> Result<()> {
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
    ) -> Result<()> {
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
    ) -> Result<Vec<u32>> {
        let mut selected_notes: Vec<u32> = vec![];
        let target_amount = Amount::from_u64(target_amount).unwrap();
        let mut t_amount = Amount::zero();
        // If we use the transparent address, we use all the utxos
        if !utxos.is_empty() {
            for utxo in utxos.iter() {
                let mut tx_hash = [0u8; 32];
                tx_hash.copy_from_slice(&utxo.txid);
                let op = OutPoint::new(tx_hash, utxo.index as u32);
                self.add_t_input(op, utxo.value_zat as u64, &utxo.script);
                t_amount += Amount::from_i64(utxo.value_zat).unwrap();
            }
        }
        let target_amount_with_fee =
            (target_amount + DEFAULT_FEE).ok_or(anyhow!("Invalid amount"))?;
        if target_amount_with_fee > t_amount {
            // We need to use some shielded notes because the transparent balance is not enough
            let mut amount = (target_amount_with_fee - t_amount).unwrap();

            // Pick spendable notes until we exceed the target_amount_with_fee or we ran out of notes
            let mut notes = notes.to_vec();
            notes.shuffle(&mut OsRng);

            for n in notes.iter() {
                if amount.is_positive() {
                    let a = amount.min(
                        Amount::from_u64(n.note.value().inner())
                            .map_err(|_| anyhow::anyhow!("Invalid amount"))?,
                    );
                    amount -= a;
                    let mut witness_bytes: Vec<u8> = vec![];
                    n.witness.write(&mut witness_bytes)?;
                    if let Rseed::BeforeZip212(rseed) = n.note.rseed {
                        // rseed are stored as pre-zip212
                        self.add_z_input(
                            &n.diversifier,
                            fvk,
                            Amount::from_u64(n.note.value().inner()).unwrap(),
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
    ) -> Result<()> {
        let ovk = &fvk.fvk.ovk;
        let (_, change) = fvk.default_address();
        self.set_change(ovk, &change)?;

        for r in recipients.iter() {
            let to_addr = RecipientAddress::decode(self.chain().network(), &r.address)
                .ok_or(anyhow::anyhow!("Invalid address"))?;
            let memo = &r.memo;

            let amount = Amount::from_u64(r.amount).unwrap();
            let max_amount_per_note = r.max_amount_per_note;
            let max_amount_per_note = if max_amount_per_note != 0 {
                Amount::from_u64(max_amount_per_note).unwrap()
            } else {
                Amount::from_i64(MAX_MONEY).unwrap()
            };

            let mut is_first = true; // make at least an output note
            let mut remaining_amount = amount;
            while remaining_amount.is_positive() || is_first {
                is_first = false;
                let note_amount = remaining_amount.min(max_amount_per_note);
                remaining_amount -= note_amount;

                match &to_addr {
                    RecipientAddress::Shielded(_pa) => {
                        log::info!("Sapling output: {}", r.amount);
                        self.add_z_output(&r.address, ovk, note_amount, memo)
                    }
                    RecipientAddress::Transparent(_address) => {
                        self.add_t_output(&r.address, note_amount)
                    }
                    RecipientAddress::Unified(_ua) => {
                        todo!() // TODO
                    }
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
        prover: &impl TxProver,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> Result<Vec<u8>> {
        let chain = get_coin_chain(self.coin_type);
        let last_height = BlockHeight::from_u32(self.height as u32);
        let mut builder = Builder::new(*chain.network(), last_height);
        let efvk = zsk.to_extended_full_viewing_key();

        if let Some(tsk) = tsk {
            for txin in self.t_inputs.iter() {
                let mut txid = [0u8; 32];
                hex::decode_to_slice(&txin.op, &mut txid)?;
                builder
                    .add_transparent_input(
                        tsk,
                        OutPoint::new(txid, txin.n),
                        ZTxOut {
                            value: Amount::from_u64(txin.amount).unwrap(),
                            script_pubkey: Script(hex::decode(&txin.script).unwrap()),
                        },
                    )
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        } else if !self.t_inputs.is_empty() {
            anyhow::bail!("Missing secret key of transparent account");
        }

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
            let note = pa.create_note(txin.amount, Rseed::BeforeZip212(rseed));
            let w = hex::decode(&txin.witness)?;
            let witness = IncrementalWitness::<Node>::read(&*w)?;
            let merkle_path = witness.path().unwrap();

            builder
                .add_sapling_spend(zsk.clone(), diversifier, note, merkle_path)
                .map_err(|e| anyhow!(e.to_string()))?;
        }

        for txout in self.outputs.iter() {
            let recipient = RecipientAddress::decode(chain.network(), &txout.addr).unwrap();
            let amount = Amount::from_u64(txout.amount).unwrap();
            match recipient {
                RecipientAddress::Transparent(ta) => {
                    builder
                        .add_transparent_output(&ta, amount)
                        .map_err(|e| anyhow!(e.to_string()))?;
                }
                RecipientAddress::Shielded(pa) => {
                    let mut ovk = [0u8; 32];
                    hex::decode_to_slice(&txout.ovk, &mut ovk)?;
                    let ovk = OutgoingViewingKey(ovk);
                    let mut memo = vec![0; 512];
                    let m = hex::decode(&txout.memo)?;
                    memo[..m.len()].copy_from_slice(&m);
                    let memo = MemoBytes::from_bytes(&memo)?;
                    builder
                        .add_sapling_output(Some(ovk), pa, amount, memo)
                        .map_err(|e| anyhow!(e.to_string()))?;
                }
                RecipientAddress::Unified(_ua) => {
                    todo!() // TODO
                }
            }
        }

        let (progress_tx, progress_rx) = mpsc::channel::<Progress>();

        builder.with_progress_notifier(progress_tx);
        tokio::spawn(async move {
            while let Ok(progress) = progress_rx.recv() {
                log::info!("Progress: {}", progress.cur());
                progress_callback(progress);
            }
        });
        let (tx, _) = builder.build(prover, &FeeRule::standard())?;
        let mut raw_tx = vec![];
        tx.write(&mut raw_tx)?;

        Ok(raw_tx)
    }
}

/// Broadcast a raw signed transaction to the network
pub async fn broadcast_tx(url: &str, tx: &[u8]) -> Result<String> {
    let mut client = connect_lightwalletd(url).await?;
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

pub fn get_tx_summary(tx: &Tx) -> Result<TxSummary> {
    let mut recipients = vec![];
    for tx_out in tx.outputs.iter() {
        recipients.push(RecipientSummary {
            address: tx_out.addr.clone(),
            amount: tx_out.amount,
        });
    }
    Ok(TxSummary { recipients })
}

pub async fn build_tx_plan_with_utxos(
    network: &Network,
    connection: &Connection,
    account: u32,
    checkpoint_height: u32,
    expiry_height: u32,
    recipients: &[RecipientMemo],
    utxos: &[crate::note_selection::UTXO],
) -> crate::note_selection::Result<TransactionPlan> {
    let mut recipient_fee = false;
    for r in recipients {
        if r.fee_included {
            if recipient_fee {
                return Err(TransactionBuilderError::DuplicateRecipientFee);
            }
            recipient_fee = true;
        }
    }

    let taddr = crate::db::transparent::get_transparent(connection, account)?.and_then(|d| d.address).unwrap_or_default();
    let fvk = crate::db::account::get_account(connection, account)?.and_then(|d| d.ivk).unwrap_or_default();
    let orchard_fvk = crate::db::orchard::get_orchard(connection, account)?.map(|d| hex::encode(&d.fvk)).unwrap_or_default();
    let change_address = crate::unified::get_unified_address(network, connection, account, Some(UnifiedAddressType {
        transparent: true,
        sapling: true,
        orchard: true,
    }))?;
    let context = crate::TxBuilderContext::from_height(network, connection, checkpoint_height)?;

    let mut orders = vec![];
    let mut id_order = 0;
    for r in recipients {
        let mut amount = r.amount;
        let max_amount_per_note = if r.max_amount_per_note == 0 {
            u64::MAX
        } else {
            r.max_amount_per_note
        };
        loop {
            let a = std::cmp::min(amount, max_amount_per_note);
            let memo_bytes: MemoBytes = r.memo.clone().into();
            let order = crate::note_selection::Order::new(network, id_order, &r.address, a, false, memo_bytes);
            orders.push(order);
            amount -= a;
            id_order += 1;
            if amount == 0 {
                break;
            } // at least one note even when amount = 0
        }
        orders.last_mut().unwrap().take_fee = r.fee_included;
    }

    let config = crate::TransactionBuilderConfig::new(&change_address);
    let tx_plan = crate::note_selection::build_tx_plan::<crate::note_selection::FeeFlat>(
        network,
        &fvk,
        &taddr,
        &orchard_fvk,
        checkpoint_height,
        expiry_height,
        &context.orchard_anchor,
        &utxos,
        &orders,
        &config,
    )?;
    Ok(tx_plan)
}

pub async fn build_tx_plan(
    network: &Network,
    connection: &Connection,
    url: &str,
    account: u32,
    last_height: u32,
    recipients: &[RecipientMemo],
    excluded_pools: u8,
    confirmations: u32,
) -> crate::note_selection::Result<TransactionPlan> {
    let max_height = last_height.saturating_sub(confirmations);
    let checkpoint_height =
        db::checkpoint::get_last_sync_height(network, connection, Some(max_height))?;
    let expiry_height = last_height + EXPIRY_HEIGHT_OFFSET;
    let utxos = crate::note_selection::fetch_utxos(connection, url, account, checkpoint_height, excluded_pools).await?;
    let tx_plan = build_tx_plan_with_utxos(
        network,
        connection,
        account,
        checkpoint_height,
        expiry_height,
        recipients,
        &utxos,
    )
    .await?;
    Ok(tx_plan)
}

pub fn sign_plan(
    network: &Network,
    connection: &Connection,
    account: u32,
    tx_plan: &TransactionPlan,
) -> Result<Vec<u8>> {
    let z_details = crate::db::account::get_account(connection, account)?.ok_or(anyhow!("No account"))?;
    let fvk = z_details.ivk.ok_or(anyhow!("No FVK"))?;
    let fvk =
        decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk)
            .unwrap()
            .to_diversifiable_full_viewing_key();
    let tx_plan_fvk = decode_extended_full_viewing_key(
        network.hrp_sapling_extended_full_viewing_key(),
        &tx_plan.fvk,
    )
    .unwrap()
    .to_diversifiable_full_viewing_key();

    if fvk.to_bytes() != tx_plan_fvk.to_bytes() {
        return Err(anyhow::anyhow!("Account does not match transaction"));
    }

    let keys = db::key::get_secret_keys(network, connection, account)?;
    let tx = build_tx(network, &keys, &tx_plan, OsRng)?;
    Ok(tx)
}

pub fn mark_inputs_spent(
    connection: &Connection,
    tx_plan: &TransactionPlan,
    spent_height: u32,
) -> Result<()> {
    let id_notes: Vec<_> = tx_plan
        .spends
        .iter()
        .filter_map(|n| if n.id != 0 { Some(n.id) } else { None })
        .collect();
    for id in id_notes {
        db::purge::mark_spent(connection, id, spent_height)?;
    }
    Ok(())
}

/// Build a payment URI
/// # Arguments
/// * `address`: recipient address
/// * `amount`: amount in zats
/// * `memo`: memo text
pub fn make_payment_uri(
    network: &Network,
    scheme: &str,
    address: &str,
    amount: u64,
    memo: &str,
) -> Result<String> {
    let addr = RecipientAddress::decode(network, address).ok_or_else(|| anyhow::anyhow!("Invalid address"))?;
    let payment = Payment {
        recipient_address: addr,
        amount: Amount::from_u64(amount).map_err(|_| anyhow::anyhow!("Invalid amount"))?,
        memo: Some(Memo::from_str(memo)?.into()),
        label: None,
        message: None,
        other_params: vec![],
    };
    let treq = TransactionRequest {
        payments: vec![payment],
    };
    let uri = treq
        .to_uri(network)
        .ok_or_else(|| anyhow::anyhow!("Cannot build Payment URI"))?;
    let uri = format!("{}{}", scheme, &uri[5..]); // hack to replace the URI scheme
    Ok(uri)
}

/// Decode a payment uri
/// # Arguments
/// * `uri`: payment uri
pub fn parse_payment_uri(network: &Network, scheme: &str, uri: &str) -> anyhow::Result<PaymentURIT> {
    let scheme_len = scheme.len();
    if uri[..scheme_len].ne(scheme) {
        anyhow::bail!("Invalid Payment URI: Invalid scheme");
    }
    let uri = format!("zcash{}", &uri[scheme_len..]); // hack to replace the URI scheme
    let treq = TransactionRequest::from_uri(network, &uri)
        .map_err(|e| anyhow::anyhow!("Invalid Payment URI: {:?}", e))?;
    if treq.payments.len() != 1 {
        anyhow::bail!("Invalid Payment URI: Exactly one payee expected")
    }
    let payment = &treq.payments[0];
    let memo = match payment.memo {
        Some(ref memo) => {
            let memo = Memo::try_from(memo.clone())?;
            match memo {
                Memo::Text(text) => Ok(text.to_string()),
                Memo::Empty => Ok(String::new()),
                _ => Err(anyhow::anyhow!("Invalid Memo")),
            }
        }
        None => Ok(String::new()),
    }?;
    let payment = PaymentURIT {
        address: Some(payment.recipient_address.encode(network)),
        amount: u64::from(payment.amount),
        memo: Some(memo),
    };

    Ok(payment)
}

#[derive(Serialize, Deserialize)]
pub struct PaymentURI {
    pub address: String,
    pub amount: u64,
    pub memo: String,
}
