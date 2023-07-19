use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};

pub const DEPTH: usize = 33usize;

pub trait Hashable: Copy + Clone + PartialEq + Debug {
    fn empty() -> Self;
    fn is_empty(&self) -> bool;
    fn combine(depth: u8, l: &Self, r: &Self, check: bool) -> Self;
    fn write<W: Write>(&self, w: W);
    fn read<R: Read>(r: R) -> Self;
}

pub type SaplingHash = [u8; 32];
const SAPLING_EMPTY: SaplingHash = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

impl Hashable for SaplingHash {
    fn empty() -> Self {
        SAPLING_EMPTY
    }

    fn is_empty(&self) -> bool {
        *self == SAPLING_EMPTY
    }

    fn combine(depth: u8, l: &Self, r: &Self, check: bool) -> Self {
        // println!("{} {} {}", depth, hex::encode(l), hex::encode(r));
        crate::sapling::sapling_hash(depth, l, r)
    }

    fn write<W: Write>(&self, mut w: W) {
        w.write_all(self).unwrap();
    }

    fn read<R: Read>(mut r: R) -> Self {
        let mut h = [0u8; 32];
        r.read_exact(&mut h).unwrap();
        h
    }
}

pub mod witness;
pub mod tree;
pub mod bridge;

#[derive(Debug)]
pub struct Path<N: Hashable> {
    pub value: N,
    pub pos: usize,
    pub siblings: Vec<N>,
}

impl <N: Hashable> Path<N> {
    pub fn empty() -> Self {
        Path {
            value: N::empty(),
            pos: 0,
            siblings: vec![],
        }
    }
}

pub fn empty_roots<N: Hashable>() -> [N; DEPTH] {
    let mut roots = [N::empty(); DEPTH];
    for i in 0..DEPTH-1 {
        roots[i+1] = N::combine(i as u8, &roots[i], &roots[i], false);
    }
    roots
}

