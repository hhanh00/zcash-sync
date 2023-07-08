use crate::db::data_generated::fb::{SendTemplateT, SendTemplateVecT};
use anyhow::Result;
use rusqlite::{params, Connection};

pub fn store_template(connection: &Connection, t: &SendTemplateT) -> Result<u32> {
    let id = if t.id == 0 {
        connection.execute("INSERT INTO \
                send_templates(title, address, amount, fiat_amount, fee_included, fiat, include_reply_to, subject, body) \
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                                params![t.title, t.address, t.amount, t.fiat_amount, t.fee_included, t.fiat,
                t.include_reply_to, t.subject, t.body])?;
        connection.last_insert_rowid() as u32
    } else {
        connection.execute("UPDATE send_templates SET \
                title=?1, address=?2, amount=?3, fiat_amount=?4, fee_included=?5, fiat=?6, include_reply_to=?7, subject=?8, body=?9 \
                WHERE id_send_template=?10",
                                params![t.title, t.address, t.amount, t.fiat_amount, t.fee_included, t.fiat,
                t.include_reply_to, t.subject, t.body, t.id])?;
        t.id
    };
    Ok(id)
}

pub fn delete_template(connection: &Connection, id: u32) -> Result<()> {
    connection.execute("DELETE FROM send_templates WHERE id_send_template=?1", [id])?;
    Ok(())
}

pub fn get_templates(connection: &Connection) -> Result<SendTemplateVecT> {
    let mut stmt = connection.prepare(
        "SELECT id_send_template, title, address, amount, fiat_amount, fee_included, fiat, include_reply_to, subject, body FROM send_templates")?;
    let templates = stmt.query_map([], |row| {
        let id_msg: u32 = row.get("id_send_template")?;
        let title: String = row.get("title")?;
        let address: String = row.get("address")?;
        let amount: u64 = row.get("amount")?;
        let fiat_amount: f64 = row.get("fiat_amount")?;
        let fee_included: bool = row.get("fee_included")?;
        let fiat: Option<String> = row.get("fiat")?;
        let include_reply_to: bool = row.get("include_reply_to")?;
        let subject: String = row.get("subject")?;
        let body: String = row.get("body")?;

        let template = SendTemplateT {
            id: id_msg,
            title: Some(title),
            address: Some(address),
            amount,
            fiat_amount,
            fee_included,
            fiat,
            include_reply_to,
            subject: Some(subject),
            body: Some(body),
        };
        Ok(template)
    })?;
    let templates: Result<Vec<_>, _> = templates.collect();
    let templates = SendTemplateVecT {
        templates: templates.ok(),
    };
    Ok(templates)
}
