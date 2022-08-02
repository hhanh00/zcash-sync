use std::sync::Mutex;
use lazy_static::lazy_static;

mod processor;
use processor::CudaProcessor;

lazy_static! {
    pub static ref CUDA_PROCESSOR: Mutex<Option<CudaProcessor>> = Mutex::new(CudaProcessor::new().ok());
}
