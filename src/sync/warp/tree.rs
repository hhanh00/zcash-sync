use anyhow::Result;
use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use rusqlite::Connection;
use tonic::Request;
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::Node;
use crate::chain::get_latest_height;
use crate::{BlockId, BlockRange, connect_lightwalletd, Hash};
use super::{DEPTH, ReadWrite, Hasher, Path, SaplingHasher};
use super::bridge::{Bridge, CompactLayer};
use super::witness::Witness;

#[derive(Debug)]
pub struct MerkleTree<D: Clone + PartialEq + Debug + ReadWrite> {
    pub pos: usize,
    pub prev: [D; DEPTH+1],
    pub witnesses: Vec<Witness<D>>,
}

impl <D: Clone + PartialEq + Debug + ReadWrite> MerkleTree<D> {
    fn empty<H: Hasher<D>>() -> Self {
        MerkleTree {
            pos: 0,
            prev: std::array::from_fn(|_| H::empty()),
            witnesses: vec![],
        }
    }

    pub fn add_nodes<H: Hasher<D>>(&mut self, block_len: usize, nodes: &[(D, bool)]) -> Bridge<D> {
        // let ns: Vec<_> = nodes.iter().map(|n| n.0).collect();
        // println!("{ns:?}");
        assert!(!nodes.is_empty());
        let mut compact_layers = vec![];
        let mut new_witnesses = vec![];
        for (i, n) in nodes.iter().enumerate() {
            if n.1 {
                self.witnesses.push(Witness {
                    path: Path {
                        pos: self.pos + i,
                        value: n.0.clone(),
                        siblings: vec![],
                    },
                    fills: vec![],
                });
                new_witnesses.push(self.witnesses.len() - 1);
            }
        }
        log::debug!("{:?}", new_witnesses);

        let mut layer = vec![];
        let mut fill = H::empty();
        if !H::is_empty(&self.prev[0]) {
            layer.push(self.prev[0].clone());
            fill = nodes[0].0.clone();
        }
        layer.extend(nodes.iter().map(|n| n.0.clone()));

        for depth in 0..DEPTH {
            let mut new_fill = H::empty();
            let len = layer.len();
            let start = (self.pos >> depth) & 0xFFFE;
            for &wi in new_witnesses.iter() {
                let w = &mut self.witnesses[wi];
                let i = (w.path.pos >> depth) - start;
                if i & 1 == 1 {
                    assert_ne!(layer[i - 1], H::empty());
                    w.path.siblings.push(layer[i - 1].clone());
                }
            }
            for w in self.witnesses.iter_mut() {
                if (w.path.pos >> depth) >= start {
                    let i = (w.path.pos >> depth) - start;
                    if i & 1 == 0 && i < len - 1 && !H::is_empty(&layer[i + 1]) {
                        w.fills.push(layer[i + 1].clone());
                    }
                }
            }
            log::debug!("w {:?}", self.witnesses);

            let pairs = (len + 1) / 2;
            let mut new_layer = vec![];
            if !H::is_empty(&self.prev[depth + 1]) {
                new_layer.push(self.prev[depth + 1].clone());
            }
            self.prev[depth] = H::empty();
            for i in 0..pairs {
                let l = &layer[2 * i];
                if 2 * i + 1 < len {
                    if !H::is_empty(&layer[2 * i + 1]) {
                        let hn = H::combine(depth as u8, l, &layer[2 * i + 1], true);
                        if (i == 0 && !H::is_empty(&self.prev[depth + 1])) ||
                            (i == 1 && H::is_empty(&self.prev[depth + 1])) {
                            new_fill = hn.clone();
                        }
                        new_layer.push(hn.clone());
                    } else {
                        new_layer.push(H::empty());
                        self.prev[depth] = l.clone();
                    }
                } else {
                    if !H::is_empty(l) {
                        self.prev[depth] = l.clone();
                    }
                    new_layer.push(H::empty());
                }
            }

            compact_layers.push(CompactLayer {
                prev: self.prev[depth].clone(),
                fill,
            });

            layer = new_layer;
            fill = new_fill;
            log::debug!("{layer:?}");
        }
        let pos = self.pos;
        self.pos += nodes.len();
        Bridge {
            pos,
            block_len,
            len: nodes.len(),
            layers: compact_layers.try_into().unwrap(),
        }
    }

    pub fn add_bridge<H: Hasher<D>>(&mut self, bridge: &Bridge<D>) {
        for h in 0..DEPTH {
            if !H::is_empty(&bridge.layers[h].fill) {
                let s = self.pos >> (h + 1);
                for w in self.witnesses.iter_mut() {
                    let p = w.path.pos >> h;
                    if p & 1 == 0 && p >> 1 == s {
                        w.fills.push(bridge.layers[h].fill.clone());
                    }
                }
            }
            self.prev[h] = bridge.layers[h].prev.clone();
        }
        self.pos += bridge.len;
    }

