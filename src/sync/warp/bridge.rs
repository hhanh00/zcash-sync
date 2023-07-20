use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use super::{DEPTH, ReadWrite, Hasher};

#[derive(Clone, Copy, Debug)]
pub struct CompactLayer<D: ReadWrite> {
    pub fill: D,
    pub prev: D,
}

impl <D: ReadWrite> CompactLayer<D> {
    fn write<W: Write>(&self, mut w: W) {
        self.fill.write(&mut w);
        self.prev.write(&mut w);
    }

    fn read<R: Read>(mut r: R) -> Self {
        let fill = D::read(&mut r);
        let prev = D::read(&mut r);
        CompactLayer {
            fill, prev
        }
    }
}

#[derive(Debug)]
pub struct Bridge<D: ReadWrite> {
    pub pos: usize,
    pub block_len: usize,
    pub len: usize,
    pub layers: [CompactLayer<D>; DEPTH],
}

impl <D: Clone + PartialEq + Debug + ReadWrite> Bridge<D> {
    pub fn merge<H: Hasher<D>>(&mut self, other: &Bridge<D>) {
        for i in 0..DEPTH {
            if H::is_empty(&self.layers[i].fill) && !H::is_empty(&other.layers[i].fill) {
                self.layers[i].fill = other.layers[i].fill.clone();
            }
            self.layers[i].prev = other.layers[i].prev.clone();
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
        for _ in 0..DEPTH {
            let layer = CompactLayer::read(&mut r);
            layers.push(layer);
        }
        Bridge {
            pos, len, block_len,
            layers: layers.try_into().unwrap()
        }
    }
}

impl <D: Clone + PartialEq + Debug + ReadWrite> Bridge<D> {
    fn empty<H: Hasher<D>>() -> Self {
        Bridge {
            pos: 0,
            block_len: 0,
            len: 0,
            layers: std::array::from_fn(|_| CompactLayer { fill: H::empty(), prev: H::empty() })
        }
    }
}
