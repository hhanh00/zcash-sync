use anyhow::Result;
use orchard::{
    keys::{Diversifier, FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey},
    note::{Nullifier, RandomSeed},
    pob::create_proof,
    tree::MerklePath,
    value::NoteValue,
};
use pasta_curves::Fp;
use rand::{CryptoRng, RngCore};
use rusqlite::params;

use crate::Connection;

use super::Election;

pub fn build_proof<Rng: RngCore + CryptoRng>(
    connection: &Connection,
    account: u32,
    e: &Election,
    mut rng: Rng,
) -> Result<()> {
    let domain = Fp::zero();
    let mut sk = [0u8; 32];
    rng.fill_bytes(&mut sk);
    let sk = loop {
        let sk = SpendingKey::from_bytes(sk);
        if sk.is_some().into() {
            break sk.unwrap();
        }
    };
    let spauth = SpendAuthorizingKey::from(&sk);

    let fvk = connection.query_row(
        "SELECT fvk FROM orchard_addrs WHERE account = ?1",
        [account],
        |r| r.get::<_, Vec<u8>>(0),
    )?;
    let fvk = FullViewingKey::from_bytes(&fvk.try_into().unwrap()).unwrap();

    let (position, diversifier, value, rcm, nf, rho) = connection.query_row(
        "SELECT position, diversifier, value, rcm, nf, rho FROM received_notes WHERE orchard = 1 AND spent IS NULL
    AND height >= ?2 AND height <= ?3 AND account = ?1",
        params![account, e.start_height, e.end_height],
        |r| {
            let position = r.get::<_, u32>(0)?;
            let diversifier = r.get::<_, Vec<u8>>(1)?;
            let value = r.get::<_, u64>(2)?;
            let rcm = r.get::<_, Vec<u8>>(3)?;
            let nf = r.get::<_, Vec<u8>>(4)?;
            let rho = r.get::<_, Vec<u8>>(5)?;
            Ok((position, diversifier, value, rcm, nf, rho))
        },
    )?;
    let d = Diversifier::from_bytes(diversifier.try_into().unwrap());
    let recipient = fvk.address(d, Scope::External);
    let value = NoteValue::from_raw(value);
    let rho = Nullifier::from_bytes(&rho.try_into().unwrap()).unwrap();
    let rseed = RandomSeed::from_bytes(rcm.try_into().unwrap(), &rho).unwrap();

    let note = orchard::note::Note::from_parts(recipient, value, rho, rseed).unwrap();

    // create_proof(
    //     domain, spauth, &fvk, &note, cmx_path, nf_path, nf_start, alpha, rng,
    // )?;
    todo!()
}
