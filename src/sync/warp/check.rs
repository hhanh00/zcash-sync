use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::slice;
use anyhow::Result;
use std::time::{Duration, Instant};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use rusqlite::Connection;
use zcash_primitives::merkle_tree::IncrementalWitness;
use crate::sync::Witness;
use zcash_primitives::sapling::{Node, SaplingIvk};
use crate::{CompactBlock, CompactSaplingOutput, Hash, lw_rpc};
use crate::sync::warp;
use crate::sync::warp::hasher::{SaplingHasher, OrchardHasher};
use tonic::{Request, Response};
use crate::connect_lightwalletd;
use crate::lw_rpc::*;
use prost::Message;
use rayon::prelude::*;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_note_encryption::batch::try_compact_note_decryption;
use zcash_note_encryption::{EphemeralKeyBytes, ShieldedOutput};
use zcash_primitives::consensus::{BlockHeight, Network, Parameters};
use zcash_primitives::sapling::keys::PreparedIncomingViewingKey;
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use crate::sync::warp::{MerkleTree, Bridge};
use zcash_primitives::sapling::Note;

struct W {
    pub id: u32,
    pub data: Vec<u8>,
}

pub fn check_warp2(connection: &Connection, height: u32) -> Result<()> {
    let mut s = connection.prepare("SELECT note, witness FROM sapling_witnesses WHERE height = ?1")?;
    let ws = s.query_map([height], |r| {
        Ok(W {
            id: r.get(0)?,
            data: r.get::<_, Vec<u8>>(1)?,
        })
    })?;
    let ws1 = ws.collect::<Result<Vec<_>, _>>()?;

    let h = warp::SaplingHasher::default();
    let er = warp::empty_roots(&h);
    let data = connection.query_row("SELECT data FROM sapling_cmtree WHERE height = ?1", [height], |r|
        r.get::<_, Vec<u8>>(0))?;
    let tree = warp::MerkleTree::read(&*data, h.clone())?;
    let edge = tree.edge(&er);
    println!("ROOT {}", hex::encode(&edge[31]));

    let mut s = connection.prepare("SELECT note, data FROM sapling_cmwitnesses WHERE height = ?1")?;
    let ws = s.query_map([height], |r| {
        Ok(W {
            id: r.get(0)?,
            data: r.get::<_, Vec<u8>>(1)?,
        })
    })?;

    let ws2 = ws.collect::<Result<Vec<_>, _>>()?;
    for (w1, w2) in ws1.iter().zip(ws2.iter()) {
        println!("WITNESS {}", w1.id);
        let w1 = IncrementalWitness::<Node>::read(&*w1.data)?;
        let path1 = w1.path().unwrap();

        let w2 = warp::Witness::<SaplingHasher>::read(&*w2.data)?;
        let (root, path2) = w2.root(&er, &edge, &h);
        println!("ROOT {}", hex::encode(&root));

        for ((p1, _), p2) in path1.auth_path.iter().zip(path2.iter()) {
            // assert_eq!(p1.repr, *p2);
            if p1.repr != *p2 {
                println!("{} {}", hex::encode(&p1.repr), hex::encode(p2));
            }
        }
    }

    Ok(())
}

pub fn process_block_file(filename: &str) -> Result<()> {
    let f = File::open(filename)?;
    let mut f = BufReader::new(f);
    let of = File::create("compact.dat")?;
    let mut of = BufWriter::new(of);
    let mut buf = vec![0; 4_000_000];
    let mut z_tree = MerkleTree::empty(SaplingHasher::default());
    let mut o_tree = MerkleTree::empty(OrchardHasher::default());
    let mut z_tree_block = MerkleTree::empty(SaplingHasher::default());
    let mut o_tree_block = MerkleTree::empty(OrchardHasher::default());
    while let Ok(len) = f.read_u32::<LE>() {
        let len = len as usize;
        f.read_exact(&mut buf[0..len])?;
        let mut block = CompactBlock::decode(&buf[0..len])?;
        println!("{}", block.height);

        let mut z_nodes: Vec<(Hash, bool)> = vec![];
        let mut o_nodes: Vec<(Hash, bool)> = vec![];
        for tx in block.vtx.iter_mut() {
            let nodes: Vec<(Hash, bool)> = tx.outputs.iter().map(|o| (o.cmu.clone().try_into().unwrap(), false)).collect();
            z_nodes.extend(nodes.iter());
            if !nodes.is_empty() {
                let b = z_tree.add_nodes(0, 0, &nodes);
                if tx.outputs.iter().any(|o| o.epk.is_empty()) { // spam filtered
                    let mut bb = vec![];
                    b.write(&mut bb, &z_tree.h)?;
                    tx.outputs.clear();
                    tx.spends.clear();
                    let bridge = lw_rpc::Bridge { len: nodes.len() as u32, data: bb };
                    tx.sapling_bridge = Some(bridge);
                }
            }

            let nodes: Vec<(Hash, bool)> = tx.actions.iter().map(|o| (o.cmx.clone().try_into().unwrap(), false)).collect();
            o_nodes.extend(nodes.iter());
            if !nodes.is_empty() {
                let b = o_tree.add_nodes(0, 0, &nodes);
                if tx.actions.iter().any(|o| o.ephemeral_key.is_empty()) { // spam filtered
                    let mut bb = vec![];
                    b.write(&mut bb, &o_tree.h)?;
                    tx.actions.clear();
                    let bridge = lw_rpc::Bridge { len: nodes.len() as u32, data: bb };
                    tx.orchard_bridge = Some(bridge);
                }
            }
        }
        if !z_nodes.is_empty() {
            let b = z_tree_block.add_nodes(block.height as u32, 1, &z_nodes);
            let mut bb = vec![];
            b.write(&mut bb, &z_tree_block.h)?;
            let bridge = lw_rpc::Bridge { len: z_nodes.len() as u32, data: bb };
            block.sapling_bridge = Some(bridge);
        }
        if !o_nodes.is_empty() {
            let b = o_tree_block.add_nodes(block.height as u32, 1, &o_nodes);
            let mut bb = vec![];
            b.write(&mut bb, &o_tree_block.h)?;
            let bridge = lw_rpc::Bridge { len: o_nodes.len() as u32, data: bb };
            block.orchard_bridge = Some(bridge);
        }

        of.write_u32::<LE>(block.encoded_len() as u32)?;
        let mut bb = vec![];
        block.encode(&mut bb)?;
        of.write(&bb)?;
    }

    Ok(())
}


