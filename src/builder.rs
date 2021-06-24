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
    total_len: usize,
    depth: usize,
    offset: Option<Node>,
}

impl Builder<CTree, ()> for CTreeBuilder {
    fn collect(&mut self, commitments: &[Node], _context: &()) -> usize {
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

        let n =
            if self.total_len > 0 {
                if self.depth == 0 {
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
                }
            }
        else { 0 };
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
        if self.total_len > 0 {
            self.next_tree
        } else {
            self.prev_tree
        }
    }
}

impl CTreeBuilder {
    fn new(prev_tree: CTree, len: usize) -> CTreeBuilder {
        let start = prev_tree.get_position();
        CTreeBuilder {
            left: prev_tree.left,
            right: prev_tree.right,
            prev_tree,
            next_tree: CTree::new(),
            start,
            total_len: len,
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

    fn adjusted_start(&self, prev: &Option<Node>, _depth: usize) -> usize {
        if prev.is_some() {
            self.start - 1
        } else {
            self.start
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

        // println!("D {}", depth);
        // println!("O {:?}", offset.map(|r| hex::encode(r.repr)));
        // println!("R {:?}", right.map(|r| hex::encode(r.repr)));
        // for c in commitments.iter() {
        //     println!("{}", hex::encode(c.repr));
        // }
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
        if context.total_len == 0 {
            self.witness.cursor = CTree::new();

            let mut final_position = context.prev_tree.get_position() as u32;
            let mut witness_position = self.witness.tree.get_position() as u32;
            assert_ne!(witness_position, 0);
            witness_position = witness_position - 1;

            // look for first not equal bit in MSB order
            final_position = final_position.reverse_bits();
            witness_position = witness_position.reverse_bits();
            let mut bit: i32 = 31;
            // reverse bits because it is easier to do in LSB
            // it should not underflow because these numbers are not equal
            while bit >= 0 {
                if final_position & 1 != witness_position & 1 {
                    break;
                }
                final_position >>= 1;
                witness_position >>= 1;
                bit -= 1;
            }
            // look for the first bit set in final_position after
            final_position >>= 1;
            bit -= 1;
            while bit >= 0 {
                if final_position & 1 == 1 {
                    break;
                }
                final_position >>= 1;
                bit -= 1;
            }
            if bit >= 0 {
                self.witness.cursor = context.prev_tree.clone_trimmed(bit as usize)
            }
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
    let mut builder = CTreeBuilder::new(prev_tree, commitments.len());
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
        builder.up();
        for b in witness_builders.iter_mut() {
            b.up();
        }
        commitments = &mut commitments[0..nn];
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
    use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
    use zcash_primitives::sapling::Node;
    use crate::chain::DecryptedNote;
    use crate::print::{print_witness, print_witness2, print_tree, print_ctree};

    #[test]
    fn test_advance_tree() {
        for num_nodes in 1..=10 {
            for num_chunks in 1..=10 {
                test_advance_tree_helper(num_nodes, num_chunks, 100.0);
            }
        }

        test_advance_tree_helper(100, 50, 1.0);
        // test_advance_tree_helper(2, 10, 100.0);
        // test_advance_tree_helper(1, 40, 100.0);
        // test_advance_tree_helper(10, 2, 100.0);
    }

    fn test_advance_tree_helper(num_nodes: usize, num_chunks: usize, witness_percent: f64) {
        let witness_freq = (100.0 / witness_percent) as usize;

        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2 = CTree::new();
        let mut ws: Vec<IncrementalWitness<Node>> = vec![];
        let mut ws2: Vec<Witness> = vec![];
        for i in 0..num_chunks {
            println!("{}", i);
            let mut nodes: Vec<_> = vec![];
            for j in 0..num_nodes {
                let mut bb = [0u8; 32];
                let v = i * num_nodes + j;
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                let node = Node::new(bb);
                tree1.append(node).unwrap();
                for w in ws.iter_mut() {
                    w.append(node).unwrap();
                }
                if v % witness_freq == 0 {
                // if v == 0 {
                    let w = IncrementalWitness::from_tree(&tree1);
                    ws.push(w);
                    ws2.push(Witness::new(v, 0, None));
                }
                nodes.push(node);
            }

            let (new_tree, new_witnesses) = advance_tree(tree2, &ws2, &mut nodes);
            tree2 = new_tree;
            ws2 = new_witnesses;
        }

        // Push an empty block
        // It will calculate the tail of the tree
        // This step is required at the end of a series of chunks
        let (new_tree, new_witnesses) = advance_tree(tree2, &ws2, &mut []);
        tree2 = new_tree;
        ws2 = new_witnesses;

        // check final state
        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        let equal = bb1.as_slice() == bb2.as_slice();
        if !equal {
            println!("FAILED FINAL STATE");
            print_tree(&tree1);
            print_ctree(&tree2);
        }

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
                println!("FAILED AT {}", i);
                if let Some(ref c) = w1.cursor {
                    print_tree(c);
                }
                else { println!("NONE"); }

                println!("GOOD");
                print_witness(&w1);
                println!("BAD");
                print_witness2(&w2);
            }
        }

        assert!(equal && failed_index.is_none());
    }
}
