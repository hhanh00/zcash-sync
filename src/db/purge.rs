use anyhow::Result;
use rusqlite::Connection;
use zcash_primitives::consensus::Network;

pub fn mark_spent(connection: &Connection, id: u32, height: u32) -> anyhow::Result<()> {
    connection.execute(
        "UPDATE received_notes SET spent = ?1 WHERE id_note = ?2",
        [height, id],
    )?;
    Ok(())
}

pub fn truncate_data(connection: &Connection) -> Result<()> {
    truncate_sync_data(connection)?;
    connection.execute("DELETE FROM diversifiers", [])?;
    Ok(())
}

pub fn truncate_sync_data(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM blocks", [])?;
    connection.execute("DELETE FROM sapling_tree", [])?;
    connection.execute("DELETE FROM orchard_tree", [])?;
    connection.execute("DELETE FROM sapling_cmtree", [])?;
    connection.execute("DELETE FROM orchard_cmtree", [])?;
    connection.execute("DELETE FROM contacts", [])?;
    connection.execute("DELETE FROM diversifiers", [])?;
    connection.execute("DELETE FROM historical_prices", [])?;
    connection.execute("DELETE FROM received_notes", [])?;
    connection.execute("DELETE FROM sapling_witnesses", [])?;
    connection.execute("DELETE FROM orchard_witnesses", [])?;
    connection.execute("DELETE FROM sapling_cmwitnesses", [])?;
    connection.execute("DELETE FROM orchard_cmwitnesses", [])?;
    connection.execute("DELETE FROM transactions", [])?;
    connection.execute("DELETE FROM messages", [])?;
    Ok(())
}

pub fn delete_incomplete_scan(connection: &mut Connection, network: &Network) -> Result<()> {
    let synced_height = super::checkpoint::get_last_sync_height(network, connection, None)?;
    super::checkpoint::trim_to_height(connection, synced_height)?;
    Ok(())
}

pub fn delete_account(connection: &Connection, account: u32) -> Result<()> {
    connection.execute("DELETE FROM received_notes WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM transactions WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM diversifiers WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM accounts WHERE id_account = ?1", [account])?;
    connection.execute("DELETE FROM taddrs WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM orchard_addrs WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM ua_settings WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM messages WHERE account = ?1", [account])?;
    connection.execute("DELETE FROM hw_wallets WHERE account = ?1", [account])?;
    Ok(())
}

pub fn delete_orphan_transactions(connection: &Connection) -> anyhow::Result<()> {
    connection.execute("DELETE FROM transactions WHERE id_tx IN (SELECT tx.id_tx FROM transactions tx LEFT JOIN accounts a ON tx.account = a.id_account WHERE a.id_account IS NULL)",
                            [])?;
    Ok(())
}

pub fn clear_tx_details(connection: &Connection, account: u32) -> Result<()> {
    connection.execute(
        "UPDATE transactions SET address = NULL, memo = NULL WHERE account = ?1",
        [account],
    )?;
    connection.execute("DELETE FROM messages WHERE account = ?1", [account])?;
    Ok(())
}