pub async fn full_scan(network: &Network, url: &str, phrase: &str) -> Result<()> {
    let (_, _, zfvk, pa, _ofvk) = crate::key::decode_key(network, phrase, 0)?;
    println!("{pa}");
    let zfvk = decode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &zfvk).unwrap();
    let nk = &zfvk.fvk.vk.nk;
    let ivk = zfvk.fvk.vk.ivk();
    let pivk = PreparedIncomingViewingKey::new(&ivk);
    let f = File::open("compact.dat")?;
    let mut f = BufReader::new(f);
    let final_height = 2166554;
    let mut block_chunk = vec![];
    let mut block_height = 0;
    let mut height = 0;
    let mut tx_count = 0;
    let mut pos = 0;
    let mut cmtree = MerkleTree::empty(SaplingHasher::default());
    let mut nfs: HashMap<[u8; 32], (u32, u64)> = HashMap::new();
    let mut balance = 0i64;

    let start_time = Instant::now();
    while let Ok(len) = f.read_u32::<LE>() {
        let mut buf = vec![0; len as usize];
        f.read_exact(&mut buf)?;
        let cb = CompactBlock::decode(&*buf)?;
        block_height = cb.height;
        tx_count += cb.vtx.len();
        block_chunk.push(cb);

        if tx_count > 100_000 || block_height == final_height {
            height = block_height;
            println!("Height: {height}");
            let dec_block_chunk: Vec<_> = block_chunk
                .par_iter()
                .map(|b| decrypt_block(network, b, &pivk).unwrap())
                .collect();

            let mut notes = vec![];
            let mut bridges: Option<Bridge<SaplingHasher>> = None;
            let mut pos_start = pos;
            for db in dec_block_chunk.iter() {
                for n in db.notes.iter() {
                    let note = &n.1;
                    let p = pos + n.0;
                    let nf = note.nf(nk, p as u64);
                    let nv = note.value.inner();
                    balance += nv as i64;
                    nfs.insert(nf.0, (p, nv));
                    notes.push((p, n.1.clone()));
                }
                pos += db.count_outputs;
            }

            let mut cmus: Vec<(Hash, bool)> = vec![];
            for (b, db) in block_chunk.iter().zip(dec_block_chunk.iter()) {
                if db.notes.is_empty() { // block has no new notes, use the block bridge
                    // flush bridges or cmus (only one should exist)
                    if let Some(bridge) = bridges.take() { // flush bridges
                        cmtree.add_bridge(&bridge);
                        pos_start += bridge.len as u32;
                    }
                    if !cmus.is_empty() { // flush nodes
                        cmtree.add_nodes(0, 0, &cmus);
                        cmus.clear();
                    }

                    if let Some(bridge) = b.sapling_bridge.as_ref() {
                        let bridge = Bridge::read(&*bridge.data, &SaplingHasher::default())?;
                        cmtree.add_bridge(&bridge);
                        pos_start += bridge.len as u32;
                    }
                }
                else {
                    for tx in b.vtx.iter() {
                        if let Some(sapling_bridge) = tx.sapling_bridge.as_ref() { // tx was pruned
                            if !cmus.is_empty() { // flush nodes
                                cmtree.add_nodes(0, 0, &cmus);
                                pos_start += cmus.len() as u32;
                                cmus.clear();
                            }

                            // accumulate bridge
                            let bridge = Bridge::read(&*sapling_bridge.data, &SaplingHasher::default())?;
                            bridges = match bridges.take() {
                                Some(mut b) => {
                                    b.merge(&bridge, &cmtree.h);
                                    Some(b)
                                }
                                None => Some(bridge)
                            }
                        } else {
                            if let Some(bridge) = bridges.take() { // flush bridges
                                cmtree.add_bridge(&bridge);
                                pos_start += bridge.len as u32;
                            }

                            // accumulate cmus
                            for o in tx.outputs.iter() {
                                cmus.push((o.cmu.clone().try_into().unwrap(), false));
                            }
                            while !notes.is_empty() {
                                let n = &notes[0];
                                if (n.0 - pos_start) as usize >= cmus.len() { break }
                                cmus[(n.0 - pos_start) as usize].1 = true;
                                notes.remove(0);
                            }
                        }
                    }
                }
            }

            // flush bridges or cmus (only one should exist)
            if let Some(bridge) = bridges.take() { // flush bridges
                cmtree.add_bridge(&bridge);
                pos_start += bridge.len as u32;
            }
            if !cmus.is_empty() { // flush nodes
                cmtree.add_nodes(0, 0, &cmus);
                cmus.clear();
            }

            // detect spends
            for b in block_chunk.iter() {
                for tx in b.vtx.iter() {
                    for s in tx.spends.iter() {
                        if nfs.contains_key(&*s.nf) {
                            let (p, nv) = nfs[&*s.nf];
                            nfs.remove(&*s.nf);
                            cmtree.remove_witness(p as usize);
                            println!("Spent {nv}");
                            balance -= nv as i64;
                        }
                    }
                }
            }

            block_chunk.clear();
            tx_count = 0;
        }
    }

    println!("Final height = {block_height}");
    let duration = start_time.elapsed();
    println!("Time elapsed in sapling full scan is: {:?}", duration);

    let er = warp::empty_roots(&cmtree.h);
    let edge = cmtree.edge(&er);
    for w in cmtree.witnesses.iter() {
        let (root, _proof) = w.root(&er, &edge, &cmtree.h);
        println!("{} {}", w.path.pos, hex::encode(&root));
    }

    let mut client = connect_lightwalletd(url).await?;
    let rep = client.get_tree_state(Request::new(BlockId { height: height as u64, hash: vec![] })).await?.into_inner();
    let tree = hex::decode(&rep.sapling_tree).unwrap();
    let tree = zcash_primitives::merkle_tree::CommitmentTree::<Node>::read(&*tree)?;
    let root = tree.root();
    println!("server root {}", hex::encode(&root.repr));

    for (i, (p, v)) in nfs.values().enumerate() {
        println!("Note #{i} / {p} = {v}");
    }
    println!("Balance = {balance}");

    Ok(())
}

