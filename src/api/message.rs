//! Mark messages read

/// Mark a given message as read or unread
/// # Arguments
/// * `message`: message id
/// * `read`: read or unread
pub fn mark_message_read(message: u32, read: bool) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    c.db()?.mark_message_read(message, read)?;
    Ok(())
}

/// Mark all messages as read or unread
/// # Arguments
/// * `read`: read or unread
pub fn mark_all_messages_read(read: bool) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    c.db()?.mark_all_messages_read(c.id_account, read)?;
    Ok(())
}
