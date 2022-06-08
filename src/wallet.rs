use crate::chain::{get_activation_date, get_block_by_time};
use crate::contact::{serialize_contacts, Contact};
use crate::db::{AccountBackup, ZMessage};
use crate::key::KeyHelpers;
use crate::pay::Tx;
use crate::pay::TxBuilder;
use crate::prices::fetch_historical_prices;
use crate::scan::AM_ProgressCallback;
use crate::taddr::{get_taddr_balance, get_utxos, scan_transparent_accounts};
use crate::{
    broadcast_tx, connect_lightwalletd, get_latest_height, BlockId, CTree, CompactTxStreamerClient,
    DbAdapter,
};
use anyhow::anyhow;
use bech32::FromBase32;
use bip39::{Language, Mnemonic};
use chacha20poly1305::aead::{Aead, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use lazycell::AtomicLazyCell;
use rand::rngs::OsRng;
use rand::RngCore;
use secp256k1::SecretKey;
use serde::Deserialize;
use serde::Serialize;
use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_extended_spending_key, encode_payment_address,
};
use zcash_client_backend::zip321::{Payment, TransactionRequest};
use zcash_params::coin::{get_coin_chain, CoinChain, CoinType};
use zcash_params::{OUTPUT_PARAMS, SPEND_PARAMS};
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::memo::Memo;
use zcash_primitives::transaction::builder::Progress;
use zcash_primitives::transaction::components::Amount;
use zcash_proofs::prover::LocalTxProver;

const DEFAULT_CHUNK_SIZE: u32 = 100_000;

pub struct Wallet {
    coin_type: CoinType,
    pub db_path: String,
    db: DbAdapter,
    key_helpers: KeyHelpers,
    prover: AtomicLazyCell<LocalTxProver>,
    pub ld_url: String,
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

//     to_address: &str,
//     amount: u64,
//     memo: &str,
//     max_amount_per_note: u64,

#[derive(Deserialize)]
pub struct Recipient {
    pub address: String,
    pub amount: u64,
    pub reply_to: bool,
    pub subject: String,
    pub memo: String,
    pub max_amount_per_note: u64,
}

pub struct RecipientMemo {
    pub address: String,
    pub amount: u64,
    pub memo: Memo,
    pub max_amount_per_note: u64,
}

impl RecipientMemo {
    pub fn from_recipient(from: &str, r: &Recipient) -> Self {
        let memo = if !r.reply_to && r.subject.is_empty() {
            r.memo.clone()
        } else {
            encode_memo(from, r.reply_to, &r.subject, &r.memo)
        };
        RecipientMemo {
            address: r.address.clone(),
            amount: r.amount,
            memo: Memo::from_str(&memo).unwrap(),
            max_amount_per_note: r.max_amount_per_note,
        }
    }
}

pub fn encode_memo(from: &str, include_from: bool, subject: &str, body: &str) -> String {
    let from = if include_from { from } else { &"" };
    let msg = format!("\u{1F6E1}MSG\n{}\n{}\n{}", from, subject, body);
    msg
}

pub fn decode_memo(memo: &str, recipient: &str, timestamp: u32, height: u32) -> ZMessage {
    let memo_lines: Vec<_> = memo.splitn(4, '\n').collect();
    let msg = if memo_lines[0] == "\u{1F6E1}MSG" {
        ZMessage {
            sender: if memo_lines[1].is_empty() {
                None
            } else {
                Some(memo_lines[1].to_string())
            },
            recipient: recipient.to_string(),
            subject: memo_lines[2].to_string(),
            body: memo_lines[3].to_string(),
            timestamp,
            height,
        }
    } else {
        ZMessage {
            sender: None,
            recipient: recipient.to_string(),
            subject: memo_lines[0].chars().take(20).collect(),
            body: memo.to_string(),
            timestamp,
            height,
        }
    };
    msg
}

impl Wallet {
    pub fn new(coin_type: CoinType, db_path: &str) -> Wallet {
        let db = DbAdapter::new(coin_type, db_path).unwrap();
        let key_helpers = KeyHelpers::new(coin_type);
        db.init_db().unwrap();
        Wallet {
            coin_type,
            db_path: db_path.to_string(),
            db,
            key_helpers,
            prover: AtomicLazyCell::new(),
            ld_url: "".to_string(),
        }
    }

