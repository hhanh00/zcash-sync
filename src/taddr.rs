use crate::chain::send_transaction;
use crate::{
    connect_lightwalletd, get_branch, get_latest_height, AddressList, CompactTxStreamerClient,
    DbAdapter, GetAddressUtxosArg, NETWORK,
};
use anyhow::Context;
use bip39::{Language, Mnemonic, Seed};
use ripemd160::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use std::str::FromStr;
use tiny_hderive::bip32::ExtendedPrivKey;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::{
    decode_extended_full_viewing_key, decode_payment_address, encode_transparent_address,
};
use zcash_primitives::consensus::{BlockHeight, Parameters};
use zcash_primitives::legacy::{Script, TransparentAddress};
use zcash_primitives::transaction::builder::Builder;
use zcash_primitives::transaction::components::amount::DEFAULT_FEE;
use zcash_primitives::transaction::components::{Amount, OutPoint, TxOut};
use zcash_proofs::prover::LocalTxProver;

pub const BIP44_PATH: &str = "m/44'/133'/0'/0/0";

pub async fn get_taddr_balance(
    client: &mut CompactTxStreamerClient<Channel>,
    address: &str,
) -> anyhow::Result<u64> {
    let req = AddressList {
        addresses: vec![address.to_string()],
    };
    let rep = client
        .get_taddress_balance(Request::new(req))
        .await?
        .into_inner();
    Ok(rep.value_zat as u64)
}

pub async fn shield_taddr(
    db: &DbAdapter,
    account: u32,
    prover: &LocalTxProver,
    ld_url: &str,
) -> anyhow::Result<String> {
    let mut client = connect_lightwalletd(ld_url).await?;
    let last_height = get_latest_height(&mut client).await?;
    let ivk = db.get_ivk(account)?;
    let fvk =
        decode_extended_full_viewing_key(NETWORK.hrp_sapling_extended_full_viewing_key(), &ivk)?
            .unwrap();
    let z_address = db.get_address(account)?;
    let pa = decode_payment_address(NETWORK.hrp_sapling_payment_address(), &z_address)?.unwrap();
    let t_address = db.get_taddr(account)?;
    if t_address.is_none() {
        anyhow::bail!("No transparent address");
    }
    let t_address = t_address.unwrap();
    let mut builder = Builder::new(NETWORK, BlockHeight::from_u32(last_height));
    let amount = Amount::from_u64(get_taddr_balance(&mut client, &t_address).await?).unwrap();
    if amount <= DEFAULT_FEE {
        anyhow::bail!("Not enough balance");
    }
    let amount = amount - DEFAULT_FEE;

    let sk = db.get_tsk(account)?;
    let seckey = secp256k1::SecretKey::from_str(&sk).context("Cannot parse secret key")?;

    let req = GetAddressUtxosArg {
        addresses: vec![t_address.to_string()],
        start_height: 0,
        max_entries: 0,
    };
    let utxo_rep = client
        .get_address_utxos(Request::new(req))
        .await?
        .into_inner();

    for utxo in utxo_rep.address_utxos.iter() {
        let mut tx_hash = [0u8; 32];
        tx_hash.copy_from_slice(&utxo.txid);
        let op = OutPoint::new(tx_hash, utxo.index as u32);
        let script = Script(utxo.script.clone());
        let txout = TxOut {
            value: Amount::from_i64(utxo.value_zat).unwrap(),
            script_pubkey: script,
        };
        builder.add_transparent_input(seckey, op, txout)?;
    }

    let ovk = fvk.fvk.ovk;
    builder.add_sapling_output(Some(ovk), pa, amount, None)?;
    let consensus_branch_id = get_branch(last_height);
    let (tx, _) = builder.build(consensus_branch_id, prover)?;
    let mut raw_tx: Vec<u8> = vec![];
    tx.write(&mut raw_tx)?;

    let tx_id = send_transaction(&mut client, &raw_tx, last_height).await?;
    log::info!("Tx ID = {}", tx_id);

    Ok(tx_id)
}

pub fn derive_tkeys(phrase: &str, path: &str) -> anyhow::Result<(String, String)> {
    let mnemonic = Mnemonic::from_phrase(&phrase, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let secp = Secp256k1::<All>::new();
    let ext = ExtendedPrivKey::derive(&seed.as_bytes(), path).unwrap();
    let secret_key = SecretKey::from_slice(&ext.secret()).unwrap();
    let pub_key = PublicKey::from_secret_key(&secp, &secret_key);
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let address = TransparentAddress::PublicKey(pub_key.into());
    let address = encode_transparent_address(
        &NETWORK.b58_pubkey_address_prefix(),
        &NETWORK.b58_script_address_prefix(),
        &address,
    );
    let sk = secret_key.to_string();
    Ok((sk, address))
}

#[cfg(test)]
mod tests {
    use crate::db::DEFAULT_DB_PATH;
    use crate::taddr::{derive_tkeys, shield_taddr};
    use crate::{DbAdapter, LWD_URL};
    use zcash_proofs::prover::LocalTxProver;

    #[tokio::test]
    async fn test_shield_addr() {
        let prover = LocalTxProver::with_default_location().unwrap();
        let db = DbAdapter::new(DEFAULT_DB_PATH).unwrap();
        let txid = shield_taddr(&db, 1, &prover, LWD_URL).await.unwrap();
        println!("{}", txid);
    }

    #[test]
    fn test_derive() {
        let seed = dotenv::var("SEED").unwrap();
        for i in 0..10 {
            let (_sk, addr) = derive_tkeys(&seed, &format!("m/44'/133'/0'/0/{}", i)).unwrap();
            println!("{}", addr);
        }
    }
}
