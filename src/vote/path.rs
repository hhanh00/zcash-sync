use std::mem::swap;

use anyhow::Result;
use orchard::Note;
use pasta_curves::Fp;

use super::vote_generated::fb::*;
use super::{prevhash::PreviousHashes, Hash, DEPTH};

#[derive(Clone, Default)]
pub struct MerklePath {
    pub value: Hash,
    pub position: u32,
    pub path: [Hash; DEPTH],
    p: usize,
}

pub fn calculate_merkle_paths(
    ph: &PreviousHashes,
    positions: &[u32],
    hashes: &[Hash],
) -> Result<Vec<MerklePath>> {
    let mut start = ph.position();
    println!("start {start}");
    let mut paths = positions
        .iter()
        .map(|p| MerklePath {
            value: hashes[*p as usize - start],
            position: *p,
            path: [Hash::default(); DEPTH],
            p: *p as usize,
        })
        .collect::<Vec<_>>();
    let mut er = orchard::pob::empty_hash();
    let mut layer = Vec::with_capacity(positions.len() + 2);
    for i in 0..32 {
        if i == 0 {
            if let Some(h) = ph.lefts[i] {
                layer.push(h);
                start -= 1;
            }
            layer.extend(hashes);
            if layer.len() & 1 == 1 {
                layer.push(er);
            }
        }

        for path in paths.iter_mut() {
            let idx = path.p - start;
            if idx & 1 == 1 {
                path.path[i] = layer[idx as usize - 1];
            } else {
                path.path[i] = layer[idx as usize + 1];
            }
            path.p /= 2;
        }
        start /= 2;

        let pairs = layer.len() / 2;
        let mut next_layer = Vec::with_capacity(pairs + 2);
        if i < 31 {
            if let Some(h) = ph.lefts[i + 1] {
                next_layer.push(h);
            }
        }

        for j in 0..pairs {
            let h = orchard::pob::cmx_hash(i as u8, &layer[j * 2], &layer[j * 2 + 1]);
            next_layer.push(h);
        }

        er = orchard::pob::cmx_hash(i as u8, &er, &er);
        if next_layer.len() & 1 == 1 {
            next_layer.push(er);
        }

        swap(&mut layer, &mut next_layer);
    }

    Ok(paths)
}

pub struct NotePosition {
    pub note: Note,
    pub position: u32,
    pub nf_start_range: Fp,
    pub nf_position: u32,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use blake2b_simd::Params;
    use ff::PrimeField;
    use orchard::{
        keys::{Diversifier, FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey},
        note::{ExtractedNoteCommitment, Nullifier, RandomSeed},
        pob::create_proof,
        primitives::redpallas::{Binding, SigningKey},
        tree::{MerkleHashOrchard, MerklePath as OrchardMerklePath},
        value::{NoteValue, ValueCommitTrapdoor},
        Note,
    };
    use pasta_curves::{Fp, Fq};
    use rand::{rngs::OsRng, RngCore};
    use rusqlite::params;

    use crate::vote::{
        path::{calculate_merkle_paths, BallotT, InputT, NotePosition, ProofT, SignatureT},
        prevhash::{fetch_tree_state, PreviousHashes},
        trees::download_reference_data,
        vote_generated::fb::*,
        Election,
    };

