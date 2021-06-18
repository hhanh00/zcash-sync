use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use sync::{NETWORK, scan_all};
use zcash_primitives::consensus::Parameters;

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    env_logger::init();

    let ivk = dotenv::var("IVK").unwrap();
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)
            .unwrap()
            .unwrap();
    let ivk = fvk.fvk.vk.ivk();

    scan_all(&vec![ivk]).await.unwrap();
}
