use crate::chain::{DecryptedBlock, DecryptedNote};
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
use std::sync::Mutex;
use lazy_static::lazy_static;
use zcash_note_encryption::Domain;
use zcash_primitives::consensus::{BlockHeight, Network};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use zcash_primitives::zip32::ExtendedFullViewingKey;
use crate::gpu::{collect_nf, GPUProcessor};

const THREADS_PER_BLOCK: usize = 256usize;
const BUFFER_SIZE: usize = 128usize;

lazy_static! {
    pub static ref CUDA_CONTEXT: Mutex<Option<CudaContext>> =
        Mutex::new(CudaContext::new().ok());
}

pub struct CudaContext {
    device: Device,
    context: Context,
    hash_module: Module,
    trial_decrypt_module: Module,
    stream: Stream,
    generators: DeviceBuffer<u8>,
}

unsafe impl Send for CudaContext {}

impl CudaContext {
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

        let ptx = CString::new(include_str!("./cuda/hash.ptx"))?;
        let hash_module = Module::load_from_string(&ptx)?;

        let ptx = CString::new(include_str!("./cuda/trial_decrypt.ptx"))?;
        let trial_decrypt_module = Module::load_from_string(&ptx)?;

        let stream = Stream::new(StreamFlags::DEFAULT, None)?;

        log::info!("Prepare Generators");
        let generators_len = GENERATORS_EXP.len();
        let mut gens = vec![0u8; generators_len * 128];
        for i in 0..generators_len {
            GENERATORS_EXP[i].copy_to_slice(&mut gens[i * 128..(i + 1) * 128]);
        }

        let generators = DeviceBuffer::from_slice(&gens)?;
        Ok(CudaContext {
            device,
            context,
            hash_module,
            trial_decrypt_module,
            stream,
            generators,
        })
    }

    pub fn total_memory(&self) -> Result<usize> {
        let mem = self.device.total_memory()?.saturating_sub(500_000_000);
        // leave 500 MB of GPU for other stuff;
        log::info!("Cuda memory {}", mem);
        Ok(mem)
    }

    pub fn batch_hash(&mut self, depth: u8, data: &[Hash]) -> Result<Vec<Hash>> {
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
}

pub struct CudaProcessor<'a> {
    network: Network,
    decrypted_blocks: Vec<DecryptedBlock<'a>>,
    encrypted_data: Vec<u8>,
    encrypted_data_device: DeviceBuffer<u8>,
    ivk_device: DeviceBuffer<u8>,
    n: usize,
    block_count: usize,
}

impl <'a> CudaProcessor<'a> {
    pub fn setup_decrypt(network: &Network, blocks: &'a [CompactBlock]) -> Result<Self> {
        let m = CUDA_CONTEXT.lock().unwrap();
        let cuda_context = m.as_ref().unwrap();
        CurrentContext::set_current(&cuda_context.context).unwrap();

        let n = blocks
            .iter()
            .map(|b| b.vtx.iter().map(|tx| tx.outputs.len()).sum::<usize>())
            .sum::<usize>();
        let block_count = (n + THREADS_PER_BLOCK - 1) / THREADS_PER_BLOCK;

        let decrypted_blocks = collect_nf(blocks)?;

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

        let encrypted_data_device = unsafe { DeviceBuffer::uninitialized(data_buffer.len())? };
        let ivk_device = unsafe { DeviceBuffer::zeroed(32)? };

        let this = CudaProcessor {
            network: network.clone(),
            decrypted_blocks,
            encrypted_data: data_buffer,
            encrypted_data_device: encrypted_data_device,
            ivk_device: ivk_device,
            n,
            block_count,
        };
        Ok(this)
    }
}

impl <'a> GPUProcessor<'a> for CudaProcessor<'a> {
    fn decrypt_account(&mut self, account: u32, fvk: &ExtendedFullViewingKey) -> Result<()> {
        let ivk = fvk.fvk.vk.ivk();
        let mut ivk_fr = ivk.0;
        ivk_fr = ivk_fr.double(); // multiply by cofactor
        ivk_fr = ivk_fr.double();
        ivk_fr = ivk_fr.double();

        self.encrypted_data_device.copy_from(&self.encrypted_data)?;
        self.ivk_device.copy_from(&ivk_fr.to_bytes())?;

        let m = CUDA_CONTEXT.lock().unwrap();
        let cuda_context = m.as_ref().unwrap();
        // decrypt all the blocks for the current account
        unsafe {
            // Launch the kernel again using the `function` form:
            let function_name = CString::new("trial_decrypt_full").unwrap();
            let trial_decrypt_full = cuda_context
                .trial_decrypt_module
                .get_function(&function_name)
                .unwrap();

            let stream = &cuda_context.stream;
            let result = launch!(trial_decrypt_full<<<(self.block_count as u32, 1, 1), (THREADS_PER_BLOCK as u32, 1, 1), 0, stream>>>(
                    self.n,
                    self.ivk_device.as_device_ptr(),
                    self.encrypted_data_device.as_device_ptr()
                ));
            result.unwrap();
        }
        cuda_context.stream.synchronize().unwrap();

        let mut output_buffer = vec![0u8; self.n * BUFFER_SIZE];
        self.encrypted_data_device.copy_to(&mut output_buffer).unwrap();

        // merge the decrypted blocks
        let mut i = 0;
        for db in self.decrypted_blocks.iter_mut() {
            let b = db.compact_block;
            let mut decrypted_notes = vec![];
            let mut position_in_block = 0;
            let domain =
                SaplingDomain::for_height(self.network, BlockHeight::from_u32(b.height as u32));
            for (tx_index, tx) in b.vtx.iter().enumerate() {
                for (output_index, co) in tx.outputs.iter().enumerate() {
                    let plaintext = &output_buffer[i * BUFFER_SIZE + 64..i * BUFFER_SIZE + 116];
                    // version and amount must be in range - 21 million ZEC is less than 0x0008 0000 0000 0000
                    if plaintext[0] <= 2 || plaintext[18] <= 0x07 || plaintext[19] != 0 {
                        if let Some((note, pa)) =
                        domain.parse_note_plaintext_without_memo_ivk(&ivk, plaintext)
                        {
                            let cmu = note.cmu().to_bytes();
                            if &cmu == co.cmu.as_slice() {
                                log::info!("Note {} {}", account, u64::from(note.value));
                                decrypted_notes.push(DecryptedNote {
                                    account,
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

        Ok(())
    }

    fn get_decrypted_blocks(self) -> Result<Vec<DecryptedBlock<'a>>> {
        Ok(self.decrypted_blocks)
    }
}

#[derive(Default, Clone)]
struct AffinePoint {
    u: Fq,
    v: Fq,
}