    pub fn reset_db(&self) -> anyhow::Result<()> {
        self.db.reset_db()
    }

    pub fn valid_key(&self, key: &str) -> i8 {
        self.key_helpers.is_valid_key(key)
    }

    pub fn new_account(&self, name: &str, data: &str, index: u32) -> anyhow::Result<i32> {
        if data.is_empty() {
            let mut entropy = [0u8; 32];
            OsRng.fill_bytes(&mut entropy);
            let mnemonic = Mnemonic::from_entropy(&entropy, Language::English)?;
            let seed = mnemonic.phrase();
            self.new_account_with_key(name, seed, index)
        } else {
            self.new_account_with_key(name, data, index)
        }
    }

    pub fn new_sub_account(&self, id: u32, name: &str) -> anyhow::Result<i32> {
        let (seed, _) = self.db.get_seed(id)?;
        let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
        let index = self.db.next_account_id(&seed)?;
        let new_id = self.new_account_with_key(name, &seed, index as u32)?;
        Ok(new_id)
    }

    pub fn new_sub_account_index(&self, id: u32, name: &str, index: u32) -> anyhow::Result<i32> {
        let (seed, _) = self.db.get_seed(id)?;
        let seed = seed.ok_or_else(|| anyhow!("Account has no seed"))?;
        let new_id = self.new_account_with_key(name, &seed, index)?;
        Ok(new_id)
    }

    pub fn get_backup(&self, account: u32) -> anyhow::Result<String> {
        let (seed, sk, ivk) = self.db.get_backup(account)?;
        if let Some(seed) = seed {
            return Ok(seed);
        }
        if let Some(sk) = sk {
            return Ok(sk);
        }
        Ok(ivk)
    }

    pub fn get_sk(&self, account: u32) -> anyhow::Result<String> {
        let sk = self.db.get_sk(account)?;
        Ok(sk)
    }

    pub fn new_account_with_key(&self, name: &str, key: &str, index: u32) -> anyhow::Result<i32> {
        let (seed, sk, ivk, pa) = self.key_helpers.decode_key(key, index)?;
        let account =
            self.db
                .store_account(name, seed.as_deref(), index, sk.as_deref(), &ivk, &pa)?;
        if account > 0 {
            self.db.create_taddr(account as u32)?;
        }
        Ok(account)
    }

    async fn scan_async(
        coin_type: CoinType,
        get_tx: bool,
        db_path: &str,
        chunk_size: u32,
        target_height_offset: u32,
        progress_callback: AM_ProgressCallback,
        ld_url: &str,
    ) -> anyhow::Result<()> {
        crate::scan::sync_async(
            coin_type,
            chunk_size,
            get_tx,
            db_path,
            target_height_offset,
            progress_callback,
            ld_url,
        )
        .await
    }

    pub async fn get_latest_height(&self) -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;
        Ok(last_height)
    }

    // Not a method in order to avoid locking the instance
    pub async fn sync_ex(
        coin_type: CoinType,
        get_tx: bool,
        anchor_offset: u32,
        db_path: &str,
        progress_callback: impl Fn(u32) + Send + 'static,
        ld_url: &str,
    ) -> anyhow::Result<()> {
        let cb = Arc::new(Mutex::new(progress_callback));
        Self::scan_async(
            coin_type,
            get_tx,
            db_path,
            DEFAULT_CHUNK_SIZE,
            anchor_offset,
            cb.clone(),
            ld_url,
        )
        .await?;
        Self::scan_async(
            coin_type,
            get_tx,
            db_path,
            DEFAULT_CHUNK_SIZE,
            0,
            cb.clone(),
            ld_url,
        )
        .await?;
        Ok(())
    }

    pub async fn sync(
        &self,
        get_tx: bool,
        anchor_offset: u32,
        progress_callback: impl Fn(u32) + Send + 'static,
    ) -> anyhow::Result<()> {
        Self::sync_ex(
            self.db.coin_type,
            get_tx,
            anchor_offset,
            &self.db_path,
            progress_callback,
            &self.ld_url,
        )
        .await
    }

