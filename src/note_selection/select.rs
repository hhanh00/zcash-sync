use std::cmp::min;
use std::slice;
use anyhow::anyhow;
use zcash_primitives::memo::MemoBytes;
use crate::note_selection::decode;
use crate::note_selection::fee::FeeCalculator;
use crate::note_selection::fill::execute_orders;
use crate::note_selection::types::{NoteSelectConfig, Order, PoolAllocation, UTXO, Destination, TransactionPlan, Fill, PoolPrecedence};

pub fn select_notes(allocation: &PoolAllocation, utxos: &[UTXO]) -> anyhow::Result<Vec<UTXO>> {
    let mut allocation = allocation.clone();

    let mut selected = vec![];
    for utxo in utxos {
        let pool = utxo.source.pool() as usize;
        if allocation.0[pool] > 0 {
            let amount = min(allocation.0[pool], utxo.amount);
            selected.push(utxo.clone());
            allocation.0[pool] -= amount;
        }
    }
    Ok(selected)
}

fn has_unfilled(orders: &[Order]) -> bool {
    orders.iter().any(|o| o.filled != o.amount)
}

struct OrderExecutor {
    pub pool_available: PoolAllocation,
    pub pool_used: PoolAllocation,
    pub config: NoteSelectConfig,
    pub fills: Vec<Fill>,
}

impl OrderExecutor {
    pub fn new(initial_pool: PoolAllocation, config: NoteSelectConfig) -> Self {
        OrderExecutor {
            pool_available: initial_pool,
            pool_used: PoolAllocation::default(),
            config,
            fills: vec![],
        }
    }

    pub fn execute(&mut self, orders: &mut [Order], precedence: &PoolPrecedence) -> anyhow::Result<bool> {
        let order_execution = execute_orders(orders, &self.pool_available,
                                             self.config.use_transparent, self.config.use_shielded,
                                             self.config.privacy_policy.clone(), precedence)?; // calculate an execution plan without considering the fee
        self.fills.extend(order_execution.fills);
        self.pool_available = self.pool_available - order_execution.allocation;
        self.pool_used = self.pool_used + order_execution.allocation;
        let fully_filled = orders.iter().all(|o| o.amount == o.filled);
        Ok(fully_filled)
    }

    pub fn add(&mut self, fill: Fill) {
        self.fills.push(fill);
    }

    pub fn select_notes(&self, utxos: &[UTXO]) -> anyhow::Result<Vec<UTXO>> {
        select_notes(&self.pool_used, &utxos)
    }
}

const ANY_DESTINATION: [Option<Destination>; 3] = [Some(Destination::Transparent([0u8; 20])), Some(Destination::Sapling([0u8; 43])), Some(Destination::Orchard([0u8; 43]))];
const MAX_ATTEMPTS: usize = 10;
/// Select notes from the `utxos` that can pay for the `orders`
///
pub fn note_select_with_fee<F: FeeCalculator>(utxos: &[UTXO], orders: &mut [Order], config: &NoteSelectConfig) -> anyhow::Result<TransactionPlan> {
    let initial_pool = PoolAllocation::from(&*utxos); // amount of funds available in each pool
    let mut fee = 0;
    let mut n_attempts = 0;
    let change_order = decode(u32::MAX, &config.change_address, 0, MemoBytes::empty())?;
    let change_destinations = change_order.destinations;

    let mut plan = loop {
        for o in orders.iter_mut() {
            o.filled = 0;
        }
        let mut executor = OrderExecutor::new(initial_pool, config.clone());
        if !executor.execute(orders, &config.precedence)? {
            anyhow::bail!("Unsufficient Funds")
        }
        if fee == 0 {
            let notes = executor.select_notes(utxos)?;
            fee = F::calculate_fee(&notes, &executor.fills);
            log::info!("base fee: {}", fee);
        }
        let mut fee_order = Order {
            id: u32::MAX,
            destinations: ANY_DESTINATION,
            amount: fee,
            memo: MemoBytes::empty(),
            is_fee: true, // do not include in fee calculation
            filled: 0,
        };

        // Favor the pools that are already used, because it may cause lower fees
        let pool_needed = executor.pool_used;
        let prec_1 = config.precedence.iter().filter(|&&p| pool_needed.0[p as usize] != 0);
        let prec_2 = config.precedence.iter().filter(|&&p| pool_needed.0[p as usize] == 0);
        let fee_precedence: PoolPrecedence = prec_1.chain(prec_2).cloned().collect::<Vec<_>>().try_into().unwrap();
        log::info!("Fee precedence: {:?}", fee_precedence);

        if !executor.execute(slice::from_mut(&mut fee_order), &fee_precedence)? {
            anyhow::bail!("Unsufficient Funds [fees]")
        }

        // Let's figure out the change
        let pool_needed = executor.pool_used;
        let total_needed = pool_needed.total();

        let notes = executor.select_notes(utxos)?;
        let pool_spent = PoolAllocation::from(&*notes);
        let total_spent = pool_spent.total();
        let change = pool_spent - pool_needed; // must be >= 0 because the note selection covers the fills

        log::info!("pool_needed: {:?} {}", pool_needed, total_needed);
        log::info!("pool_spent: {:?} {}", pool_spent, total_spent);
        log::info!("change: {:?}", change);

        for pool in 0..3 {
            if change.0[pool] != 0 {
                executor.add(Fill {
                    id_order: u32::MAX,
                    destination: change_destinations[pool].unwrap(),
                    amount: change.0[pool],
                    is_fee: false
                })
            }
        }

        let notes = executor.select_notes(utxos)?;
        let new_fee = F::calculate_fee(&notes, &executor.fills);
        log::info!("new fee: {}", new_fee);

        if new_fee == fee || n_attempts == MAX_ATTEMPTS {
            let plan = TransactionPlan {
                spends: notes.clone(),
                outputs: executor.fills.clone(),
                fee: 0,
            };
            break plan;
        }
        fee = new_fee; // retry with the new fee
        n_attempts += 1;
    };

    plan.fee = plan.outputs.iter().filter_map(|f| if f.is_fee { Some(f.amount) } else { None }).sum();
    plan.outputs = plan.outputs.into_iter().filter(|f| !f.is_fee).collect();

    Ok(plan)
}
