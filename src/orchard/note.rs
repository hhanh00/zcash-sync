use orchard::keys::Scope;
use orchard::note_encryption::OrchardDomain;
use zcash_primitives::consensus::{BlockHeight, Parameters};
use crate::chain::Nf;
use crate::{CompactTx, DbAdapterBuilder};
use crate::db::ReceivedNote;
use crate::sync::{CompactOutputBytes, DecryptedNote, Node, OutputPosition, TrialDecrypter, ViewKey};
use zcash_note_encryption;
use zcash_params::coin::CoinType;

#[derive(Clone, Debug)]
pub struct OrchardViewKey {
    pub account: u32,
    pub fvk: orchard::keys::FullViewingKey,
}

impl ViewKey<OrchardDomain> for OrchardViewKey {
    fn account(&self) -> u32 {
        self.account
    }

    fn ivk(&self) -> orchard::keys::IncomingViewingKey {
        self.fvk.to_ivk(orchard::keys::Scope::External)
    }
}

pub struct DecryptedOrchardNote {
    pub vk: OrchardViewKey,
    pub note: orchard::Note,
    pub pa: orchard::Address,
    pub output_position: OutputPosition,
    pub cmx: Node,
}

impl DecryptedNote<OrchardDomain, OrchardViewKey> for DecryptedOrchardNote {
    fn from_parts(vk: OrchardViewKey, note: orchard::Note, pa: orchard::Address, output_position: OutputPosition, cmx: Node) -> Self {
        DecryptedOrchardNote {
            vk,
            note,
            pa,
            output_position,
            cmx
        }
    }

    fn position(&self, block_offset: usize) -> usize {
        block_offset + self.output_position.position_in_block
    }

    fn cmx(&self) -> Node {
        self.cmx
    }

    fn to_received_note(&self, _position: u64) -> ReceivedNote {
        ReceivedNote {
            account: self.vk.account,
            height: self.output_position.height,
            output_index: self.output_position.output_index as u32,
            diversifier: self.pa.diversifier().as_array().to_vec(),
            value: self.note.value().inner(),
            rcm: self.note.rseed().as_bytes().to_vec(),
            nf: self.note.nullifier(&self.vk.fvk).to_bytes().to_vec(),
            rho: Some(self.note.rho().to_bytes().to_vec()),
            spent: None
        }
    }
}

#[derive(Clone)]
pub struct OrchardDecrypter<N> {
    pub network: N,
}

impl <N> OrchardDecrypter<N> {
    pub fn new(network: N) -> Self {
        OrchardDecrypter {
            network,
        }
    }
}

impl <N: Parameters> TrialDecrypter<N, OrchardDomain, OrchardViewKey, DecryptedOrchardNote> for OrchardDecrypter<N> {
    fn domain(&self, _height: BlockHeight, cob: &CompactOutputBytes) -> OrchardDomain {
        OrchardDomain::for_nullifier(orchard::note::Nullifier::from_bytes(&cob.nullifier).unwrap())
    }

    fn spends(&self, vtx: &CompactTx) -> Vec<Nf> {
        vtx.actions.iter().map(|co| {
            let nf: [u8; 32] = co.nullifier.clone().try_into().unwrap();
            Nf(nf)
        }).collect()
    }

    fn outputs(&self, vtx: &CompactTx) -> Vec<CompactOutputBytes> {
        vtx.actions.iter().map(|co| co.into()).collect()
    }
}

#[test]
pub fn test_decrypt() -> anyhow::Result<()> {
    // let mut nullifier = hex::decode("951ab285b0f4df3ff24f24470dbb8bafa3b5caeeb204fc4465f7ea9c3d5a980a").unwrap();
    // let mut epk = hex::decode("182d698c3bb8b168d5f9420f1c2e32d94b4dbc0826181c1783ea47fedd31b710").unwrap();
    // let mut cmx = hex::decode("df45e00eb39e4c281e2804a366d3010b7f663724472d12637e0a749e6ce22719").unwrap();
    // let ciphertext = hex::decode("d9bc6ee09b0afde5dd69bfdf4b667a38da3e1084e84eb6752d54800b9f5110203b60496ab5313dba3f2acb9ef30bcaf68fbfcc59").unwrap();

    let mut nullifier = hex::decode("ea1b97cc83d326db4130433022f68dd32a0bc707448b19b0980e4e6404412b29").unwrap();
    let mut epk = hex::decode("e2f666e905666f29bb678c694602b2768bea655c0f2b18f9c342ad8b64b18c0c").unwrap();
    let mut cmx = hex::decode("4a95dbf0d1d0cac1376a0b8fb0fc2ed2843d0e2670dd976a63386b293f30de25").unwrap();
    let ciphertext = hex::decode("73640095a90bb03d14f687d6acf4822618a3def1da3b71a588da1c68e25042f7c9aa759778e73aa2bb39d1061e51c1e8cf5e0bce").unwrap();

    let db_builder = DbAdapterBuilder {
        coin_type: CoinType::Zcash,
        db_path: "./zec.db".to_string()
    };
    let db = db_builder.build()?;
    let keys = db.get_orchard_fvks()?.first().unwrap().clone();
    let fvk = keys.fvk;

    let output = CompactOutputBytes {
        nullifier: nullifier.clone().try_into().unwrap(),
        epk: epk.try_into().unwrap(),
        cmx: cmx.try_into().unwrap(),
        ciphertext: ciphertext.try_into().unwrap()
    };
    let domain = OrchardDomain::for_nullifier(orchard::note::Nullifier::from_bytes(&nullifier.try_into().unwrap()).unwrap());
    let r = zcash_note_encryption::try_compact_note_decryption(&domain, &fvk.to_ivk(Scope::External), &output);
    println!("{:?}", r);
    Ok(())
}

