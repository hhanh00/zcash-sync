use anyhow::Result;
use rusqlite::{params, Connection};
use tonic::{transport::Channel, Request};

use crate::{BlockId, BlockRange, CompactTxStreamerClient};

use super::prevhash::PreviousHashes;

pub struct Election {
    pub start_height: u32,
    pub end_height: u32,
}

pub async fn download_reference_data(
    connection: &Connection,
    client: &mut CompactTxStreamerClient<Channel>,
    election: &Election,
) -> Result<()> {
    connection.execute("DELETE FROM nullifiers", [])?;
    connection.execute("DELETE FROM cmxs", [])?;
    let mut block_stream = client
        .get_block_range(Request::new(BlockRange {
            start: Some(BlockId {
                height: election.start_height as u64,
                hash: vec![],
            }),
            end: Some(BlockId {
                height: election.end_height as u64,
                hash: vec![],
            }),
            spam_filter_threshold: 0,
        }))
        .await?
        .into_inner();

    let mut s_nf = connection.prepare(
        "INSERT INTO nullifiers(hash, revhash)
        VALUES (?1, ?2)",
    )?;
    let mut s_cmx = connection.prepare(
        "INSERT INTO cmxs(hash)
        VALUES (?1)",
    )?;
    let mut pos = 0;
    while let Some(block) = block_stream.message().await? {
        for tx in block.vtx.iter() {
            for a in tx.actions.iter() {
                let nf = &*a.nullifier;
                let mut rev_nf = [0u8; 32];
                rev_nf.copy_from_slice(nf);
                rev_nf.reverse();
                s_nf.execute(params![nf, &rev_nf])?;
                let cmx = &*a.cmx;
                s_cmx.execute(params![cmx])?;
                pos += 1;
            }
        }
    }
    if pos & 1 == 1 {
        let er = orchard::pob::empty_hash();
        s_cmx.execute(params![&er])?;
    }
    Ok(())
}

pub fn build_merkle_tree<F>(
    connection: &Connection,
    table_name: &str,
    prev: &PreviousHashes,
    builder: F,
) -> Result<()>
where
    F: Fn(&Connection) -> Result<()>,
{
    connection.execute(&format!("DROP TABLE IF EXISTS {table_name}"), [])?;
    connection.execute(
        &format!(
            "CREATE TABLE {table_name}(
            id INTEGER PRIMARY KEY NOT NULL,
            depth INTEGER NOT NULL,
            hash BLOB NOT NULL)"
        ),
        [],
    )?;

    let mut er = orchard::pob::empty_hash();
    let mut add = connection.prepare(&format!(
        "INSERT INTO {table_name}(depth, hash)
        VALUES (?1, ?2)"
    ))?;
    if let Some(l) = prev.lefts[0] {
        add.execute(params![0, &l])?;
    }
    builder(connection)?; // populates the leaves

    for depth in 0..32 {
        // pad to even count
        let c = connection.query_row(
            &format!("SELECT COUNT(*) FROM {table_name} WHERE depth = ?1"),
            [depth],
            |r| r.get::<_, u32>(0),
        )?;
        if c & 1 == 1 {
            add.execute(params![depth, &er])?;
        }

        if depth < 31 {
            if let Some(l) = prev.lefts[depth + 1] {
                add.execute(params![depth + 1, &l])?;
            }
        }

        let mut hashes = connection.prepare(&format!(
            "SELECT hash FROM {table_name} WHERE depth = ?1 ORDER BY id"
        ))?;
        let hashes = hashes.query_map([depth], |r| r.get::<_, Vec<u8>>(0))?;
        let mut left = None;
        for h in hashes {
            let h: [u8; 32] = h?.try_into().unwrap();
            match left {
                Some(l) => {
                    let parent = orchard::pob::cmx_hash(depth as u8, &l, &h);
                    add.execute(params![depth + 1, &parent])?;
                    left = None;
                }
                None => {
                    left = Some(h);
                }
            }
        }

        er = orchard::pob::cmx_hash(depth as u8, &er, &er)
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::vote::prevhash::{fetch_tree_state, get_tree_root};

    use super::Election;

    #[tokio::test]
    async fn test() -> anyhow::Result<()> {
        let e = Election {
            start_height: 2541400,
            end_height: 2541500,
        };
        crate::set_coin_lwd_url(0, "https://lwd5.zcash-infra.com:9067");
        let mut c = crate::CoinConfig::get(0);
        c.set_db_path("/Users/hanhhuynhhuu/Library/Containers/me.hanh.ywallet/Data/Library/Application Support/me.hanh.ywallet/databases/zec.db")?;

        let mut client = c.connect_lwd().await?;
        let connection = c.connection();

        crate::vote::db::create_tables(&connection)?;

        let start_root = get_tree_root(&mut client, e.start_height - 1).await?;
        println!("start root: {}", hex::encode(start_root));

        let end_root = get_tree_root(&mut client, e.end_height).await?;
        println!("end root: {}", hex::encode(end_root));

        super::download_reference_data(&connection, &mut client, &e).await?;
        // super::build_nullifier_tree(&connection)?;
        // super::build_cmx_tree(&connection)?;
        // super::build_inner_nodes(&connection, "tree_nullifiers")?;
        // super::build_inner_nodes(&connection, "tree_cmxs")?;

        super::build_merkle_tree(
            &connection,
            "nullifier_tree",
            &super::PreviousHashes::default(),
            |connection| {
                connection.execute("INSERT INTO nullifier_tree(depth, hash)
                SELECT 0, hash FROM nullifiers ORDER BY revhash", [])?;
                Ok(())
            },
        )?;

        let ph = fetch_tree_state(&mut client, e.start_height - 1).await?;
        super::build_merkle_tree(
            &connection,
            "cmx_tree",
            &ph,
            |connection| {
                connection.execute("INSERT INTO cmx_tree(depth, hash)
                SELECT 0, hash FROM cmxs ORDER BY id_cmx", [])?;
                Ok(())
            },
        )?;
        Ok(())
    }
}
