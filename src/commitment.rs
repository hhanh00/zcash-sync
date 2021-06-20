use crate::path::MerklePath;
use byteorder::WriteBytesExt;
use rayon::prelude::*;
use std::io::Write;
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;
use zcash_primitives::serialize::{Optional, Vector};

/*
Same behavior and structure as CommitmentTree<Node> from librustzcash
It represents the data required to build a merkle path from a note commitment (leaf)
to the root.
The Merkle Path is the minimal set of nodes needed to recalculate the Merkle root
that includes our note.
It starts with our note commitment (because it is already a hash, it doesn't need
to be hashed). The value is stored in either `left` or `right` slot depending on the parity
of the note index. If there is a sibling, its value is in the other slot.
`parents` is the list of hashes that are siblings to the nodes along the path to the root.
If a hash has no sibling yet, then the parent is None. It means that the placeholder hash
value should be used there.

Remark: It's possible to have a grand parent but no parent.
 */
#[derive(Clone)]
pub struct CTree {
    left: Option<Node>,
    right: Option<Node>,
    parents: Vec<Option<Node>>,
}

/*
Witness is the data required to maintain the Merkle Path of a given note after more
notes are added.
Once a node has two actual children values (i.e. not a placeholder), its value
is constant because leaves can't change.
However, it doesn't mean that our Merkle Path is immutable. As the tree fills up,
previous entries that were None could end up getting a value.
- `tree` is the Merkle Path at the time the note is inserted. It does not change
- `filled` are the hash values that replace the "None" values in `tree`. It gets populated as
more notes are added and the sibling sub trees fill up
- `cursor` is a sibling subtree that is not yet full. It is tracked as a sub Merkle Tree

Example:
Let's say the `tree` has parents [ hash, None, hash ] and left = hash, right = None.
Based on this information, we know the position is 1010b = 10 (11th leaf)

                   o
           /              \
        hash              o
     /        \          /   \
    *          *        o     .
  /   \      /  \     /   \
  *    *    *    *  hash  o
 /\   /\   /\   /\   /\   /\
0  1 2  3 4  5 6  7 8  9 10 .

legend:
o is a hash value that we calculate as part of the merkle path verification
. is a placeholder hash and denotes a non existent node

We have two missing nodes (None):
- the `right` node,
- the 2nd parent

When node 11 comes, `filled` gets the value since it is filling the first None.
Then when node 12 comes, we are starting to fill a new sub tree in `cursor`
cursor -> left = 12, right = None, parents = []
After node 13, cursor continues to fill up:
cursor -> left = 12, right = 13, parents = []
With node 14, the cursor tree gains one level
cursor -> left = 14, right = None, parents = [hash(12,13)]
With node 15, the subtree is full, `filled` gets the value of the 2nd parent
and the cursor is empty
With node 16, the tree gains a level but `tree` remains the same (it is immutable).
Instead, a new cursor starts. Eventually, it fills up and a new value
gets pushed into `filled`.
*/
#[derive(Clone)]
pub struct Witness {
    tree: CTree,       // commitment tree at the moment the witness is created: immutable
    filled: Vec<Node>, // as more nodes are added, levels get filled up: won't change anymore
    cursor: CTree,     // partial tree which still updates when nodes are added
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
        } else {
            writer.write_u8(1)?;
            self.cursor.write(writer)?;
        };
        Ok(())
    }
}

pub struct NotePosition {
    p0: usize,
    p: usize,
    p2: usize,
    c: usize,
    pub witness: Witness,
}

fn collect(
    tree: &mut CTree,
    mut p: usize,
    depth: usize,
    commitments: &[Node],
    offset: usize,
) -> usize {
    // println!("--> {} {} {}", depth, p, offset);
    if p < offset { return p }
    if depth == 0 {
        if p % 2 == 0 {
            tree.left = Some(commitments[p - offset]);
        } else {
            tree.left = Some(commitments[p - 1 - offset]);
            tree.right = Some(commitments[p - offset]);
            p -= 1;
        }
    } else {
        // the rest gets combined as a binary tree
        if p % 2 != 0 {
            tree.parents.push(Some(commitments[p - 1 - offset]));
        } else if !(p == 0 && offset == 0) {
            tree.parents.push(None);
        }
    }
    p
}

impl NotePosition {
    pub fn new(position: usize, count: usize) -> NotePosition {
        let c = cursor_start_position(position, count);
        NotePosition {
            p0: position,
            p: position,
            p2: count - 1,
            c,
            witness: Witness::new(),
        }
    }

    pub fn reset(&mut self, count: usize) {
        let c = cursor_start_position(self.p0, count);
        self.p = self.p0;
        self.p2 = count - 1;
        self.c = c;
    }

