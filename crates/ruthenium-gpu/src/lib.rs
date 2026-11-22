//! Ruthenium's Vulkan layer.
//!
//! The baseline is Vulkan 1.2 plus dynamic rendering, synchronization2 (core in 1.3, or
//! as extensions), timeline semaphores, the descriptor-indexing subset for one bindless
//! table, multi-draw-indirect and shader draw parameters: the same envelope vanilla 26.3's
//! Vulkan backend needs, so any GPU that runs vanilla on Vulkan runs Ruthenium. Newer
//! features are probed into [`DeviceCaps`] and used only with a fallback.
//!
//! On macOS the same code runs on Metal through MoltenVK, enumerated as a portability
//! driver; the portability subset is enabled whenever the device reports it.

mod device;
mod frame;
mod frame_data;
mod instance;
mod memory;
mod pipeline;
mod screenshot;
mod swapchain;
mod textures;
mod upload;
mod upload_check;

pub use ash::vk;
pub use device::{DeviceCaps, Gpu, GpuOptions, Rejected};
pub use frame::{
    FrameCtx, FrameLoop, GpuTime, GpuZone, MAX_FRAME_WAITS, MAX_FRAMES_IN_FLIGHT, MAX_GPU_ZONES,
    Presented, Target,
};
pub use frame_data::{FRAME_DATA_SET, FrameData};
pub use instance::{Instance, InstanceDesc, Validation, validation_messages};
pub use memory::texel_bytes;
pub use pipeline::{
    Blend, GraphicsDesc, PUSH_CONSTANT_BYTES, PUSH_CONSTANT_STAGES, PipelineCache, PipelineLayout,
    Pipelines,
};
pub use screenshot::Screenshot;
pub use swapchain::{COLOR_RANGE, PresentPolicy};
pub use textures::{
    Arrival, MAX_MIP_LEVELS, SAMPLER_COUNT, SamplerId, TextureDesc, TextureId, TextureTable,
};
pub use upload::{ImageDst, ImageRegion, UploadError, UploadRing, UploadTicket};
pub use upload_check::{UploadCheck, UploadCheckReport};

use std::fmt;

/// A Vulkan or device-selection failure.
#[derive(Debug)]
pub enum Error {
    /// A Vulkan call returned an error.
    Vk {
        call: &'static str,
        result: vk::Result,
    },
    /// The Vulkan loader or instance is unusable (too old, missing extensions, …).
    Unsupported(String),
    /// No GPU meets the baseline; one entry per GPU with its reasons.
    NoSuitableDevice(Vec<Rejected>),
    /// Data could not be staged for upload.
    Upload(UploadError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Vk { call, result } => write!(f, "{call} failed: {result}"),
            Error::Unsupported(why) => f.write_str(why),
            Error::Upload(e) => e.fmt(f),
            Error::NoSuitableDevice(rejected) if rejected.is_empty() => {
                f.write_str("no Vulkan GPU found; update your graphics driver")
            }
            Error::NoSuitableDevice(rejected) => {
                writeln!(
                    f,
                    "no GPU supports what Ruthenium needs (Vulkan 1.2 with dynamic rendering, synchronization2 and descriptor indexing); updating the graphics driver usually fixes this:"
                )?;
                for r in rejected {
                    writeln!(f, "  {}: {}", r.name, r.reasons.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<UploadError> for Error {
    fn from(e: UploadError) -> Error {
        Error::Upload(e)
    }
}

/// Attaches the call name to a Vulkan result.
pub(crate) trait VkContext<T> {
    fn ctx(self, call: &'static str) -> Result<T, Error>;
}

impl<T> VkContext<T> for Result<T, vk::Result> {
    fn ctx(self, call: &'static str) -> Result<T, Error> {
        self.map_err(|result| Error::Vk { call, result })
    }
}

/// `major.minor.patch` of a Vulkan version number.
pub fn version_string(v: u32) -> String {
    format!(
        "{}.{}.{}",
        vk::api_version_major(v),
        vk::api_version_minor(v),
        vk::api_version_patch(v)
    )
}
