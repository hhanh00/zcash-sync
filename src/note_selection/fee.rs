use super::types::*;
use crate::db::data_generated::fb::FeeT;
use std::cmp::max;

const MARGINAL_FEE: u64 = 5000;
const GRACE_ACTIONS: u64 = 2;

pub trait FeeCalculator {
    fn calculate_fee(&self, inputs: &[UTXO], outputs: &[Fill]) -> u64;
}

pub struct FeeZIP327;

impl FeeCalculator for FeeZIP327 {
    fn calculate_fee(&self, inputs: &[UTXO], outputs: &[Fill]) -> u64 {
        let mut n_in = [0; 3]; // count of inputs
        let mut n_out = [0; 3];

        for i in inputs {
            let pool = i.source.pool() as usize;
            n_in[pool] += 1;
        }
        for o in outputs {
            let pool = o.destination.pool() as usize;
            n_out[pool] += 1;
        }

        let n_logical_actions =
            max(n_in[0], n_out[0]) + max(n_in[1], n_out[1]) + max(n_in[2], n_out[2]);

        log::info!(
            "fee: {}/{} {}/{} {}/{} = {}",
            n_in[0],
            n_out[0],
            n_in[1],
            n_out[1],
            n_in[2],
            n_out[2],
            n_logical_actions
        );
        let fee = MARGINAL_FEE * max(n_logical_actions, GRACE_ACTIONS);
        fee
    }
}

pub struct FeeFlat {
    fee: u64,
}

impl FeeFlat {
    pub fn from_rule(fee_rule: &FeeT) -> Self {
        FeeFlat { fee: fee_rule.fee }
    }
}

impl FeeCalculator for FeeFlat {
    fn calculate_fee(&self, _inputs: &[UTXO], _outputs: &[Fill]) -> u64 {
        self.fee
    }
}
