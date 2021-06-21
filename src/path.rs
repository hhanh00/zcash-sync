use zcash_primitives::merkle_tree::Hashable;
use zcash_primitives::sapling::Node;

pub struct MerklePath {
    left: Option<Node>,
    right: Option<Node>,
}

impl MerklePath {
    pub fn new(left: Option<Node>, right: Option<Node>) -> Self {
        MerklePath { left, right }
    }

    pub fn get(&self) -> Node {
        self.left.unwrap() // shouldn't call if empty
    }

    pub fn set(&mut self, right: Node) {
        assert!(self.left.is_some());
        self.right = Some(right);
    }

    pub fn up(&mut self, depth: usize, parent: &Option<Node>) {
        let node = if self.left.is_some() && self.right.is_some() {
            Some(Node::combine(depth, &self.left.unwrap(), &self.right.unwrap()))
        } else {
            None
        };
        if parent.is_some() {
            self.left = *parent;
            self.right = node;
        } else {
            self.left = node;
            self.right = None;
        }
    }
}
