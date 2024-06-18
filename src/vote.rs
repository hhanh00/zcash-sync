use anyhow::Result;
use rand::rngs::OsRng;
use rusqlite::params;
use zcash_vote::{create_ballot, download_reference_data, vote_data::BallotEnvelopeT, Election};

use crate::{db::data_generated::fb::IdListT, CoinConfig, Connection};

pub fn populate_vote_notes(
    coin: u8,
    account: u32,
    start_height: u32,
    end_height: u32,
) -> Result<()> {
    let c = CoinConfig::get(coin);
    let connection = c.connection();
    connection.execute(
        "CREATE TABLE IF NOT EXISTS vote_notes(
        id_note INTEGER PRIMARY KEY NOT NULL,
        account INTEGER NOT NULL,
        value INTEGER NOT NULL,
        spent INTEGER,
        used BOOL NOT NULL DEFAULT FALSE)",
        [],
    )?;
    connection.execute(
        "INSERT INTO vote_notes(id_note, account, value, spent)
        SELECT id_note, account, value, spent FROM received_notes
        WHERE account = ?1 AND height >= ?2 AND height <= ?3 AND orchard = 1
        ON CONFLICT(id_note) DO UPDATE SET
        spent = EXCLUDED.spent",
        params![account, start_height, end_height],
    )?;
    Ok(())
}

pub fn list_notes(connection: &Connection, account: u32) -> Result<IdListT> {
    let mut s = connection
        .prepare("SELECT id_note FROM vote_notes WHERE account = ?1 AND spent IS NULL")?;
    let rows = s.query_map(params![account], |r| r.get::<_, u32>(0))?;
    let ids = rows.collect::<Result<Vec<_>, _>>()?;
    let ids = IdListT { ids: Some(ids) };
    Ok(ids)
}

pub async fn download_data(coin: u8, election: &str) -> Result<()> {
    let c = CoinConfig::get(coin);
    let lwd_url = c.lwd_url.as_ref().ok_or(anyhow::anyhow!("No LWD"))?;
    let election = serde_json::from_str::<Election>(election)?;
    download_reference_data(&c.connection(), lwd_url, &election).await?;
    Ok(())
}

pub async fn vote(
    coin: u8,
    account: u32,
    id_notes: &[u32],
    candidate: u32,
    election: &str,
) -> Result<BallotEnvelopeT> {
    let c = CoinConfig::get(coin);
    let connection = c.connection();
    let lwd_url = c.lwd_url.ok_or(anyhow::anyhow!("No LWD"))?;
    let election = serde_json::from_str::<Election>(election)?;

    let ballot = create_ballot(
        &connection,
        &lwd_url,
        account,
        &election,
        id_notes,
        candidate,
        OsRng,
    )
    .await?;

    Ok(ballot)
}
