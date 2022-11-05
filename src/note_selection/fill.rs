use std::cmp::min;
use zcash_address::{AddressKind, ZcashAddress};
use zcash_address::unified::{Container, Receiver};
use zcash_primitives::memo::MemoBytes;
use crate::note_selection::types::{PrivacyPolicy, Fill, Execution, Order, Pool, PoolAllocation, Destination, PoolPrecedence, PoolPriority};

/// Decode address and return it as an order
///
pub fn decode(id: u32, address: &str, amount: u64, memo: MemoBytes) -> anyhow::Result<Order> {
    let address = ZcashAddress::try_from_encoded(address)?;
    let mut order = Order::default();
    let mut precedence = order.priority.to_pool_precedence().clone();
    order.id = id;
    match address.kind {
        AddressKind::Sprout(_) => {}
        AddressKind::Sapling(data) => {
            let destination = Destination::Sapling(data);
            order.destinations[Pool::Sapling as usize] = Some(destination);
        }
        AddressKind::Unified(unified_address) => {
            for (i, address) in unified_address.items().into_iter().enumerate() {
                match address {
                    Receiver::Orchard(data) => {
                        let destination = Destination::Orchard(data);
                        order.destinations[Pool::Orchard as usize] = Some(destination);
                    }
                    Receiver::Sapling(data) => {
                        let destination = Destination::Sapling(data);
                        order.destinations[Pool::Sapling as usize] = Some(destination);
                    }
                    Receiver::P2pkh(data) => {
                        let destination = Destination::Transparent(data);
                        order.destinations[Pool::Transparent as usize] = Some(destination);
                    }
                    Receiver::P2sh(_) => {}
                    Receiver::Unknown { .. } => {}
                }
            }
        }
        AddressKind::P2pkh(data) => {
            let destination = Destination::Transparent(data);
            order.destinations[Pool::Transparent as usize] = Some(destination);
        }
        AddressKind::P2sh(_) => {}
    }
    order.amount = amount;
    order.memo = memo;

    Ok(order)
}

pub fn execute_orders(orders: &mut [Order], initial_pool: &PoolAllocation, use_transparent: bool, use_shielded: bool,
                      privacy_policy: PrivacyPolicy, precedence: &PoolPrecedence) -> anyhow::Result<Execution> {
    let mut allocation: PoolAllocation = PoolAllocation::default();
    let mut fills = vec![];

    for order in orders.iter_mut() {
        let order_precedence = order.priority.to_pool_precedence();
        // log::info!("Order {:?}", order);
        // Direct Shielded Fill - s2s, o2o
        // t2t only for fees
        if use_shielded {
            for &pool in order_precedence {
                if pool == Pool::Transparent && !order.is_fee { continue }
                if order.destinations[pool as usize].is_some() {
                    fill_order(pool, pool, order, initial_pool, &mut allocation, &mut fills);
                }
            }
        }

        if privacy_policy != PrivacyPolicy::SamePoolOnly {
            // Indirect Shielded - z2z: s2o, o2s
            for &pool in order_precedence {
                if order.destinations[pool as usize].is_none() { continue }
                if !use_shielded { continue }
                if let Some(from_pool) = pool.other_shielded() {
                    fill_order(from_pool, pool, order, initial_pool, &mut allocation, &mut fills);
                }
            }

            if privacy_policy == PrivacyPolicy::AnyPool {
                // Other - s2t, o2t, t2s, t2o
                for &pool in order_precedence {
                    if order.destinations[pool as usize].is_none() { continue }
                    match pool {
                        Pool::Transparent if use_shielded => {
                            for &from_pool in precedence {
                                if from_pool != Pool::Transparent {
                                    fill_order(from_pool, pool, order, initial_pool, &mut allocation, &mut fills);
                                }
                            }
                        }
                        Pool::Sapling | Pool::Orchard if use_transparent => {
                            fill_order(Pool::Transparent, pool, order, initial_pool, &mut allocation, &mut fills);
                        }
                        _ => {}
                    };
                }

                // t2t
                if use_transparent && order.destinations[Pool::Transparent as usize].is_some() {
                    fill_order(Pool::Transparent, Pool::Transparent, order, initial_pool, &mut allocation, &mut fills);
                }
            }
        }
    }

    let execution = Execution {
        allocation,
        fills,
    };

    Ok(execution)
}

fn fill_order(from: Pool, to: Pool, order: &mut Order, initial_pool: &PoolAllocation,
              fills: &mut PoolAllocation, executions: &mut Vec<Fill>) {
    let from = from as usize;
    let to = to as usize;
    let destination = order.destinations[to].as_ref().unwrap(); // Checked by caller
    let order_remaining = order.amount - order.filled;
    let pool_remaining = initial_pool.0[from] - fills.0[from];
    let amount = min(pool_remaining, order_remaining);
    order.filled += amount;
    fills.0[from] += amount;
    if amount > 0 {
        let execution = Fill {
            id_order: order.id,
            destination: destination.clone(),
            amount,
            memo: order.memo.clone(),
            is_fee: order.is_fee,
        };
        log::debug!("{:?}", execution);
        executions.push(execution);
    }
    assert!(order.amount == order.filled || initial_pool.0[from] == fills.0[from]); // fill must be to the max
}

impl Pool {
    fn other_shielded(&self) -> Option<Self> {
        match self {
            Pool::Transparent => None,
            Pool::Sapling => Some(Pool::Orchard),
            Pool::Orchard => Some(Pool::Sapling),
        }
    }
}

#[cfg(test)]
mod tests {
}
