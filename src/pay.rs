use crate::db::SpendableNote;
use crate::wallet::Recipient;
use crate::{connect_lightwalletd, get_latest_height, RawTransaction, NETWORK};
use jubjub::Fr;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tonic::Request;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, encode_extended_full_viewing_key,
};
use zcash_primitives::consensus::{BlockHeight, BranchId, Network, Parameters};
use zcash_primitives::memo::{MemoBytes, Memo};
use zcash_primitives::merkle_tree::IncrementalWitness;
use zcash_primitives::sapling::keys::OutgoingViewingKey;
use zcash_primitives::sapling::{Diversifier, Node, Rseed};
use zcash_primitives::transaction::builder::Builder;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;
use zcash_primitives::transaction::components::Amount;
use zcash_primitives::zip32::{ExtendedFullViewingKey, ExtendedSpendingKey};
use zcash_proofs::prover::LocalTxProver;
use std::str::FromStr;
use std::convert::TryFrom;

#[derive(Serialize, Deserialize, Debug)]
pub struct Tx {
    height: u32,
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
}

impl Tx {
    pub fn new(height: u32) -> Self {
        Tx {
            height,
            inputs: vec![],
            outputs: vec![],
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TxIn {
    diversifier: String,
    fvk: String,
    amount: u64,
    rseed: String,
    witness: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct TxOut {
    addr: String,
    amount: u64,
    ovk: String,
    memo: String,
}

pub trait TxBuilder {
    fn add_input(
        &mut self,
        skey: Option<ExtendedSpendingKey>,
        diversifier: &Diversifier,
        fvk: &ExtendedFullViewingKey,
        amount: Amount,
        rseed: &[u8],
        witness: &[u8],
    ) -> anyhow::Result<()>;
    fn add_t_output(&mut self, address: &str, amount: Amount) -> anyhow::Result<()>;
    fn add_z_output(
        &mut self,
        address: &str,
        ovk: &OutgoingViewingKey,
        amount: Amount,
        memo: &MemoBytes,
    ) -> anyhow::Result<()>;
}

pub struct ColdTxBuilder {
    pub tx: Tx,
}

impl ColdTxBuilder {
    pub fn new(height: u32) -> Self {
        ColdTxBuilder {
            tx: Tx::new(height),
        }
    }
}

impl TxBuilder for ColdTxBuilder {
    fn add_input(
        &mut self,
        _skey: Option<ExtendedSpendingKey>,
        diversifier: &Diversifier,
        fvk: &ExtendedFullViewingKey,
        amount: Amount,
        rseed: &[u8],
        witness: &[u8],
    ) -> anyhow::Result<()> {
        let tx_in = TxIn {
            diversifier: hex::encode(diversifier.0),
            fvk: encode_extended_full_viewing_key(
                NETWORK.hrp_sapling_extended_full_viewing_key(),
                &fvk,
            ),
            amount: u64::from(amount),
            rseed: hex::encode(rseed),
            witness: hex::encode(witness),
        };
        self.tx.inputs.push(tx_in);
        Ok(())
    }

    fn add_t_output(&mut self, address: &str, amount: Amount) -> anyhow::Result<()> {
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
        memo: &MemoBytes,
    ) -> anyhow::Result<()> {
        let tx_out = TxOut {
            addr: address.to_string(),
            amount: u64::from(amount),
            ovk: hex::encode(ovk.0),
            memo: hex::encode(memo.as_slice()),
        };
        self.tx.outputs.push(tx_out);
        Ok(())
    }
}

impl TxBuilder for Builder<'_, Network, OsRng> {
    fn add_input(
        &mut self,
        skey: Option<ExtendedSpendingKey>,
        diversifier: &Diversifier,
        fvk: &ExtendedFullViewingKey,
        amount: Amount,
        rseed: &[u8],
        witness: &[u8],
    ) -> anyhow::Result<()> {
        let pa = fvk.fvk.vk.to_payment_address(diversifier.clone()).unwrap();
        let mut rseed_bytes = [0u8; 32];
        rseed_bytes.copy_from_slice(rseed);
        let fr = Fr::from_bytes(&rseed_bytes).unwrap();
        let note = pa
            .create_note(u64::from(amount), Rseed::BeforeZip212(fr))
            .unwrap();
        let witness = IncrementalWitness::<Node>::read(&*witness).unwrap();
        let merkle_path = witness.path().unwrap();
        self.add_sapling_spend(skey.unwrap(), diversifier.clone(), note, merkle_path)?;
        Ok(())
    }

    fn add_t_output(&mut self, address: &str, amount: Amount) -> anyhow::Result<()> {
        let to_addr = RecipientAddress::decode(&NETWORK, address)
            .ok_or(anyhow::anyhow!("Not a valid address"))?;
        if let RecipientAddress::Transparent(t_address) = to_addr {
            self.add_transparent_output(&t_address, amount)?;
        }
        Ok(())
    }

    fn add_z_output(
        &mut self,
        address: &str,
        ovk: &OutgoingViewingKey,
        amount: Amount,
        memo: &MemoBytes,
    ) -> anyhow::Result<()> {
        let to_addr = RecipientAddress::decode(&NETWORK, address)
            .ok_or(anyhow::anyhow!("Not a valid address"))?;
        if let RecipientAddress::Shielded(pa) = to_addr {
            self.add_sapling_output(Some(ovk.clone()), pa.clone(), amount, Some(memo.clone()))?;
        }
        Ok(())
    }
}

pub fn prepare_tx<B: TxBuilder>(
    builder: &mut B,
    skey: Option<ExtendedSpendingKey>,
    notes: &[SpendableNote],
    target_amount: Amount,
    fvk: &ExtendedFullViewingKey,
    recipients: &[Recipient],
) -> anyhow::Result<Vec<u32>> {
    let mut amount = target_amount;
    amount += DEFAULT_FEE;
    let target_amount_with_fee = amount;
    let mut selected_notes: Vec<u32> = vec![];
    for n in notes.iter() {
        if amount.is_positive() {
            let a = amount.min(
                Amount::from_u64(n.note.value).map_err(|_| anyhow::anyhow!("Invalid amount"))?,
            );
            amount -= a;
            let mut witness_bytes: Vec<u8> = vec![];
            n.witness.write(&mut witness_bytes)?;
            if let Rseed::BeforeZip212(rseed) = n.note.rseed {
                // rseed are stored as pre-zip212
                builder.add_input(
                    skey.clone(),
                    &n.diversifier,
                    fvk,
                    Amount::from_u64(n.note.value).unwrap(),
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

    log::info!("Preparing tx");
    let ovk = &fvk.fvk.ovk;

    for r in recipients.iter() {
        let to_addr = RecipientAddress::decode(&NETWORK, &r.address)
            .ok_or(anyhow::anyhow!("Invalid address"))?;
        let amount = Amount::from_u64(r.amount).unwrap();
        match &to_addr {
            RecipientAddress::Shielded(_pa) => {
                log::info!("Sapling output: {}", r.amount);
                let memo = Memo::from_str(&r.memo)?;
                let memo = MemoBytes::try_from(memo)?;
                builder.add_z_output(&r.address, ovk, amount, &memo)
            }
            RecipientAddress::Transparent(_address) => builder.add_t_output(&r.address, amount),
        }?;
    }

    Ok(selected_notes)
}

pub fn sign_offline_tx(tx: &Tx, sk: &ExtendedSpendingKey) -> anyhow::Result<Vec<u8>> {
    let last_height = BlockHeight::from_u32(tx.height as u32);
    let mut builder = Builder::new(NETWORK, last_height);
    for txin in tx.inputs.iter() {
        let mut diversifier = [0u8; 11];
        hex::decode_to_slice(&txin.diversifier, &mut diversifier)?;
        let diversifier = Diversifier(diversifier);
        let fvk = decode_extended_full_viewing_key(
            NETWORK.hrp_sapling_extended_full_viewing_key(),
            &txin.fvk,
        )?
        .unwrap();
        let pa = fvk.fvk.vk.to_payment_address(diversifier).unwrap();
        let mut rseed_bytes = [0u8; 32];
        hex::decode_to_slice(&txin.rseed, &mut rseed_bytes)?;
        let rseed = Fr::from_bytes(&rseed_bytes).unwrap();
        let note = pa
            .create_note(txin.amount, Rseed::BeforeZip212(rseed))
            .unwrap();
        let w = hex::decode(&txin.witness)?;
        let witness = IncrementalWitness::<Node>::read(&*w)?;
        let merkle_path = witness.path().unwrap();

        builder.add_sapling_spend(sk.clone(), diversifier, note, merkle_path)?;
    }
    for txout in tx.outputs.iter() {
        let recipient = RecipientAddress::decode(&NETWORK, &txout.addr).unwrap();
        let amount = Amount::from_u64(txout.amount).unwrap();
        match recipient {
            RecipientAddress::Transparent(ta) => {
                builder.add_transparent_output(&ta, amount)?;
            }
            RecipientAddress::Shielded(pa) => {
                let mut ovk = [0u8; 32];
                hex::decode_to_slice(&txout.ovk, &mut ovk)?;
                let ovk = OutgoingViewingKey(ovk);
                let mut memo = vec![0; 512];
                let m = hex::decode(&txout.memo)?;
                memo[..m.len()].copy_from_slice(&m);
                let memo = MemoBytes::from_bytes(&memo)?;
                builder.add_sapling_output(Some(ovk), pa, amount, Some(memo))?;
            }
        }
    }

    let prover = LocalTxProver::with_default_location().unwrap();
    let consensus_branch_id = BranchId::for_height(&NETWORK, last_height);
    let (tx, _) = builder.build(consensus_branch_id, &prover)?;
    let mut raw_tx = vec![];
    tx.write(&mut raw_tx)?;

    Ok(raw_tx)
}

pub async fn broadcast_tx(tx: &[u8], ld_url: &str) -> anyhow::Result<String> {
    let mut client = connect_lightwalletd(ld_url).await?;
    let latest_height = get_latest_height(&mut client).await?;
    let raw_tx = RawTransaction {
        data: tx.to_vec(),
        height: latest_height as u64,
    };
    let rep = client
        .send_transaction(Request::new(raw_tx))
        .await?
        .into_inner();
    Ok(rep.error_message)
}
