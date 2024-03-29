use super::{types::*, Result};
use crate::note_selection::fee::FeeCalculator;
use crate::note_selection::ua::decode;
use crate::note_selection::TransactionBuilderError::TxTooComplex;
use crate::note_selection::{TransactionBuilderError, MAX_ATTEMPTS};
use crate::Hash;
use zcash_primitives::consensus::Network;
use zcash_primitives::memo::MemoBytes;

pub fn sum_utxos(utxos: &[UTXO]) -> Result<PoolAllocation> {
    let mut pool = PoolAllocation::default();
    for utxo in utxos {
        match utxo.source {
            Source::Transparent { .. } => {
                pool.0[0] += utxo.amount;
            }
            Source::Sapling { .. } => {
                pool.0[1] += utxo.amount;
            }
            Source::Orchard { .. } => {
                pool.0[2] += utxo.amount;
            }
        }
    }
    Ok(pool)
}

pub fn group_orders(orders: &[Order], fee: u64) -> Result<(Vec<OrderInfo>, OrderGroupAmounts)> {
    let mut order_info = vec![];
    for order in orders {
        let mut group_type = 0;
        for i in 0..3 {
            if order.destinations[i].is_some() {
                group_type |= 1 << i;
            }
        }
        let amount = order.amount(fee)?;
        order_info.push(OrderInfo { group_type, amount });
    }

    let mut t0 = 0u64;
    let mut s0 = 0u64;
    let mut o0 = 0u64;
    let mut x = 0u64;
    for info in order_info.iter_mut() {
        if info.group_type != 1 {
            info.group_type &= 6; // unselect transparent outputs except for t-addr
        }
        match info.group_type {
            1 => {
                t0 += info.amount;
            }
            2 => {
                s0 += info.amount;
            }
            4 => {
                o0 += info.amount;
            }
            6 => {
                x += info.amount;
            }
            _ => unreachable!(),
        }
    }
    log::debug!("{} {} {} {}", t0, s0, o0, x);
    let amounts = OrderGroupAmounts { t0, s0, o0, x, fee };
    Ok((order_info, amounts))
}

fn get_net_chg(t0: i64, s0: i64, o0: i64, x: i64, t2: i64, fee: i64) -> (i64, i64) {
    let (d_s, d_o) = match (x, s0, o0) {
        (0, 0, _) => (0, t0 + fee - t2), // only orchard
        (0, _, 0) => (t0 + fee - t2, 0), // only sapling
        _ => ((t0 - t2 + fee) / 2, t0 + fee - t2 - (t0 - t2 + fee) / 2), // do not simplify because of rounding
    };
    log::info!("{} {}", d_s, d_o);
    (d_s, d_o)
}

pub fn allocate_funds(
    amounts: &OrderGroupAmounts,
    initial: &PoolAllocation,
) -> Result<FundAllocation> {
    log::debug!("{:?}", initial);

    let OrderGroupAmounts { t0, s0, o0, x, fee } = *amounts;
    let (t0, s0, o0, x, fee) = (t0 as i64, s0 as i64, o0 as i64, x as i64, fee as i64);

    let sum = t0 + s0 + o0 + x + fee;
    let tmax = initial.0[0] as i64;
    let smax = initial.0[1] as i64;
    let omax = initial.0[2] as i64;

    let mut s1;
    let mut o1;
    let mut s2;
    let mut o2;
    let mut t2 = sum - smax - omax;
    if t2 > 0 {
        if t2 > tmax {
            return Err(TransactionBuilderError::NotEnoughFunds((t2 - tmax) as u64));
        }
        // Not enough shielded notes. Use them all before using transparent notes
        s2 = smax;
        o2 = omax;
        let (d_s, d_o) = get_net_chg(t0, s0, o0, x, t2, fee);
        s1 = s2 - d_s - s0;
        o1 = o2 - d_o - o0;
    } else {
        t2 = 0;
        let (d_s, d_o) = get_net_chg(t0, s0, o0, x, t2, fee);

        // Solve relaxed problem
        let inp = sum / 2;
        s2 = inp;
        o2 = sum - inp;
        s1 = s2 - d_s - s0;
        o1 = o2 - d_o - o0;

        // Check solution validity
        if s1 < 0 {
            s1 = 0;
            o1 = x;
            s2 = s0 + d_s;
            o2 = o0 + d_o + x;
        } else if o1 < 0 {
            o1 = 0;
            s1 = x;
            o2 = o0 + d_o;
            s2 = s0 + d_s + x;
        }

        assert!(s2 >= 0);
        assert!(o2 >= 0);

        // Check account balances

        if s2 > smax {
            s2 = smax;
            o2 = sum - s2;
            s1 = s2 - d_s - s0;
            o1 = x - s1;
        }
        if o2 > omax {
            o2 = omax;
            s2 = sum - o2;
            o1 = o2 - d_o - o0;
            s1 = x - o1;
        }
    }

    if s1 < 0 {
        s1 = 0;
        o1 = x;
    } else if o1 < 0 {
        o1 = 0;
        s1 = x;
    }

    assert!(s1 >= 0);
    assert!(o1 >= 0);
    assert!(t2 >= 0);
    assert!(s2 >= 0);
    assert!(o2 >= 0);
    assert!(t2 <= tmax);
    assert!(s2 <= smax);
    assert!(o2 <= omax);

    assert_eq!(sum, t2 + s2 + o2);
    assert_eq!(x, s1 + o1);

    log::debug!("{} {}", s1, o1);
    log::debug!("{} {} {}", t2, s2, o2);

    let fund_allocation = FundAllocation {
        s1: s1 as u64,
        o1: o1 as u64,
        t2: t2 as u64,
        s2: s2 as u64,
        o2: o2 as u64,
    };
    Ok(fund_allocation)
}

