use std::ffi::CStr;
use std::time::SystemTime;
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read, Write};
use std::mem::size_of;
use std::os::raw::c_void;
use std::path::Path;
use std::sync::Mutex;
use anyhow::{anyhow, Result};
use ash::Entry;
use ash::extensions::ext::DebugUtils;
use ash::util::read_spv;
use ash::vk::*;
use lazy_static::lazy_static;
use zcash_primitives::consensus::Network;
use zcash_primitives::sapling::SaplingIvk;
use crate::chain::DecryptedBlock;
use crate::CompactBlock;
use crate::gpu::{collect_nf, GPUProcessor};

pub const N: usize = 200_000; // must be a multiple of THREADS_PER_BLOCK
pub const DATA_SIZE: usize = 416usize;
pub const THREADS_PER_BLOCK: usize = 64usize;

#[repr(C)]
#[derive(Default)]
pub struct InputParams {
    n: u32,
    ivk: [u8; 32],
}

lazy_static! {
    pub static ref VULKAN_CONTEXT: Mutex<VulkanContext> =
        Mutex::new(VulkanContext::new(Path::new("")).unwrap());
}

pub struct VulkanContext {
    device: ash::Device,
    command_buffers: Vec<CommandBuffer>,
    queue: Queue,
    input_buffer_mem: DeviceMemory,
    input_buffer_mem_req: MemoryRequirements,
    data_buffer_mem: DeviceMemory,
    data_buffer_mem_req: MemoryRequirements,
}

