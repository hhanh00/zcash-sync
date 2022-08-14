use crate::chain::{DecryptedBlock, DecryptedNote, Nf};
use crate::lw_rpc::CompactBlock;
use crate::{Hash, GENERATORS_EXP};
use anyhow::Result;
use ff::BatchInverter;
use jubjub::Fq;
use rustacuda::context::CurrentContext;
use rustacuda::launch;
use rustacuda::prelude::*;
use std::convert::TryInto;
use std::ffi::CString;
use zcash_note_encryption::Domain;
use zcash_primitives::consensus::{BlockHeight, Network};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use crate::db::AccountViewKey;

const THREADS_PER_BLOCK: usize = 256usize;
const BUFFER_SIZE: usize = 128usize;

pub struct CudaProcessor {
    device: Device,
    context: Context,
    hash_module: Module,
    trial_decrypt_module: Module,
    stream: Stream,
    generators: DeviceBuffer<u8>,
}

unsafe impl Send for CudaProcessor {}

impl CudaProcessor {
    pub fn new() -> Result<Self> {
        let r = Self::new_inner();
        if let Err(ref err) = r {
            log::info!("CUDA Initialization Error {:?}", err);
        }
        log::info!("CUDA Initialized");
        r
    }

    fn new_inner() -> Result<Self> {
        rustacuda::init(rustacuda::CudaFlags::empty())?;

        let device = Device::get_device(0)?;
        let context =
            Context::create_and_push(ContextFlags::MAP_HOST | ContextFlags::SCHED_AUTO, device)?;

        let ptx = CString::new(include_str!("../cuda/hash.ptx"))?;
        let hash_module = Module::load_from_string(&ptx)?;

        let ptx = CString::new(include_str!("../cuda/trial_decrypt.ptx"))?;
        let trial_decrypt_module = Module::load_from_string(&ptx)?;

        let stream = Stream::new(StreamFlags::DEFAULT, None)?;

        log::info!("Prepare Generators");
        let generators_len = GENERATORS_EXP.len();
        let mut gens = vec![0u8; generators_len * 128];
        for i in 0..generators_len {
            GENERATORS_EXP[i].copy_to_slice(&mut gens[i * 128..(i + 1) * 128]);
        }

        let generators = DeviceBuffer::from_slice(&gens)?;
        Ok(CudaProcessor {
            device,
            context,
            hash_module,
            trial_decrypt_module,
            stream,
            generators,
        })
    }

    pub fn total_memory(&self) -> Result<usize> {
        let mem = self.device.total_memory()?.saturating_sub(500_000_000); // leave 500 MB of GPU for other stuff;
        log::info!("Cuda memory {}", mem);
        Ok(mem)
    }

    pub fn batch_hash_cuda(&mut self, depth: u8, data: &[Hash]) -> Result<Vec<Hash>> {
        CurrentContext::set_current(&self.context)?;

        let n = data.len() / 2;
        let mut in_data = DeviceBuffer::from_slice(data)?;
        let mut out_data = unsafe { DeviceBuffer::<u8>::zeroed(n * 32 * 2)? };

        unsafe {
            // Launch the kernel again using the `function` form:
            let function_name = CString::new("pedersen_hash")?;
            let hash = self.hash_module.get_function(&function_name)?;

            let blocks = (n + THREADS_PER_BLOCK - 1) / THREADS_PER_BLOCK;

            let stream = &self.stream;
            let result = launch!(hash<<<(blocks as u32, 1, 1), (THREADS_PER_BLOCK as u32, 1, 1), 1024, stream>>>(
                n,
                depth,
                self.generators.as_device_ptr(),
                in_data.as_device_ptr(),
                out_data.as_device_ptr()
            ));
            result?;
        }
        self.stream.synchronize()?;

        let mut res = vec![0u8; n * 32 * 2];
        out_data.copy_to(&mut res)?;

        let mut p = vec![];
        let mut q: Vec<AffinePoint> = vec![AffinePoint::default(); n];
        for i in 0..n {
            let b = i * 64;
            let u = Fq::from_bytes(&res[b..b + 32].try_into().unwrap()).unwrap();
            let z = Fq::from_bytes(&res[b + 32..b + 64].try_into().unwrap()).unwrap();
            q[i].u = z;
            p.push(u);
        }
        BatchInverter::invert_with_internal_scratch(&mut q, |q| &mut q.u, |q| &mut q.v);
        let mut out = vec![];
        for i in 0..n {
            let hash: Hash = (p[i] * &q[i].u).to_bytes();
            // println!("{} {} {} {}", i, hex::encode(&data[i * 2]), hex::encode(&data[i * 2 + 1]), hex::encode(&hash));
            out.push(hash);
        }

        Ok(out)
    }

