use crate::ledger::{APDUReply, APDURequest};
use crate::taddr::{get_taddr_balance, get_utxos};
use crate::{connect_lightwalletd, GetAddressUtxosArg, LWD_URL};
use anyhow::Result;
use byteorder::{BigEndian as BE, LittleEndian as LE, ReadBytesExt, WriteBytesExt};
use ledger_apdu::{APDUAnswer, APDUCommand};
use ledger_transport_hid::hidapi::HidApi;
use ledger_transport_hid::TransportNativeHID;
use ripemd::Digest;
use ripemd::Ripemd160;
use secp256k1::PublicKey;
use sha2::Sha256;
use std::io::{Read, Write};
use std::str::from_utf8;
use tonic::Request;
use zcash_client_backend::encoding::encode_transparent_address;
use zcash_primitives::consensus::Network::MainNetwork;
use zcash_primitives::consensus::Parameters;
use zcash_primitives::legacy::TransparentAddress;

const HARDENED: u32 = 0x80000000;

pub async fn sweep_ledger() -> Result<()> {
    let api = HidApi::new()?;
    let device = TransportNativeHID::list_ledgers(&api).next().unwrap();
    let transport = TransportNativeHID::open_device(&api, &device).unwrap();

    let mut data = vec![];
    data.write_u8(5)?;
    data.write_u32::<BE>(44 | HARDENED)?;
    data.write_u32::<BE>(MainNetwork.coin_type() | HARDENED)?;
    data.write_u32::<BE>(HARDENED)?;
    data.write_u32::<BE>(0x0)?;
    data.write_u32::<BE>(0x0)?;

    let res = transport
        .exchange(&APDUCommand {
            cla: 0xE0,
            ins: 0x40,
            p1: 0,
            p2: 0,
            data: data.as_slice(),
        })
        .unwrap();
    println!("{}", res.retcode());
    let mut data = res.apdu_data();
    println!("{}", hex::encode(&data));
    let len = data.read_u8()?;
    let mut pk = vec![0u8; len as usize];
    data.read_exact(&mut pk).unwrap();
    println!("{}", hex::encode(&pk));
    let pub_key = PublicKey::from_slice(&pk).unwrap();
    let pub_key = pub_key.serialize();
    let pub_key = Ripemd160::digest(&Sha256::digest(&pub_key));
    let pub_key_hash: [u8; 20] = pub_key.into();
    let address = TransparentAddress::PublicKey(pub_key_hash.clone());
    let address = encode_transparent_address(
        &MainNetwork.b58_pubkey_address_prefix(),
        &MainNetwork.b58_script_address_prefix(),
        &address,
    );

    println!("{}", address);

    let mut client = connect_lightwalletd(LWD_URL).await?;
    let balance = get_taddr_balance(&mut client, &address).await.unwrap();

    println!("{}", balance);

    let req = GetAddressUtxosArg {
        addresses: vec![address.to_string()],
        start_height: 0,
        max_entries: 0,
    };
    let utxo_rep = client
        .get_address_utxos(Request::new(req))
        .await?
        .into_inner();
    let mut first = true;
    for utxo_reply in utxo_rep.address_utxos.iter() {
        let mut data = vec![];
        data.write_u8(0)?;
        data.write_all(&utxo_reply.txid)?;
        data.write_u32::<LE>(utxo_reply.index as u32)?;

        let res = transport
            .exchange(&APDUCommand {
                cla: 0xE0,
                ins: 0x40,
                p1: if first { 0 } else { 0x80 },
                p2: 0,
                data: data.as_slice(),
            })
            .unwrap();
        first = false;
    }

    let data = [0u8];
    let res = transport
        .exchange(&APDUCommand {
            cla: 0xE0,
            ins: 0x4A,
            p1: 0xFF,
            p2: 0,
            data: data.as_slice(),
        })
        .unwrap();
    println!("{}", res.retcode());

    let mut data = vec![];
    data.write_u8(1)?;
    data.write_u64::<LE>(balance)?;
    data.write_u8(25)?;
    data.write_u8(0x76)?;
    data.write_u8(0xa9)?;
    data.write_u8(0x14)?;
    data.write_all(&pub_key_hash)?;
    data.write_u8(0x88)?;
    data.write_u8(0xac)?;
    let res = transport
        .exchange(&APDUCommand {
            cla: 0xE0,
            ins: 0x4A,
            p1: 0x80,
            p2: 0,
            data: data.as_slice(),
        })
        .unwrap();
    println!("{}", res.retcode());

    Ok(())
}
