use clap::Clap;
use sync::{decode_key, Tx, sign_offline_tx, NETWORK};
use std::fs::File;
use std::io::{Read, Write};
use zcash_client_backend::encoding::decode_extended_spending_key;
use zcash_primitives::consensus::Parameters;

#[derive(Clap, Debug)]
struct SignArgs {
    tx_filename: String,
    out_filename: String,
}

fn main() -> anyhow::Result<()> {
    let key = dotenv::var("KEY").unwrap();
    let (_seed, sk, _ivk, _address) = decode_key(&key)?;

    let opts: SignArgs = SignArgs::parse();
    let sk = sk.unwrap();
    let sk = decode_extended_spending_key(NETWORK.hrp_sapling_extended_spending_key(), &sk)?.unwrap();

    let file_name = opts.tx_filename;
    let mut file = File::open(file_name)?;
    let mut s = String::new();
    file.read_to_string(&mut s).unwrap();
    let tx: Tx = serde_json::from_str(&s)?;
    let raw_tx = sign_offline_tx(&tx, &sk)?;

    let mut out_file = File::create(opts.out_filename)?;
    writeln!(out_file, "{}", hex::encode(&raw_tx))?;
    Ok(())
}