type BD = SaplingDomain<Network>;

struct EncryptedOutput {
    pos: u32,
    epk: [u8; 32],
    cmu: [u8; 32],
    enc: [u8; 52],
}

impl EncryptedOutput {
    pub fn new(pos: u32, co: CompactSaplingOutput) -> Self {
        Self {
            pos,
            epk: co.epk.try_into().unwrap(),
            cmu: co.cmu.try_into().unwrap(),
            enc: co.ciphertext.try_into().unwrap(),
        }
    }
}

impl ShieldedOutput<BD, 52> for EncryptedOutput {
    fn ephemeral_key(&self) -> EphemeralKeyBytes {
        EphemeralKeyBytes::from(self.epk)
    }

    fn cmstar_bytes(&self) -> [u8; 32] {
        self.cmu
    }

    fn enc_ciphertext(&self) -> &[u8; 52] {
        &self.enc
    }
}

struct DecBlock {
    count_outputs: u32,
    notes: Vec<(u32, Note)>,
}

fn decrypt_block(network: &Network, block: &CompactBlock, ivk: &PreparedIncomingViewingKey) -> Result<DecBlock> {
    let mut outputs = vec![];
    let d = SaplingDomain::for_height(*network, BlockHeight::from_u32(block.height as u32));
    let mut pos = 0u32;
    for tx in block.vtx.iter() {
        for o in tx.outputs.iter() {
            outputs.push((d.clone(), EncryptedOutput::new(pos, o.clone())));
            pos += 1;
        }
        if let Some(sapling_bridge) = tx.sapling_bridge.as_ref() {
            pos += sapling_bridge.len;
        }
    }
    let decrypted = try_compact_note_decryption::<BD, EncryptedOutput>(slice::from_ref(ivk), &outputs);
    let mut notes = vec![];
    for (pos, dec) in decrypted.iter().enumerate() {
        if let Some(((note, _), _)) = dec {
            println!("Received {}", note.value.inner());
            notes.push((pos as u32, note.clone()));
        }
    }
    let block = DecBlock {
        count_outputs: pos,
        notes,
    };
    Ok(block)
}
