use anyhow::Result;
use std::fmt::{Debug, Formatter};
use std::fs::File;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use tonic::Request;
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};
use zcash_primitives::sapling::Node;
use crate::{BlockId, BlockRange, connect_lightwalletd, fb, Hash};
use crate::sync::warp::tree::from_tree_state;
use self::{tree::MerkleTree, bridge::Bridge};

pub const DEPTH: usize = 32usize;

pub trait ReadWrite {
    fn write<W: Write>(&self, w: W) -> Result<()>;
    fn read<R: Read>(r: R) -> Result<Self> where Self: Sized;
}

pub trait Hasher {
    type D: Clone + PartialEq + Debug + ReadWrite;
    fn empty() -> Self::D;
    fn is_empty(d: &Self::D) -> bool;
    fn combine(depth: u8, l: &Self::D, r: &Self::D, check: bool) -> Self::D;
    fn parallel_combine(depth: u8, layer: &[Self::D], pairs: usize) -> Vec<Self::D>;
}

impl ReadWrite for Hash {
    fn write<W: Write>(&self, mut w: W) -> Result<()> {
        w.write_all(self)?;
        Ok(())
    }

    fn read<R: Read>(mut r: R) -> Result<Self> {
        let mut h = [0u8; 32];
        r.read_exact(&mut h)?;
        Ok(h)
    }
}

pub struct SaplingHasher;

const SAPLING_EMPTY: Hash = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

impl Hasher for SaplingHasher {
    type D = Hash;
    fn empty() -> Hash {
        SAPLING_EMPTY
    }

    fn is_empty(d: &Hash) -> bool {
        *d == SAPLING_EMPTY
    }

    fn combine(depth: u8, l: &Hash, r: &Hash, _check: bool) -> Hash {
        // println!("> {} {} {}", depth, hex::encode(l), hex::encode(r));
        crate::sapling::sapling_hash(depth, l, r)
    }

    fn parallel_combine(depth: u8, layer: &[[u8; 32]], pairs: usize) -> Vec<Hash> {
        crate::sapling::sapling_parallel_hash(depth, layer, pairs)
    }
}

pub mod witness;
pub mod tree;
pub mod bridge;

pub struct Path<H: Hasher> {
    pub value: H::D,
    pub pos: usize,
    pub siblings: Vec<H::D>,
}

impl <H: Hasher> Debug for Path<H> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "value {:?}", self.value)?;
        write!(f, ", pos {}", self.pos)?;
        writeln!(f, ", siblings {:?}", self.siblings)
    }
}

impl <H: Hasher> Path<H> {
    pub fn empty() -> Self {
        Path {
            value: H::empty(),
            pos: 0,
            siblings: vec![],
        }
    }

    fn write<W: Write>(&self, mut w: W) -> Result<()> {
        w.write_u64::<LE>(self.pos as u64)?;
        self.value.write(&mut w)?;
        w.write_u8(self.siblings.len() as u8)?;
        for s in self.siblings.iter() {
            s.write(&mut w)?;
        }
        Ok(())
    }

    fn read<R: Read>(mut r: R) -> Result<Self> {
        let pos = r.read_u64::<LE>()? as usize;
        let value = H::D::read(&mut r)?;
        let len = r.read_u8()? as usize;
        let mut siblings = vec![];
        for _ in 0..len {
            let s = H::D::read(&mut r)?;
            siblings.push(s);
        }
        Ok(Self {
            value,
            pos,
            siblings,
        })
    }
}

pub fn empty_roots<H: Hasher>() -> [H::D; DEPTH] {
    let mut roots = vec![];
    roots.push(H::empty());
    for i in 0..DEPTH-1 {
        roots.push(H::combine(i as u8, &roots[i], &roots[i], false));
    }
    roots.try_into().unwrap()
}

pub fn migrate_db(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS warp_tunnels(
            height INTEGER PRIMARY KEY NOT NULL,
            block_len INTEGER NOT NULL,
            pos INTEGER NOT NULL,
            len INTEGER NOT NULL,
            data BLOB NOT NULL)",
        [],
    )?;
    Ok(())
}

