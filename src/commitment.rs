use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;
use std::io::Write;
use zcash_primitives::serialize::{Optional, Vector};
use byteorder::WriteBytesExt;
use rayon::prelude::*;

#[derive(Clone)]
pub struct CTree {
    left: Option<Node>,
    right: Option<Node>,
    parents: Vec<Option<Node>>,
}

#[derive(Clone)]
pub struct Witness {
    tree: CTree,       // commitment tree at the moment the witness is created: immutable
    filled: Vec<Node>, // as more nodes are added, levels get filled up: won't change anymore
    cursor: CTree, // partial tree which still updates when nodes are added
}

impl Witness {
    pub fn new() -> Witness {
        Witness {
            tree: CTree::new(),
            filled: vec![],
            cursor: CTree::new(),
        }
    }

    pub fn write<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        self.tree.write(&mut writer)?;
        Vector::write(&mut writer, &self.filled, |w, n| n.write(w))?;
        if self.cursor.left == None && self.cursor.right == None {
            writer.write_u8(0)?;
        }
        else {
            writer.write_u8(1)?;
            self.cursor.write(writer)?;
        };
        Ok(())
    }
}

pub struct NotePosition {
    p: usize,
    p2: Option<usize>,
    c: usize,
    pub witness: Witness,
    is_last: bool,
}

fn collect(tree: &mut CTree, mut p: usize, depth: usize, commitments: &[Node]) -> usize {
    if depth == 0 {
        if p % 2 == 0 {
            tree.left = Some(commitments[p]);
        } else {
            tree.left = Some(commitments[p - 1]);
            tree.right = Some(commitments[p]);
            p -= 1;
        }
    } else {
        // the rest gets combined as a binary tree
        if p % 2 != 0 {
            tree.parents.push(Some(commitments[p - 1]));
        } else if p != 0 {
            tree.parents.push(None);
        }
    }
    p
}

impl NotePosition {
    fn new(position: usize, count: usize) -> NotePosition {
        let is_last = position == count - 1;
        let c = if !is_last {
            cursor_start_position(position, count)
        } else {
            0
        };
        let cursor_length = count - c;
        NotePosition {
            p: position,
            p2: if cursor_length > 0 {
                Some(cursor_length - 1)
            } else {
                None
            },
            c,
            witness: Witness::new(),
            is_last,
        }
    }

    fn collect(&mut self, depth: usize, commitments: &[Node]) {
        let count = commitments.len();
        let p = self.p;

        self.p = collect(&mut self.witness.tree, p, depth, commitments);

        if !self.is_last {
            if p % 2 == 0 && p + 1 < commitments.len() {
                let filler = commitments[p + 1];
                self.witness.filled.push(filler);
            }
        }

        if let Some(ref mut p2) = self.p2 {
            if !self.is_last {
                let cursor_commitments = &commitments[self.c..count];
                *p2 = collect(&mut self.witness.cursor, *p2, depth, cursor_commitments);
            }
            *p2 /= 2;
        }
        self.p /= 2;
        self.c /= 2;
    }
}

fn cursor_start_position(mut position: usize, mut count: usize) -> usize {
    assert!(position < count);
    // same logic as filler
    let mut depth = 0;
    loop {
        if position % 2 == 0 {
            if position + 1 < count {
                position += 1;
            } else {
                break;
            }
        }

        position /= 2;
        count /= 2;
        depth += 1;
    }
    (position + 1) << depth
}

impl CTree {
    pub fn calc_state(commitments: &mut [Node], positions: &[usize]) -> (CTree, Vec<NotePosition>) {
        let mut n = commitments.len();
        let mut positions: Vec<_> = positions.iter().map(|&p| NotePosition::new(p, n)).collect();
        assert_ne!(n, 0);

        let mut depth = 0usize;
        let mut frontier = NotePosition::new(n - 1, n);
        while n > 0 {
            let commitment_slice = &commitments[0..n];
            frontier.collect(depth, commitment_slice);

            for p in positions.iter_mut() {
                p.collect(depth, commitment_slice);
            }

            let nn = n / 2;
            let next_level: Vec<_> = (0..nn).into_par_iter().map(|i| {
                Node::combine(depth, &commitments[2 * i], &commitments[2 * i + 1])
            }).collect();
            commitments[0..nn].copy_from_slice(&next_level);

            depth += 1;
            n = nn;
        }

        (frontier.witness.tree, positions)
    }

