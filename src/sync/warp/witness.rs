use anyhow::Result;
use std::fmt::Debug;
use std::io::{Read, Write};
use byteorder::{ReadBytesExt, WriteBytesExt};
use super::{DEPTH, ReadWrite, Path, Hasher};

#[derive(Debug)]
pub struct Witness<D: ReadWrite> {
    pub path: Path<D>,
    pub fills: Vec<D>,
}

impl <D: Clone + PartialEq + Debug + ReadWrite> Witness<D> {
    pub fn root<H: Hasher<D>>(&self, empty_roots: &[D; DEPTH], edge: &[D; DEPTH]) -> (D, [D; DEPTH]) {
        let mut p = self.path.pos;
        let mut h = self.path.value.clone();
        let mut j = 0;
        let mut k = 0;
        let mut edge_used = false;
        let mut path = vec![];

        for i in 0..DEPTH {
            h =
                if p & 1 == 0 {
                    let r = if k < self.fills.len() {
                        let r = &self.fills[k];
                        k += 1;
                        r
                    }
                    else if !edge_used {
                        edge_used = true;
                        &edge[i-1]
                    }
                    else {
                        &empty_roots[i]
                    };
                    path.push(r.clone());
                    H::combine(i as u8, &h, r, false)
                }
                else {
                    let l = &self.path.siblings[j];
                    path.push(l.clone());
                    let v = H::combine(i as u8, l, &h, true);
                    j += 1;
                    v
                };
            p = p / 2;
        }

        (h.clone(), path.try_into().unwrap())
    }

    pub fn write<W: Write>(&self, mut w: W) -> Result<()> {
        self.path.write(&mut w)?;
        w.write_u8(self.fills.len() as u8).unwrap();
        for f in self.fills.iter() {
            f.write(&mut w)?;
        }
        Ok(())
    }

    pub fn read<R: Read>(mut r: R) -> Result<Self> {
        let path = Path::read(&mut r)?;
        let len = r.read_u8()? as usize;
        let mut fills = vec![];
        for _ in 0..len {
            let fill = D::read(&mut r)?;
            fills.push(fill);
        }
        Ok(Self {
            path,
            fills,
        })
    }
}