    pub async fn skip_to_last_height(&self) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;
        self.store_tree_state(&mut client, last_height).await?;
        Ok(())
    }

    async fn store_tree_state(
        &self,
        client: &mut CompactTxStreamerClient<Channel>,
        height: u32,
    ) -> anyhow::Result<()> {
        let block_id = BlockId {
            height: height as u64,
            hash: vec![],
        };
        let block = client.get_block(block_id.clone()).await?.into_inner();
        let tree_state = client
            .get_tree_state(Request::new(block_id))
            .await?
            .into_inner();
        let tree = CTree::read(&*hex::decode(&tree_state.tree)?)?;
        self.db
            .store_block(height, &block.hash, block.time, &tree)?;
        Ok(())
    }

    pub async fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        self.db.trim_to_height(height)?;
        self.store_tree_state(&mut client, height).await?;
        Ok(())
    }

    async fn prepare_multi_payment(
        &self,
        account: u32,
        last_height: u32,
        recipients: &[RecipientMemo],
        use_transparent: bool,
        anchor_offset: u32,
    ) -> anyhow::Result<(Tx, Vec<u32>)> {
        let mut tx_builder = TxBuilder::new(self.db.coin_type, last_height);

        let fvk = self.db.get_ivk(account)?;
        let fvk = decode_extended_full_viewing_key(
            self.network().hrp_sapling_extended_full_viewing_key(),
            &fvk,
        )
        .unwrap()
        .unwrap();
        let utxos = if use_transparent {
            let mut client = connect_lightwalletd(&self.ld_url).await?;
            get_utxos(&mut client, &self.db, account).await?
        } else {
            vec![]
        };

        let target_amount: u64 = recipients.iter().map(|r| r.amount).sum();
        let anchor_height = last_height.saturating_sub(anchor_offset);
        let spendable_notes = self.db.get_spendable_notes(account, anchor_height, &fvk)?;
        let note_ids = tx_builder.select_inputs(&fvk, &spendable_notes, &utxos, target_amount)?;
        tx_builder.select_outputs(&fvk, recipients)?;
        Ok((tx_builder.tx, note_ids))
    }

    fn sign(
        &mut self,
        tx: &Tx,
        account: u32,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<Vec<u8>> {
        self._ensure_prover()?;
        let zsk = self.db.get_sk(account)?;
        let tsk = self
            .db
            .get_tsk(account)?
            .map(|tsk| SecretKey::from_str(&tsk).unwrap());
        let extsk =
            decode_extended_spending_key(self.network().hrp_sapling_extended_spending_key(), &zsk)
                .unwrap()
                .unwrap();
        let prover = self
            .prover
            .borrow()
            .ok_or_else(|| anyhow::anyhow!("Prover not initialized"))?;
        let raw_tx = tx.sign(tsk, &extsk, prover, progress_callback)?;
        Ok(raw_tx)
    }

    fn mark_spend(&mut self, selected_notes: &[u32]) -> anyhow::Result<()> {
        let db_tx = self.db.begin_transaction()?;
        for id_note in selected_notes.iter() {
            DbAdapter::mark_spent(*id_note, 0, &db_tx)?;
        }
        db_tx.commit()?;
        Ok(())
    }

    /// Build a multi payment for offline signing
    pub async fn build_only_multi_payment(
        &mut self,
        account: u32,
        last_height: u32,
        recipients: &[RecipientMemo],
        use_transparent: bool,
        anchor_offset: u32,
    ) -> anyhow::Result<String> {
        let (tx, _) = self
            .prepare_multi_payment(
                account,
                last_height,
                recipients,
                use_transparent,
                anchor_offset,
            )
            .await?;
        let tx_str = serde_json::to_string(&tx)?;
        Ok(tx_str)
    }

    pub async fn sign_only_multi_payment(
        &mut self,
        tx_string: &str,
        account: u32,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<Vec<u8>> {
        let tx = serde_json::from_str::<Tx>(tx_string)?;
        let raw_tx = self.sign(&tx, account, progress_callback)?;
        Ok(raw_tx)
    }

    /// Build, sign and broadcast a multi payment
    pub async fn build_sign_send_multi_payment(
        &mut self,
        account: u32,
        last_height: u32,
        recipients: &[RecipientMemo],
        use_transparent: bool,
        anchor_offset: u32,
        progress_callback: impl Fn(Progress) + Send + 'static,
    ) -> anyhow::Result<String> {
        let (tx, note_ids) = self
            .prepare_multi_payment(
                account,
                last_height,
                recipients,
                use_transparent,
                anchor_offset,
            )
            .await?;
        let raw_tx = self.sign(&tx, account, progress_callback)?;
        let tx_id = broadcast_tx(&raw_tx, &self.ld_url).await?;
        self.mark_spend(&note_ids)?;
        Ok(tx_id)
    }

    pub async fn shield_taddr(&mut self, account: u32, last_height: u32) -> anyhow::Result<String> {
        let tx_id = self
            .build_sign_send_multi_payment(account, last_height, &[], true, 0, |_| {})
            .await?;
        Ok(tx_id)
    }

    fn _ensure_prover(&mut self) -> anyhow::Result<()> {
        if !self.prover.filled() {
            let prover = LocalTxProver::from_bytes(SPEND_PARAMS, OUTPUT_PARAMS);
            self.prover
                .fill(prover)
                .map_err(|_| anyhow::anyhow!("dup prover"))?;
        }
        Ok(())
    }

    pub fn get_ivk(&self, account: u32) -> anyhow::Result<String> {
        self.db.get_ivk(account)
    }

    pub fn new_diversified_address(&self, account: u32) -> anyhow::Result<String> {
        let ivk = self.get_ivk(account)?;
        let fvk = decode_extended_full_viewing_key(
            self.network().hrp_sapling_extended_full_viewing_key(),
            &ivk,
        )?
        .unwrap();
        let mut diversifier_index = self.db.get_diversifier(account)?;
        diversifier_index.increment().unwrap();
        let (new_diversifier_index, pa) = fvk
            .find_address(diversifier_index)
            .ok_or_else(|| anyhow::anyhow!("Cannot generate new address"))?;
        self.db.store_diversifier(account, &new_diversifier_index)?;
        let pa = encode_payment_address(self.network().hrp_sapling_payment_address(), &pa);
        Ok(pa)
    }

    pub async fn get_taddr_balance(&self, account: u32) -> anyhow::Result<u64> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let address = self.db.get_taddr(account)?;
        let balance = match address {
            None => 0u64,
            Some(address) => get_taddr_balance(&mut client, &address).await?,
        };
        Ok(balance)
    }

    pub fn store_contact(
        &self,
        id: u32,
        name: &str,
        address: &str,
        dirty: bool,
    ) -> anyhow::Result<()> {
        let contact = Contact {
            id,
            name: name.to_string(),
            address: address.to_string(),
        };
        self.db.store_contact(&contact, dirty)?;
        Ok(())
    }

    pub async fn commit_unsaved_contacts(
        &mut self,
        account: u32,
        anchor_offset: u32,
    ) -> anyhow::Result<String> {
        let contacts = self.db.get_unsaved_contacts()?;
        let memos = serialize_contacts(&contacts)?;
        let tx_id = self
            .save_contacts_tx(&memos, account, anchor_offset)
            .await?;
        Ok(tx_id)
    }

    pub async fn save_contacts_tx(
        &mut self,
        memos: &[Memo],
        account: u32,
        anchor_offset: u32,
    ) -> anyhow::Result<String> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let last_height = get_latest_height(&mut client).await?;
        let address = self.db.get_address(account)?;
        let recipients: Vec<_> = memos
            .iter()
            .map(|m| RecipientMemo {
                address: address.clone(),
                amount: 0,
                memo: m.clone(),
                max_amount_per_note: 0,
            })
            .collect();

        let tx_id = self
            .build_sign_send_multi_payment(
                account,
                last_height,
                &recipients,
                false,
                anchor_offset,
                |_| {},
            )
            .await?;
        Ok(tx_id)
    }

    pub fn mark_message_read(&self, _account: u32, message: u32, read: bool) -> anyhow::Result<()> {
        self.db.mark_message_read(message, read)?;
        Ok(())
    }

    pub fn mark_all_messages_read(&self, account: u32, read: bool) -> anyhow::Result<()> {
        self.db.mark_all_messages_read(account, read)?;
        Ok(())
    }

    pub fn set_lwd_url(&mut self, ld_url: &str) -> anyhow::Result<()> {
        self.ld_url = ld_url.to_string();
        Ok(())
    }

    pub async fn sync_historical_prices(
        &mut self,
        now: i64,
        days: u32,
        currency: &str,
    ) -> anyhow::Result<u32> {
        let quotes = fetch_historical_prices(now, days, currency, &self.db).await?;
        self.db.store_historical_prices(&quotes, currency)?;
        Ok(quotes.len() as u32)
    }

    pub fn delete_account(&self, account: u32) -> anyhow::Result<()> {
        self.db.delete_account(account)?;
        Ok(())
    }

    pub fn truncate_data(&self) -> anyhow::Result<()> {
        self.db.truncate_data()
    }

    pub fn make_payment_uri(
        &self,
        address: &str,
        amount: u64,
        memo: &str,
    ) -> anyhow::Result<String> {
        let addr = RecipientAddress::decode(self.network(), address)
            .ok_or_else(|| anyhow::anyhow!("Invalid address"))?;
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
            .to_uri(self.network())
            .ok_or_else(|| anyhow::anyhow!("Cannot build Payment URI"))?;
        let uri = format!("{}{}", self.chain().ticker(), &uri[5..]); // hack to replace the URI scheme
        Ok(uri)
    }

    pub fn parse_payment_uri(&self, uri: &str) -> anyhow::Result<String> {
        if uri[..5].ne(self.chain().ticker()) {
            anyhow::bail!("Invalid Payment URI");
        }
        let uri = format!("zcash{}", &uri[5..]); // hack to replace the URI scheme
        let treq = TransactionRequest::from_uri(self.network(), &uri)
            .map_err(|_| anyhow::anyhow!("Invalid Payment URI"))?;
        if treq.payments.len() != 1 {
            anyhow::bail!("Invalid Payment URI")
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
        let payment = MyPayment {
            address: payment.recipient_address.encode(self.network()),
            amount: u64::from(payment.amount),
            memo,
        };

        let payment_json = serde_json::to_string(&payment)?;

        Ok(payment_json)
    }

    pub fn get_full_backup(&self) -> anyhow::Result<Vec<AccountBackup>> {
        self.db.get_full_backup()
    }

    pub fn restore_full_backup(&self, accounts: &[AccountBackup]) -> anyhow::Result<()> {
        self.db.restore_full_backup(accounts)
    }

    pub fn store_share_secret(
        &self,
        account: u32,
        secret: &str,
        index: usize,
        threshold: usize,
        participants: usize,
    ) -> anyhow::Result<()> {
        self.db
            .store_share_secret(account, secret, index, threshold, participants)
    }

    pub fn get_share_secret(&self, account: u32) -> anyhow::Result<String> {
        self.db.get_share_secret(account)
    }

    pub fn parse_recipients(
        &self,
        account: u32,
        recipients: &str,
    ) -> anyhow::Result<Vec<RecipientMemo>> {
        let address = self.db.get_address(account)?;
        let recipients: Vec<Recipient> = serde_json::from_str(recipients)?;
        let recipient_memos: Vec<_> = recipients
            .iter()
            .map(|r| RecipientMemo::from_recipient(&address, r))
            .collect();
        Ok(recipient_memos)
    }

    #[cfg(feature = "ledger_sapling")]
    pub async fn ledger_sign(&mut self, tx_filename: &str) -> anyhow::Result<String> {
        self._ensure_prover()?;
        let file = std::file::File::open(tx_filename)?;
        let mut tx: Tx = serde_json::from_reader(&file)?;
        let raw_tx = crate::build_tx_ledger(&mut tx, self.prover.borrow().unwrap()).await?;
        let tx_id = broadcast_tx(&raw_tx, &self.ld_url).await?;
        Ok(tx_id)
    }

    #[cfg(not(feature = "ledger_sapling"))]
    pub async fn ledger_sign(&mut self, _tx_filename: &str) -> anyhow::Result<String> {
        unimplemented!()
    }

    pub async fn get_activation_date(&self) -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let date_time = get_activation_date(self.network(), &mut client).await?;
        Ok(date_time)
    }

    pub async fn get_block_by_time(&self, time: u32) -> anyhow::Result<u32> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        let date_time = get_block_by_time(self.network(), &mut client, time).await?;
        Ok(date_time)
    }

    pub async fn scan_transparent_accounts(
        &self,
        account: u32,
        gap_limit: usize,
    ) -> anyhow::Result<()> {
        let mut client = connect_lightwalletd(&self.ld_url).await?;
        scan_transparent_accounts(self.network(), &mut client, &self.db, account, gap_limit)
            .await?;
        Ok(())
    }

    fn chain(&self) -> &dyn CoinChain {
        get_coin_chain(self.coin_type)
    }
    fn network(&self) -> &Network {
        self.chain().network()
    }
}

