use crate::coinconfig::CoinConfig;
use crate::{
    AddressList, CompactTxStreamerClient, DbAdapter, GetAddressUtxosArg, GetAddressUtxosReply,
};
use bip39::{Language, Mnemonic, Seed};
use ripemd::{Digest, Ripemd160};
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
    t_address: &str,
    _account: u32,
) -> anyhow::Result<Vec<GetAddressUtxosReply>> {
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
}

pub async fn scan_transparent_accounts(
    network: &Network,
    client: &mut CompactTxStreamerClient<Channel>,
    gap_limit: usize,
) -> anyhow::Result<()> {
    let c = CoinConfig::get_active();
    let mut addresses = vec![];
    let db = c.db()?;
    let (seed, mut index) = db.get_seed(c.id_account)?;
    if let Some(seed) = seed {
        let mut gap = 0;
        while gap < gap_limit {
            let bip44_path = format!("m/44'/{}'/0'/0/{}", network.coin_type(), index);
            log::info!("{} {}", index, bip44_path);
            let (_, address) = derive_tkeys(network, &seed, &bip44_path)?;
            let balance = get_taddr_balance(client, &address).await?;
            if balance > 0 {
                addresses.push(TBalance {
                    index,
                    address,
                    balance,
                });
                gap = 0;
            } else {
                gap += 1;
            }
            index += 1;
        }
    }
    db.store_t_scan(&addresses)?;
    Ok(())
}

pub fn derive_tkeys(
    network: &Network,
    phrase: &str,
    path: &str,
) -> anyhow::Result<(String, String)> {
    let mnemonic = Mnemonic::from_phrase(phrase, Language::English)?;
    let seed = Seed::new(&mnemonic, "");
    let secp = Secp256k1::<All>::new();
    let ext = ExtendedPrivKey::derive(seed.as_bytes(), path).unwrap();
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
    let sk = secret_key.display_secret().to_string();
    Ok((sk, address))
}

pub struct TBalance {
    pub index: u32,
    pub address: String,
    pub balance: u64,
}