pub fn import_tunnels(network: &Network, connection: &Connection, filename: &str) -> Result<()> {
    let mut height = u64::from(network.activation_height(NetworkUpgrade::Sapling).unwrap());
    let mut s = connection.prepare("INSERT INTO warp_tunnels(height, block_len, pos, len, data) \
        VALUES (?1, ?2, ?3, ?4, ?5)")?;
    let mut file = File::open(filename)?;
    while let Ok(bridge) = Bridge::<SaplingHasher>::read(&mut file) {
        println!("{}", bridge.height);
        let mut data = vec![];
        bridge.write(&mut data)?;
        s.execute(params![height, bridge.block_len, bridge.pos, bridge.len, data])?;
        height += bridge.block_len as u64;
    }

    Ok(())
}

pub async fn calc_merkle_proof(network: &Network, connection: &Connection, url: &str, account: u32) -> Result<()> {
    let mut height = u64::from(network.activation_height(NetworkUpgrade::Sapling).unwrap());
    let checkpoint_height = crate::db::checkpoint::get_last_sync_height(network, connection, None)?;
    let mut s = connection.prepare("SELECT id_note, position, value FROM received_notes WHERE account = ?1 AND orchard = 0")?;
    let notes = s.query_map([account], |r| {
        Ok(fb::NoteT {
            id: r.get(0)?,
            account,
            pos: r.get(1)?,
            value: r.get(2)?
        })
    })?;
    let notes= notes.collect::<Result<Vec<_>, _>>()?;
    for n in notes.iter() {
        println!("{:?}", n);
    }

    let mut s = connection.prepare("SELECT data FROM warp_tunnels ORDER BY height")?;
    let bridges = s.query_map([], |r| {
        let bridge = Bridge::<SaplingHasher>::read(&*r.get::<_, Vec<u8>>(0)?).unwrap();
        Ok(bridge)
    })?;
    let bridges = bridges.collect::<Result<Vec<_>, _>>()?;

    let mut client = connect_lightwalletd(url).await?;
    let mut tree = MerkleTree::<SaplingHasher>::empty();
    for b in bridges {
        println!("{} {}", b.pos, b.len);
        assert_eq!(b.pos, tree.pos);
        let new_notes: Vec<_> = notes.iter().filter(|n| (n.pos as usize) >= b.pos && (n.pos as usize) < b.pos + b.len).collect();
        if !new_notes.is_empty() {
            println!("Fetch blocks {} {}", height, b.block_len);
            let mut blocks = client.get_block_range(Request::new(BlockRange {
                start: Some(BlockId { height, hash: vec![] }),
                end: Some(BlockId { height: height + b.block_len as u64 - 1, hash: vec![] }),
                spam_filter_threshold: 50,
            })).await?.into_inner();
            let mut nodes = vec![];
            while let Some(block) = blocks.message().await? {
                for tx in block.vtx.iter() {
                    for o in tx.outputs.iter() {
                        nodes.push((o.cmu.clone().try_into().unwrap(), false));
                    }
                }
            }
            for &n in new_notes.iter() {
                println!("Adding witness {}", n.pos);
                nodes[n.pos as usize - b.pos].1 = true;
            }
            println!("Processing blocks");
            tree.add_nodes(b.height, b.block_len, &nodes);
        }
        else {
            tree.add_bridge(&b);
        }
        height += b.block_len as u64;
    }

    Ok(())
}

