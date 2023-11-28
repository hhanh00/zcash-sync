use super::types::*;
use crate::db::data_generated::fb::FeeT;
use std::cmp::{max, min};

const MARGINAL_FEE: u64 = 5000;
const GRACE_ACTIONS: u64 = 2;

pub trait FeeCalculator {
    fn calculate_fee(&self, inputs: &[UTXO], outputs: &[Fill]) -> u64;
}

pub enum FeeRule {
    FeeZIP327(FeeZIP327),
    FeeFlat(FeeFlat),
}

impl FeeRule {
    pub fn from_rule(fee_config: &FeeT) -> Self {
        match fee_config.scheme {
            0 => FeeRule::FeeZIP327(FeeZIP327 {}),
            1 => FeeRule::FeeFlat(FeeFlat {
                fee: fee_config.fee,
            }),
            _ => unreachable!(),
        }
    }
}

impl FeeCalculator for FeeRule {
    fn calculate_fee(&self, inputs: &[UTXO], outputs: &[Fill]) -> u64 {
        match self {
            FeeRule::FeeFlat(f) => f.calculate_fee(inputs, outputs),
            FeeRule::FeeZIP327(f) => f.calculate_fee(inputs, outputs),
        }
    }
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

        for i in 1..3 {
            if n_out[i] != 0 { // if used, shielded pools have a min of 2 outputs
                n_out[i] = max(2, n_out[i]);
            }
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
    #[allow(dead_code)]
    pub fn from_rule(fee_rule: &FeeT) -> Self {
        FeeFlat { fee: fee_rule.fee }
    }
}

impl FeeCalculator for FeeFlat {
    fn calculate_fee(&self, _inputs: &[UTXO], _outputs: &[Fill]) -> u64 {
        self.fee
    }
}