const NONCE: &'static [u8; 12] = b"unique nonce";

pub fn encrypt_backup(accounts: &[AccountBackup], key: &str) -> anyhow::Result<String> {
    let accounts_bin = bincode::serialize(&accounts)?;
    let backup = if !key.is_empty() {
        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" {
            anyhow::bail!("Invalid backup key")
        }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        // nonce is constant because we always use a different key!
        let cipher_text = cipher
            .encrypt(Nonce::from_slice(NONCE), &*accounts_bin)
            .map_err(|_e| anyhow::anyhow!("Failed to encrypt backup"))?;
        base64::encode(cipher_text)
    } else {
        base64::encode(accounts_bin)
    };
    Ok(backup)
}

pub fn decrypt_backup(key: &str, backup: &str) -> anyhow::Result<Vec<AccountBackup>> {
    let backup = if !key.is_empty() {
        let (hrp, key, _) = bech32::decode(key)?;
        if hrp != "zwk" {
            anyhow::bail!("Not a valid decryption key");
        }
        let key = Vec::<u8>::from_base32(&key)?;
        let key = Key::from_slice(&key);

        let cipher = ChaCha20Poly1305::new(key);
        let backup = base64::decode(backup)?;
        cipher
            .decrypt(Nonce::from_slice(NONCE), &*backup)
            .map_err(|_e| anyhow::anyhow!("Failed to decrypt backup"))?
    } else {
        base64::decode(backup)?
    };

    let accounts: Vec<AccountBackup> = bincode::deserialize(&backup)?;
    Ok(accounts)
}

