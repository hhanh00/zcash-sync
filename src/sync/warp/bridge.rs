use anyhow::Result;
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
    pub fn write<W: Write>(&self, mut w: W) -> Result<()> {
        self.fill.write(&mut w)?;
        self.prev.write(&mut w)?;
        Ok(())
    }

    pub fn read<R: Read>(mut r: R) -> Result<Self> {
        let fill = D::read(&mut r)?;
        let prev = D::read(&mut r)?;
        Ok(CompactLayer {
            fill, prev
        })
    }
}

#[derive(Debug)]
pub struct Bridge<H: Hasher> {
    pub height: u32,
    pub block_len: u32,
    pub pos: usize,
    pub len: usize,
    pub layers: [CompactLayer<H::D>; DEPTH],
}

impl <H: Hasher> Bridge<H> {
    pub fn merge(&mut self, other: &Bridge<H>) {
        for i in 0..DEPTH {
            if H::is_empty(&self.layers[i].fill) && !H::is_empty(&other.layers[i].fill) {
                self.layers[i].fill = other.layers[i].fill.clone();
            }
            self.layers[i].prev = other.layers[i].prev.clone();
        }
        self.len += other.len;
    }

    pub fn write<W: Write>(&self, mut w: W) -> Result<()> {
        w.write_u32::<LE>(self.height as u32)?;
        w.write_u32::<LE>(self.block_len as u32)?;
        w.write_u64::<LE>(self.pos as u64)?;
        w.write_u64::<LE>(self.len as u64)?;
        for layer in self.layers.iter() {
            layer.write(&mut w)?;
        }
        Ok(())
    }

    pub fn read<R: Read>(mut r: R) -> Result<Self> {
        let height = r.read_u32::<LE>()?;
        let block_len = r.read_u32::<LE>()?;
        let pos = r.read_u64::<LE>()? as usize;
        let len = r.read_u64::<LE>()? as usize;
        let mut layers = vec![];
        for _ in 0..DEPTH {
            let layer = CompactLayer::read(&mut r)?;
            layers.push(layer);
        }
        Ok(Bridge {
            pos, len, height, block_len,
            layers: layers.try_into().unwrap()
        })
    }

    pub fn empty() -> Self {
        Bridge {
            height: 0,
            block_len: 0,
            pos: 0,
            len: 0,
            layers: std::array::from_fn(|_| CompactLayer { fill: H::empty(), prev: H::empty() })
        }
    }
}
