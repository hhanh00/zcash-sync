use zcash_primitives::memo::Memo;
use crate::{CoinConfig, init_coin, set_coin_lwd_url};
use crate::api::payment::RecipientMemo;
use crate::unified::UnifiedAddressType;
use super::{*, types::*, fill::*};

// has T+S receivers
const CHANGE_ADDRESS: &str = "u1utw7uds5fr5ephnrm8u57ytwgpcktdmvv3q0kmfapnhyy4g7n7wqce6d8xfqeewgd9td9a57qera92apjtljs543j5yl2kks0rpaf3y4hvsmpep5ajvd2s4ggc9dxjtek8s4x92j6p9";

fn init() {
    env_logger::init();
    init_coin(0, "./zec.db").unwrap();
    set_coin_lwd_url(0, "https://lwdv3.zecwallet.co:443");
}

#[tokio::test]
async fn test_fetch_utxo() {
    init();
    let utxos = fetch_utxos(0, 1, 1900000, true, 3).await.unwrap();

    for utxo in utxos.iter() {
        log::info!("{:?}", utxo);
    }
}

#[test]
fn test_fill1() {
    let mut config = NoteSelectConfig {
        privacy_policy: PrivacyPolicy::AnyPool,
        use_transparent: true,
        precedence: [Pool::Transparent, Pool::Sapling, Pool::Orchard],
        change_address: CHANGE_ADDRESS.to_string(),
    };

    // T2T
    let selection = execute_orders(
        &mut vec![
            mock_order(1, 100, 1), // taddr
        ],
        &PoolAllocation([100, 0, 0]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[0], 100);

    config.use_transparent = false; // disable transparent inputs
    let mut orders = vec![
        mock_order(1, 100, 1), // taddr
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([100, 0, 0]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[0], 0);
    assert_eq!(orders[0].filled, 0); // no fill

    // add sapling inputs: S2T
    let mut orders = vec![
        mock_order(1, 100, 1), // taddr
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([100, 80, 0]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[1], 80);
    assert_eq!(orders[0].filled, 80); // partial fill

    // add orchard inputs: S2T + O2T
    let mut orders = vec![
        mock_order(1, 100, 1), // taddr
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([100, 80, 40]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[1], 80);
    assert_eq!(selection.allocation.0[2], 20);
    assert_eq!(orders[0].filled, 100); // fill

    // Orchard pool preference: O2T + S2T
    config.precedence = [Pool::Transparent, Pool::Orchard, Pool::Sapling];
    let mut orders = vec![
        mock_order(1, 100, 1), // taddr
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([100, 80, 40]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[1], 60);
    assert_eq!(selection.allocation.0[2], 40);
    assert_eq!(orders[0].filled, 100); // fill

    // UA T+S: T2T + S2S + O2S
    config.use_transparent = true;
    let mut orders = vec![
        mock_order(1, 100, 3), // ua: t+s
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([10, 80, 40]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[0], 10);
    assert_eq!(selection.allocation.0[1], 80);
    assert_eq!(selection.allocation.0[2], 10);
    assert_eq!(orders[0].filled, 100); // fill

    // UA T+S, UA S+O: T2T + S2S + O2O - pool is empty
    let mut orders = vec![
        mock_order(1, 100, 3), // ua: t+s
        mock_order(2, 100, 6), // ua: s+o
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([10, 80, 40]),
        &config,
    ).unwrap();
    assert_eq!(selection.allocation.0[0], 10);
    assert_eq!(selection.allocation.0[1], 80);
    assert_eq!(selection.allocation.0[2], 40);
    assert_eq!(orders[0].filled, 90); // partial fill
    assert_eq!(orders[1].filled, 40); // partial fill

    // Same as previously with more O inputs
    let mut orders = vec![
        mock_order(1, 100, 3), // ua: t+s
        mock_order(2, 100, 6), // ua: s+o
    ];
    let selection = execute_orders(
        &mut orders,
        &PoolAllocation([10, 80, 120]),
        &config,
    ).unwrap();
    println!("{:?}", selection);
    assert_eq!(selection.allocation.0[0], 10);
    assert_eq!(selection.allocation.0[1], 80);
    assert_eq!(selection.allocation.0[2], 110);
    assert_eq!(orders[0].filled, 100); // fill
    assert_eq!(orders[1].filled, 100); // fill
}

#[test]
fn test_ua() {
    init();
    let c = CoinConfig::get(0);
    let db = c.db().unwrap();
    let address = crate::get_unified_address(c.chain.network(), &db, 1,
                                             Some(UnifiedAddressType { transparent: true, sapling: true, orchard: false })).unwrap(); // use ua settings from db
    println!("{}", address);
}

#[tokio::test]
async fn test_payment() {
    init();
    let config = NoteSelectConfig {
        privacy_policy: PrivacyPolicy::SamePoolTypeOnly, // Minimum privacy level required for this tx, because the change has to go from orchard to sapling
        use_transparent: true,
        precedence: [Pool::Transparent, Pool::Sapling, Pool::Orchard],
        change_address: CHANGE_ADDRESS.to_string(), // does not have orchard receiver
    };

    let recipients = vec![
        RecipientMemo {
            // has T+S+O receivers
            address: "u1pncsxa8jt7aq37r8uvhjrgt7sv8a665hdw44rqa28cd9t6qqmktzwktw772nlle6skkkxwmtzxaan3slntqev03g70tzpky3c58hfgvfjkcky255cwqgfuzdjcktfl7pjalt5sl33se75pmga09etn9dplr98eq2g8cgmvgvx6jx2a2xhy39x96c6rumvlyt35whml87r064qdzw30e".to_string(),
            amount: 89000,
            memo: Memo::Empty.into(),
            max_amount_per_note: 0,
        }
    ];
    let tx_plan = prepare_multi_payment(0, 1, 1900000,
                                        &recipients, &config, 3,
    ).await.unwrap();

    let tx_json = serde_json::to_string(&tx_plan).unwrap();
    println!("{}", tx_json);

    // expected: o2o because the recipient ua has an orchard receiver
    assert_eq!(tx_plan.outputs[0].destination.pool(), Pool::Orchard);
    assert_eq!(tx_plan.outputs[0].amount, 89000);
    // fee = 10000 per zip-317
    assert_eq!(tx_plan.outputs[1].amount, 10000);
    assert_eq!(tx_plan.outputs[1].is_fee, true);
    // change has to cross pools because the change address does not receive orchard
    assert_eq!(tx_plan.outputs[2].destination.pool(), Pool::Sapling);
    assert_eq!(tx_plan.outputs[2].amount, 1000);
}

fn mock_order(id: u32, amount: u64, tpe: u8) -> Order {
    assert!(tpe > 0 && tpe < 8);
    let mut destinations = [None; 3];
    if tpe & 1 != 0 {
        destinations[0] = Some(Destination::Transparent([0u8; 20]));
    }
    if tpe & 2 != 0 {
        destinations[1] = Some(Destination::Sapling([0u8; 43]));
    }
    if tpe & 4 != 0 {
        destinations[2] = Some(Destination::Orchard([0u8; 43]));
    }
    Order {
        id,
        destinations,
        amount,
        memo: MemoBytes::empty(),
        no_fee: false,
        filled: 0,
    }
}

