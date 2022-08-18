use std::convert::TryInto;
use std::sync::Mutex;
use std::mem;
use std::ptr::slice_from_raw_parts;
use std::time::SystemTime;
use jubjub::Fq;
use metal::*;
use objc::rc::autoreleasepool;
use rand::rngs::OsRng;
use ff::Field;
use lazy_static::lazy_static;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaChaRng;
use zcash_client_backend::encoding::decode_extended_full_viewing_key;
use zcash_note_encryption::Domain;
use zcash_primitives::consensus::{BlockHeight, MainNetwork, Network, Parameters};
use zcash_primitives::sapling::note_encryption::SaplingDomain;
use zcash_primitives::sapling::SaplingIvk;
use crate::chain::DecryptedBlock;
use crate::CompactBlock;
use crate::gpu::{collect_nf, GPUProcessor};

lazy_static! {
    pub static ref METAL_CONTEXT: Mutex<MetalContext> =
        Mutex::new(MetalContext::new());
}

pub const N: usize = 200_000;
const WIDTH: u64 = 256;

#[derive(Clone)]
pub struct CompactOutput {
    pub height: u32,
    pub epk: [u8; 32],
    pub cmu: [u8; 32],
    pub ciphertext: [u8; 52],
}

#[repr(C)]
#[derive(Clone)]
struct Data {
    key: [u8; 32],
    epk: [u8; 32],
    cipher: [u8; 64],
}

impl Default for Data {
    fn default() -> Self {
        Data {
            key: [0; 32],
            epk: [0; 32],
            cipher: [0; 64],
        }
    }
}

pub struct MetalContext {
    device: Device,
    command_queue: CommandQueue,
    kernel: Function,
    ivk_buffer: Buffer,
    data_buffer: Buffer,
}

unsafe impl Send for MetalContext {}

impl MetalContext {
    pub fn new() -> Self {
        let library_data = include_bytes!("./metal/main.metallib");

        let device = Device::system_default().expect("no device found");
        let command_queue = device.new_command_queue();

        let library = device.new_library_with_data(&*library_data).unwrap();
        let kernel = library.get_function("decrypt", None).unwrap();

        let ivk_buffer = device.new_buffer(32, MTLResourceOptions::CPUCacheModeDefaultCache);
        let data_buffer = device.new_buffer((N * MetalProcessor::buffer_stride()) as u64, MTLResourceOptions::CPUCacheModeDefaultCache);

        MetalContext {
            device,
            command_queue,
            kernel,
            ivk_buffer,
            data_buffer,
        }
    }
}

pub struct MetalProcessor {
    network: Network,
    decrypted_blocks: Vec<DecryptedBlock>,
    encrypted_data: Vec<Data>,
    decrypted_data: Vec<u8>,
    n: usize,
}

impl MetalProcessor {
    pub fn setup_decrypt(network: &Network, blocks: Vec<CompactBlock>) -> anyhow::Result<Self> {
        log::info!("Metal::setup_decrypt");
        let decrypted_blocks = collect_nf(blocks)?;

        let mut encrypted_data: Vec<Data> = vec![];
        for db in decrypted_blocks.iter() {
            let b = &db.compact_block;
            for tx in b.vtx.iter() {
                for co in tx.outputs.iter() {
                    let mut cipher = [0u8; 64];
                    cipher[0..52].copy_from_slice(&co.ciphertext);
                    let data = Data {
                        key: [0u8; 32],
                        epk: co.clone().epk.try_into().unwrap(),
                        cipher,
                    };
                    encrypted_data.push(data);
                }
            }
        }
        let n = encrypted_data.len();

        let mp = MetalProcessor {
            network: network.clone(),
            decrypted_blocks,
            encrypted_data,
            decrypted_data: vec![0u8; N * Self::buffer_stride()],
            n
        };
        Ok(mp)
    }
}