    pub fn trial_decrypt<'a, 'b, FVKIter: Iterator<Item = (&'b u32, &'b AccountViewKey)>>(
        &mut self,
        network: &Network,
        fvks: FVKIter,
        blocks: &'a [CompactBlock],
    ) -> Result<Vec<DecryptedBlock<'a>>> {
        CurrentContext::set_current(&self.context).unwrap();

        let n = blocks
            .iter()
            .map(|b| b.vtx.iter().map(|tx| tx.outputs.len()).sum::<usize>())
            .sum::<usize>();
        let block_count = (n + THREADS_PER_BLOCK - 1) / THREADS_PER_BLOCK;
        if n == 0 { return Ok(vec![]); }

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

        let mut data_buffer = vec![0u8; n * BUFFER_SIZE];
        let mut i = 0;
        for b in blocks.iter() {
            for tx in b.vtx.iter() {
                for co in tx.outputs.iter() {
                    data_buffer[i * BUFFER_SIZE..i * BUFFER_SIZE + 32].copy_from_slice(&co.epk);
                    data_buffer[i * BUFFER_SIZE + 64..i * BUFFER_SIZE + 116]
                        .copy_from_slice(&co.ciphertext);
                    i += 1;
                }
            }
        }

        let mut data_device_buffer = DeviceBuffer::from_slice(&data_buffer).unwrap();

        for (account, avk) in fvks {
            let fvk = &avk.fvk;
            let ivk = fvk.fvk.vk.ivk();

            let mut ivk_fr = ivk.0;
            ivk_fr = ivk_fr.double(); // multiply by cofactor
            ivk_fr = ivk_fr.double();
            ivk_fr = ivk_fr.double();
            let mut ivk_device_buffer = DeviceBuffer::from_slice(&ivk_fr.to_bytes()).unwrap();

            // decrypt all the blocks for the current account
            unsafe {
                // Launch the kernel again using the `function` form:
                let function_name = CString::new("trial_decrypt_full").unwrap();
                let trial_decrypt_full = self
                    .trial_decrypt_module
                    .get_function(&function_name)
                    .unwrap();

                let stream = &self.stream;
                let result = launch!(trial_decrypt_full<<<(block_count as u32, 1, 1), (THREADS_PER_BLOCK as u32, 1, 1), 0, stream>>>(
                n,
                ivk_device_buffer.as_device_ptr(),
                data_device_buffer.as_device_ptr()
            ));
                result.unwrap();
            }
            self.stream.synchronize().unwrap();

            data_device_buffer.copy_to(&mut data_buffer).unwrap();

            // merge the decrypted blocks
            let mut i = 0;
            for db in decrypted_blocks.iter_mut() {
                let b = db.compact_block;
                let mut decrypted_notes = vec![];
                let mut position_in_block = 0;
                let domain =
                    SaplingDomain::for_height(*network, BlockHeight::from_u32(b.height as u32));
                for (tx_index, tx) in b.vtx.iter().enumerate() {
                    for (output_index, co) in tx.outputs.iter().enumerate() {
                        let plaintext = &data_buffer[i * BUFFER_SIZE + 64..i * BUFFER_SIZE + 116];
                        // version and amount must be in range - 21 million ZEC is less than 0x0008 0000 0000 0000
                        if plaintext[0] <= 2 || plaintext[18] <= 0x07 || plaintext[19] != 0 {                           
                            if let Some((note, pa)) = domain.parse_note_plaintext_without_memo_ivk(&ivk, plaintext) {
                                let cmu = note.cmu().to_bytes();
                                if &cmu == co.cmu.as_slice() {
                                    decrypted_notes.push(DecryptedNote {
                                        account: *account,
                                        ivk: fvk.clone(),
                                        note,
                                        pa,
                                        position_in_block,
                                        viewonly: false,
                                        height: b.height as u32,
                                        txid: tx.hash.clone(),
                                        tx_index,
                                        output_index,
                                    });
                                }
                            }
                        }
                        i += 1;
                        position_in_block += 1;
                    }
                }
                db.notes.extend(decrypted_notes);
            }
        }

        Ok(decrypted_blocks)
    }
}

#[derive(Default, Clone)]
struct AffinePoint {
    u: Fq,
    v: Fq,
}
