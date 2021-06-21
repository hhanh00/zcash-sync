use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use sync::{NETWORK, scan_all, Witness, CTree, advance_tree};
use zcash_primitives::consensus::Parameters;
use zcash_primitives::merkle_tree::{CommitmentTree, IncrementalWitness};
use zcash_primitives::sapling::Node;
use std::time::Instant;

#[tokio::main]
#[allow(dead_code)]
async fn main_scan() {
    dotenv::dotenv().unwrap();
    env_logger::init();

    let ivk = dotenv::var("IVK").unwrap();
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
            .unwrap()
            .unwrap();
    let ivk = fvk.fvk.vk.ivk();

    scan_all(&vec![ivk]).await.unwrap();
}

fn test_advance_tree() {
    const NUM_NODES: usize = 1000;
    const NUM_CHUNKS: usize = 50;
    const WITNESS_PERCENT: f64 = 1.0; // percentage of notes that are ours
    let witness_freq = (100.0 / WITNESS_PERCENT) as usize;

    let mut _tree1: CommitmentTree<Node> = CommitmentTree::empty();
    let mut tree2 = CTree::new();
    let mut _ws: Vec<IncrementalWitness<Node>> = vec![];
    let mut ws2: Vec<Witness> = vec![];
    let start = Instant::now();
    for i in 0..NUM_CHUNKS {
        eprintln!("{}, {}", i, start.elapsed().as_millis());
        let mut nodes: Vec<_> = vec![];
        for j in 0..NUM_NODES {
            let mut bb = [0u8; 32];
            let v = i * NUM_NODES + j;
            bb[0..8].copy_from_slice(&v.to_be_bytes());
            let node = Node::new(bb);
            // tree1.append(node).unwrap();
            // for w in ws.iter_mut() {
            //     w.append(node).unwrap();
            // }
            if v % witness_freq == 0 {
                // let w = IncrementalWitness::from_tree(&tree1);
                // ws.push(w);
                ws2.push(Witness::new(v));
            }
            nodes.push(node);
        }
        let (new_tree, new_witnesses) = advance_tree(tree2, &ws2, &mut nodes);
        tree2 = new_tree;
        ws2 = new_witnesses;
    }

    println!("# witnesses = {}", ws2.len());
}

fn main() {
    test_advance_tree();
}
