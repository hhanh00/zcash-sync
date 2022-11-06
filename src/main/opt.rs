use thiserror::Error;
use warp_api_ffi::{fetch_utxos, init_test, note_select_with_fee_v2, TransactionBuilderConfig, TransactionPlan};

#[tokio::main]
async fn main() {
    init_test();

    let config = TransactionBuilderConfig::new();

    let utxos = fetch_utxos(0, 1, 220, true, 0).await.unwrap();
    let mut orders = vec![];

    note_select_with_fee_v2("", 0, &utxos, &mut orders, &config).unwrap();
}


