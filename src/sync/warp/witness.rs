use anyhow::Result;
use std::fmt::{Debug, Formatter};
use std::io::{Read, Write};
use byteorder::{ReadBytesExt, WriteBytesExt};
use super::{DEPTH, ReadWrite, Path, Hasher};

pub struct Witness<H: Hasher> {
    pub path: Path<H>,
    pub fills: Vec<H::D>,
}

impl <H: Hasher> Debug for Witness<H> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "path: {:?}", self.path)?;
        writeln!(f, ", fills: {:?}", self.fills)
    }
}

impl <H: Hasher> Witness<H> {
    pub fn root(&self, empty_roots: &[H::D; DEPTH], edge: &[H::D; DEPTH], h: &H) -> (H::D, [H::D; DEPTH]) {
        let mut p = self.path.pos;
        let mut hash = self.path.value.clone();
        let mut j = 0;
        let mut k = 0;
        let mut edge_used = false;
        let mut path = vec![];

        for i in 0..DEPTH {
            hash =
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
                    h.combine(i as u8, &hash, r, false)
                }
                else {
                    let l = &self.path.siblings[j];
                    path.push(l.clone());
                    let v = h.combine(i as u8, l, &hash, true);
                    j += 1;
                    v
                };
            p = p / 2;
        }

        (hash.clone(), path.try_into().unwrap())
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
            let fill = H::D::read(&mut r)?;
            fills.push(fill);
        }
        Ok(Self {
            path,
            fills,
        })
    }
}
