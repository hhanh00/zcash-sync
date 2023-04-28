use blake2b_simd::Params;
use byteorder::{WriteBytesExt, LE};
use group::{Group, GroupEncoding};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path, env,
};

use halo2_proofs::pasta::pallas::{self};
use orchard::{
    circuit::{Circuit, ProvingKey},
    keys::{Scope, SpendValidatingKey, SpendingKey, SpendAuthorizingKey, FullViewingKey},
    note::{ExtractedNoteCommitment, Nullifier, RandomSeed},
    primitives::redpallas::{Signature, SpendAuth},
    tree::MerklePath,
    value::ValueCommitTrapdoor, Note,
};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use ripemd::Digest;

use anyhow::Result;
use warp_api_ffi::{
    connect_lightwalletd,
    ledger::{
        build_broadcast_tx, ledger_add_o_action, ledger_confirm_fee, ledger_init, ledger_init_tx,
        ledger_set_net_orchard, ledger_set_orchard_merkle_proof, ledger_set_stage, ledger_test_math, ledger_get_o_fvk, ledger_get_debug, ledger_cmu,
    },
    TransactionPlan,
};

use zcash_primitives::{
    consensus::{BlockHeight, BranchId, Network::MainNetwork},
    transaction::{
        sighash::SignableInput, sighash_v5, txid::TxIdDigester, TransactionData, TxVersion,
    },
};
use zcash_proofs::prover::LocalTxProver;

use group::ff::Field;
use nonempty::NonEmpty;

use hex_literal::hex;

#[tokio::main]
async fn main() {
    // let args = env::args();
    // let device: u32 = args.next().unwrap().parse().unwrap();
    // orchard_bundle::build_orchard().await.unwrap();
    // ledger_init().await.unwrap();

    // let sk = ledger_test_math(0).await.unwrap();
    // println!("SK {}", hex::encode(&sk));

    let mut rng = ChaCha20Rng::from_seed([4u8; 32]);
    let (_, _, note) = Note::dummy(&mut rng, None);
    let cmx: ExtractedNoteCommitment = note.commitment().into();
    println!("cmx {:?}", cmx);

    let address = note.recipient().to_raw_address_bytes();
    let value = note.value().inner();
    let rseed = note.rseed().as_bytes();
    let rho = note.rho().to_bytes();

    println!("{}", hex::encode(&address));
    println!("{} {}", value, hex::encode(value.to_le_bytes()));
    println!("{}", hex::encode(rseed));
    println!("{}", hex::encode(&rho));

    let mut buffer = vec![];
    buffer.write(&address).unwrap();
    buffer.write_u64::<LE>(value).unwrap();
    buffer.write(rseed).unwrap();
    buffer.write(&rho).unwrap();

    let cmu = ledger_cmu(&buffer).await.unwrap();
    println!("cmx {:?}", hex::encode(&cmu));

    // for i in 0..3 {
    //     let debug_data = ledger_get_debug(i).await.unwrap();
    //     println!("debug {}", hex::encode(&debug_data));
    // }

    // for i in 0..10 {
    //     let sk = ledger_test_math(i).await.unwrap();
    //     println!("SK {}", hex::encode(&sk));
    // }

    // let o_fvk = ledger_get_o_fvk().await.unwrap();
    // println!("FVK {}", hex::encode(&o_fvk));

    // let sk = hex::decode("19778746bfdf33616075940a21c4011263e974ff7b7341a9a8e5713908d39dab").unwrap();
    // let sk = SpendingKey::from_bytes(sk.try_into().unwrap()).unwrap();
    // let ask = SpendAuthorizingKey::from(&sk);
    // println!("ask {:?}", ask);
    // let fvk = FullViewingKey::from(&sk);
    // println!("fvk {}", hex::encode(fvk.to_bytes()));


}

