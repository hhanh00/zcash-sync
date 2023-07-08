use crate::db::data_generated::fb::{MessageT, MessageVecT, PrevNextT};
use crate::db::ZMessage;
use anyhow::Result;
use rusqlite::{params, Connection};
use zcash_primitives::consensus::Network;

pub fn store_message(connection: &Connection, account: u32, message: &ZMessage) -> Result<()> {
    connection.execute("INSERT INTO messages(account, id_tx, sender, recipient, subject, body, timestamp, height, incoming, read) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                            params![account, message.id_tx, message.sender, message.recipient, message.subject, message.body, message.timestamp, message.height, message.incoming, false])?;
    Ok(())
}

pub fn mark_message_read(connection: &Connection, message_id: u32, read: bool) -> Result<()> {
    connection.execute(
        "UPDATE messages SET read = ?1 WHERE id = ?2",
        params![read, message_id],
    )?;
    Ok(())
}

pub fn mark_all_messages_read(connection: &Connection, account: u32, read: bool) -> Result<()> {
    connection.execute(
        "UPDATE messages SET read = ?1 WHERE account = ?2",
        params![read, account],
    )?;
    Ok(())
}

pub fn get_messages(connection: &Connection, network: &Network, id: u32) -> Result<MessageVecT> {
    let addresses = super::contact::resolve_addresses(network, connection)?;

    let mut stmt = connection.prepare(
        "SELECT m.id, m.id_tx, m.timestamp, m.sender, m.recipient, m.incoming, \
        subject, body, height, read FROM messages m \
        WHERE account = ?1 ORDER BY timestamp DESC",
    )?;
    let messages = stmt.query_map([id], |row| {
        let id_msg: u32 = row.get("id")?;
        let id_tx: Option<u32> = row.get("id_tx")?;
        let timestamp: u32 = row.get("timestamp")?;
        let height: u32 = row.get("height")?;
        let sender: Option<String> = row.get("sender")?;
        let recipient: Option<String> = row.get("recipient")?;
        let subject: String = row.get("subject")?;
        let body: String = row.get("body")?;
        let read: bool = row.get("read")?;
        let incoming: bool = row.get("incoming")?;

        let id_tx = id_tx.unwrap_or(0);
        let from = match sender {
            None => String::new(),
            Some(a) => addresses.get(&a).cloned().unwrap_or(a.clone()),
        };
        let to = match recipient {
            None => String::new(),
            Some(a) => addresses.get(&a).cloned().unwrap_or(a.clone()),
        };
        let message = MessageT {
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
        };
        Ok(message)
    })?;
    let messages: Result<Vec<_>, _> = messages.collect();
    let messages = MessageVecT {
        messages: messages.ok(),
    };
    Ok(messages)
}

pub fn get_prev_next_message(
    connection: &Connection,
    subject: &str,
    height: u32,
    account: u32,
) -> Result<PrevNextT> {
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
    let prev_next = PrevNextT { prev, next };
    Ok(prev_next)
}
