use super::{DEPTH, Hashable, Path};
use super::bridge::{Bridge, CompactLayer};
use super::witness::Witness;

#[derive(Debug)]
pub struct MerkleTree<N: Hashable> {
    pub pos: usize,
    pub prev: [N; DEPTH+1],
    pub witnesses: Vec<Witness<N>>,
}

impl <N: Hashable> Default for MerkleTree<N> {
    fn default() -> Self {
        MerkleTree {
            pos: 0,
            prev: [N::empty(); DEPTH+1],
            witnesses: vec![],
        }
    }
}

impl <N: Hashable> MerkleTree<N> {
    pub fn add_nodes(&mut self, block_len: usize, nodes: &[(N, bool)]) -> Bridge<N> {
        // let ns: Vec<_> = nodes.iter().map(|n| n.0).collect();
        // println!("{ns:?}");
        assert!(!nodes.is_empty());
        let mut compact_layers = vec![];
        let mut new_witnesses = vec![];
        for (i, n) in nodes.iter().enumerate() {
            if n.1 {
                self.witnesses.push(Witness {
                    path: Path {
                        pos: self.pos + i,
                        value: n.0,
                        siblings: vec![],
                    },
                    fills: vec![],
                });
                new_witnesses.push(self.witnesses.len() - 1);
            }
        }
        log::debug!("{:?}", new_witnesses);

        let mut layer = vec![];
        let mut fill = N::empty();
        if !self.prev[0].is_empty() {
            layer.push(self.prev[0]);
            fill = nodes[0].0;
        }
        layer.extend(nodes.iter().map(|n| n.0));

        for depth in 0..DEPTH {
            let mut new_fill = N::empty();
            let len = layer.len();
            let start = (self.pos >> depth) & 0xFFFE;
            for &wi in new_witnesses.iter() {
                let w = &mut self.witnesses[wi];
                let i = (w.path.pos >> depth) - start;
                if i & 1 == 1 {
                    assert_ne!(layer[i - 1], N::empty());
                    w.path.siblings.push(layer[i - 1]);
                }
            }
            for w in self.witnesses.iter_mut() {
                if (w.path.pos >> depth) >= start {
                    let i = (w.path.pos >> depth) - start;
                    if i & 1 == 0 && i < len - 1 && !layer[i + 1].is_empty() {
                        w.fills.push(layer[i + 1]);
                    }
                }
            }
            log::debug!("w {:?}", self.witnesses);

            let pairs = (len + 1) / 2;
            let mut new_layer = vec![];
            if !self.prev[depth + 1].is_empty() {
                new_layer.push(self.prev[depth + 1]);
            }
            self.prev[depth] = N::empty();
            for i in 0..pairs {
                let l = layer[2 * i];
                if 2 * i + 1 < len {
                    if !layer[2 * i + 1].is_empty() {
                        let hn = N::combine(depth as u8, &l, &layer[2 * i + 1], true);
                        if (i == 0 && self.prev[depth + 1] != N::empty()) ||
                            (i == 1 && self.prev[depth + 1] == N::empty()) {
                            new_fill = hn;
                        }
                        new_layer.push(hn);
                    } else {
                        new_layer.push(N::empty());
                        self.prev[depth] = l;
                    }
                } else {
                    if !l.is_empty() {
                        self.prev[depth] = l;
                    }
                    new_layer.push(N::empty());
                }
            }

            compact_layers.push(CompactLayer {
                prev: self.prev[depth],
                fill,
            });

            layer = new_layer;
            fill = new_fill;
            log::debug!("{layer:?}");
        }
        let pos = self.pos;
        self.pos += nodes.len();
        Bridge {
            pos,
            block_len,
            len: nodes.len(),
            layers: compact_layers.try_into().unwrap(),
        }
    }

    pub fn add_bridge(&mut self, bridge: &Bridge<N>) {
        for h in 0..DEPTH {
            if !bridge.layers[h].fill.is_empty() {
                let s = self.pos >> (h + 1);
                for w in self.witnesses.iter_mut() {
                    let p = w.path.pos >> h;
                    if p & 1 == 0 && p >> 1 == s {
                        w.fills.push(bridge.layers[h].fill);
                    }
                }
            }
            self.prev[h] = bridge.layers[h].prev;
        }
        self.pos += bridge.len;
    }

    pub fn edge(&self, empty_roots: &[N]) -> [N; DEPTH]{
        let mut path = vec![N::empty()];
        let mut h = N::empty();
        for depth in 0..DEPTH-1 {
            let n = self.prev[depth];
            if !n.is_empty() {
                h = N::combine(depth as u8, &n, &h, false);
            }
            else {
                h = N::combine(depth as u8, &h, &empty_roots[depth], false);
            }
            path.push(h);
        }
        path.try_into().unwrap()
    }
}
