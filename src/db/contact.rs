use crate::contact::Contact;
use crate::db::data_generated::fb::{ContactT, ContactVecT};
use crate::unified::orchard_as_unified;
use anyhow::Result;
use orchard::keys::{FullViewingKey, Scope};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use zcash_client_backend::address::RecipientAddress;
use zcash_client_backend::encoding::AddressCodec;
use zcash_primitives::consensus::Network;

pub fn store_contact(connection: &Connection, contact: &ContactT, dirty: bool) -> Result<()> {
    if contact.id == 0 {
        connection.execute(
            "INSERT INTO contacts(name, address, dirty)
                VALUES (?1, ?2, ?3)",
            params![&contact.name.unwrap(), &contact.address.unwrap(), dirty],
        )?;
    } else {
        connection.execute(
            "INSERT INTO contacts(id, name, address, dirty)
                VALUES (?1, ?2, ?3, ?4) ON CONFLICT (id) DO UPDATE SET
                name = excluded.name, address = excluded.address, dirty = excluded.dirty",
            params![contact.id, &contact.name.unwrap(), &contact.address.unwrap(), dirty],
        )?;
    }
    Ok(())
}

pub fn list_unsaved_contacts(connection: &Connection) -> Result<Vec<Contact>> {
    let mut statement =
        connection.prepare("SELECT id, name, address FROM contacts WHERE dirty = TRUE")?;
    let contacts = statement.query_map([], |r| {
        Ok(Contact {
            id: r.get(0)?,
            name: r.get(1)?,
            address: r.get(2)?,
        })
    })?;
    let contacts: Result<Vec<_>, _> = contacts.collect();
    Ok(contacts?)
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

pub fn get_contacts(connection: &Connection) -> Result<ContactVecT> {
    let mut stmt = connection
        .prepare("SELECT id, name, address FROM contacts WHERE address <> '' ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        let id: u32 = row.get("id")?;
        let name: String = row.get("name")?;
        let address: String = row.get("address")?;
        Ok(ContactT {
            id,
            name: Some(name),
            address: Some(address),
        })
    })?;
    let contacts: Result<Vec<_>, _> = rows.collect();
    let contacts = ContactVecT {
        contacts: contacts.ok(),
    };
    Ok(contacts)
}