pub fn fill(
    orders: &[Order],
    order_infos: &[OrderInfo],
    amounts: &OrderGroupAmounts,
    allocation: &FundAllocation,
    fee: u64,
) -> Result<Vec<Fill>> {
    assert_eq!(orders.len(), order_infos.len());
    let mut fills = vec![];
    let mut f = 0f64;
    if amounts.x != 0 {
        f = allocation.s1 as f64 / amounts.x as f64;
    }
    for (order, info) in orders.iter().zip(order_infos) {
        match info.group_type {
            1 | 2 | 4 => {
                let fill = Fill {
                    id_order: Some(order.id),
                    destination: order.destinations[ilog2(info.group_type)]
                        .as_ref()
                        .unwrap()
                        .clone(),
                    amount: order.amount(fee)?,
                    memo: order.memo.clone(),
                };
                fills.push(fill);
            }
            6 => {
                let fill1 = Fill {
                    id_order: Some(order.id),
                    destination: order.destinations[1].as_ref().unwrap().clone(),
                    amount: (order.amount(fee)? as f64 * f).round() as u64,
                    memo: order.memo.clone(),
                };
                let fill2 = Fill {
                    id_order: Some(order.id),
                    destination: order.destinations[2].as_ref().unwrap().clone(),
                    amount: order.amount(fee)? - fill1.amount,
                    memo: order.memo.clone(),
                };
                if fill1.amount != 0 {
                    fills.push(fill1);
                }
                if fill2.amount != 0 {
                    fills.push(fill2);
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(fills)
}

pub fn select_inputs(
    utxos: &[UTXO],
    allocation: &FundAllocation,
) -> Result<(Vec<UTXO>, PoolAllocation)> {
    let mut needed = [allocation.t2, allocation.s2, allocation.o2];
    let mut change = [0u64; 3];
    let mut inputs = vec![];
    for utxo in utxos {
        let idx = match utxo.source {
            Source::Transparent { .. } => 0,
            Source::Sapling { .. } => 1,
            Source::Orchard { .. } => 2,
        };
        if needed[idx] > 0 {
            let available = utxo.amount;
            let a = available.min(needed[idx]);
            inputs.push(utxo.clone());
            needed[idx] -= a;
            change[idx] += available - a;
        }
    }

    Ok((inputs, PoolAllocation(change)))
}

pub fn outputs_for_change(
    change_destinations: &[Option<Destination>],
    change: &PoolAllocation,
) -> Result<Vec<Fill>> {
    let mut change_fills = vec![];
    for i in 0..3 {
        let destination = &change_destinations[i];
        match destination {
            Some(destination) => {
                let change_fill = Fill {
                    id_order: None,
                    destination: destination.clone(),
                    amount: change.0[i],
                    memo: MemoBytes::empty(),
                };
                if change_fill.amount != 0 {
                    change_fills.push(change_fill);
                }
            }
            None if change.0[i] == 0 => {}
            None => {
                return Err(anyhow::anyhow!("No change address").into());
            }
        }
    }
    Ok(change_fills)
}

pub fn build_tx_plan<F: FeeCalculator>(
    network: &Network,
    account_address: &str,
    anchor_height: u32,
    expiry_height: u32,
    orchard_anchor: &Option<Hash>,
    utxos: &[UTXO],
    orders: &[Order],
    config: &TransactionBuilderConfig,
) -> Result<TransactionPlan> {
    let mut fee = 0;

    for _ in 0..MAX_ATTEMPTS {
        let balances = sum_utxos(utxos)?;
        let (groups, amounts) = group_orders(&orders, fee)?;
        let allocation = allocate_funds(&amounts, &balances)?;

        let OrderGroupAmounts { s0, o0, .. } = amounts;
        let FundAllocation { s1, o1, s2, o2, .. } = allocation;
        let (s0, o0, s1, o1, s2, o2) = (
            s0 as i64, o0 as i64, s1 as i64, o1 as i64, s2 as i64, o2 as i64,
        );
        let net_chg = [s0 + s1 - s2, o0 + o1 - o2];

        let mut fills = fill(&orders, &groups, &amounts, &allocation, fee)?;

        let (notes, change) = select_inputs(&utxos, &allocation)?;
        let change_destinations = decode(network, &config.change_address)?;
        let change_outputs = outputs_for_change(&change_destinations, &change)?;
        fills.extend(change_outputs);

        let updated_fee = F::calculate_fee(&notes, &fills);
        if updated_fee == fee {
            let tx_plan = TransactionPlan {
                account_address: account_address.to_string(),
                anchor_height,
                expiry_height,
                orchard_anchor: orchard_anchor.unwrap_or(Hash::default()),
                spends: notes,
                outputs: fills,
                net_chg,
                fee,
            };
            return Ok(tx_plan);
        }
        fee = updated_fee;
    }
    Err(TxTooComplex)
}

fn ilog2(u: usize) -> usize {
    match u {
        1 => 0,
        2 => 1,
        4 => 2,
        _ => unreachable!(),
    }
}
