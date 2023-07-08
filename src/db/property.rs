use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub fn get_property(connection: &Connection, name: &str) -> Result<String> {
    let url = connection
        .query_row(
            "SELECT value FROM properties WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(url.unwrap_or(String::new()))
}

pub fn set_property(connection: &Connection, name: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO properties(name, value) VALUES (?1, ?2) ON CONFLICT (name) \
    DO UPDATE SET value = excluded.value",
        params![name, value],
    )?;
    Ok(())
}
