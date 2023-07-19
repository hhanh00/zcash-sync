use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use super::{DEPTH, Hashable};

#[derive(Clone, Copy, Debug)]
pub struct CompactLayer<N: Hashable> {
    pub fill: N,
    pub prev: N,
}

impl <N: Hashable> CompactLayer<N> {
    fn write<W: Write>(&self, mut w: W) {
        self.fill.write(&mut w);
        self.prev.write(&mut w);
    }

    fn read<R: Read>(mut r: R) -> Self {
        let fill = N::read(&mut r);
        let prev = N::read(&mut r);
        CompactLayer {
            fill, prev
        }
    }
}

#[derive(Debug)]
pub struct Bridge<N: Hashable> {
    pub pos: usize,
    pub block_len: usize,
    pub len: usize,
    pub layers: [CompactLayer<N>; DEPTH],
}

impl <N: Hashable> Bridge<N> {
    pub fn merge(&mut self, other: &Bridge<N>) {
        for i in 0..DEPTH {
            if self.layers[i].fill.is_empty() && !other.layers[i].fill.is_empty() {
                self.layers[i].fill = other.layers[i].fill;
            }
            self.layers[i].prev = other.layers[i].prev;
        }
        self.len += other.len;
    }

    pub fn write<W: Write>(&self, mut w: W) {
        w.write_u64::<LE>(self.pos as u64).unwrap();
        w.write_u64::<LE>(self.len as u64).unwrap();
        w.write_u32::<LE>(self.block_len as u32).unwrap();
        for layer in self.layers.iter() {
            layer.write(&mut w);
        }
    }

    pub fn read<R: Read>(mut r: R) -> Self {
        let pos = r.read_u64::<LE>().unwrap() as usize;
        let len = r.read_u64::<LE>().unwrap() as usize;
        let block_len = r.read_u32::<LE>().unwrap() as usize;
        let mut layers = vec![];
        for i in 0..DEPTH {
            let layer = CompactLayer::read(&mut r);
            layers.push(layer);
        }
        Bridge {
            pos, len, block_len,
            layers: layers.try_into().unwrap()
        }
    }
}

impl <N: Hashable> Default for Bridge<N> {
    fn default() -> Self {
        Bridge {
            pos: 0,
            block_len: 0,
            len: 0,
            layers: [CompactLayer { fill: N::empty(), prev: N::empty() }; DEPTH]
        }
    }
}
