use crate::commitment::CTree;
use rayon::prelude::IntoParallelIterator;
use rayon::prelude::*;
use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;

trait Builder {
    fn collect(&mut self, commitments: &[Node]) -> (Option<Node>, usize);
    fn up(&mut self);
    fn finished(&self) -> bool;
    fn finalize(self) -> CTree;
}

struct CTreeBuilder {
    left: Option<Node>,
    right: Option<Node>,
    prev_tree: CTree,
    next_tree: CTree,
    depth: usize,
}

impl Builder for CTreeBuilder {
    fn collect(&mut self, commitments: &[Node]) -> (Option<Node>, usize) {
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

        (offset, n)
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
        CTreeBuilder {
            left: prev_tree.left,
            right: prev_tree.right,
            prev_tree,
            next_tree: CTree::new(),
            depth: 0,
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

#[allow(dead_code)]
fn advance_tree(prev_tree: CTree, mut commitments: &mut [Node]) -> CTree {
    if commitments.is_empty() {
        return prev_tree;
    }
    let mut builder = CTreeBuilder::new(prev_tree);
    while !commitments.is_empty() || !builder.finished() {
        let (offset, n) = builder.collect(commitments);
        let nn = combine_level(commitments, offset, n, builder.depth);
        commitments = &mut commitments[0..nn];
        builder.up();
    }

    builder.finalize()
}

#[cfg(test)]
mod tests {
    use crate::builder::advance_tree;
    use crate::commitment::CTree;
    use zcash_primitives::sapling::Node;
    use zcash_primitives::merkle_tree::CommitmentTree;

    #[test]
    fn test_advance_tree() {
        const NUM_NODES: usize = 100;
        const NUM_CHUNKS: usize = 100;
        let mut tree1: CommitmentTree<Node> = CommitmentTree::empty();
        let mut tree2 = CTree::new();
        for i in 0..NUM_CHUNKS {
            let mut nodes: Vec<_> = (0..NUM_NODES).map(|k| {
                let mut bb = [0u8; 32];
                let v = i*NUM_NODES + k;
                bb[0..8].copy_from_slice(&v.to_be_bytes());
                let node = Node::new(bb);
                tree1.append(node).unwrap();
                node
            }).collect();

            tree2 = advance_tree(tree2, &mut nodes);
        }
        let mut bb1: Vec<u8> = vec![];
        tree1.write(&mut bb1).unwrap();

        let mut bb2: Vec<u8> = vec![];
        tree2.write(&mut bb2).unwrap();

        let equal = bb1.as_slice() == bb2.as_slice();

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
        println!("-----");

        assert!(equal, "not equal");
    }
}