use serde_json::Value;
use zcash_primitives::memo::Memo;
use crate::{CoinConfig, init_coin, set_coin_lwd_url};
use crate::api::payment::RecipientMemo;
use crate::unified::UnifiedAddressType;
use super::{*, types::*};

// must have T+S+O receivers
const CHANGE_ADDRESS: &str = "u1pncsxa8jt7aq37r8uvhjrgt7sv8a665hdw44rqa28cd9t6qqmktzwktw772nlle6skkkxwmtzxaan3slntqev03g70tzpky3c58hfgvfjkcky255cwqgfuzdjcktfl7pjalt5sl33se75pmga09etn9dplr98eq2g8cgmvgvx6jx2a2xhy39x96c6rumvlyt35whml87r064qdzw30e";
const UA_TSO: &str = "uregtest1mxy5wq2n0xw57nuxa4lqpl358zw4vzyfgadsn5jungttmqcv6nx6cpx465dtpzjzw0vprjle4j4nqqzxtkuzm93regvgg4xce0un5ec6tedquc469zjhtdpkxz04kunqqyasv4rwvcweh3ue0ku0payn29stl2pwcrghyzscrrju9ar57rn36wgz74nmynwcyw27rjd8yk477l97ez8";
const UA_O: &str = "uregtest1mzt5lx5s5u8kczlfr82av97kjckmfjfuq8y9849h6cl9chhdekxsm6r9dklracflqwplrnfzm5rucp5txfdm04z5myrde8y3y5rayev8";

fn init() {
    let _ = env_logger::try_init();
    init_coin(0, "./zec.db").unwrap();
    set_coin_lwd_url(0, "http://127.0.0.1:9067");
}

#[tokio::test]
async fn test_fetch_utxo() {
    init();
    let utxos = fetch_utxos(0, 1, 235, true, 0).await.unwrap();

    for utxo in utxos.iter() {
        log::info!("{:?}", utxo);
    }

    assert_eq!(utxos[0].amount, 624999000);
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
    let config = NoteSelectConfig::new(CHANGE_ADDRESS);

    let recipients = vec![
        RecipientMemo {
            address: UA_O.to_string(),
            amount: 89000,
            memo: Memo::Empty.into(),
            max_amount_per_note: 0,
        }
    ];
    let tx_plan = prepare_multi_payment(0, 1, 205,
                                        &recipients, &config, 3,
    ).await.unwrap();

    let tx_json = serde_json::to_string(&tx_plan).unwrap();
    println!("{}", tx_json);

    // expected: s2o because the recipient ua has only an orchard receiver
    assert_eq!(tx_plan.outputs[0].destination.pool(), Pool::Orchard);
    assert_eq!(tx_plan.outputs[0].amount, 89000);
    assert_eq!(tx_plan.outputs[1].destination.pool(), Pool::Sapling); // change goes back to sapling
    assert_eq!(tx_plan.outputs[1].amount, 624900000);
    // fee = 10000 per zip-317
    assert_eq!(tx_plan.fee, 10000);

    assert_eq!(tx_plan.spends[0].amount, tx_plan.outputs[0].amount + tx_plan.outputs[1].amount + tx_plan.fee);
}

macro_rules! order {
    ($id:expr, $q:expr, $destinations:expr) => {
        Order {
            id: $id,
            amount: $q * 1000,
            destinations: $destinations,
            priority: PoolPriority::OS,
            filled: 0,
            is_fee: false,
            memo: MemoBytes::empty(),
        }
    };
}

macro_rules! utxo {
    ($id:expr, $q:expr) => {
        UTXO {
            amount: $q * 1000,
            source: Source::Transparent { txid: [0u8; 32], index: $id },
        }
    };
}

macro_rules! sapling {
    ($id:expr, $q:expr) => {
        UTXO {
            amount: $q * 1000,
            source: Source::Sapling {
                id_note: $id,
                diversifier: [0u8; 11],
                rseed: [0u8; 32],
                witness: vec![],
            },
        }
    };
}

macro_rules! orchard {
    ($id:expr, $q:expr) => {
        UTXO {
            amount: $q * 1000,
            source: Source::Orchard {
                id_note: $id,
                diversifier: [0u8; 11],
                rseed: [0u8; 32],
                rho: [0u8; 32],
                witness: vec![],
            },
        }
    };
}

