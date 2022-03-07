use crate::{
    AddressList, CompactTxStreamerClient, DbAdapter, GetAddressUtxosArg, GetAddressUtxosReply,
};
use bip39::{Language, Mnemonic, Seed};
use ripemd160::{Digest, Ripemd160};
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use sha2::Sha256;
use tiny_hderive::bip32::ExtendedPrivKey;
use tonic::transport::Channel;
use tonic::Request;
use zcash_client_backend::encoding::encode_transparent_address;
use zcash_primitives::consensus::{Network, Parameters};
use zcash_primitives::legacy::TransparentAddress;

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

pub async fn get_utxos(
    client: &mut CompactTxStreamerClient<Channel>,
    db: &DbAdapter,
    account: u32,
) -> anyhow::Result<Vec<GetAddressUtxosReply>> {
    let t_address = db.get_taddr(account)?;
    if let Some(t_address) = t_address {
        let req = GetAddressUtxosArg {
            addresses: vec![t_address.to_string()],
            start_height: 0,
            max_entries: 0,
        };
        let utxo_rep = client
            .get_address_utxos(Request::new(req))
            .await?
            .into_inner();
        Ok(utxo_rep.address_utxos)
    } else {
        Ok(vec![])
    }
}

pub fn derive_tkeys(network: &Network, phrase: &str, path: &str) -> anyhow::Result<(String, String)> {
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
        &network.b58_pubkey_address_prefix(),
        &network.b58_script_address_prefix(),
        &address,
    );
    let sk = secret_key.to_string();
    Ok((sk, address))
}
