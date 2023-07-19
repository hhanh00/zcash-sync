use super::{DEPTH, Hashable, Path};

#[derive(Debug)]
pub struct Witness<N: Hashable> {
    pub path: Path<N>,
    pub fills: Vec<N>,
}

impl <N: Hashable> Witness<N> {
    pub fn root(&self, empty_roots: &[N; DEPTH], edge: &[N; DEPTH]) -> (N, [N; DEPTH]) {
        let mut p = self.path.pos;
        let mut h = self.path.value;
        let mut j = 0;
        let mut k = 0;
        let mut edge_used = false;
        let mut path = vec![];

        for i in 0..DEPTH {
            h =
                if p & 1 == 0 {
                    let r = if k < self.fills.len() {
                        let r = self.fills[k];
                        k += 1;
                        r
                    }
                    else if !edge_used {
                        edge_used = true;
                        edge[i]
                    }
                    else {
                        empty_roots[i]
                    };
                    path.push(r);
                    N::combine(i as u8, &h, &r, false)
                }
                else {
                    let l = self.path.siblings[j];
                    path.push(l);
                    let v = N::combine(i as u8, &l, &h, true);
                    j += 1;
                    v
                };
            p = p / 2;
        }
        (h, path.try_into().unwrap())
    }
}

