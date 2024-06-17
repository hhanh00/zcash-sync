use anyhow::Result;
use orchard::tree::MerkleHashOrchard;
use tonic::{transport::Channel, Request};
use zcash_primitives::merkle_tree::CommitmentTree;

use crate::{
    vote::{Hash, DEPTH},
    BlockId, CompactTxStreamerClient,
};

#[derive(Default)]
pub struct PreviousHashes {
    pub lefts: [Option<Hash>; DEPTH],
}

impl PreviousHashes {
    pub fn position(&self) -> usize {
        let mut p = 0;
        for i in 0..DEPTH {
            if self.lefts[i].is_some() {
                p |= 1 << i;
            }
        }
        p
    }
}

pub async fn fetch_tree_state(
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
) -> Result<PreviousHashes> {
    let tree_state = client
        .get_tree_state(Request::new(BlockId {
            height: height as u64,
            hash: vec![],
        }))
        .await?
        .into_inner();
    let orchard_tree_state = hex::decode(&tree_state.orchard_tree).unwrap();
    let tree = CommitmentTree::<MerkleHashOrchard>::read(&*orchard_tree_state).unwrap();
    // We are not allowed to have any layer with both left & right
    // but the tree state may have both of the leaves
    // When this happens, we merge the leaves and push the resulting hash
    // up. It cascades further when the upper layer also has a node
    let mut ph = [None; DEPTH];
    ph[0] = tree.left.map(|v| v.to_bytes());
    for (i, p) in tree.parents.iter().enumerate() {
        ph[i + 1] = p.map(|p| p.to_bytes());
    }
    if let Some(mut r) = tree.right.map(|v| v.to_bytes()) {
        for (i, left) in ph.iter_mut().enumerate() {
            if let Some(l) = left {
                r = orchard::pob::cmx_hash(i as u8, &l, &r);
                *left = None;
            } else {
                *left = Some(r);
                break;
            }
        }
    }

    let ph = PreviousHashes { lefts: ph };

    assert_eq!(ph.position(), tree.size());

    Ok(ph)
}

pub async fn get_tree_root(
    client: &mut CompactTxStreamerClient<Channel>,
    height: u32,
) -> Result<Hash> {
    let tree_state = client
        .get_tree_state(Request::new(BlockId {
            height: height as u64,
            hash: vec![],
        }))
        .await?
        .into_inner();
    let orchard_tree = hex::decode(&tree_state.orchard_tree).unwrap();
    let tree_state = CommitmentTree::<MerkleHashOrchard>::read(&*orchard_tree).unwrap();
    let root = tree_state.root().to_bytes();
    Ok(root)
}