    #[tokio::test]
    async fn test() -> Result<()> {
        let mut rng = OsRng;
        let account = 4;
        let e = Election {
            name: "Devfund Poll".to_string(),
            start_height: 2540000,
            end_height: 2541500,
        };
        crate::set_coin_lwd_url(0, "https://lwd5.zcash-infra.com:9067");
        let mut c = crate::CoinConfig::get(0);
        c.set_db_path("/Users/hanhhuynhhuu/Library/Containers/me.hanh.ywallet/Data/Library/Application Support/me.hanh.ywallet/databases/zec.db")?;

        let mut client = c.connect_lwd().await?;
        let connection = c.connection();

        download_reference_data(&connection, &mut client, &e).await?;
        let ph = fetch_tree_state(&mut client, e.start_height - 1).await?;

        println!("Creating proof...");
        let (sk, fvk) = connection.query_row(
            "SELECT sk, fvk FROM orchard_addrs WHERE account = ?1",
            [account],
            |r| {
                let sk = r.get::<_, Vec<u8>>(0)?;
                let fvk = r.get::<_, Vec<u8>>(1)?;
                Ok((sk, fvk))
            },
        )?;
        let fvk = FullViewingKey::from_bytes(&fvk.try_into().unwrap()).unwrap();

        println!("Scanning notes...");
        let mut notes = vec![];
        let mut s = connection.prepare("SELECT position, diversifier, value, rcm, nf, rho FROM received_notes WHERE account = ?1 AND height >= ?2 AND height <= ?3 AND orchard = 1 AND spent IS NULL")?;
        let rows = s.query_map(params![account, e.start_height, e.end_height], |r| {
            let position = r.get::<_, u32>(0)?;
            let diversifier = r.get::<_, Vec<u8>>(1)?;
            let value = r.get::<_, u64>(2)?;
            let rcm = r.get::<_, Vec<u8>>(3)?;
            let nf = r.get::<_, Vec<u8>>(4)?;
            let rho = r.get::<_, Vec<u8>>(5)?;
            Ok((position, diversifier, value, rcm, nf, rho))
        })?;
        for r in rows {
            let (position, diversifier, value, rcm, nf, rho) = r?;
            let d = Diversifier::from_bytes(diversifier.try_into().unwrap());
            let recipient = fvk.address(d, Scope::External);
            let value = NoteValue::from_raw(value);
            let rho = Nullifier::from_bytes(&rho.try_into().unwrap()).unwrap();
            let rseed = RandomSeed::from_bytes(rcm.try_into().unwrap(), &rho).unwrap();

            let note = Note::from_parts(recipient, value, rho, rseed).unwrap();
            notes.push(NotePosition {
                note,
                position,
                nf_start_range: Fp::zero(),
                nf_position: 0,
            });
        }

        println!("Building cmx tree...");
        s = connection.prepare("SELECT hash FROM cmxs")?;
        let rows = s.query_map([], |r| r.get::<_, [u8; 32]>(0))?;
        let hashes = rows.collect::<Result<Vec<_>, _>>()?;

        let positions = notes.iter().map(|n| n.position).collect::<Vec<_>>();
        let cmx_paths = calculate_merkle_paths(&ph, &positions, &hashes)?;

        println!("Building nf tree...");
        s = connection.prepare("SELECT hash FROM nullifiers ORDER BY revhash")?;
        let rows = s.query_map([], |r| {
            let h = r.get::<_, [u8; 32]>(0)?;
            Ok(Fp::from_repr(h).unwrap())
        })?;
        let mut nfs = vec![];
        nfs.push(Fp::zero());
        for r in rows {
            let r = r?;
            nfs.push(r - Fp::one());
            nfs.push(r + Fp::one());
        }
        nfs.push(Fp::one().neg());
        for n in notes.iter_mut() {
            let NotePosition { note, .. } = n;
            let nf: Nullifier = note.nullifier(&fvk);
            let nf = Fp::from_repr(nf.to_bytes()).unwrap();
            match nfs.binary_search(&nf) {
                Ok(_) => anyhow::bail!("Duplicate nullifier"),
                Err(idx) => {
                    n.nf_start_range = nfs[idx - 1];
                    n.nf_position = (idx - 1) as u32;
                }
            }
        }

        let positions = notes.iter().map(|n| n.nf_position).collect::<Vec<_>>();
        let nfs = nfs.iter().map(|nf| nf.to_repr()).collect::<Vec<_>>();
        let nf_paths = calculate_merkle_paths(&PreviousHashes::default(), &positions, &nfs)?;

        let domain = orchard::pob::domain(e.name.as_bytes());
        // let sk = Fp::one().to_repr();
        let sk = SpendingKey::from_bytes(sk.try_into().unwrap()).unwrap();
        let spauth = SpendAuthorizingKey::from(&sk);

        let mut inputs = vec![];
        let mut proofs = vec![];
        let mut amount = 0u64;
        let mut rcv_total = ValueCommitTrapdoor::zero();
        for (n, (cmx_path, nf_path)) in notes.iter().zip(cmx_paths.iter().zip(nf_paths.iter())) {
            let NotePosition {
                note,
                position,
                nf_start_range,
                nf_position,
            } = n;

            let cmx_path = OrchardMerklePath::from_parts(
                *position,
                cmx_path
                    .path
                    .map(|h| MerkleHashOrchard::from_bytes(&h).unwrap()),
            );
            let nf_path = OrchardMerklePath::from_parts(
                *nf_position,
                nf_path
                    .path
                    .map(|h| MerkleHashOrchard::from_bytes(&h).unwrap()),
            );

            // let cmx = ExtractedNoteCommitment::from_bytes(&p.value).unwrap();
            // let root = path.root(cmx);

            let proof = create_proof(
                domain,
                spauth.clone(),
                &fvk,
                note,
                cmx_path,
                nf_path,
                nf_start_range.clone(),
                Fq::one(),
                &mut rng,
            )?;

            let proof_public = proof.public;
            proofs.push(ProofT {
                data: Some(proof_public.proof.as_ref().to_vec()),
            });

            let input = InputT {
                cv: proof_public.cv.to_bytes(),
                nf: proof_public.domain_nf.to_bytes(),
                rk: [0u8; 32],
            };
            inputs.push(input);

            let rcv = proof.private.rcv;
            rcv_total = rcv_total + &rcv;
            amount += note.value().inner();

            // domain
            // notes
            //   proof, cv, domain_nf

            println!("{:?} {}", note, position);
        }

        let header = HeaderT {
            version: 1,
            domain: domain.to_repr(),
        };
        let ballot = BallotT {
            header: Some(header),
            inputs: Some(inputs),
            amount,
            payload: Some(0u32.to_le_bytes().to_vec()),
        };
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let root = ballot.pack(&mut fbb);
        fbb.finish_minimal(root);
        let ballot_data = fbb.finished_data();

        let sig_hash = Params::new()
            .personal(b"ZcashVoteSighash")
            .hash_length(32)
            .hash(ballot_data);

        let bsk = rcv_total.to_bytes();
        let bsk: SigningKey<Binding> = bsk.try_into().unwrap();

        let binding_signature = bsk.sign(&mut rng, sig_hash.as_ref());
        let binding_signature: [u8; 64] = (&binding_signature).into();
        let binding_signature = SignatureT {
            r_part: binding_signature[0..32].try_into().unwrap(),
            s_part: binding_signature[32..64].try_into().unwrap(),
        };

        let witness = BallotWitnessT {
            proofs: Some(proofs),
            binding_signature: Some(binding_signature),
        };

        let ballot_envelope = BallotEnvelopeT {
            ballot: Some(Box::new(ballot)),
            witness: Some(Box::new(witness)),
        };
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let root = ballot_envelope.pack(&mut fbb);
        fbb.finish_minimal(root);
        let data = fbb.finished_data().to_vec();

        println!("{}", hex::encode(&data));
        Ok(())
    }
}
