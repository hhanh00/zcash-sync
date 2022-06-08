use crate::coinconfig::CoinConfig;

pub fn mark_message_read(message: u32, read: bool) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    c.db()?.mark_message_read(message, read)?;
    Ok(())
}

pub fn mark_all_messages_read(read: bool) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    c.db()?.mark_all_messages_read(c.id_account, read)?;
    Ok(())
}