pub async fn build_bridges(connection: &Connection, url: &str, path: &std::path::Path) -> Result<()> {
    let er = empty_roots::<SaplingHasher>();
    let mut blocks = File::open(path.join("block.dat"))?;
    let mut bridges = File::create(path.join("bridge.dat"))?;
    let checkpoints = crate::db::checkpoint::list_checkpoints(connection)?.checkpoints.unwrap();
    let mut heights = checkpoints.iter();
    let mut tree = MerkleTree::<SaplingHasher>::empty();
    let mut nodes = vec![];
    let mut big_total = 0;
    let mut total = 0;
    let mut start = 0;
    let mut height = 0;
    let mut next_height = heights.next().unwrap().height;
    while let Ok(h) = blocks.read_u32::<LE>() {
        height = h;
        if start == 0 {
            start = height;
        }
        let count = blocks.read_u32::<LE>().unwrap();
        for _ in 0..count {
            let mut hash = [0u8; 32];
            blocks.read_exact(&mut hash).unwrap();
            nodes.push((hash, false));
            // ref_tree.append(zcash_primitives::sapling::Node::new(hash)).unwrap();
        }
        total += count;
        big_total += count;
        if height == next_height {
            match heights.next() {
                Some(cp) => {
                    next_height = cp.height;
                    let block_len  = height - start + 1;
                    println!("{start} {height} {count} {block_len}");
                    let bridge = tree.add_nodes(start, block_len, &nodes);
                    check_tree(url, height, &tree, &er).await?;
                    bridge.write(&mut bridges)?;
                    start = 0;
                    total = 0;
                    nodes.clear();
                }
                None => break,
            }
        }
        if total > 100_000 {
            println!("{start} {height} {count}");
            let block_len  = height - start + 1;
            let bridge = tree.add_nodes(start, block_len, &nodes);
            check_tree(url, height, &tree, &er).await?;
            bridge.write(&mut bridges)?;
            start = 0;
            total = 0;
            nodes.clear();
        }
        // if big_total > 1000_000 { break }
    }
    if !nodes.is_empty() {
        let block_len  = height - start + 1;
        let bridge = tree.add_nodes(start, block_len, &nodes);
        bridge.write(&mut bridges)?;
    }
    Ok(())
}

async fn check_tree(url: &str, height: u32, tree: &MerkleTree<SaplingHasher>, er: &[Hash]) -> Result<()> {
    let edge = tree.edge(er);
    let root = edge[31].clone();
    println!("{} {}", height, hex::encode(&root));
    let mut client = connect_lightwalletd(url).await?;
    let rep = client.get_tree_state(Request::new(BlockId { height: height as u64, hash: vec![] })).await?.into_inner();
    let tree = hex::decode(&rep.sapling_tree).unwrap();
    let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
    // calculate the root hash
    let root = tree.root();
    println!("server root {}", hex::encode(&root.repr));
    Ok(())
}

pub async fn test_bridges(connection: &Connection, url: &str) -> Result<()> {
    let mut s = connection.prepare("SELECT data FROM warp_tunnels ORDER BY height")?;
    let bridges = s.query_map([], |r| {
        let bridge = Bridge::<SaplingHasher>::read(&*r.get::<_, Vec<u8>>(0)?).unwrap();
        Ok(bridge)
    })?;
    let bridges = bridges.collect::<Result<Vec<_>, _>>()?;
    let mut tree = MerkleTree::<SaplingHasher>::empty();
    let er = empty_roots::<SaplingHasher>();
    let mut client = connect_lightwalletd(url).await?;

    for b in bridges.iter().take(10) {
        tree.add_bridge(&b);
        let edge = tree.edge(&er);
        let root = edge[31].clone();
        println!("{}", hex::encode(&root));

        let end = b.height + b.block_len - 1;
        println!("{}", end);
        let rep = client.get_tree_state(Request::new(BlockId { height: end as u64, hash: vec![] })).await?.into_inner();
        let tree = hex::decode(&rep.sapling_tree).unwrap();
        let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
        // calculate the root hash
        let root = tree.root();
        println!("server root {}", hex::encode(&root.repr));
    }
    Ok(())
}

struct Note {
    height: u32,
    position: u64,
}

pub async fn recover_tree(url: &str, height: u32) -> Result<()> {
    let mut client = connect_lightwalletd(url).await?;
    for i in 0..10 {
        let h = height + i;
        let rep = client.get_tree_state(Request::new(BlockId { height: h as u64, hash: vec![] })).await?.into_inner();
        let tree = hex::decode(&rep.sapling_tree).unwrap();
        let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
        let root1 = tree.root();
        println!("server root {}", hex::encode(&root1.repr));
        let tree = from_tree_state::<SaplingHasher>(&tree);
        let er = empty_roots::<SaplingHasher>();
        let edge = tree.edge(&er);
        let root2 = edge[31].clone();
        println!("{}", hex::encode(&root2));
        assert_eq!(root1.repr, root2);
    }

    Ok(())
}