macro_rules! t {
    ($id: expr, $q:expr) => {
        order!($id, $q, [Some(Destination::Transparent([0u8; 20])), None, None])
    };
}

macro_rules! s {
    ($id: expr, $q:expr) => {
        order!($id, $q, [None, Some(Destination::Sapling([0u8; 43])), None])
    };
}

macro_rules! o {
    ($id: expr, $q:expr) => {
        order!($id, $q, [None, None, Some(Destination::Orchard([0u8; 43]))])
    };
}

macro_rules! ts {
    ($id: expr, $q:expr) => {
        order!($id, $q, [Some(Destination::Transparent([0u8; 20])), Some(Destination::Sapling([0u8; 43])), None])
    };
}

macro_rules! to {
    ($id: expr, $q:expr) => {
        order!($id, $q, [Some(Destination::Transparent([0u8; 20])), None, Some(Destination::Orchard([0u8; 43]))])
    };
}

macro_rules! so {
    ($id: expr, $q:expr) => {
        order!($id, $q, [None, Some(Destination::Sapling([0u8; 43])), Some(Destination::Orchard([0u8; 43]))])
    };
}

macro_rules! tso {
    ($id: expr, $q:expr) => {
        order!($id, $q, [Some(Destination::Transparent([0u8; 20])), Some(Destination::Sapling([0u8; 43])), Some(Destination::Orchard([0u8; 43]))])
    };
}

