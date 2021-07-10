use rusqlite::{NO_PARAMS, Connection, OptionalExtension, params};

pub fn get_schema_version(connection: &Connection) -> anyhow::Result<u32> {
    let version: Option<u32> = connection.query_row("SELECT version FROM schema_version WHERE id = 1", NO_PARAMS,
    |row| row.get(0)).optional()?;
    Ok(version.unwrap_or(0))
}

pub fn update_schema_version(connection: &Connection, version: u32) -> anyhow::Result<()> {
    connection.execute("INSERT INTO schema_version(id, version) VALUES (1, ?1) \
    ON CONFLICT (id) DO UPDATE SET version = excluded.version", params![version])?;
    Ok(())
}

