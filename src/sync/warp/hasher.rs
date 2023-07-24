use super::{Hash, Hasher};
use ff::PrimeField;
use group::Curve;
use group::cofactor::CofactorCurveAffine;
use halo2_gadgets::sinsemilla::primitives::SINSEMILLA_S;
use pasta_curves::arithmetic::{CurveAffine, CurveExt};
use pasta_curves::{EpAffine, pallas};
use pasta_curves::pallas::{Affine, Point};
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub struct SaplingHasher {
    empty: Hash,
}

impl SaplingHasher {
    pub fn new() -> Self {
        let mut empty = [0u8; 32];
        empty[0] = 1;

        Self {
            empty
        }
    }
}

impl Default for SaplingHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher for SaplingHasher {
    type D = Hash;
    fn empty(&self) -> Hash {
        self.empty
    }

    fn is_empty(&self, d: &Hash) -> bool {
        *d == self.empty
    }

    fn combine(&self, depth: u8, l: &Hash, r: &Hash, _check: bool) -> Hash {
        // println!("> {} {} {}", depth, hex::encode(l), hex::encode(r));
        crate::sapling::sapling_hash(depth, l, r)
    }

    fn parallel_combine(&self, depth: u8, layer: &[[u8; 32]], pairs: usize) -> Vec<Hash> {
        crate::sapling::sapling_parallel_hash(depth, layer, pairs)
    }
}

#[derive(Clone, Debug)]
pub struct OrchardHasher {
    Q: Point,
    empty: Hash,
}

impl OrchardHasher {
    pub fn new() -> Self {
        let Q: Point =
            Point::hash_to_curve(halo2_gadgets::sinsemilla::primitives::Q_PERSONALIZATION)(halo2_gadgets::sinsemilla::merkle::MERKLE_CRH_PERSONALIZATION.as_bytes());
        let empty = pallas::Base::from(2).to_repr();
        OrchardHasher {
            Q,
            empty,
        }
    }

    fn node_combine_inner(&self, depth: u8, left: &Hash, right: &Hash) -> Point {
        let mut acc = self.Q;
        let (S_x, S_y) = SINSEMILLA_S[depth as usize];
        let S_chunk = Affine::from_xy(S_x, S_y).unwrap();
        acc = (acc + S_chunk) + acc; // TODO Bail if + gives point at infinity? Shouldn't happen if data was validated

        // Shift right by 1 bit and overwrite the 256th bit of left
        let mut left = *left;
        let mut right = *right;
        left[31] |= (right[0] & 1) << 7; // move the first bit of right into 256th of left
        for i in 0..32 {
            // move by 1 bit to fill the missing 256th bit of left
            let carry = if i < 31 { (right[i + 1] & 1) << 7 } else { 0 };
            right[i] = right[i] >> 1 | carry;
        }

        // we have 255*2/10 = 51 chunks
        let mut bit_offset = 0;
        let mut byte_offset = 0;
        for _ in 0..51 {
            let mut v = if byte_offset < 31 {
                left[byte_offset] as u16 | (left[byte_offset + 1] as u16) << 8
            } else if byte_offset == 31 {
                left[31] as u16 | (right[0] as u16) << 8
            } else {
                right[byte_offset - 32] as u16 | (right[byte_offset - 31] as u16) << 8
            };
            v = v >> bit_offset & 0x03FF; // keep 10 bits
            let (S_x, S_y) = SINSEMILLA_S[v as usize];
            let S_chunk = Affine::from_xy(S_x, S_y).unwrap();
            acc = (acc + S_chunk) + acc;
            bit_offset += 10;
            if bit_offset >= 8 {
                byte_offset += bit_offset / 8;
                bit_offset %= 8;
            }
        }
        acc
    }
}

impl Default for OrchardHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher for OrchardHasher {
    type D = Hash;

    fn empty(&self) -> Self::D {
        self.empty
    }

    fn is_empty(&self, d: &Self::D) -> bool {
        *d == self.empty
    }

    fn combine(&self, depth: u8, l: &Self::D, r: &Self::D, check: bool) -> Self::D {
        let acc = self.node_combine_inner(depth, l, r);
        let p = acc
            .to_affine()
            .coordinates()
            .map(|c| *c.x())
            .unwrap_or_else(pallas::Base::zero);
        p.to_repr()
    }

    fn parallel_combine(&self, depth: u8, layer: &[Self::D], pairs: usize) -> Vec<Self::D> {
        let hash_extended: Vec<_> = (0..pairs)
            .into_par_iter()
            .map(|i| {
                self.node_combine_inner(
                    depth,
                    &layer[2*i],
                    &layer[2*i+1],
                )
            })
            .collect();
        let mut hash_affine = vec![EpAffine::identity(); hash_extended.len()];
        Point::batch_normalize(&hash_extended, &mut hash_affine);
        hash_affine
            .iter()
            .map(|p| {
                p.coordinates()
                    .map(|c| *c.x())
                    .unwrap_or_else(pallas::Base::zero)
                    .to_repr()
            })
            .collect()
    }
}