pub async fn get_merkle_proof(connection: &Connection, url: &str, id_note: u32, target_height: u32) -> Result<()> {
    let note = connection.query_row("SELECT height, position FROM received_notes WHERE id_note = ?1",
    [id_note], |r| {
            Ok(Note {
                height: r.get(0)?,
                position: r.get(1)?,
            })
        })?;
    let mut client = connect_lightwalletd(url).await?;
    let prev_height = note.height - 1;
    let rep = client.get_tree_state(Request::new(BlockId { height: prev_height as u64, hash: vec![] })).await?.into_inner();
    let tree = hex::decode(&rep.sapling_tree).unwrap();
    let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
    let start_pos = tree.size();
    let mut tree = from_tree_state::<SaplingHasher>(&tree);

    let mut s = connection.prepare("SELECT data FROM warp_tunnels WHERE height > ?1 AND height + block_len <= ?2 ORDER BY height")?;
    let bridges = s.query_map([note.height, target_height], |r| {
        let bridge = Bridge::<SaplingHasher>::read(&*r.get::<_, Vec<u8>>(0)?).unwrap();
        Ok(bridge)
    })?;
    let bridges = bridges.collect::<Result<Vec<_>, _>>()?;
    assert!(!bridges.is_empty()); // test is not useful otherwise

    let mut blocks = client.get_block_range(Request::new(BlockRange {
        start: Some(BlockId { height: note.height as u64, hash: vec![] }),
        end: Some(BlockId { height: bridges[0].height as u64 - 1, hash: vec![] }),
        spam_filter_threshold: 50,
    })).await?.into_inner();
    let mut nodes: Vec<(Hash, bool)> = vec![];
    while let Some(block) = blocks.message().await? {
        println!("Processing Block {}", block.height);
        for tx in block.vtx.iter() {
            for o in tx.outputs.iter() {
                nodes.push((o.cmu.clone().try_into().unwrap(), false));
            }
        }
    }
    let rel_pos = note.position as usize - start_pos;
    nodes[rel_pos].1 = true;
    tree.add_nodes(note.height, bridges[0].height - note.height, &nodes);
    let mut bridge_end = 0;
    let er = empty_roots::<SaplingHasher>();
    let w = &tree.witnesses[0];
    let edge = tree.edge(&er);
    let (root, _proof) = w.root(&er, &edge);
    println!("Tree Root {}", hex::encode(&edge[31]));
    println!("Witness Root {}", hex::encode(root));
    for b in bridges {
        println!("Processing Bridge {} to {}", b.height, b.height + b.block_len);
        for l in b.layers.iter() {
            println!("{} {}", hex::encode(&l.fill), hex::encode(&l.prev));
        }
        bridge_end = b.height + b.block_len;
        tree.add_bridge(&b);
        let w = &tree.witnesses[0];
        let edge = tree.edge(&er);
        let (root, _proof) = w.root(&er, &edge);
        println!("Tree Root {}", hex::encode(&edge[31]));
        println!("Witness Root {}", hex::encode(root));
    }
    let mut blocks = client.get_block_range(Request::new(BlockRange {
        start: Some(BlockId { height: bridge_end as u64, hash: vec![] }),
        end: Some(BlockId { height: target_height as u64, hash: vec![] }),
        spam_filter_threshold: 50,
    })).await?.into_inner();
    let mut nodes: Vec<(Hash, bool)> = vec![];
    while let Some(block) = blocks.message().await? {
        println!("Processing Block {}", block.height);
        for tx in block.vtx.iter() {
            for o in tx.outputs.iter() {
                nodes.push((o.cmu.clone().try_into().unwrap(), false));
            }
        }
    }
    tree.add_nodes(bridge_end, target_height - bridge_end + 1, &nodes);

    let w = &tree.witnesses[0];
    let edge = tree.edge(&er);
    let (root, proof) = w.root(&er, &edge);
    for p in proof.iter() {
        println!("Path {}", hex::encode(p));
    }
    println!("Tree Root {}", hex::encode(&edge[31]));
    println!("Witness Root {}", hex::encode(root));

    let rep = client.get_tree_state(Request::new(BlockId { height: target_height as u64, hash: vec![] })).await?.into_inner();
    let tree = hex::decode(&rep.sapling_tree).unwrap();
    let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
    let root2 = tree.root();
    println!("Server Tree Root {}", hex::encode(&root2.repr));

    Ok(())
}
