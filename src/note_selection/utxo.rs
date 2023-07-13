use rusqlite::Connection;
use crate::note_selection::types::Source;
use crate::note_selection::UTXO;
use crate::connect_lightwalletd;

pub async fn fetch_utxos(
    connection: &Connection,
    url: &str,
    account: u32,
    checkpoint_height: u32,
    excluded_pools: u8,
) -> anyhow::Result<Vec<UTXO>> {
    let mut utxos = vec![];
    if excluded_pools & 1 == 0 {
        utxos.extend(crate::note_selection::utxo::get_transparent_utxos(connection, url, account).await?);
    }
    if excluded_pools & 2 == 0 {
        utxos.extend(
            crate::db::checkpoint::get_unspent_received_notes::<'S'>(connection, account, checkpoint_height)?);
    }
    if excluded_pools & 4 == 0 {
        utxos.extend(
            crate::db::checkpoint::get_unspent_received_notes::<'O'>(connection, account, checkpoint_height)?);
    }
    Ok(utxos)
}

async fn get_transparent_utxos(
    connection: &Connection,
    url: &str,
    account: u32) -> anyhow::Result<Vec<UTXO>> {
    let taddr = crate::db::transparent::get_transparent(connection, account)?.and_then(|d| d.address);
    if let Some(taddr) = taddr {
        let mut client = connect_lightwalletd(url).await?;
        let utxos = crate::taddr::get_utxos(&mut client, &taddr).await?;
        let utxos: Vec<_> = utxos
            .iter()
            .map(|utxo| {
                let source = Source::Transparent {
                    txid: utxo.txid.clone().try_into().unwrap(),
                    index: utxo.index as u32,
                };
                UTXO {
                    id: 0,
                    source,
                    amount: utxo.value_zat as u64,
                }
            })
            .collect();
        Ok(utxos)
    } else {
        Ok(vec![])
    }
}