    pub fn edge<H: Hasher<D>>(&self, empty_roots: &[D]) -> [D; DEPTH]{
        let mut path = vec![];
        let mut h = H::empty();
        for depth in 0..DEPTH {
            let n = &self.prev[depth];
            if !H::is_empty(n) {
                h = H::combine(depth as u8, n, &h, false);
            }
            else {
                h = H::combine(depth as u8, &h, &empty_roots[depth], false);
            }
            path.push(h.clone());
        }
        path.try_into().unwrap()
    }

    pub fn add_witness(&mut self, w: Witness<D>) {
        self.witnesses.push(w);
    }

    pub fn write<W: Write>(&self, mut w: W) {
        w.write_u64::<LE>(self.pos as u64).unwrap();
        for p in self.prev.iter() {
            p.write(&mut w);
        }
    }

    pub fn read<R: Read>(mut r: R) -> Self {
        let pos = r.read_u64::<LE>().unwrap() as usize;
        let mut prev = vec![];
        for _ in 0..DEPTH+1 {
            let p = D::read(&mut r);
            prev.push(p);
        }
        Self {
            pos,
            prev: prev.try_into().unwrap(),
            witnesses: vec![],
        }
    }
}

pub async fn test_warp(network: Network, connection: &Connection, url: &str) -> Result<()> {
    let mut client = connect_lightwalletd(url).await?;
    let start = u64::from(network.activation_height(NetworkUpgrade::Sapling).unwrap());
    let end = start + 100_000;
    println!("{end}");
    // Retrieve the "official" CMU tree state from the server
    let rep = client.get_tree_state(Request::new(BlockId { height: end, hash: vec![] })).await?.into_inner();
    let mut tree = hex::decode(&rep.sapling_tree).unwrap();
    let tree = CommitmentTree::<Node>::read(&*tree)?;
    // calculate the root hash
    let root = tree.root();
    println!("root  {}", hex::encode(&root.repr));

    // Do the same calculation locally using our Merkle Tree Warp
    // Get the same range from the server
    let mut blocks = client.get_block_range(Request::new(BlockRange {
        start: Some(BlockId { height: start, hash: vec![] }),
        end: Some(BlockId { height: end, hash: vec![] }),
        spam_filter_threshold: 0
    })).await?.into_inner();
    let mut tree = MerkleTree::empty::<SaplingHasher>();

    let mut ref_tree = CommitmentTree::<Node>::empty();
    let mut ref_w = IncrementalWitness::<Node>::from_tree(&ref_tree);
    let mut first = true;
    while let Some(block) = blocks.message().await? {
        // Extract the CMU and add them to the tree
        let mut nodes: Vec<(Hash, bool)> = vec![];
        for tx in block.vtx.iter() {
            for o in tx.outputs.iter() {
                ref_tree.append(Node::new(o.cmu.clone().try_into().unwrap())).unwrap();
                if first {
                    ref_w = IncrementalWitness::from_tree(&ref_tree);
                }
                else {
                    ref_w.append(Node::new(o.cmu.clone().try_into().unwrap())).unwrap();
                }
                nodes.push((o.cmu.clone().try_into().unwrap(), first));
                first = false;
            }
        }
        if !nodes.is_empty() {
            tree.add_nodes::<SaplingHasher>(1, &nodes);
        }
    }

    // Calculate the root of the tree by getting the merkle path of the tree right edge
    let er = super::empty_roots::<_, SaplingHasher>();
    let edge = tree.edge::<SaplingHasher>(&er);
    println!("root2 {}", hex::encode(edge[31]));

    let ref_proof = ref_w.path().unwrap();
    for (n, b) in ref_proof.auth_path.iter() {
        println!("{} {}", hex::encode(&n.repr), b);
    }
    println!("rootw {}", hex::encode(&ref_w.root().repr));

    let w = &tree.witnesses[0];
    for p in w.path.siblings.iter() {
        println!("L {}", hex::encode(p));
    }
    for p in w.fills.iter() {
        println!("R {}", hex::encode(p));
    }
    for p in edge.iter() {
        println!("E {}", hex::encode(p));
    }
    let (root, proof) = w.root::<SaplingHasher>(&er, &edge);
    for p in proof.iter() {
        println!("P {}", hex::encode(p));
    }
    println!("rootw2 {}", hex::encode(&root));

    Ok(())
}
