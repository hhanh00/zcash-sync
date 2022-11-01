use std::cmp::max;
use crate::note_selection::types::{Fill, UTXO};

const MARGINAL_FEE: u64 = 5000;
const GRACE_ACTIONS: u64 = 2;

pub trait FeeCalculator {
    fn calculate_fee(inputs: &[UTXO], outputs: &[Fill]) -> u64;
}

pub struct FeeZIP327;

impl FeeCalculator for FeeZIP327 {
    fn calculate_fee(inputs: &[UTXO], outputs: &[Fill]) -> u64 {
        let mut n_in = [0; 3]; // count of inputs
        let mut n_out = [0; 3];

        for i in inputs {
            let pool = i.source.pool() as usize;
            n_in[pool] += 1;
        }
        for o in outputs {
            if !o.is_fee {
                let pool = o.destination.pool() as usize;
                n_out[pool] += 1;
            }
        }

        let n_logical_actions = max(n_in[0], n_out[0]) +
            max(n_in[1], n_out[1]) +
            max(n_in[2], n_out[2]);
        let fee = MARGINAL_FEE * max(n_logical_actions, GRACE_ACTIONS);
        fee
    }
}

pub struct FeeFlat;

impl FeeCalculator for FeeFlat {
    fn calculate_fee(_inputs: &[UTXO], _outputs: &[Fill]) -> u64 {
        1000
    }
}