    fn new() -> CTree {
        CTree {
            left: None,
            right: None,
            parents: vec![],
        }
    }

    fn write<W: Write>(&self, mut writer: W) -> std::io::Result<()> {
        Optional::write(&mut writer, &self.left, |w, n| n.write(w))?;
        Optional::write(&mut writer, &self.right, |w, n| n.write(w))?;
        Vector::write(&mut writer, &self.parents, |w, e| {
            Optional::write(w, e, |w, n| n.write(w))
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::commitment::{cursor_start_position, CTree};
    #[allow(unused_imports)]
    use crate::print::{print_tree, print_witness};
    use std::time::Instant;
    use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
    use zcash_primitives::sapling::Node;

    /*
    Build incremental witnesses with both methods and compare their binary serialization
     */
    #[test]
    fn test_calc_witnesses() {
        const NUM_NODES: u32 = 100000; // number of notes
        const WITNESS_PERCENT: u32 = 1; // percentage of notes that are ours
        const DEBUG_PRINT: bool = false;

        let witness_freq = 100 / WITNESS_PERCENT;
        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut nodes: Vec<Node> = vec![];
        let mut witnesses: Vec<IncrementalWitness<Node>> = vec![];
        let mut positions: Vec<usize> = vec![];
        for i in 1..=NUM_NODES {
            let mut bb = [0u8; 32];
            bb[0..4].copy_from_slice(&i.to_be_bytes());
            let node = Node::new(bb);

            tree1.append(node).unwrap();

            for w in witnesses.iter_mut() {
                w.append(node).unwrap();
            }

            if i % witness_freq == 0 {
                let w = IncrementalWitness::<Node>::from_tree(&tree1);
                witnesses.push(w);
                positions.push((i - 1) as usize);
            }

            nodes.push(node);
        }

        let start = Instant::now();
        let (tree2, positions) = CTree::calc_state(&mut nodes, &positions);
        eprintln!(
            "Update State & Witnesses: {} ms",
            start.elapsed().as_millis()
        );

        println!("# witnesses = {}", positions.len());

        for (w, p) in witnesses.iter().zip(&positions) {
            let mut bb1: Vec<u8> = vec![];
            w.write(&mut bb1).unwrap();

            let mut bb2: Vec<u8> = vec![];
            p.witness.write(&mut bb2).unwrap();

            assert_eq!(bb1.as_slice(), bb2.as_slice());
        }

        if DEBUG_PRINT {
            print_witness(&witnesses[0]);

            println!("Tree");
            let t = &positions[0].witness.tree;
            println!("{:?}", t.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", t.right.map(|n| hex::encode(n.repr)));
            for p in t.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }
            println!("Filled");
            for f in positions[0].witness.filled.iter() {
                println!("{:?}", hex::encode(f.repr));
            }
            println!("Cursor");
            let t = &positions[0].witness.cursor;
            println!("{:?}", t.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", t.right.map(|n| hex::encode(n.repr)));
            for p in t.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }

            println!("{:?}", tree1.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", tree1.right.map(|n| hex::encode(n.repr)));
            for p in tree1.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }

            println!("-----");

            println!("{:?}", tree2.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", tree2.right.map(|n| hex::encode(n.repr)));
            for p in tree2.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }
        }
    }

    #[test]
    fn test_cursor() {
        // println!("{}", cursor_start_position(8, 14));
        println!("{}", cursor_start_position(9, 14));
        // println!("{}", cursor_start_position(10, 14));
    }
}
