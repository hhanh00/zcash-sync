use crate::db::data_generated::fb::*;
use crate::unified::orchard_as_unified;
use anyhow::Result;
use orchard::keys::{FullViewingKey, Scope};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::AddressCodec;
use zcash_primitives::consensus::Network;

pub fn get_account_list(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare("WITH notes AS (SELECT a.id_account, a.name, CASE WHEN r.spent IS NULL THEN r.value ELSE 0 END AS nv FROM accounts a LEFT JOIN received_notes r ON a.id_account = r.account), \
                       accounts2 AS (SELECT id_account, name, COALESCE(sum(nv), 0) AS balance FROM notes GROUP by id_account) \
                       SELECT a.id_account, a.name, a.balance FROM accounts2 a")?;
    let rows = stmt.query_map([], |row| {
        let id: u32 = row.get("id_account")?;
        let name: String = row.get("name")?;
        let balance: i64 = row.get("balance")?;
        let name = builder.create_string(&name);
        let account = Account::create(
            &mut builder,
            &AccountArgs {
                id,
                name: Some(name),
                balance: balance as u64,
            },
        );
        Ok(account)
    })?;
    let mut accounts = vec![];
    for r in rows {
        accounts.push(r?);
    }
    let accounts = builder.create_vector(&accounts);
    let accounts = AccountVec::create(
        &mut builder,
        &AccountVecArgs {
            accounts: Some(accounts),
        },
    );
    builder.finish(accounts, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_active_account(connection: &Connection) -> Result<u32> {
    let id = connection
        .query_row(
            "SELECT value FROM properties WHERE name = 'account'",
            [],
            |row| {
                let value: String = row.get(0)?;
                let value: u32 = value.parse().unwrap();
                Ok(value)
            },
        )
        .optional()?
        .unwrap_or(0);
    let id = get_available_account_id(connection, id)?;
    set_active_account(connection, id)?;
    Ok(id)
}

pub fn set_active_account(connection: &Connection, id: u32) -> Result<()> {
    connection.execute(
        "INSERT INTO properties(name, value) VALUES ('account',?1) \
    ON CONFLICT (name) DO UPDATE SET value = excluded.value",
        [id],
    )?;
    Ok(())
}

pub fn get_available_account_id(connection: &Connection, id: u32) -> Result<u32> {
    let r = connection
        .query_row("SELECT 1 FROM accounts WHERE id_account = ?1", [id], |_| {
            Ok(())
        })
        .optional()?;
    if r.is_some() {
        return Ok(id);
    }
    let r = connection
        .query_row("SELECT MAX(id_account) FROM accounts", [], |row| {
            let id: Option<u32> = row.get(0)?;
            Ok(id)
        })?
        .unwrap_or(0);
    Ok(r)
}

pub fn get_t_addr(connection: &Connection, id: u32) -> Result<String> {
    let address = connection
        .query_row(
            "SELECT address FROM taddrs WHERE account = ?1",
            [id],
            |row| {
                let address: String = row.get(0)?;
                Ok(address)
            },
        )
        .optional()?;
    Ok(address.unwrap_or(String::new()))
}

pub fn get_sk(connection: &Connection, id: u32) -> Result<String> {
    let sk = connection.query_row(
        "SELECT sk FROM accounts WHERE id_account = ?1",
        [id],
        |row| {
            let sk: Option<String> = row.get(0)?;
            Ok(sk.unwrap_or(String::new()))
        },
    )?;
    Ok(sk)
}

pub fn update_account_name(connection: &Connection, id: u32, name: &str) -> Result<()> {
    connection.execute(
        "UPDATE accounts SET name = ?2 WHERE id_account = ?1",
        params![id, name],
    )?;
    Ok(())
}

pub fn get_balances(connection: &Connection, id: u32, confirmed_height: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let shielded = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL",
        params![id],
        |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        },
    )?; // funds not spent yet
    let unconfirmed_spent = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent = 0",
        params![id],
        |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        },
    )?; // funds used in unconfirmed tx
    let balance = shielded + unconfirmed_spent;
    let under_confirmed = connection.query_row(
        "SELECT SUM(value) AS value FROM received_notes WHERE account = ?1 AND spent IS NULL AND height > ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?; // funds received but not old enough
    let excluded = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL \
        AND height <= ?2 AND excluded",
        params![id, confirmed_height],
        |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        },
    )?; // funds excluded from spending
    let sapling = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 0 AND height <= ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?;
    let orchard = connection.query_row(
        "SELECT SUM(value) FROM received_notes WHERE account = ?1 AND spent IS NULL AND orchard = 1 AND height <= ?2",
        params![id, confirmed_height], |row| {
            let value: Option<i64> = row.get(0)?;
            Ok(value.unwrap_or(0) as u64)
        })?;

    let balance = Balance::create(
        &mut builder,
        &BalanceArgs {
            shielded,
            unconfirmed_spent,
            balance,
            under_confirmed,
            excluded,
            sapling,
            orchard,
        },
    );
    builder.finish(balance, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_db_height(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let height = connection
        .query_row(
            "SELECT height, timestamp FROM blocks WHERE height = (SELECT MAX(height) FROM blocks)",
            [],
            |row| {
                let height: u32 = row.get(0)?;
                let timestamp: u32 = row.get(1)?;
                let height = Height::create(&mut builder, &HeightArgs { height, timestamp });
                Ok(height)
            },
        )
        .optional()?;
    let data = height
        .map(|h| {
            builder.finish(h, None);
            builder.finished_data().to_vec()
        })
        .unwrap_or(vec![]);
    Ok(data)
}

pub fn get_notes(connection: &Connection, id: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT n.id_note, n.height, n.value, t.timestamp, n.orchard, n.excluded, n.spent FROM received_notes n, transactions t \
           WHERE n.account = ?1 AND (n.spent IS NULL OR n.spent = 0) \
           AND n.tx = t.id_tx ORDER BY n.height DESC")?;
    let rows = stmt.query_map(params![id], |row| {
        let id: u32 = row.get("id_note")?;
        let height: u32 = row.get("height")?;
        let value: i64 = row.get("value")?;
        let timestamp: u32 = row.get("timestamp")?;
        let orchard: u8 = row.get("orchard")?;
        let excluded: Option<bool> = row.get("excluded")?;
        let spent: Option<u32> = row.get("spent")?;
        let note = ShieldedNote::create(
            &mut builder,
            &ShieldedNoteArgs {
                id,
                height,
                value: value as u64,
                timestamp,
                orchard: orchard == 1,
                excluded: excluded.unwrap_or(false),
                spent: spent.is_some(),
            },
        );
        Ok(note)
    })?;
    let mut notes = vec![];
    for r in rows {
        notes.push(r?);
    }
    let notes = builder.create_vector(&notes);
    let notes = ShieldedNoteVec::create(&mut builder, &ShieldedNoteVecArgs { notes: Some(notes) });
    builder.finish(notes, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_txs(network: &Network, connection: &Connection, id: u32) -> Result<ShieldedTxVecT> {
    let addresses = resolve_addresses(network, connection)?;
    let mut stmt = connection.prepare(
        "SELECT id_tx, txid, height, timestamp, t.address, value, memo FROM transactions t \
        WHERE account = ?1 ORDER BY height DESC",
    )?;
    let rows = stmt.query_map(params![id], |row| {
        let id_tx: u32 = row.get("id_tx")?;
        let height: u32 = row.get("height")?;
        let mut tx_id: Vec<u8> = row.get("txid")?;
        tx_id.reverse();
        let tx_id = hex::encode(&tx_id);
        let short_tx_id = tx_id[..8].to_string();
        let timestamp: u32 = row.get("timestamp")?;
        let address: Option<String> = row.get("address")?;
        let value: i64 = row.get("value")?;
        let memo: Option<String> = row.get("memo")?;
        let name = address.as_ref().and_then(|a| addresses.get(a)).cloned();
        let tx = ShieldedTxT {
            id: id_tx,
            height,
            tx_id: Some(tx_id),
            short_tx_id: Some(short_tx_id),
            timestamp,
            name,
            value: value as u64,
            address,
            memo,
        };
        Ok(tx)
    })?;
    let mut txs = vec![];
    for r in rows {
        txs.push(r?);
    }
    let txs = ShieldedTxVecT { txs: Some(txs) };
    Ok(txs)
}

pub fn get_messages(connection: &Connection, id: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT m.id, m.id_tx, m.timestamp, m.sender, m.recipient, m.incoming, c.name as scontact, a.name as saccount, c2.name as rcontact, a2.name as raccount, \
        subject, body, height, read FROM messages m \
        LEFT JOIN contacts c ON m.sender = c.address \
        LEFT JOIN accounts a ON m.sender = a.address \
        LEFT JOIN contacts c2 ON m.recipient = c2.address \
        LEFT JOIN accounts a2 ON m.recipient = a2.address \
        WHERE account = ?1 ORDER BY timestamp DESC")?;
    let rows = stmt.query_map(params![id], |row| {
        let id_msg: u32 = row.get("id")?;
        let id_tx: Option<u32> = row.get("id_tx")?;
        let timestamp: u32 = row.get("timestamp")?;
        let height: u32 = row.get("height")?;
        let sender: Option<String> = row.get("sender")?;
        let scontact: Option<String> = row.get("scontact")?;
        let saccount: Option<String> = row.get("saccount")?;
        let recipient: Option<String> = row.get("recipient")?;
        let rcontact: Option<String> = row.get("rcontact")?;
        let raccount: Option<String> = row.get("raccount")?;
        let subject: String = row.get("subject")?;
        let body: String = row.get("body")?;
        let read: bool = row.get("read")?;
        let incoming: bool = row.get("incoming")?;

        let id_tx = id_tx.unwrap_or(0);
        let from = scontact.or(saccount).or(sender).unwrap_or(String::new());
        let to = rcontact.or(raccount).or(recipient).unwrap_or(String::new());

        let from = builder.create_string(&from);
        let to = builder.create_string(&to);
        let subject = builder.create_string(&subject);
        let body = builder.create_string(&body);

        let message = Message::create(
            &mut builder,
            &MessageArgs {
                id_msg,
                id_tx,
                height,
                timestamp,
                from: Some(from),
                to: Some(to),
                subject: Some(subject),
                body: Some(body),
                read,
                incoming,
            },
        );
        Ok(message)
    })?;
    let mut messages = vec![];
    for r in rows {
        messages.push(r?);
    }
    let messages = builder.create_vector(&messages);
    let messages = MessageVec::create(
        &mut builder,
        &MessageVecArgs {
            messages: Some(messages),
        },
    );
    builder.finish(messages, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_prev_next_message(
    connection: &Connection,
    subject: &str,
    height: u32,
    account: u32,
) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let prev = connection
        .query_row(
            "SELECT MAX(id) FROM messages WHERE subject = ?1 AND height < ?2 and account = ?3",
            params![subject, height, account],
            |row| {
                let id: Option<u32> = row.get(0)?;
                Ok(id)
            },
        )?
        .unwrap_or(0);
    let next = connection
        .query_row(
            "SELECT MIN(id) FROM messages WHERE subject = ?1 AND height > ?2 and account = ?3",
            params![subject, height, account],
            |row| {
                let id: Option<u32> = row.get(0)?;
                Ok(id)
            },
        )?
        .unwrap_or(0);
    let prev_next = PrevNext::create(&mut builder, &PrevNextArgs { prev, next });
    builder.finish(prev_next, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_templates(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT id_send_template, title, address, amount, fiat_amount, fee_included, fiat, include_reply_to, subject, body FROM send_templates")?;
    let rows = stmt.query_map([], |row| {
        let id_msg: u32 = row.get("id_send_template")?;
        let title: String = row.get("title")?;
        let address: String = row.get("address")?;
        let amount: i64 = row.get("amount")?;
        let fiat_amount: f64 = row.get("fiat_amount")?;
        let fee_included: bool = row.get("fee_included")?;
        let fiat: Option<String> = row.get("fiat")?;
        let include_reply_to: bool = row.get("include_reply_to")?;
        let subject: String = row.get("subject")?;
        let body: String = row.get("body")?;

        let title = builder.create_string(&title);
        let address = builder.create_string(&address);
        let fiat = fiat.map(|fiat| builder.create_string(&fiat));
        let subject = builder.create_string(&subject);
        let body = builder.create_string(&body);

        let template = SendTemplate::create(
            &mut builder,
            &SendTemplateArgs {
                id: id_msg,
                title: Some(title),
                address: Some(address),
                amount: amount as u64,
                fiat_amount,
                fee_included,
                fiat,
                include_reply_to,
                subject: Some(subject),
                body: Some(body),
            },
        );
        Ok(template)
    })?;
    let mut templates = vec![];
    for r in rows {
        templates.push(r?);
    }
    let templates = builder.create_vector(&templates);
    let templates = SendTemplateVec::create(
        &mut builder,
        &SendTemplateVecArgs {
            templates: Some(templates),
        },
    );
    builder.finish(templates, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_contacts(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection
        .prepare("SELECT id, name, address FROM contacts WHERE address <> '' ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        let id: u32 = row.get("id")?;
        let name: String = row.get("name")?;
        let address: String = row.get("address")?;
        let name = builder.create_string(&name);
        let address = builder.create_string(&address);
        let contact = Contact::create(
            &mut builder,
            &ContactArgs {
                id,
                name: Some(name),
                address: Some(address),
            },
        );
        Ok(contact)
    })?;
    let mut contacts = vec![];
    for r in rows {
        contacts.push(r?);
    }
    let contacts = builder.create_vector(&contacts);
    let contacts = ContactVec::create(
        &mut builder,
        &ContactVecArgs {
            contacts: Some(contacts),
        },
    );
    builder.finish(contacts, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_pnl_txs(connection: &Connection, id: u32, timestamp: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT timestamp, value FROM transactions WHERE timestamp >= ?2 AND account = ?1 ORDER BY timestamp DESC")?;
    let rows = stmt.query_map([id, timestamp], |row| {
        let timestamp: u32 = row.get(0)?;
        let value: i64 = row.get(1)?;
        let tx = TxTimeValue::create(
            &mut builder,
            &TxTimeValueArgs {
                timestamp,
                value: value as u64,
            },
        );
        Ok(tx)
    })?;
    let mut txs = vec![];
    for r in rows {
        txs.push(r?);
    }
    let txs = builder.create_vector(&txs);
    let txs = TxTimeValueVec::create(&mut builder, &TxTimeValueVecArgs { values: Some(txs) });
    builder.finish(txs, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_historical_prices(
    connection: &Connection,
    timestamp: u32,
    currency: &str,
) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT timestamp, price FROM historical_prices WHERE timestamp >= ?2 AND currency = ?1",
    )?;
    let rows = stmt.query_map(params![currency, timestamp], |row| {
        let timestamp: u32 = row.get(0)?;
        let price: f64 = row.get(1)?;
        let quote = Quote::create(&mut builder, &QuoteArgs { timestamp, price });
        Ok(quote)
    })?;
    let mut quotes = vec![];
    for r in rows {
        quotes.push(r?);
    }
    let quotes = builder.create_vector(&quotes);
    let quotes = QuoteVec::create(
        &mut builder,
        &QuoteVecArgs {
            values: Some(quotes),
        },
    );
    builder.finish(quotes, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn get_spendings(connection: &Connection, id: u32, timestamp: u32) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare(
        "SELECT SUM(value) as v, t.address, c.name FROM transactions t LEFT JOIN contacts c ON t.address = c.address \
        WHERE account = ?1 AND timestamp >= ?2 AND value < 0 GROUP BY t.address ORDER BY v ASC LIMIT 5")?;
    let rows = stmt.query_map([id, timestamp], |row| {
        let value: i64 = row.get(0)?;
        let address: Option<String> = row.get(1)?;
        let name: Option<String> = row.get(2)?;

        let recipient = name.or(address);
        let recipient = recipient.unwrap_or(String::new());
        let recipient = builder.create_string(&recipient);

        let spending = Spending::create(
            &mut builder,
            &SpendingArgs {
                recipient: Some(recipient),
                amount: (-value) as u64,
            },
        );
        Ok(spending)
    })?;
    let mut spendings = vec![];
    for r in rows {
        spendings.push(r?);
    }
    let spendings = builder.create_vector(&spendings);
    let spendings = SpendingVec::create(
        &mut builder,
        &SpendingVecArgs {
            values: Some(spendings),
        },
    );
    builder.finish(spendings, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn update_excluded(connection: &Connection, id: u32, excluded: bool) -> Result<()> {
    connection.execute(
        "UPDATE received_notes SET excluded = ?2 WHERE id_note = ?1",
        params![id, excluded],
    )?;
    Ok(())
}

pub fn invert_excluded(connection: &Connection, id: u32) -> Result<()> {
    connection.execute(
        "UPDATE received_notes SET excluded = NOT(COALESCE(excluded, 0)) WHERE account = ?1",
        [id],
    )?;
    Ok(())
}

pub fn get_checkpoints(connection: &Connection) -> Result<Vec<u8>> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut stmt = connection.prepare("SELECT height, timestamp FROM blocks ORDER by height")?;
    let rows = stmt.query_map([], |row| {
        let height: u32 = row.get(0)?;
        let timestamp: u32 = row.get(1)?;

        let checkpoint = Checkpoint::create(&mut builder, &CheckpointArgs { height, timestamp });
        Ok(checkpoint)
    })?;
    let mut checkpoints = vec![];
    for r in rows {
        checkpoints.push(r?);
    }
    let checkpoints = builder.create_vector(&checkpoints);
    let checkpoints = CheckpointVec::create(
        &mut builder,
        &CheckpointVecArgs {
            values: Some(checkpoints),
        },
    );
    builder.finish(checkpoints, None);
    let data = builder.finished_data().to_vec();
    Ok(data)
}

pub fn resolve_addresses(
    network: &Network,
    connection: &Connection,
) -> Result<HashMap<String, String>> {
    let mut addresses: HashMap<String, String> = HashMap::new();
    let mut stmt = connection.prepare("SELECT name, address FROM contacts WHERE address <> ''")?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let address: String = row.get(1)?;
        let ra = RecipientAddress::decode(network, &address);
        if let Some(ra) = ra {
            match ra {
                RecipientAddress::Unified(ua) => {
                    if let Some(ta) = ua.transparent() {
                        addresses.insert(ta.encode(network), name.clone());
                    }
                    if let Some(pa) = ua.sapling() {
                        addresses.insert(pa.encode(network), name.clone());
                    }
                    if let Some(oa) = ua.orchard() {
                        let oa = orchard_as_unified(network, oa);
                        addresses.insert(oa.encode(), name.clone());
                    }
                }
                _ => {
                    addresses.insert(address, name);
                }
            }
        }
        Ok(())
    })?;
    for r in rows {
        r?;
    }

    let mut stmt = connection.prepare(
        "SELECT a.name, a.address, t.address, o.fvk FROM accounts a LEFT JOIN taddrs t ON a.id_account = t.account \
        LEFT JOIN orchard_addrs o ON a.id_account = o.account",
    )?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let z_addr: String = row.get(1)?;
        let t_addr: Option<String> = row.get(2)?;
        let o_fvk: Option<Vec<u8>> = row.get(3)?;
        addresses.insert(z_addr, name.clone());
        if let Some(t_addr) = t_addr {
            addresses.insert(t_addr, name.clone());
        }
        if let Some(o_fvk) = o_fvk {
            let o_fvk = FullViewingKey::from_bytes(&o_fvk.try_into().unwrap()).unwrap();
            let o_addr = o_fvk.address_at(0usize, Scope::External);
            let o_addr = orchard_as_unified(network, &o_addr);
            addresses.insert(o_addr.encode(), name.clone());
        }
        Ok(())
    })?;
    for r in rows {
        r?;
    }

    Ok(addresses)
}

// TODO: In get_tx, resolve addresses to contact
