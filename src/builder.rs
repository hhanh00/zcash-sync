use crate::commitment::{CTree, Witness};
use rayon::prelude::IntoParallelIterator;
use rayon::prelude::*;
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;

trait Builder<T, C> {
    fn collect(&mut self, commitments: &[Node], context: &C) -> usize;
    fn up(&mut self);
    fn finished(&self) -> bool;
    fn finalize(self) -> T;
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
                self.next_tree.left = Some(Self::get(commitments, m - 2, offset));
                self.next_tree.right = Some(Self::get(commitments, m - 1, offset));
                m - 2
            } else {
                self.next_tree.left = Some(Self::get(commitments, m - 1, offset));
                self.next_tree.right = None;
                m - 1
            }
        } else {
            if m % 2 == 0 {
                self.next_tree.parents.push(None);
                m
            } else {
                let last_node = Self::get(commitments, m - 1, offset);
                self.next_tree.parents.push(Some(last_node));
                m - 1
            }
        };
        assert_eq!(n % 2, 0);

        self.offset = offset;
        n
    }

    fn up(&mut self) {
        let h = if self.left.is_some() && self.right.is_some() {
            Some(Node::combine(self.depth, &self.left.unwrap(), &self.right.unwrap()))
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

    fn finalize(self) -> CTree {
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
    fn get(commitments: &[Node], index: usize, offset: Option<Node>) -> Node {
        match offset {
            Some(offset) => {
                if index > 0 {
                    commitments[index - 1]
                } else {
                    offset
                }
            }
            None => commitments[index],
        }
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
                &CTreeBuilder::get(commitments, 2 * i, offset),
                &CTreeBuilder::get(commitments, 2 * i + 1, offset),
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

        if self.inside {
            let rp = self.p - context.start;
            if depth == 0 {
                if self.p % 2 == 1 {
                    self.witness.tree.left = Some(CTreeBuilder::get(commitments, rp - 1, offset));
                    self.witness.tree.right = Some(CTreeBuilder::get(commitments, rp, offset));
                }
                else {
                    self.witness.tree.left = Some(CTreeBuilder::get(commitments, rp, offset));
                    self.witness.tree.right = None;
                }
            }
            else {
                if self.p % 2 == 1 {
                    self.witness.tree.parents.push(Some(CTreeBuilder::get(commitments, rp, offset)));
                }
                else {
                    self.witness.tree.parents.push(None);
                }
            }
        }

        // TODO: update filled

        0
    }

    fn up(&mut self) {
        self.p /= 2;
    }

    fn finished(&self) -> bool {
        false
    }

    fn finalize(self) -> Witness {
        self.witness
    }
}

#[allow(dead_code)]
fn advance_tree(prev_tree: CTree, prev_witnesses: &[Witness], mut commitments: &mut [Node]) -> (CTree, Vec<Witness>) {
    if commitments.is_empty() {
        return (prev_tree, prev_witnesses.to_vec());
    }
    let mut builder = CTreeBuilder::new(prev_tree);
    let mut witness_builders: Vec<_> = prev_witnesses.iter().map(|witness| {
        WitnessBuilder::new(&builder, witness.clone(), commitments.len())
    }).collect();
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

    let tree = builder.finalize();
    let witnesses = witness_builders.into_iter().map(|b| b.finalize()).collect();
    (tree, witnesses)
}

#[cfg(test)]
mod tests {
    use crate::builder::{advance_tree, WitnessBuilder};
    use crate::commitment::{CTree, Witness};
    use zcash_primitives::sapling::Node;
    use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
    use crate::print::{print_witness, print_tree};

    #[test]
    fn test_advance_tree() {
        const NUM_NODES: usize = 10;
        const NUM_CHUNKS: usize = 10;
        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2 = CTree::new();
        let mut ws: Vec<IncrementalWitness<Node>> = vec![];
        let mut ws2: Vec<Witness> = vec![];
        for i in 0..NUM_CHUNKS {
            let mut nodes: Vec<_> = (0..NUM_NODES).map(|k| {
                let mut bb = [0u8; 32];
                let v = i*NUM_NODES + k;
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                let node = Node::new(bb);
                tree1.append(node).unwrap();
                for w in ws.iter_mut() {
                    w.append(node).unwrap();
                }
                if v == 55 {
                    let w = IncrementalWitness::from_tree(&tree1);
                    ws.push(w);
                    ws2.push(Witness::new(v));
                }
                node
            }).collect();

            let (new_tree, new_witnesses) = advance_tree(tree2, &ws2, &mut nodes);
            tree2 = new_tree;
            ws2 = new_witnesses;
        }
        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        let equal = bb1.as_slice() == bb2.as_slice();

        print_tree(&ws[0].tree);
        println!("-----");

        let tree2 = &ws2[0].tree;

        // println!("{:?}", tree1.left.map(|n| hex::encode(n.repr)));
        // println!("{:?}", tree1.right.map(|n| hex::encode(n.repr)));
        // for p in tree1.parents.iter() {
        //     println!("{:?}", p.map(|n| hex::encode(n.repr)));
        // }
        // println!("-----");
        //
        println!("{:?}", tree2.left.map(|n| hex::encode(n.repr)));
        println!("{:?}", tree2.right.map(|n| hex::encode(n.repr)));
        for p in tree2.parents.iter() {
            println!("{:?}", p.map(|n| hex::encode(n.repr)));
        }
        println!("-----");
        //
        // assert!(equal, "not equal");
    }
}