#[tokio::main]
async fn main1() -> Result<()> {
    let params_dir = Path::new(&std::env::var("HOME").unwrap()).join(".zcash-params");
    let prover = LocalTxProver::new(
        &params_dir.join("sapling-spend.params"),
        &params_dir.join("sapling-output.params"),
    );
    let mut file = File::open("/tmp/tx.json").unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let tx_plan: TransactionPlan = serde_json::from_str(&data).unwrap();

    let mut client = connect_lightwalletd("https://lwdv3.zecwallet.co").await?;

    build_broadcast_tx(&MainNetwork, &mut client, &tx_plan, &prover).await?;

    Ok(())
}

const ORCHARD_SPENDAUTHSIG_BASEPOINT_BYTES: [u8; 32] = [
    99, 201, 117, 184, 132, 114, 26, 141, 12, 161, 112, 123, 227, 12, 127, 12, 95, 68, 95, 62, 124,
    24, 141, 59, 6, 214, 241, 40, 179, 35, 85, 183,
];

async fn main3() -> Result<()> {
    dotenv::dotenv()?;
    let spending_key = hex::decode(dotenv::var("SPENDING_KEY").unwrap()).unwrap();

    let x =
        hex::decode("063b1e0d8b7f64bb2a9465903b46c9019b552951afa5d735817d449d1334c510").unwrap();

    let expiry_height = 2_500_000;
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdHeadersHash")
        .to_state();
    h.update(&hex!("050000800a27a726b4d0d6c200000000"));
    h.write_u32::<LE>(expiry_height)?;
    let header_digest = h.finalize();

    // let Q: Point = Point::hash_to_curve(Q_PERSONALIZATION)(MERKLE_CRH_PERSONALIZATION.as_bytes());
    // let p = <Ep as GroupEncoding>::from_bytes(&ORCHARD_SPENDAUTHSIG_BASEPOINT_BYTES).unwrap();
    // println!("p {:?}", p);
    // let q = Ep::identity() + p;
    // println!("q {:?}", q);
    // let p = p.double();
    // println!("{:?}", p);
    // println!("{}", hex::encode(ORCHARD_SPENDAUTHSIG_BASEPOINT_BYTES));
    let sk = orchard::keys::SpendingKey::from_bytes(spending_key.try_into().unwrap()).unwrap();
    let fvk = orchard::keys::FullViewingKey::from(&sk);
    println!("FVK {:?}", fvk);
    println!("FVK {}", hex::encode(fvk.to_bytes()));
    // let ivk = fvk.to_ivk(Scope::External);
    // println!("IVK {:?}", ivk);
    // println!("DK {:?}", hex::encode(ivk.dk.to_bytes()));
    // let rho = Nullifier::dummy(&mut OsRng);
    // println!("rho {}", hex::encode(rho.to_bytes()));

    let rho = Nullifier::from_bytes(&x.clone().try_into().unwrap()).unwrap();

    let mut prng = ChaCha20Rng::from_seed([0; 32]);
    let rcv = ValueCommitTrapdoor::random(&mut prng);

    let mut alpha_rng = ChaCha20Rng::from_seed([1; 32]);
    let mut rseed_rng = ChaCha20Rng::from_seed([2; 32]);

    let alpha = pallas::Scalar::random(&mut alpha_rng);
    let ak: SpendValidatingKey = fvk.clone().into();
    let rk = ak.randomize(&alpha);
    let rk_bytes: [u8; 32] = rk.clone().0.into();

    let v_net = orchard::value::ValueSum::from_raw(-1000);
    let cv_net = orchard::value::ValueCommitment::derive(v_net, rcv.clone());

    let mut rseed = [0u8; 32];
    rseed_rng.fill_bytes(&mut rseed);
    println!("rseed {}", hex::encode(&rseed));

    let rseed = RandomSeed::from_bytes(rseed, &rho).unwrap();
    println!("esk {:?}", &rseed.esk(&rho));
    println!("psi {:?}", &rseed.psi(&rho));
    println!("rcm {:?}", &rseed.rcm(&rho));

    let address = fvk.address_at(0u64, Scope::External);
    let mut buf = vec![];
    buf.write_all(&rho.to_bytes())?;
    buf.write_all(&address.to_raw_address_bytes())?;
    buf.write_u64::<LE>(400_000)?;

    let note = orchard::Note::from_parts(
        address,
        orchard::value::NoteValue::from_raw(400_000),
        rho,
        rseed,
    )
    .unwrap();
    println!("note {:?}", note);
    let cmx: ExtractedNoteCommitment = note.commitment().into();
    println!("cmx {:?}", cmx);
    // ledger_test_math(&buf).await?;

    let mut memo = [0u8; 512];
    memo[0] = 0xF6;

    let encryptor = orchard::note_encryption::OrchardNoteEncryption::new(
        None,
        note.clone(),
        address.clone(),
        memo,
    );

    let epk = encryptor.epk().to_bytes().0;
    let enc = encryptor.encrypt_note_plaintext();
    let out = encryptor.encrypt_outgoing_plaintext(&cv_net.clone(), &cmx, &mut prng);
    let encrypted_note = orchard::note::TransmittedNoteCiphertext {
        epk_bytes: epk.clone(),
        enc_ciphertext: enc.clone(),
        out_ciphertext: out.clone(),
    };

    let merkle_path = MerklePath::dummy(&mut prng);
    let anchor = merkle_path.root(cmx);

    let _authorization = zcash_primitives::transaction::components::orchard::Unauthorized {};

    let action = orchard::Action::from_parts(
        rho.clone(),
        rk,
        cmx,
        encrypted_note,
        cv_net.clone(),
        Signature::<SpendAuth>::from([0; 64]),
    );
    let _actions = NonEmpty::new(action);
    let _circuit = Circuit::from_action_context_unchecked(
        orchard::builder::SpendInfo::new(fvk.clone(), note.clone(), merkle_path).unwrap(),
        note,
        alpha,
        rcv.clone(),
    );

    let mut orchard_memos_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdOrcActMHash")
        .to_state();
    orchard_memos_hasher.update(&enc[52..564]);
    let orchard_memos_hash = orchard_memos_hasher.finalize();

    let mut orchard_nc_hasher = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdOrcActNHash")
        .to_state();
    orchard_nc_hasher.update(&cv_net.to_bytes());
    orchard_nc_hasher.update(&rk_bytes);
    orchard_nc_hasher.update(&enc[564..]);
    orchard_nc_hasher.update(&out);
    let orchard_nc_hash = orchard_nc_hasher.finalize();

    // let mut orchard_builder = orchard::builder::Builder::new(Flags::from_byte(3).unwrap(),
    //     anchor);
    // orchard_builder.add_spend(fvk.clone(), note, merkle_path).unwrap();
    // orchard_builder.add_recipient(None, address.clone(),
    //     orchard::value::NoteValue::from_raw(399_000), None).unwrap();
    // let bundle: orchard::Bundle<orchard::builder::InProgress<orchard::builder::Unproven, orchard::builder::Unauthorized>, _> = orchard_builder.build(&mut prng).unwrap();

    let _bsk = rcv.into_bsk();

    let _pk = ProvingKey::build();
    // let instance = action.to_instance(Flags::from_parts(true, true), anchor.clone());
    // let bundle = orchard::Bundle::from_parts(actions,
    //     Flags::from_byte(3).unwrap(),
    //     Amount::from_i64(-1000).unwrap(),
    //     anchor,
    //     InProgress::<Unproven, OrchardUnauthorized> {
    //         proof: Unproven { circuits: vec![circuit] },
    //         sigs: OrchardUnauthorized { bsk }
    //     }
    // );
    // let proof = bundle.create_proof(&pk, &mut prng).unwrap();
    // let proof = proof.authorization();
    // let proof = &proof.proof;

    // let bundle = orchard::Bundle::from_parts(actions,
    //     Flags::from_byte(3).unwrap(),
    //     Amount::from_i64(-1000).unwrap(),
    //     anchor,
    //     orchard::bundle::Authorized {
    //         proof: todo!(),
    //         binding_signature: todo!(),
    //     }
    // );

    let tx_data: TransactionData<zcash_primitives::transaction::Unauthorized> = TransactionData {
        version: TxVersion::Zip225,
        consensus_branch_id: BranchId::Nu5,
        lock_time: 0,
        expiry_height: BlockHeight::from_u32(expiry_height),
        transparent_bundle: None,
        sprout_bundle: None,
        sapling_bundle: None,
        orchard_bundle: None,
    };

    let txid_parts = tx_data.digest(TxIdDigester);
    let sig_hash = sighash_v5::v5_signature_hash(&tx_data, &SignableInput::Shielded, &txid_parts);

    println!("ORCHARD memos {:?}", orchard_memos_hash);
    println!("ORCHARD nc {:?}", orchard_nc_hash);

    println!("SIGHASH PARTS {:?}", txid_parts);
    println!("SIGHASH {:?}", sig_hash);

    ledger_init().await.unwrap();
    ledger_init_tx(header_digest.as_bytes()).await.unwrap();
    ledger_set_orchard_merkle_proof(
        &anchor.to_bytes(),
        orchard_memos_hash.as_bytes(),
        orchard_nc_hash.as_bytes(),
    )
    .await
    .unwrap();

    // no t-in
    ledger_set_stage(2).await.unwrap();
    // no t-out
    ledger_set_stage(3).await.unwrap();
    // no s-out
    ledger_set_stage(4).await.unwrap();
    ledger_add_o_action(
        &x,
        400_000,
        &epk,
        &address.to_raw_address_bytes(),
        &enc[0..52],
    )
    .await
    .unwrap();
    ledger_set_stage(5).await.unwrap();

    ledger_set_net_orchard(-1000).await.unwrap();

    ledger_confirm_fee().await.unwrap();

    // println!("address {}", hex::encode(address.to_raw_address_bytes()));

    // let esk = ExtendedSpendingKey::master(&[1; 32]);
    // let (_, pa) = esk.default_address();
    // let ta = TransparentAddress::PublicKey([1; 20]);

    // let ua = UnifiedAddress::from_receivers(
    //     Some(address),
    //     Some(pa),
    //     Some(ta),
    // ).unwrap();
    // let ua = ua.encode(&MainNetwork);
    // println!("UA {}", ua);

    // let mut message = [1u8; 128];
    // f4jumble::f4jumble_mut(&mut message[..]).unwrap();
    // println!("f4 {}", hex::encode(message));

    // println!("{}", hex::encode(address.to_raw_address_bytes()));
    // let a = zcash_address::ZcashAddress::try_from_encoded("u1hlpt22xwe3sy034cdjhtnlp4l39zwa7xsj6tsxq0d2p9mv0gzsxnzkpc3wxpv6nh9s2kyxe54qnnxujxc8wqjemvlelzsdxlm7feqdjuksgpx45w3we563apmtmxhql6aa584u9569pdaq3q8h9p8gma67z5td3sckhamh99aqkgf3cg76rykn2e2pwxdjm8wdya0w39355rgvhxvpw").unwrap();
    // if let zcash_address::AddressKind::Unified(r) = a.kind {
    //     let Address(rs) = r;
    //     let r = &rs[0];
    //     if let Receiver::Orchard(address) = r {
    //         println!("address {}", hex::encode(address));
    //     }
    // }
    // println!("a {:?}", a);

    // ledger_init().await?;
    // ledger_test_math().await?;
    // let fvk = ledger_get_o_fvk().await?;
    // println!("FVK {}", hex::encode(&fvk));

    Ok(())
}