impl VulkanContext {
    pub fn new(cache_dir: &Path) -> Result<Self> {
        unsafe {
            let entry = Entry::linked();
            let app_name = CStr::from_bytes_with_nul_unchecked(b"vulkan_compute\0");

            let layer_names = vec![
                CStr::from_bytes_with_nul_unchecked(b"VK_LAYER_KHRONOS_validation\0"),
            ];
            let layer_names: Vec<_> = layer_names.iter().map(|n| n.as_ptr()).collect();
            let extension_names = [DebugUtils::name().as_ptr()];

            let app_info = ApplicationInfo::builder()
                .application_name(app_name)
                .application_version(0)
                .engine_name(app_name)
                .engine_version(0)
                .api_version(make_api_version(0, 1, 0, 0));

            let create_info = InstanceCreateInfo::builder()
                .application_info(&app_info)
                .enabled_layer_names(&layer_names)
                .enabled_extension_names(&extension_names)
                ;
            let instance = entry.create_instance(&create_info, None)?;

            let debug_info = DebugUtilsMessengerCreateInfoEXT::builder()
                .message_severity(DebugUtilsMessageSeverityFlagsEXT::ERROR |
                    DebugUtilsMessageSeverityFlagsEXT::WARNING |
                    DebugUtilsMessageSeverityFlagsEXT::INFO)
                .message_type(DebugUtilsMessageTypeFlagsEXT::GENERAL |
                    DebugUtilsMessageTypeFlagsEXT::PERFORMANCE |
                    DebugUtilsMessageTypeFlagsEXT::VALIDATION)
                .pfn_user_callback(Some(vulkan_debug_cb));
            let debug_utils_loader = DebugUtils::new(&entry, &instance);
            let _debug_callback = debug_utils_loader.create_debug_utils_messenger(&debug_info, None)?;

            let phys_devices = instance.enumerate_physical_devices()?;
            let (phys_device, queue_family_index) = phys_devices.iter().find_map(|d|
                instance.get_physical_device_queue_family_properties(*d).iter().enumerate()
                    .find_map(|(i, info)| {
                        log::info!("{:?}", info.queue_flags);
                        let compute = info.queue_flags.contains(QueueFlags::COMPUTE);
                        let graphics = info.queue_flags.contains(QueueFlags::GRAPHICS);
                        if compute && !graphics { Some((*d, i)) } else { None }
                    })).ok_or(anyhow!("No suitable physical device"))?;

            let device_mem_props = instance.get_physical_device_memory_properties(phys_device);
            let queue_family_index = queue_family_index as u32;
            let queue_info = DeviceQueueCreateInfo::builder()
                .queue_family_index(queue_family_index)
                .queue_priorities(&[1.0]);
            let features = PhysicalDeviceFeatures {
                ..Default::default()
            };
            let device_create_info = DeviceCreateInfo::builder()
                .queue_create_infos(std::slice::from_ref(&queue_info))
                .enabled_features(&features);
            let device = instance.create_device(phys_device, &device_create_info, None)?;

            let ivk_buffer_create_info = BufferCreateInfo::builder()
                .size(DeviceSize::from(size_of::<InputParams>() as u32))
                .usage(BufferUsageFlags::STORAGE_BUFFER)
                .sharing_mode(SharingMode::EXCLUSIVE);
            let input_buffer = device.create_buffer(&ivk_buffer_create_info, None)?;
            let input_buffer_mem_req = device.get_buffer_memory_requirements(input_buffer);
            log::info!("input size = {} {}", size_of::<InputParams>() as u32, input_buffer_mem_req.size);
            let flags = MemoryPropertyFlags::HOST_COHERENT | MemoryPropertyFlags::HOST_VISIBLE;
            let input_buffer_mem_idx = device_mem_props.memory_types.iter().enumerate().find_map(|(i, mem_type)| {
                if (1 << i) & input_buffer_mem_req.memory_type_bits != 0 && mem_type.property_flags & flags == flags
                { Some(i as u32) } else { None }
            }).ok_or(anyhow!("No suitable memory type"))?;
            let input_buffer_allocate_info = MemoryAllocateInfo::builder()
                .allocation_size(input_buffer_mem_req.size)
                .memory_type_index(input_buffer_mem_idx);
            let input_buffer_mem = device.allocate_memory(&input_buffer_allocate_info, None)?;
            device.bind_buffer_memory(input_buffer, input_buffer_mem, 0)?;

            let data_buffer_create_info = BufferCreateInfo::builder()
                .size(DeviceSize::from((N * DATA_SIZE) as u64))
                .usage(BufferUsageFlags::STORAGE_BUFFER)
                .sharing_mode(SharingMode::EXCLUSIVE);
            let data_buffer = device.create_buffer(&data_buffer_create_info, None)?;
            let data_buffer_mem_req = device.get_buffer_memory_requirements(data_buffer);
            log::info!("data size = {}", data_buffer_mem_req.size);
            let flags = MemoryPropertyFlags::HOST_COHERENT | MemoryPropertyFlags::HOST_VISIBLE;
            let data_buffer_mem_idx = device_mem_props.memory_types.iter().enumerate().find_map(|(i, mem_type)| {
                if (1 << i) & data_buffer_mem_req.memory_type_bits != 0 && mem_type.property_flags & flags == flags
                { Some(i as u32) } else { None }
            }).ok_or(anyhow!("No suitable memory type"))?;
            let data_buffer_allocate_info = MemoryAllocateInfo::builder()
                .allocation_size(data_buffer_mem_req.size)
                .memory_type_index(data_buffer_mem_idx);
            let data_buffer_mem = device.allocate_memory(&data_buffer_allocate_info, None)?;
            device.bind_buffer_memory(data_buffer, data_buffer_mem, 0)?;

            let bindings = vec![
                DescriptorSetLayoutBinding::builder().binding(0).stage_flags(ShaderStageFlags::COMPUTE).descriptor_type(DescriptorType::STORAGE_BUFFER).descriptor_count(1).build(),
                DescriptorSetLayoutBinding::builder().binding(1).stage_flags(ShaderStageFlags::COMPUTE).descriptor_type(DescriptorType::STORAGE_BUFFER).descriptor_count(1).build(),
            ];

            let descriptor_set_layout_create_info = DescriptorSetLayoutCreateInfo::builder()
                .bindings(&bindings);
            let descriptor_set_layout = device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None)?;

            let pipeline_layout_create_info = PipelineLayoutCreateInfo::builder()
                .set_layouts(std::slice::from_ref(&descriptor_set_layout));
            let pipeline_layout = device.create_pipeline_layout(&pipeline_layout_create_info, None)?;

            let calc_pk_stage = {
                let mut module_file = Cursor::new(&include_bytes!("./vulkan/pk.spv"));
                let module_code = read_spv(&mut module_file)?;
                let module_create_info = ShaderModuleCreateInfo::builder()
                    .code(&module_code);
                log::info!("Compiling module pk");
                let module = device.create_shader_module(&module_create_info, None)?;

                PipelineShaderStageCreateInfo::builder()
                    .stage(ShaderStageFlags::COMPUTE)
                    .name(CStr::from_bytes_with_nul_unchecked(b"main\0"))
                    .module(module)
                    .build()
            };
            let calc_dh_secret_stage = {
                let mut module_file = Cursor::new(&include_bytes!("./vulkan/dhs.spv"));
                let module_code = read_spv(&mut module_file)?;
                let module_create_info = ShaderModuleCreateInfo::builder()
                    .code(&module_code);
                log::info!("Compiling module dhs");
                let module = device.create_shader_module(&module_create_info, None)?;

                PipelineShaderStageCreateInfo::builder()
                    .stage(ShaderStageFlags::COMPUTE)
                    .name(CStr::from_bytes_with_nul_unchecked(b"main\0"))
                    .module(module)
                    .build()
            };
            let decrypt_stage = {
                let mut module_file = Cursor::new(&include_bytes!("./vulkan/decrypt.spv"));
                let module_code = read_spv(&mut module_file)?;
                let module_create_info = ShaderModuleCreateInfo::builder()
                    .code(&module_code);
                log::info!("Compiling module decrypt");
                let module = device.create_shader_module(&module_create_info, None)?;

                PipelineShaderStageCreateInfo::builder()
                    .stage(ShaderStageFlags::COMPUTE)
                    .name(CStr::from_bytes_with_nul_unchecked(b"main\0"))
                    .module(module)
                    .build()
            };

            let shader_file = File::open(cache_dir.join("shader.dat"));
            let mut pipeline_data = vec![];
            if let Ok(shader_file) = shader_file {
                let mut shader_reader = BufReader::new(&shader_file);
                shader_reader.read_to_end(&mut pipeline_data)?;
            }
            let pipeline_cache_create_info = PipelineCacheCreateInfo::builder()
                .initial_data(&pipeline_data);
            let pipeline_cache = device.create_pipeline_cache(&pipeline_cache_create_info, None)?;
            let pk_pipeline_create_info = ComputePipelineCreateInfo::builder()
                .stage(calc_pk_stage)
                .layout(pipeline_layout)
                .build();
            let dhs_pipeline_create_info = ComputePipelineCreateInfo::builder()
                .stage(calc_dh_secret_stage)
                .layout(pipeline_layout)
                .build();
            let decrypt_pipeline_create_info = ComputePipelineCreateInfo::builder()
                .stage(decrypt_stage)
                .layout(pipeline_layout)
                .build();
            let compute_pipelines = device.create_compute_pipelines(pipeline_cache, &[pk_pipeline_create_info, dhs_pipeline_create_info, decrypt_pipeline_create_info], None)
                .map_err(|_| anyhow!("Cannot create pipeline"))?;

            let description_pool_size1 = DescriptorPoolSize::builder().ty(DescriptorType::STORAGE_BUFFER).descriptor_count(2).build();
            let description_pool_sizes = &[description_pool_size1];
            let descriptor_pool_create_info = DescriptorPoolCreateInfo::builder()
                .max_sets(1)
                .pool_sizes(description_pool_sizes)
                .flags(DescriptorPoolCreateFlags::default());
            let descriptor_pool = device.create_descriptor_pool(&descriptor_pool_create_info, None)?;

            let descriptor_set_allocate_info = DescriptorSetAllocateInfo::builder()
                .descriptor_pool(descriptor_pool)
                .set_layouts(std::slice::from_ref(&descriptor_set_layout));
            let descriptor_set = device.allocate_descriptor_sets(&descriptor_set_allocate_info)?[0];

            let input_buffer_info = DescriptorBufferInfo::builder()
                .buffer(input_buffer)
                .range(DeviceSize::from(size_of::<InputParams>() as u32));
            let input_descriptor = WriteDescriptorSet::builder()
                .descriptor_type(DescriptorType::STORAGE_BUFFER)
                .dst_set(descriptor_set)
                .dst_binding(0)
                .buffer_info(std::slice::from_ref(&input_buffer_info))
                .build();

            let data_buffer_info = DescriptorBufferInfo::builder()
                .buffer(data_buffer)
                .range(data_buffer_mem_req.size);
            let data_descriptor = WriteDescriptorSet::builder()
                .descriptor_type(DescriptorType::STORAGE_BUFFER)
                .dst_set(descriptor_set)
                .dst_binding(1)
                .buffer_info(std::slice::from_ref(&data_buffer_info))
                .build();
            device.update_descriptor_sets(&[input_descriptor, data_descriptor], &[]);

            let command_pool_create_info = CommandPoolCreateInfo::builder()
                .queue_family_index(queue_family_index);
            let command_pool = device.create_command_pool(&command_pool_create_info, None)?;

            let command_buffer_allocate_info = CommandBufferAllocateInfo::builder()
                .command_buffer_count(compute_pipelines.len() as u32)
                .command_pool(command_pool)
                .level(CommandBufferLevel::PRIMARY);
            let command_buffers = device.allocate_command_buffers(&command_buffer_allocate_info)?;
            for i in 0..compute_pipelines.len() {
                let compute_command_buffer = command_buffers[i];
                let pipeline = compute_pipelines[i];

                let command_buffer_begin_create_info = CommandBufferBeginInfo::builder();
                device.begin_command_buffer(compute_command_buffer, &command_buffer_begin_create_info)?;
                device.cmd_bind_descriptor_sets(compute_command_buffer, PipelineBindPoint::COMPUTE, pipeline_layout, 0,
                                                &[descriptor_set], &[]);
                device.cmd_bind_pipeline(compute_command_buffer, PipelineBindPoint::COMPUTE, pipeline);
                device.cmd_dispatch(compute_command_buffer, (N / THREADS_PER_BLOCK) as u32, 1, 1);
                device.end_command_buffer(compute_command_buffer)?;
            }

            let queue = device.get_device_queue(queue_family_index, 0);

            let cache_data = pipeline_cache;
            let shader_data = device.get_pipeline_cache_data(pipeline_cache)?;
            let shader_filename = cache_dir.join("shader.dat");
            log::info!("Shader file = {}", &shader_filename.display());
            let shader_file = File::create(&shader_filename)?;
            let mut shader_writer = BufWriter::new(&shader_file);
            shader_writer.write_all(&shader_data)?;

            Ok(VulkanContext {
                device,
                input_buffer_mem,
                input_buffer_mem_req,
                data_buffer_mem,
                data_buffer_mem_req,
                command_buffers,
                queue,
            })
        }
    }
}