#[derive(Serialize)]
struct MyPayment {
    address: String,
    amount: u64,
    memo: String,
}

#[cfg(test)]
mod tests {
    use crate::key::KeyHelpers;
    use crate::wallet::Wallet;
    use crate::LWD_URL;
    use bip39::{Language, Mnemonic};
    use zcash_params::coin::CoinType;

    #[tokio::test]
    async fn test_wallet_seed() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let mut wallet = Wallet::new(CoinType::Zcash, "zec.db");
        wallet.set_lwd_url(LWD_URL).unwrap();
        wallet.new_account_with_key("test", &seed, 0).unwrap();
    }

    #[tokio::test]
    async fn test_payment() {
        dotenv::dotenv().unwrap();
        env_logger::init();

        let seed = dotenv::var("SEED").unwrap();
        let kh = KeyHelpers::new(CoinType::Zcash);
        let (sk, vk, pa) = kh
            .derive_secret_key(&Mnemonic::from_phrase(&seed, Language::English).unwrap(), 0)
            .unwrap();
        println!("{} {} {}", sk, vk, pa);
        // let wallet = Wallet::new("zec.db");
        //
        // let tx_id = wallet.send_payment(1, &pa, 1000).await.unwrap();
        // println!("TXID = {}", tx_id);
    }

    #[test]
    pub fn test_diversified_address() {
        let mut wallet = Wallet::new(CoinType::Zcash, "zec.db");
        wallet.set_lwd_url(LWD_URL).unwrap();
        let address = wallet.new_diversified_address(1).unwrap();
        println!("{}", address);
    }
}