impl GPUProcessor for MetalProcessor {
    fn decrypt_account(&mut self, ivk: &SaplingIvk) -> anyhow::Result<()> {
        unsafe {
            let mc = METAL_CONTEXT.lock().unwrap();

            let mut ivk_fr = ivk.0;
            ivk_fr = ivk_fr.double(); // multiply by cofactor
            ivk_fr = ivk_fr.double();
            ivk_fr = ivk_fr.double();
            let ivk = ivk_fr.to_bytes();

            mc.ivk_buffer.contents().copy_from(ivk.as_ptr().cast(), 32);
            mc.data_buffer.contents().copy_from(self.encrypted_data.as_ptr().cast(), N * Self::buffer_stride());

            let command_buffer = mc.command_queue.new_command_buffer();

            let argument_encoder = mc.kernel.new_argument_encoder(0);
            let arg_buffer = mc.device.new_buffer(
                argument_encoder.encoded_length(),
                MTLResourceOptions::empty(),
            );
            argument_encoder.set_argument_buffer(&arg_buffer, 0);
            argument_encoder.set_buffer(0, &mc.ivk_buffer, 0);
            argument_encoder.set_buffer(1, &mc.data_buffer, 0);

            let encoder = command_buffer.new_compute_command_encoder();

            let pipeline_state_descriptor = ComputePipelineDescriptor::new();
            pipeline_state_descriptor.set_compute_function(Some(&mc.kernel));

            let pipeline_state = mc.device
                .new_compute_pipeline_state_with_function(
                    pipeline_state_descriptor.compute_function().unwrap(),
                )
                .unwrap();

            encoder.set_compute_pipeline_state(&pipeline_state);
            encoder.set_buffer(0, Some(&arg_buffer), 0);
            encoder.set_buffer(1, Some(&mc.data_buffer), 0);

            encoder.use_resource(&mc.ivk_buffer, MTLResourceUsage::Read);
            encoder.use_resource(&mc.data_buffer, MTLResourceUsage::Read | MTLResourceUsage::Write);

            let width = WIDTH.into();

            let thread_group_count = MTLSize {
                width: N as u64 / width,
                height: 1,
                depth: 1,
            };

            let thread_group_size = MTLSize {
                width,
                height: 1,
                depth: 1,
            };

            encoder.dispatch_thread_groups(thread_group_count, thread_group_size);
            encoder.end_encoding();

            command_buffer.commit();
            command_buffer.wait_until_completed();

            unsafe {
                let results = mc.data_buffer.contents() as *mut u8;
                let res = std::slice::from_raw_parts::<u8>(results.cast(), N * Self::buffer_stride());
                self.decrypted_data.copy_from_slice(&res);
            }
            Ok(())
        }
    }

    fn get_decrypted_blocks(self) -> anyhow::Result<Vec<DecryptedBlock>> {
        Ok(self.decrypted_blocks)
    }

    fn network(&self) -> Network {
        self.network
    }

    fn borrow_buffers(&mut self) -> (&[u8], &mut [DecryptedBlock]) {
        (&self.decrypted_data, &mut self.decrypted_blocks)
    }

    fn buffer_stride() -> usize {
        mem::size_of::<Data>()
    }
}

const TEST_FVK: &str = "zxviews1q0kl7tavzyqqpq8efe0vpgzwc37zj0zr9j2quurncpsy74tdvh9c3racve9yfv6gkssvekw4sz6ueenvup6whupguzkg5rgp0kma37r4uxz9472w4zwra4jv6fm5dc2cevfpjsxdgndagslmgdwudhv4stklzfeszrlcnsqxyr2qt8tsf4yxs3he4rzllcly7xagfmnlycvvnvhhr9l9j6ad693rkueqys9f7mkc7aacxwp3tfc9hpvlckxnj4nwu6jef2x98jefhcgmpkrmn";
const ZECPAGES_FVK: &str = "zxviews1q0duytgcqqqqpqre26wkl45gvwwwd706xw608hucmvfalr759ejwf7qshjf5r9aa7323zulvz6plhttp5mltqcgs9t039cx2d09mgq05ts63n8u35hyv6h9nc9ctqqtue2u7cer2mqegunuulq2luhq3ywjcz35yyljewa4mgkgjzyfwh6fr6jd0dzd44ghk0nxdv2hnv4j5nxfwv24rwdmgllhe0p8568sgqt9ckt02v2kxf5ahtql6s0ltjpkckw8gtymxtxuu9gcr0swvz";

pub fn test_co() -> CompactOutput {
    let mut cmu = hex::decode("263a4c43290ce7d644c0a3ab694bb4710a4c3b20a528e2297ac1d360b017f704").unwrap();
    cmu.reverse(); // epk was is given in MSB

    let mut epk = hex::decode("d8360fc851709bb8d53e1f7ad2bab2c28c70d2c3c570af6620599f078ab37e02").unwrap();
    epk.reverse();

    let ciphertext = hex::decode("c9c2479a4c936b25c4848a15fc5debad377f0305f7e744cfb550bc09da12922669b6a4d82d2c8d56d9c804682bae459474467aad").unwrap();

    CompactOutput {
        height: 500_000,
        epk: epk.try_into().unwrap(),
        cmu: cmu.try_into().unwrap(),
        ciphertext: ciphertext.try_into().unwrap(),
    }
}