pub struct VulkanProcessor {
    network: Network,
    decrypted_blocks: Vec<DecryptedBlock>,
    encrypted_data: Vec<u8>,
    decrypted_data: Vec<u8>,
    n: usize,
}

impl VulkanProcessor {
    pub fn setup_decrypt(network: &Network, blocks: Vec<CompactBlock>, cache_dir: &Path) -> anyhow::Result<Self> {
        log::info!("Vulkan::setup_decrypt");
        let n = blocks
            .iter()
            .map(|b| b.vtx.iter().map(|tx| tx.outputs.len()).sum::<usize>())
            .sum::<usize>();
        assert!(n <= N);

        let decrypted_blocks = collect_nf(blocks)?;

        let mut encrypted_data = vec![0u8; n * DATA_SIZE];
        let mut i = 0;
        for db in decrypted_blocks.iter() {
            let b = &db.compact_block;
            for tx in b.vtx.iter() {
                for co in tx.outputs.iter() {
                    encrypted_data[i * DATA_SIZE + 32..i * DATA_SIZE + 64].copy_from_slice(&co.epk);
                    encrypted_data[i * DATA_SIZE + 64..i * DATA_SIZE + 116].copy_from_slice(&co.ciphertext);
                    i += 1;
                }
            }
        }
        let decrypted_data = vec![0u8; n * DATA_SIZE];
        Ok(VulkanProcessor {
            network: network.clone(),
            decrypted_blocks,
            encrypted_data,
            decrypted_data,
            n,
        })
    }
}