#[test]
fn test_example1() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.use_transparent = true;
    config.privacy_policy = PrivacyPolicy::AnyPool;

    let utxos = [utxo!(1, 5), utxo!(2, 7), sapling!(3, 12), orchard!(4, 10)];
    let mut orders = [t!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":1}},"amount":5000},{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":2}},"amount":7000},{"source":{"Sapling":{"id_note":3,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":12000},{"source":{"Orchard":{"id_note":4,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":10000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Orchard":"2b6dca785c846b3752d13150e1c8f197ba9c8ead0a8bee1b3a52df0ad866362941e32d1b69d438b257cf82"},"amount":4000,"memo":[246]}],"fee":20000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

#[test]
fn test_example2() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.privacy_policy = PrivacyPolicy::AnyPool;

    let utxos = [utxo!(1, 5), utxo!(2, 7), sapling!(3, 12), orchard!(4, 10), orchard!(5, 10)];
    let mut orders = [t!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":1}},"amount":5000},{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":2}},"amount":7000},{"source":{"Sapling":{"id_note":3,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":12000},{"source":{"Orchard":{"id_note":4,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":10000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Orchard":"2b6dca785c846b3752d13150e1c8f197ba9c8ead0a8bee1b3a52df0ad866362941e32d1b69d438b257cf82"},"amount":4000,"memo":[246]}],"fee":20000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

#[test]
fn test_example3() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.use_transparent = true;
    config.privacy_policy = PrivacyPolicy::AnyPool;
    config.precedence = [ Pool::Sapling, Pool::Orchard, Pool::Transparent ];

    let utxos = [utxo!(1, 100), sapling!(2, 160), orchard!(3, 70), orchard!(4, 50)];
    let mut orders = [t!(1, 10), s!(2, 20), o!(3, 30), ts!(4, 40), to!(5, 50), so!(6, 60), tso!(7, 70)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":1}},"amount":100000},{"source":{"Sapling":{"id_note":2,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":160000},{"source":{"Orchard":{"id_note":3,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":70000},{"source":{"Orchard":{"id_note":4,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":2,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":20000,"memo":[246]},{"id_order":3,"destination":{"Orchard":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":30000,"memo":[246]},{"id_order":4,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":40000,"memo":[246]},{"id_order":5,"destination":{"Orchard":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":50000,"memo":[246]},{"id_order":6,"destination":{"Orchard":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":40000,"memo":[246]},{"id_order":6,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":20000,"memo":[246]},{"id_order":7,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":70000,"memo":[246]},{"id_order":4294967295,"destination":{"Transparent":"c7b7b3d299bd173ea278d792b1bd5fbdd11afe34"},"amount":55000,"memo":[246]}],"fee":45000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

/// A simple t2t
///
#[test]
fn test_example4() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.use_transparent = true;
    config.use_shielded = false;
    config.privacy_policy = PrivacyPolicy::AnyPool;

    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    let mut orders = [t!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":1}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Transparent":"c7b7b3d299bd173ea278d792b1bd5fbdd11afe34"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

/// A simple z2z
///
#[test]
fn test_example5() {
    let _ = env_logger::try_init();
    let config = NoteSelectConfig::new(CHANGE_ADDRESS);

    // z2z are preferred over t2z, so we can keep the t-notes
    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    let mut orders = [s!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Sapling":{"id_note":2,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Sapling":"9fae6f28c245e095abf8c6730098e110bb67ae3e73302406b2b9c6d6b672ca9e64e14ef0560062a91dd429"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

/// A simple z2z
///
#[test]
fn test_example5b() {
    let _ = env_logger::try_init();
    let config = NoteSelectConfig::new(CHANGE_ADDRESS);

    // z2z are preferred over t2z, so we can keep the t-notes
    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    let mut orders = [o!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Orchard":{"id_note":3,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Orchard":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Orchard":"2b6dca785c846b3752d13150e1c8f197ba9c8ead0a8bee1b3a52df0ad866362941e32d1b69d438b257cf82"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}
 /// A simple z2t sapling
///
#[test]
fn test_example6() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.privacy_policy = PrivacyPolicy::AnyPool;

    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    // Change the destination to t
    let mut orders = [t!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

     let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
     let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Sapling":{"id_note":2,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Sapling":"9fae6f28c245e095abf8c6730098e110bb67ae3e73302406b2b9c6d6b672ca9e64e14ef0560062a91dd429"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
     assert_eq!(tx_plan_json, expected);
 }

/// A simple o2t
///
#[test]
fn test_example7() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.precedence = [ Pool::Orchard, Pool::Sapling, Pool::Transparent ];
    config.privacy_policy = PrivacyPolicy::AnyPool;

    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    // Change the destination to t
    let mut orders = [t!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Orchard":{"id_note":3,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","rho":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Transparent":"0000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Orchard":"2b6dca785c846b3752d13150e1c8f197ba9c8ead0a8bee1b3a52df0ad866362941e32d1b69d438b257cf82"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

/// A simple t2z
///
#[test]
fn test_example8() {
    let _ = env_logger::try_init();
    let mut config = NoteSelectConfig::new(CHANGE_ADDRESS);
    config.privacy_policy = PrivacyPolicy::AnyPool;
    config.use_transparent = true;
    config.use_shielded = false;

    let utxos = [utxo!(1, 50), sapling!(2, 50), orchard!(3, 50)];
    let mut orders = [s!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Transparent":{"txid":"0000000000000000000000000000000000000000000000000000000000000000","index":1}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Sapling":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Transparent":"c7b7b3d299bd173ea278d792b1bd5fbdd11afe34"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}

/// A simple z2z (Sapling/Orchard)
///
#[test]
fn test_example9() {
    let _ = env_logger::try_init();
    let config = NoteSelectConfig::new(CHANGE_ADDRESS);

    let utxos = [utxo!(1, 50), sapling!(2, 50)];
    let mut orders = [o!(1, 10)];

    let tx_plan = note_select_with_fee::<FeeZIP327>(&utxos, &mut orders, &config).unwrap();
    println!("{}", serde_json::to_string(&tx_plan).unwrap());

    let tx_plan_json = serde_json::to_value(&tx_plan).unwrap();
    let expected: Value = serde_json::from_str(r#"{"spends":[{"source":{"Sapling":{"id_note":2,"diversifier":"0000000000000000000000","rseed":"0000000000000000000000000000000000000000000000000000000000000000","witness":""}},"amount":50000}],"outputs":[{"id_order":1,"destination":{"Orchard":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},"amount":10000,"memo":[246]},{"id_order":4294967295,"destination":{"Sapling":"9fae6f28c245e095abf8c6730098e110bb67ae3e73302406b2b9c6d6b672ca9e64e14ef0560062a91dd429"},"amount":30000,"memo":[246]}],"fee":10000}"#).unwrap();
    assert_eq!(tx_plan_json, expected);
}
