use anyhow::Result;
use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use super::{DEPTH, ReadWrite, Hasher};

#[derive(Clone, Copy, Debug)]
pub struct CompactLayer<H: Hasher> {
    pub fill: H::D,
    pub prev: H::D,
}

pub fn write_data<H: Hasher, W: Write>(data: &H::D, mut w: W, h: &H) -> Result<()> {
    if h.is_empty(&data) {
        w.write_u8(0)?;
    } else {
        w.write_u8(1)?;
        data.write(&mut w)?;
    }
    Ok(())
}

pub fn read_data<H: Hasher, R: Read>(mut r: R, h: &H) -> Result<H::D> {
    let is_empty = r.read_u8()?;
    if is_empty == 0 {
        Ok(h.empty())
    }
    else {
        Ok(H::D::read(&mut r)?)
    }
}


impl <H: Hasher> CompactLayer<H> {
    pub fn write<W: Write>(&self, mut w: W, h: &H) -> Result<()> {
        write_data(&self.fill, &mut w, h)?;
        write_data(&self.prev, &mut w, h)?;
        Ok(())
    }

    pub fn read<R: Read>(mut r: R, h: &H) -> Result<Self> {
        let fill = read_data(&mut r, h)?;
        let prev = read_data(&mut r, h)?;
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
    pub layers: [CompactLayer<H>; DEPTH],
}

impl <H: Hasher> Bridge<H> {
    pub fn merge(&mut self, other: &Bridge<H>, h: &H) {
        for i in 0..DEPTH {
            if h.is_empty(&self.layers[i].fill) && !h.is_empty(&other.layers[i].fill) {
                self.layers[i].fill = other.layers[i].fill.clone();
            }
            self.layers[i].prev = other.layers[i].prev.clone();
        }
        self.len += other.len;
    }

    pub fn write<W: Write>(&self, mut w: W, h: &H) -> Result<()> {
        // w.write_u32::<LE>(self.height as u32)?;
        // w.write_u32::<LE>(self.block_len as u32)?;
        // w.write_u64::<LE>(self.pos as u64)?;
        w.write_u32::<LE>(self.len as u32)?;
        for layer in self.layers.iter() {
            layer.write(&mut w, h)?;
        }
        Ok(())
    }

    pub fn read<R: Read>(mut r: R, h: &H) -> Result<Self> {
        // let height = r.read_u32::<LE>()?;
        // let block_len = r.read_u32::<LE>()?;
        // let pos = r.read_u64::<LE>()? as usize;
        let len = r.read_u32::<LE>()? as usize;
        let mut layers = vec![];
        for _ in 0..DEPTH {
            let layer = CompactLayer::read(&mut r, h)?;
            layers.push(layer);
        }
        Ok(Bridge {
            pos: 0, len, height: 0, block_len: 0,
            layers: layers.try_into().unwrap()
        })
    }

    pub fn empty(h: &H) -> Self {
        Bridge {
            height: 0,
            block_len: 0,
            pos: 0,
            len: 0,
            layers: std::array::from_fn(|_| CompactLayer { fill: h.empty(), prev: h.empty() })
        }
    }
}