impl GPUProcessor for VulkanProcessor {
    fn decrypt_account(&mut self, ivk: &SaplingIvk) -> Result<()> {
        unsafe {
            let vc = VULKAN_CONTEXT.lock().unwrap();

            let mut ivk_fr = ivk.0;
            ivk_fr = ivk_fr.double(); // multiply by cofactor
            ivk_fr = ivk_fr.double();
            ivk_fr = ivk_fr.double();
            let ivk = ivk_fr.to_bytes();

            let data_ptr = vc.device.map_memory(vc.data_buffer_mem, 0, vc.data_buffer_mem_req.size, MemoryMapFlags::default())?;
            data_ptr.copy_from(self.encrypted_data.as_ptr().cast(), self.n * DATA_SIZE);
            vc.device.unmap_memory(vc.data_buffer_mem);

            let input = InputParams {
                n: N as u32,
                ivk,
            };
            let input = &input as *const InputParams;
            let ivk_ptr = vc.device.map_memory(vc.input_buffer_mem, 0, vc.input_buffer_mem_req.size, MemoryMapFlags::empty())?;
            ivk_ptr.copy_from(input.cast(), size_of::<InputParams>());
            vc.device.unmap_memory(vc.input_buffer_mem);

            let submit_info = SubmitInfo::builder()
                .command_buffers(&vc.command_buffers);
            let fence_create_info = FenceCreateInfo::builder();
            let fence = vc.device.create_fence(&fence_create_info, None)?;
            // log::info!("Submit task");
            vc.device.queue_submit(vc.queue, std::slice::from_ref(&submit_info), fence)?;

            // log::info!("9 - Wait for result");
            vc.device.wait_for_fences(&[fence], true, u64::MAX)?;
            vc.device.destroy_fence(fence, None);
            vc.device.queue_wait_idle(vc.queue)?;

            let out_ptr = vc.device.map_memory(vc.data_buffer_mem, 0, vc.data_buffer_mem_req.size, MemoryMapFlags::default())?;
            out_ptr.copy_to(self.decrypted_data.as_mut_ptr().cast(), self.n * DATA_SIZE as usize);
            vc.device.unmap_memory(vc.data_buffer_mem);

            Ok(())
        }
    }

    fn get_decrypted_blocks(self) -> Result<Vec<DecryptedBlock>> {
        Ok(self.decrypted_blocks)
    }

    fn network(&self) -> Network {
        self.network
    }

    fn borrow_buffers(&mut self) -> (&[u8], &mut [DecryptedBlock]) {
        (&self.decrypted_data, &mut self.decrypted_blocks)
    }

    fn buffer_stride() -> usize {
        DATA_SIZE
    }
}

unsafe extern "system" fn vulkan_debug_cb(
    severity: DebugUtilsMessageSeverityFlagsEXT,
    tpe: DebugUtilsMessageTypeFlagsEXT,
    data: *const DebugUtilsMessengerCallbackDataEXT,
    _client_data: *mut c_void) -> Bool32 {
    let data = *data;
    let id = data.message_id_number as i32;
    let name = if data.p_message_id_name.is_null() { Cow::default() }
    else { CStr::from_ptr(data.p_message_id_name).to_string_lossy() };
    let message = if data.p_message.is_null() { Cow::default() }
    else { CStr::from_ptr(data.p_message).to_string_lossy() };

    log::info!("{:?} {:?} {} {} {}", severity, tpe, id, name, message);
    FALSE
}
