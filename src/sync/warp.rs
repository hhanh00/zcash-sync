use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use crate::Hash;

pub const DEPTH: usize = 32usize;

pub trait ReadWrite {
    fn write<W: Write>(&self, w: W);
    fn read<R: Read>(r: R) -> Self;
}

pub trait Hasher<D: Clone + PartialEq + Debug + ReadWrite> {
    fn empty() -> D;
    fn is_empty(d: &D) -> bool;
    fn combine(depth: u8, l: &D, r: &D, check: bool) -> D;
}

impl ReadWrite for Hash {
    fn write<W: Write>(&self, mut w: W) {
        w.write_all(self).unwrap();
    }

    fn read<R: Read>(mut r: R) -> Self {
        let mut h = [0u8; 32];
        r.read_exact(&mut h).unwrap();
        h
    }
}

pub struct SaplingHasher;

const SAPLING_EMPTY: Hash = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

impl Hasher<Hash> for SaplingHasher {
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
}

pub mod witness;
pub mod tree;
pub mod bridge;

#[derive(Debug)]
pub struct Path<D> {
    pub value: D,
    pub pos: usize,
    pub siblings: Vec<D>,
}

impl <D: Clone + PartialEq + Debug + ReadWrite> Path<D> {
    pub fn empty<H: Hasher<D>>() -> Self {
        Path {
            value: H::empty(),
            pos: 0,
            siblings: vec![],
        }
    }

    fn write<W: Write>(&self, mut w: W) {
        w.write_u64::<LE>(self.pos as u64).unwrap();
        self.value.write(&mut w);
        w.write_u8(self.siblings.len() as u8).unwrap();
        for s in self.siblings.iter() {
            s.write(&mut w);
        }
    }

    fn read<R: Read>(mut r: R) -> Self {
        let pos = r.read_u64::<LE>().unwrap() as usize;
        let value = D::read(&mut r);
        let len = r.read_u8().unwrap() as usize;
        let mut siblings = vec![];
        for _ in 0..len {
            let s = D::read(&mut r);
            siblings.push(s);
        }
        Self {
            value,
            pos,
            siblings,
        }
    }
}

pub fn empty_roots<D: Clone + PartialEq + Debug + ReadWrite, H: Hasher<D>>() -> [D; DEPTH] {
    let mut roots = vec![];
    roots.push(H::empty());
    for i in 0..DEPTH-1 {
        roots.push(H::combine(i as u8, &roots[i], &roots[i], false));
    }
    roots.try_into().unwrap()
}
