use crate::chain::Nf;
use crate::db::ReceivedNote;
use crate::sync::Node;
use crate::sync::{CompactOutputBytes, DecryptedNote, OutputPosition, TrialDecrypter, ViewKey};
use crate::CompactTx;
use ff::PrimeField;
use std::convert::TryInto;
use zcash_note_encryption::Domain;
use zcash_protocol::consensus::{BlockHeight, NetworkConstants, Parameters};
use sapling::note_encryption::{PreparedIncomingViewingKey, SaplingDomain};
use sapling::zip32::ExtendedFullViewingKey;
use sapling::{PaymentAddress, SaplingIvk};

use crate::sapling::zip212_enforcement;

#[derive(Clone)]
pub struct SaplingViewKey {
    pub account: u32,
    pub fvk: ExtendedFullViewingKey,
    pub ivk: SaplingIvk,
}

impl ViewKey<SaplingDomain> for SaplingViewKey {
    fn account(&self) -> u32 {
        self.account
    }
    fn ivk(&self) -> <SaplingDomain as Domain>::IncomingViewingKey {
        PreparedIncomingViewingKey::new(&self.ivk)
    }
}

pub struct DecryptedSaplingNote {
    pub vk: SaplingViewKey,
    pub note: sapling::Note,
    pub pa: PaymentAddress,
    pub output_position: OutputPosition,
    pub cmx: Node,
}

impl DecryptedNote<SaplingDomain, SaplingViewKey> for DecryptedSaplingNote {
    fn from_parts(
        vk: SaplingViewKey,
        note: sapling::Note,
        pa: PaymentAddress,
        output_position: OutputPosition,
        cmx: Node,
    ) -> Self {
        DecryptedSaplingNote {
            vk,
            note,
            pa,
            output_position,
            cmx,
        }
    }

    fn position(&self, block_offset: usize) -> usize {
        block_offset + self.output_position.position_in_block
    }

    fn cmx(&self) -> Node {
        self.cmx
    }

    fn to_received_note(&self, position: u64) -> ReceivedNote {
        let viewing_key = &self.vk.fvk.fvk.vk;
        ReceivedNote {
            account: self.vk.account,
            height: self.output_position.height,
            output_index: self.output_position.output_index as u32,
            diversifier: self.pa.diversifier().0.to_vec(),
            value: self.note.value().inner(),
            rcm: self.note.rcm().to_repr().to_vec(),
            nf: self.note.nf(&viewing_key.nk, position).to_vec(),
            rho: None,
            spent: None,
        }
    }
}

#[derive(Clone)]
pub struct SaplingDecrypter<N> {
    pub network: N,
}

impl<N> SaplingDecrypter<N> {
    pub fn new(network: N) -> Self {
        SaplingDecrypter { network }
    }
}

impl<N: Parameters> TrialDecrypter<N, SaplingDomain, SaplingViewKey, DecryptedSaplingNote>
    for SaplingDecrypter<N>
{
    fn domain(&self, height: BlockHeight, _cob: &CompactOutputBytes) -> SaplingDomain {
        SaplingDomain::new(zip212_enforcement(&self.network, height))
    }

    fn spends(&self, vtx: &CompactTx) -> Vec<Nf> {
        vtx.spends
            .iter()
            .map(|co| {
                let nf: [u8; 32] = co.nf.clone().try_into().unwrap();
                Nf(nf)
            })
            .collect()
    }

    fn outputs(&self, vtx: &CompactTx) -> Vec<CompactOutputBytes> {
        vtx.outputs.iter().map(|co| co.into()).collect()
    }
}
