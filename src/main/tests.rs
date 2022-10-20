use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use tonic::Request;
use prost::Message;
use zcash_client_backend::encoding::{decode_extended_full_viewing_key, decode_extended_spending_key, encode_extended_full_viewing_key, encode_payment_address};
use zcash_primitives::consensus::{Network, NetworkUpgrade, Parameters};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use warp_api_ffi::{BlockId, BlockRange, ChainSpec, COIN_CONFIG, CoinConfig, CompactBlock, connect_lightwalletd, DbAdapter, DbAdapterBuilder, derive_zip32, init_coin};
use warp_api_ffi::sapling::{DecryptedSaplingNote, SaplingDecrypter, SaplingHasher, SaplingViewKey};
use warp_api_ffi::sync::{WarpProcessor, Synchronizer, CTree};

type SaplingSynchronizer = Synchronizer<Network, SaplingDomain<Network>, SaplingViewKey, DecryptedSaplingNote,
    SaplingDecrypter<Network>, SaplingHasher>;

#[allow(dead_code)]
async fn write_block_file() {
    init_coin(1, "yec-new.db").unwrap();
    let coin = COIN_CONFIG[1].lock().unwrap();
    let mut client = connect_lightwalletd("https://lite.ycash.xyz:9067").await.unwrap();
    let network = coin.chain.network();
    let start = u32::from(network.activation_height(NetworkUpgrade::Sapling).unwrap()) + 1;
    let end = client.get_latest_block(Request::new(ChainSpec {})).await.unwrap().into_inner();
    let end = end.height as u32;

    let mut blocks = client.get_block_range(Request::new(BlockRange {
        start: Some(BlockId { height: start as u64, hash: vec![] }),
        end: Some(BlockId { height: end as u64, hash: vec![] }),
        spam_filter_threshold: 0
    })).await.unwrap().into_inner();

    let file = File::create("ycash.bin").unwrap();
    let mut writer = BufWriter::new(file);
    while let Some(block) = blocks.message().await.unwrap() {
        println!("{}", block.height);
        let mut buf = prost::bytes::BytesMut::new();
        block.encode(&mut buf).unwrap();
        writer.write_u32::<LE>(buf.len() as u32).unwrap();
        writer.write_all(&buf).unwrap();
    }
}

fn read_block_file(coin: &CoinConfig, fvk: ExtendedFullViewingKey) {
    let network = coin.chain.network();
    let file = File::open("/home/hanh/ycash.bin").unwrap();
    let mut reader = BufReader::new(file);

    let db_builder = DbAdapterBuilder { coin_type: coin.coin_type, db_path: coin.db_path.as_ref().unwrap().to_owned() };
    let mut synchronizer = SaplingSynchronizer {
        decrypter: SaplingDecrypter::new(*network),
        warper: WarpProcessor::new(SaplingHasher::default()),
        vks: vec![SaplingViewKey {
            account: 1,
            fvk: fvk.clone(),
            ivk: fvk.fvk.vk.ivk()
        }],
        tree: CTree::new(),
        witnesses: vec![],

        db: db_builder.clone(),
        shielded_pool: "sapling".to_string(),

        note_position: 0,
        nullifiers: HashMap::new(),
        _phantom: Default::default()
    };

    synchronizer.initialize().unwrap();

    let mut blocks = vec![];
    let mut height = 0;
    let mut hash = [0u8; 32];
    let mut time = 0;
    while let Ok(len) = reader.read_u32::<LE>() {
        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf).unwrap();
        let cb: CompactBlock = CompactBlock::decode(&*buf).unwrap();
        height = cb.height;
        hash.copy_from_slice(&cb.hash);
        time = cb.time;
        blocks.push(cb);
        if height % 100_000 == 0 {
            synchronizer.process(blocks).unwrap();
            blocks = vec![];
        }
    }
    synchronizer.process(blocks).unwrap();
    let db = db_builder.build().unwrap();
    DbAdapter::store_block2(height as u32, &hash, time, &synchronizer.tree, None, &db.connection).unwrap();
}

#[tokio::main]
async fn main() {
    env_logger::init();
    init_coin(1, "yec-new.db").unwrap();
    let coin = COIN_CONFIG[1].lock().unwrap();
    let network = coin.chain.network();
    let _ = dotenv::dotenv();
    let seed_str = dotenv::var("SEED").unwrap();
    let kp = derive_zip32(&network, &seed_str, 0, 0, None).unwrap();
    let zk = kp.z_key.clone();
    let sk = decode_extended_spending_key(network.hrp_sapling_extended_spending_key(), &zk).unwrap().unwrap();

    let fvk = ExtendedFullViewingKey::from(&sk);
    let fvk_str = encode_extended_full_viewing_key(network.hrp_sapling_extended_full_viewing_key(), &fvk);
    let (_, pa) = fvk.default_address();
    let address = encode_payment_address(network.hrp_sapling_payment_address(), &pa);
    let db_builder = DbAdapterBuilder { coin_type: coin.coin_type, db_path: coin.db_path.as_ref().unwrap().to_owned() };
    let db = db_builder.build().unwrap();
    db.store_account("test", Some(&seed_str), 0, Some(&zk), &fvk_str, &address).unwrap();

    // write_block_file().await;
    read_block_file(&coin, fvk);
}