    fn collect(&mut self, depth: usize, commitments: &[Node], offset: usize) {
        let count = commitments.len();
        let p = self.p;

        self.p = collect(&mut self.witness.tree, p, depth, commitments, offset);

        if p % 2 == 0 && p + 1 >= offset && p + 1 - offset < commitments.len() {
            let filler = commitments[p + 1 - offset];
            self.witness.filled.push(filler);
        }

        let c = self.c - offset;
        let cursor_commitments = &commitments[c..count];
        // println!("c> {} {} {}", c, count, depth);
        // println!("> {} {}", self.p2, self.c);
        if !cursor_commitments.is_empty() {
            let p2 = collect(
                &mut self.witness.cursor,
                self.p2,
                depth,
                cursor_commitments,
                offset + c,
            );
            self.p2 = (p2 - self.c) / 2 + self.c / 2;
            // println!("+ {} {}", self.p2, self.c);
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
    pub fn calc_state(
        mut commitments: Vec<Node>,
        positions: &mut [NotePosition],
        prev_frontier: Option<CTree>,
    ) -> CTree {
        let mut n = commitments.len();
        assert_ne!(n, 0);

        let prev_count = prev_frontier.as_ref().map(|f| f.get_position()).unwrap_or(0);
        let count = prev_count + n;
        let mut last_path = prev_frontier.as_ref().map(|f| MerklePath::new(f.left, f.right));
        let mut frontier = NotePosition::new(count - 1, count);
        let mut offset = prev_count;

        for p in positions.iter_mut() {
            p.reset(count);
        }

        let mut depth = 0usize;
        while n + offset > 0 {
            if offset % 2 == 1 {
                // start is not aligned
                let mut lp = last_path.take().unwrap();
                let node = lp.get(); // prepend the last node from the previous run
                if n > 0 {
                    lp.set(commitments[0]); // put the right node into the path
                }
                last_path = Some(lp);
                commitments.insert(0, node);
                n += 1;
                offset -= 1;
            }
            let commitment_slice = &commitments[0..n];
            frontier.collect(depth, commitment_slice, offset);

            for p in positions.iter_mut() {
                p.collect(depth, commitment_slice, offset);
            }

            let nn = n / 2;
            let next_level: Vec<_> = (0..nn)
                .into_par_iter()
                .map(|i| Node::combine(depth, &commitments[2 * i], &commitments[2 * i + 1]))
                .collect();
            commitments[0..nn].copy_from_slice(&next_level);

            if let Some(mut lp) = last_path.take() {
                lp.up(depth, prev_frontier.as_ref().unwrap().parents.get(depth).unwrap_or(&None));
                last_path = Some(lp);
            }

            depth += 1;
            n = nn;
            offset /= 2;
        }

        frontier.witness.tree
    }

    pub fn new() -> CTree {
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

    fn get_position(&self) -> usize {
        let mut p = 0usize;
        for parent in self.parents.iter().rev() {
            if parent.is_some() {
                p += 1;
            }
            p *= 2;
        }
        if self.left.is_some() {
            p += 1;
        }
        if self.right.is_some() {
            p += 1;
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use crate::commitment::{cursor_start_position, CTree, NotePosition};
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
        const NUM_CHUNKS: usize = 20;
        const NUM_NODES: usize = 200; // number of notes
        const WITNESS_PERCENT: usize = 1; // percentage of notes that are ours
        const DEBUG_PRINT: bool = true;

        let witness_freq = 100000 / WITNESS_PERCENT;
        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2: Option<CTree> = None;
        let mut witnesses: Vec<IncrementalWitness<Node>> = vec![];
        let mut all_positions: Vec<NotePosition> = vec![];

        for c in 0..NUM_CHUNKS {
            let mut positions: Vec<usize> = vec![];
            let mut nodes: Vec<Node> = vec![];
            for i in 1..=NUM_NODES {
                let mut bb = [0u8; 32];
                bb[0..8].copy_from_slice(&i.to_be_bytes());
                let node = Node::new(bb);

                tree1.append(node).unwrap();

                for w in witnesses.iter_mut() {
                    w.append(node).unwrap();
                }

                if i % witness_freq == 0 {
                    let w = IncrementalWitness::<Node>::from_tree(&tree1);
                    witnesses.push(w);
                    positions.push((i - 1 + c * NUM_NODES) as usize);
                }

                nodes.push(node);
            }

            let start = Instant::now();
            let n = nodes.len();
            let mut positions: Vec<_> = positions.iter().map(|&p| NotePosition::new(p, n + c*NUM_NODES)).collect();
            all_positions.append(&mut positions);
            tree2 = Some(CTree::calc_state(nodes, &mut all_positions, tree2));
            eprintln!(
                "Update State & Witnesses: {} ms",
                start.elapsed().as_millis()
            );
        }
        let tree2 = tree2.unwrap();

        println!("# witnesses = {}", all_positions.len());

        for (i, (w, p)) in witnesses.iter().zip(&all_positions).enumerate() {
            let mut bb1: Vec<u8> = vec![];
            w.write(&mut bb1).unwrap();

            let mut bb2: Vec<u8> = vec![];
            p.witness.write(&mut bb2).unwrap();

            assert_eq!(bb1.as_slice(), bb2.as_slice(), "failed at {}", i);
        }

        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        assert_eq!(bb1.as_slice(), bb2.as_slice(), "tree states not equal");

        if DEBUG_PRINT {
            // let slot = 0usize;
            // print_witness(&witnesses[slot]);
            //
            // println!("Tree");
            // let t = &all_positions[slot].witness.tree;
            // println!("{:?}", t.left.map(|n| hex::encode(n.repr)));
            // println!("{:?}", t.right.map(|n| hex::encode(n.repr)));
            // for p in t.parents.iter() {
            //     println!("{:?}", p.map(|n| hex::encode(n.repr)));
            // }
            // println!("Filled");
            // for f in all_positions[slot].witness.filled.iter() {
            //     println!("{:?}", hex::encode(f.repr));
            // }
            // println!("Cursor");
            // let t = &all_positions[slot].witness.cursor;
            // println!("{:?}", t.left.map(|n| hex::encode(n.repr)));
            // println!("{:?}", t.right.map(|n| hex::encode(n.repr)));
            // for p in t.parents.iter() {
            //     println!("{:?}", p.map(|n| hex::encode(n.repr)));
            // }
            // println!("====");

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
