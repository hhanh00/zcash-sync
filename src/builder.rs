use crate::commitment::{CTree, Witness};
use rayon::prelude::IntoParallelIterator;
use rayon::prelude::*;
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;

trait Builder<T, C> {
    fn collect(&mut self, commitments: &[Node], context: &C) -> usize;
    fn up(&mut self);
    fn finished(&self) -> bool;
    fn finalize(self, context: &C) -> T;
}

struct CTreeBuilder {
    left: Option<Node>,
    right: Option<Node>,
    prev_tree: CTree,
    next_tree: CTree,
    start: usize,
    depth: usize,
    offset: Option<Node>,
}

impl Builder<CTree, ()> for CTreeBuilder {
    fn collect(&mut self, commitments: &[Node], _context: &()) -> usize {
        // assert!(!commitments.is_empty() || self.left.is_some() || self.right.is_some());
        assert!(self.right.is_none() || self.left.is_some()); // R can't be set without L

        let offset: Option<Node>;
        let m: usize;

        if self.left.is_some() && self.right.is_none() {
            offset = self.left;
            m = commitments.len() + 1;
        } else {
            offset = None;
            m = commitments.len();
        };

        let n = if self.depth == 0 {
            if m % 2 == 0 {
                self.next_tree.left = Some(*Self::get(commitments, m - 2, &offset));
                self.next_tree.right = Some(*Self::get(commitments, m - 1, &offset));
                m - 2
            } else {
                self.next_tree.left = Some(*Self::get(commitments, m - 1, &offset));
                self.next_tree.right = None;
                m - 1
            }
        } else {
            if m % 2 == 0 {
                self.next_tree.parents.push(None);
                m
            } else {
                let last_node = Self::get(commitments, m - 1, &offset);
                self.next_tree.parents.push(Some(*last_node));
                m - 1
            }
        };
        assert_eq!(n % 2, 0);

        self.offset = offset;
        n
    }

    fn up(&mut self) {
        let h = if self.left.is_some() && self.right.is_some() {
            Some(Node::combine(
                self.depth,
                &self.left.unwrap(),
                &self.right.unwrap(),
            ))
        } else {
            None
        };
        let (l, r) = match self.prev_tree.parents.get(self.depth) {
            Some(Some(p)) => (Some(*p), h),
            Some(None) => (h, None),
            None => (h, None),
        };

        self.left = l;
        self.right = r;

        assert!(self.start % 2 == 0 || self.offset.is_some());
        self.start /= 2;

        self.depth += 1;
    }

    fn finished(&self) -> bool {
        self.depth >= self.prev_tree.parents.len() && self.left.is_none() && self.right.is_none()
    }

    fn finalize(self, _context: &()) -> CTree {
        self.next_tree
    }
}

impl CTreeBuilder {
    fn new(prev_tree: CTree) -> CTreeBuilder {
        let start = prev_tree.get_position();
        CTreeBuilder {
            left: prev_tree.left,
            right: prev_tree.right,
            prev_tree,
            next_tree: CTree::new(),
            start,
            depth: 0,
            offset: None,
        }
    }

    #[inline(always)]
    fn get_opt<'a>(
        commitments: &'a [Node],
        index: usize,
        offset: &'a Option<Node>,
    ) -> Option<&'a Node> {
        if offset.is_some() {
            if index > 0 {
                commitments.get(index - 1)
            } else {
                offset.as_ref()
            }
        } else {
            commitments.get(index)
        }
    }

    #[inline(always)]
    fn get<'a>(commitments: &'a [Node], index: usize, offset: &'a Option<Node>) -> &'a Node {
        Self::get_opt(commitments, index, offset).unwrap()
    }

    fn adjusted_start(&self, prev: &Option<Node>, depth: usize) -> usize {
        if depth != 0 && prev.is_some() {
            self.start - 1
        } else {
            self.start
        }
    }

    fn clone_trimmed(&self, mut depth: usize) -> CTree {
        if depth == 0 {
            return CTree::new()
        }
        let mut tree = self.next_tree.clone();
        while depth > 0 && depth <= tree.parents.len() && tree.parents[depth - 1].is_none() {
            depth -= 1;
        }
        tree.parents.truncate(depth);
        tree
    }
}

