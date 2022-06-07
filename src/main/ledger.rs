use sync::sweep_ledger;

#[tokio::main]
async fn main() {
    sweep_ledger().await.unwrap();
}