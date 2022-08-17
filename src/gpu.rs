use zcash_primitives::zip32::ExtendedFullViewingKey;
use crate::chain::{DecryptedBlock, Nf};
use crate::db::AccountViewKey;
use anyhow::Result;
use crate::CompactBlock;

#[cfg(feature = "cuda")]
pub mod cuda;

pub trait GPUProcessor<'a> {
    fn decrypt_account(&mut self, account: u32, fvk: &ExtendedFullViewingKey) -> Result<()>;
    fn get_decrypted_blocks(self) -> Result<Vec<DecryptedBlock<'a>>>;
}

pub fn trial_decrypt<'a, 'b, FVKIter: Iterator<Item=(&'b u32, &'b AccountViewKey)>, P: GPUProcessor<'a>>(
    mut processor: P,
    fvks: FVKIter,
) -> Result<Vec<DecryptedBlock<'a>>> {
    for (account, avk) in fvks {
        let fvk = &avk.fvk;
        processor.decrypt_account(*account, fvk)?;
    }

    Ok(processor.get_decrypted_blocks()?)
}

fn collect_nf(blocks: &[CompactBlock]) -> Result<Vec<DecryptedBlock>> {
    let mut decrypted_blocks = vec![];
    // collect nullifiers
    for b in blocks.iter() {
        let mut spends = vec![];
        let mut count_outputs = 0;
        for tx in b.vtx.iter() {
            for cs in tx.spends.iter() {
                let mut nf = [0u8; 32];
                nf.copy_from_slice(&cs.nf);
                spends.push(Nf(nf));
            }
            count_outputs += tx.outputs.len();
        }
        decrypted_blocks.push(DecryptedBlock {
            height: b.height as u32,
            notes: vec![],
            count_outputs: count_outputs as u32,
            spends,
            compact_block: b,
            elapsed: 0, // TODO
        });
    }
    Ok(decrypted_blocks)
}