fn combine_level(commitments: &mut [Node], offset: Option<Node>, n: usize, depth: usize) -> usize {
    assert_eq!(n % 2, 0);

    let nn = n / 2;
    let next_level: Vec<Node> = (0..nn)
        .into_par_iter()
        .map(|i| {
            Node::combine(
                depth,
                CTreeBuilder::get(commitments, 2 * i, &offset),
                CTreeBuilder::get(commitments, 2 * i + 1, &offset),
            )
        })
        .collect();
    commitments[0..nn].copy_from_slice(&next_level);
    nn
}

struct WitnessBuilder {
    witness: Witness,
    p: usize,
    inside: bool,
}

impl WitnessBuilder {
    fn new(tree_builder: &CTreeBuilder, prev_witness: Witness, count: usize) -> WitnessBuilder {
        let position = prev_witness.position;
        let inside = position >= tree_builder.start && position < tree_builder.start + count;
        WitnessBuilder {
            witness: prev_witness,
            p: position,
            inside,
        }
    }
}

impl Builder<Witness, CTreeBuilder> for WitnessBuilder {
    fn collect(&mut self, commitments: &[Node], context: &CTreeBuilder) -> usize {
        let offset = context.offset;
        let depth = context.depth;

        let tree = &mut self.witness.tree;
        let right = if depth != 0 { context.right } else { None };

        if self.inside {
            let rp = self.p - context.adjusted_start(&offset, depth);
            if depth == 0 {
                if self.p % 2 == 1 {
                    tree.left = Some(*CTreeBuilder::get(commitments, rp - 1, &offset));
                    tree.right = Some(*CTreeBuilder::get(commitments, rp, &offset));
                } else {
                    tree.left = Some(*CTreeBuilder::get(commitments, rp, &offset));
                    tree.right = None;
                }
            } else {
                if self.p % 2 == 1 {
                    tree.parents
                        .push(Some(*CTreeBuilder::get(commitments, rp - 1, &offset)));
                } else if self.p != 0 {
                    tree.parents.push(None);
                }
            }
        }

        let p1 = self.p + 1;
        let has_p1 = p1 >= context.adjusted_start(&right, depth) && p1 < context.start + commitments.len();
        if has_p1 {
            let p1 = CTreeBuilder::get(commitments, p1 - context.adjusted_start(&right, depth), &right);
            if depth == 0 {
                if tree.right.is_none() {
                    self.witness.filled.push(*p1);
                }
            } else {
                if depth - 1 >= tree.parents.len() || tree.parents[depth - 1].is_none() {
                    self.witness.filled.push(*p1);
                }
            }
        }
        0
    }

    fn up(&mut self) {
        self.p /= 2;
    }

    fn finished(&self) -> bool {
        false
    }

    fn finalize(mut self, context: &CTreeBuilder) -> Witness {
        let tree = &self.witness.tree;
        let mut num_filled = self.witness.filled.len();

        if self.witness.position + 1 == context.next_tree.get_position() {
            self.witness.cursor = CTree::new();
        }
        else {
            let mut depth = 0;
            loop {
                let is_none = if depth == 0 { // check if this level is occupied
                    tree.right.is_none()
                } else {
                    depth > tree.parents.len() || tree.parents[depth - 1].is_none()
                };
                if is_none {
                    if num_filled > 0 {
                        num_filled -= 1; // we filled it
                    } else {
                        break
                    }
                }
                depth += 1;
                // loop terminates because we are eventually going to run out of ancestors and filled
            }

            self.witness.cursor = context.clone_trimmed(depth - 1);
        }
        self.witness
    }
}