fn main() {
    env_logger::init();
    let library_data = include_bytes!("./metal/main.metallib");

    let mut rng = ChaChaRng::from_seed([0; 32]);

    autoreleasepool(|| {
        let device = Device::system_default().expect("no device found");
        let command_queue = device.new_command_queue();

        let library = device.new_library_with_data(&*library_data).unwrap();
        let kernel = library.get_function("decrypt", None).unwrap();

        let fvk = decode_extended_full_viewing_key(Network::MainNetwork.hrp_sapling_extended_full_viewing_key(),
                                                   ZECPAGES_FVK).unwrap().unwrap();
        let ivk = fvk.fvk.vk.ivk();
        let mut ivk_fr = ivk.0;
        ivk_fr = ivk_fr.double(); // multiply by cofactor (8)
        ivk_fr = ivk_fr.double();
        ivk_fr = ivk_fr.double();
        let ivk8 = ivk_fr.to_bytes();

        println!("ivk8: {}", hex::encode(&ivk8));

        // let ivk8 = hex::decode("40c075fe695bf7135f70dc098fca6fab6a26774f8a070472579d00309386be1b").unwrap();
        // let mut ivk8 = [0u8; 32];
        // ivk8[0] = 1;
        // let x = Fq::random(&mut rng);
        // ivk8.copy_from_slice(&x.to_bytes());
        // let mut test_data = vec![Data::default(); n];
        // for i in 0..n {
        //     test_data[i].epk.copy_from_slice(&epk);
        //     test_data[i].cipher[0..52].copy_from_slice(&ciphertext);
        // }

        let mut test_data: Vec<Data> = vec![];
        let notes = vec![test_co()];
        for n in notes.iter() {
            let mut cipher = [0u8; 64];
            cipher[0..52].copy_from_slice(&n.ciphertext);
            let data = Data {
                key: [0u8; 32],
                epk: n.epk,
                cipher,
            };
            test_data.push(data);
        }
        let n = notes.len();

        let ivk_buffer = device.new_buffer_with_data(
            unsafe { mem::transmute(ivk8.as_ptr()) },
            32u64,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        let data_buffer = {
            device.new_buffer_with_data(
                unsafe { mem::transmute(test_data.as_ptr()) },
                (test_data.len() * mem::size_of::<Data>()) as u64,
                MTLResourceOptions::CPUCacheModeDefaultCache,
            )
        };

        let ptr = data_buffer.contents() as *mut u8;
        unsafe {
            let res: &[Data] = std::slice::from_raw_parts::<Data>(ptr.cast(), 1).try_into().unwrap();
            println!("Before {}", hex::encode(&res[0].epk));
        }

        let command_buffer = command_queue.new_command_buffer();

        let argument_encoder = kernel.new_argument_encoder(0);
        let arg_buffer = device.new_buffer(
            argument_encoder.encoded_length(),
            MTLResourceOptions::empty(),
        );
        argument_encoder.set_argument_buffer(&arg_buffer, 0);
        argument_encoder.set_buffer(0, &ivk_buffer, 0);
        argument_encoder.set_buffer(1, &data_buffer, 0);

        let encoder = command_buffer.new_compute_command_encoder();

        let pipeline_state_descriptor = ComputePipelineDescriptor::new();
        pipeline_state_descriptor.set_compute_function(Some(&kernel));

        let pipeline_state = device
            .new_compute_pipeline_state_with_function(
                pipeline_state_descriptor.compute_function().unwrap(),
            )
            .unwrap();

        encoder.set_compute_pipeline_state(&pipeline_state);
        encoder.set_buffer(0, Some(&arg_buffer), 0);
        encoder.set_buffer(1, Some(&data_buffer), 0);

        encoder.use_resource(&ivk_buffer, MTLResourceUsage::Read);
        encoder.use_resource(&data_buffer, MTLResourceUsage::Read | MTLResourceUsage::Write);

        let width = 256;

        let thread_group_count = MTLSize {
            width: (test_data.len() as u64 + width - 1) / width,
            height: 1,
            depth: 1,
        };

        let thread_group_size = MTLSize {
            width,
            height: 1,
            depth: 1,
        };

        encoder.dispatch_thread_groups(thread_group_count, thread_group_size);
        encoder.end_encoding();
        log::info!("Start - n = {}, n_groups = {}", n, thread_group_count.width);
        let stopwatch = SystemTime::now();
        command_buffer.commit();
        command_buffer.wait_until_completed();
        log::info!("Finish - {}", stopwatch.elapsed().unwrap().as_millis());

        let ptr = data_buffer.contents() as *mut u8;
        unsafe {
            let res: &[Data] = std::slice::from_raw_parts::<Data>(ptr.cast(), n).try_into().unwrap();
            let mut count = 0;
            for i in 0..n {
                let d = &res[i];
                // let product = Fq::from_bytes(&ivk8).unwrap() - Fq::from_bytes(&d.epk).unwrap();
                // let x = Fq::from_bytes(&d.key).unwrap();
                // println!("{} {} {}", hex::encode(&d.epk), hex::encode(product.to_bytes()), hex::encode(&d.key));
                // assert_eq!(product, Fq::from_bytes(&d.key).unwrap());
                // println!("{}", hex::encode(&d.cipher));
                let pt = &d.cipher;
                let domain = SaplingDomain::for_height(MainNetwork, BlockHeight::from_u32(500_000));
                if let Some((note, pa)) = domain.parse_note_plaintext_without_memo_ivk(&ivk, pt) {
                    if note.cmu().to_bytes() == notes[i].cmu.as_slice() {
                        // log::info!("{:?}", note);
                        // println!("{:?}", encode_payment_address(NETWORK.hrp_sapling_payment_address(), &pa));
                        count += 1;
                    }
                }
            }
            log::info!("COUNT = {}", count);
        }
    });
}
