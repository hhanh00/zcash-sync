use std::{fs::File, io::Read};

use group::{Group, GroupEncoding};
use orchard::{
    builder::SpendInfo,
    bundle::{Authorized, Flags},
    circuit::{Circuit, Instance, ProvingKey},
    keys::{Diversifier, FullViewingKey, Scope, SpendValidatingKey},
    note::{ExtractedNoteCommitment, Nullifier, RandomSeed, TransmittedNoteCiphertext},
    note_encryption::OrchardNoteEncryption,
    primitives::redpallas::{Signature, SpendAuth},
    tree::MerklePath,
    value::{NoteValue, ValueCommitTrapdoor, ValueCommitment, ValueSum},
    Action, Address, Anchor, Bundle, Note, Proof,
};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use ripemd::Digest;

use anyhow::Result;
use warp_api_ffi::{decode_orchard_merkle_path, TransactionPlan};

use zcash_primitives::transaction::components::Amount;

use group::ff::Field;
use nonempty::NonEmpty;

use warp_api_ffi::{Destination, Source};

pub async fn build_orchard() -> Result<()> {
    dotenv::dotenv()?;
    let mut prng = ChaCha20Rng::from_seed([0; 32]);
    let mut rseed_rng = ChaCha20Rng::from_seed([1; 32]);
    let mut alpha_rng = ChaCha20Rng::from_seed([2; 32]);

    let _spending_key = hex::decode(dotenv::var("SPENDING_KEY").unwrap()).unwrap();
    let mut file = File::open("/tmp/tx.json").unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let tx_plan: TransactionPlan = serde_json::from_str(&data).unwrap();

    let orchard_fvk: [u8; 96] = hex::decode(tx_plan.orchard_fvk)
        .unwrap()
        .try_into()
        .unwrap();
    let orchard_fvk = FullViewingKey::from_bytes(&orchard_fvk).unwrap();

    let anchor = Anchor::from_bytes(tx_plan.orchard_anchor).unwrap();

    let spends: Vec<_> = tx_plan
        .spends
        .iter()
        .filter_map(|s| match s.source {
            Source::Orchard {
                id_note: _,
                diversifier,
                rseed,
                rho,
                ref witness,
            } => {
                let diversifier = Diversifier::from_bytes(diversifier);
                let address = orchard_fvk.address(diversifier, Scope::External);
                let rho = Nullifier::from_bytes(&rho).unwrap();
                let rseed = RandomSeed::from_bytes(rseed, &rho).unwrap();
                let note =
                    Note::from_parts(address, NoteValue::from_raw(s.amount), rho, rseed).unwrap();
                let merkle_path = decode_orchard_merkle_path(0, &witness).unwrap();
                Some(OrchardSpend {
                    fvk: orchard_fvk.clone(),
                    note,
                    merkle_path,
                })
            }
            _ => None,
        })
        .collect();

    let outputs: Vec<_> = tx_plan
        .outputs
        .iter()
        .filter_map(|o| match o.destination {
            Destination::Orchard(address) => {
                let address = Address::from_raw_address_bytes(&address).unwrap();
                let output = OrchardOutput {
                    recipient: address,
                    amount: NoteValue::from_raw(o.amount),
                    memo: o.memo.as_array().clone(),
                };
                Some(output)
            }
            _ => None,
        })
        .collect();

    let num_actions = spends.len().max(outputs.len());
    let mut actions = vec![];
    let mut circuits = vec![];
    let mut instances = vec![];
    let mut net_value = ValueSum::default();
    let mut sum_rcv = ValueCommitTrapdoor::zero();
    for i in 0..num_actions {
        // pad with dummy spends/outputs
        let spend = if i < spends.len() {
            spends[i].clone()
        } else {
            OrchardSpend::dummy(&mut prng)
        };

        let output = if i < outputs.len() {
            outputs[i].clone()
        } else {
            OrchardOutput::dummy(&mut prng)
        };

        let rcv = ValueCommitTrapdoor::random(&mut prng);
        sum_rcv = sum_rcv + &rcv;
        let alpha = pasta_curves::Fq::random(&mut alpha_rng);
        let ak: SpendValidatingKey = orchard_fvk.clone().into();
        let rk = ak.randomize(&alpha);

        let rho = spend.note.rho();
        let mut rseed = [0u8; 32];
        rseed_rng.fill_bytes(&mut rseed);
        let rseed = RandomSeed::from_bytes(rseed, &rho).unwrap();

        let v_net: ValueSum = spend.note.value() - output.amount;
        net_value = (net_value + v_net).unwrap();
        let cv_net = ValueCommitment::derive(v_net, rcv.clone());

        let spend_info = SpendInfo::new(
            spend.fvk.clone(),
            spend.note.clone(),
            spend.merkle_path.clone(),
        )
        .unwrap();
        let output_note = Note::from_parts(
            output.recipient.clone(),
            output.amount.clone(),
            rho.clone(),
            rseed,
        )
        .unwrap();
        let cmx: ExtractedNoteCommitment = output_note.commitment().into();

        let encryptor = OrchardNoteEncryption::new(
            Some(orchard_fvk.to_ovk(Scope::External)),
            output_note.clone(),
            output.recipient.clone(),
            output.memo.clone(),
        );

        let epk = encryptor.epk().to_bytes().0;
        let enc = encryptor.encrypt_note_plaintext();
        let out = encryptor.encrypt_outgoing_plaintext(&cv_net.clone(), &cmx, &mut prng);
        let encrypted_note = TransmittedNoteCiphertext {
            epk_bytes: epk.clone(),
            enc_ciphertext: enc.clone(),
            out_ciphertext: out.clone(),
        };

        let action: Action<Signature<SpendAuth>> = Action::from_parts(
            rho.clone(),
            rk.clone(),
            cmx.clone(),
            encrypted_note,
            cv_net.clone(),
            [0; 64].into(),
        );
        actions.push(action);

        let circuit =
            Circuit::from_action_context(spend_info, output_note, alpha, rcv.clone()).unwrap();
        circuits.push(circuit);
        let instance = Instance::from_parts(anchor, cv_net, rho.clone(), rk, cmx, true, true);
        instances.push(instance);
    }

    let pk = ProvingKey::build();
    let proof = Proof::create(&pk, &circuits, &instances, &mut prng).unwrap();
    let nv = i64::try_from(net_value).unwrap();
    let amount = Amount::from_i64(nv).unwrap();

    let sig_hash = [0u8; 32];

    let bsk = sum_rcv.into_bsk();
    let binding_signature = bsk.sign(&mut prng, &sig_hash);

    let actions = NonEmpty::from_slice(&actions).unwrap();

    let _bundle: Bundle<_, Amount> = Bundle::from_parts(
        actions,
        Flags::from_parts(true, true),
        amount,
        anchor.clone(),
        Authorized::from_parts(proof, binding_signature),
    );

    Ok(())
}

#[derive(Clone, Debug)]
struct OrchardSpend {
    fvk: FullViewingKey,
    note: Note,
    merkle_path: MerklePath,
}

impl OrchardSpend {
    pub fn dummy<R: RngCore>(rng: &mut R) -> Self {
        let (_, fvk, dummy_note) = Note::dummy(rng, None);
        let dummy_path = MerklePath::dummy(rng);
        OrchardSpend {
            fvk,
            note: dummy_note,
            merkle_path: dummy_path,
        }
    }
}

#[derive(Clone, Debug)]
struct OrchardOutput {
    recipient: Address,
    amount: NoteValue,
    memo: [u8; 512],
}

impl OrchardOutput {
    pub fn dummy<R: RngCore>(rng: &mut R) -> Self {
        let (_, _, dummy_note) = Note::dummy(rng, None);
        let _address = dummy_note.recipient();
        let mut memo = [0u8; 512];
        memo[0] = 0xF6;

        OrchardOutput {
            recipient: dummy_note.recipient(),
            amount: dummy_note.value(),
            memo,
        }
    }
}