#[allow(dead_code)]
pub fn advance_tree(
    prev_tree: CTree,
    prev_witnesses: &[Witness],
    mut commitments: &mut [Node],
) -> (CTree, Vec<Witness>) {
    if commitments.is_empty() {
        return (prev_tree, prev_witnesses.to_vec());
    }
    let mut builder = CTreeBuilder::new(prev_tree);
    let mut witness_builders: Vec<_> = prev_witnesses
        .iter()
        .map(|witness| WitnessBuilder::new(&builder, witness.clone(), commitments.len()))
        .collect();
    while !commitments.is_empty() || !builder.finished() {
        let n = builder.collect(commitments, &());
        for b in witness_builders.iter_mut() {
            b.collect(commitments, &builder);
        }
        let nn = combine_level(commitments, builder.offset, n, builder.depth);
        commitments = &mut commitments[0..nn];
        builder.up();
        for b in witness_builders.iter_mut() {
            b.up();
        }
    }

    let witnesses = witness_builders
        .into_iter()
        .map(|b| b.finalize(&builder))
        .collect();
    let tree = builder.finalize(&());
    (tree, witnesses)
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use crate::builder::advance_tree;
    use crate::commitment::{CTree, Witness};
    use crate::print::{print_tree, print_witness};
    use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
    use zcash_primitives::sapling::Node;

    #[test]
    fn test_advance_tree() {
        const NUM_NODES: usize = 1000;
        const NUM_CHUNKS: usize = 50;
        const WITNESS_PERCENT: f64 = 1.0; // percentage of notes that are ours
        const DEBUG_PRINT: bool = true;
        let witness_freq = (100.0 / WITNESS_PERCENT) as usize;

        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2 = CTree::new();
        let mut ws: Vec<IncrementalWitness<Node>> = vec![];
        let mut ws2: Vec<Witness> = vec![];
        for i in 0..NUM_CHUNKS {
            println!("{}", i);
            let mut nodes: Vec<_> = vec![];
            for j in 0..NUM_NODES {
                let mut bb = [0u8; 32];
                let v = i * NUM_NODES + j;
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                let node = Node::new(bb);
                tree1.append(node).unwrap();
                for w in ws.iter_mut() {
                    w.append(node).unwrap();
                }
                if v % witness_freq == 0 {
                // if v == 499 {
                    let w = IncrementalWitness::from_tree(&tree1);
                    ws.push(w);
                    ws2.push(Witness::new(v));
                }
                nodes.push(node);
            }

            let (new_tree, new_witnesses) = advance_tree(tree2, &ws2, &mut nodes);
            tree2 = new_tree;
            ws2 = new_witnesses;
        }

        // check final state
        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        let equal = bb1.as_slice() == bb2.as_slice();

        println!("# witnesses = {}", ws.len());

        // check witnesses
        let mut failed_index: Option<usize> = None;
        for (i, (w1, w2)) in ws.iter().zip(&ws2).enumerate() {
            let mut bb1: Vec<u8> = vec![];
            w1.write(&mut bb1).unwrap();

            let mut bb2: Vec<u8> = vec![];
            w2.write(&mut bb2).unwrap();

            if bb1.as_slice() != bb2.as_slice() {
                failed_index = Some(i);
            }
        }

        if DEBUG_PRINT && (failed_index.is_some() || !equal) {
            let i = failed_index.unwrap();
            println!("FAILED AT {}", i);
            print_witness(&ws[i]);

            // println!("-----");
            // println!("Final-----");
            //
            // println!("{:?}", tree2.left.map(|n| hex::encode(n.repr)));
            // println!("{:?}", tree2.right.map(|n| hex::encode(n.repr)));
            // for p in tree2.parents.iter() {
            //     println!("{:?}", p.map(|n| hex::encode(n.repr)));
            // }
            // println!("-----");

            // println!("{:?}", tree1.left.map(|n| hex::encode(n.repr)));
            // println!("{:?}", tree1.right.map(|n| hex::encode(n.repr)));
            // for p in tree1.parents.iter() {
            //     println!("{:?}", p.map(|n| hex::encode(n.repr)));
            // }
            println!("----- {}", ws2[i].position);
            let tree2 = &ws2[i].tree;
            println!("{:?}", tree2.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", tree2.right.map(|n| hex::encode(n.repr)));
            for p in tree2.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }
            println!("-----");
            let filled2 = &ws2[i].filled;
            println!("Filled");
            for f in filled2.iter() {
                println!("{:?}", hex::encode(f.repr));
            }
            println!("Cursor");
            let cursor2 = &ws2[i].cursor;
            println!("{:?}", cursor2.left.map(|n| hex::encode(n.repr)));
            println!("{:?}", cursor2.right.map(|n| hex::encode(n.repr)));
            for p in cursor2.parents.iter() {
                println!("{:?}", p.map(|n| hex::encode(n.repr)));
            }

            assert!(false);
        }
    }
}
