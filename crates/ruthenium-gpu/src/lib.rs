//! Ruthenium's Vulkan layer.
//!
//! The baseline is Vulkan 1.2 plus dynamic rendering, synchronization2 (core in 1.3, or
//! as extensions), timeline semaphores and the descriptor-indexing subset for one indexed
//! texture table: less than vanilla 26.3's Vulkan backend needs, so any GPU that runs
//! vanilla on Vulkan runs Ruthenium. Everything else (multi-draw indirect, shader draw
//! parameters, anisotropic filtering, newer extensions) is probed into [`DeviceCaps`] and
//! used only with a fallback.
//!
//! On macOS the same code runs on Metal through MoltenVK, enumerated as a portability
//! driver; the portability subset is enabled whenever the device reports it.

mod device;
mod frame;
mod frame_data;
mod instance;
mod memory;
mod pipeline;
mod readback;
mod screenshot;
mod swapchain;
mod textures;
mod upload;
mod upload_check;

pub use ash::vk;
pub use device::{
    DEFAULT_TEXTURE_SLOTS, DeviceCaps, Gpu, GpuOptions, MAX_TEXTURE_SLOTS, MIN_TEXTURE_SLOTS,
    OptionalFeatures, PretendSrgb, Rejected, SwapchainMaintenance, TextureTableLimits,
    TextureTablePath,
};
pub use frame::{
    ACQUIRE_TIMEOUT, Acquired, FrameCounters, FrameCtx, FrameLoop, FrameTimings, GpuTime, GpuZone,
    MAX_FRAME_WAITS, MAX_FRAMES_IN_FLIGHT, MAX_GPU_ZONES, Presented, SwapchainInfo, Target,
};
pub use frame_data::{FRAME_DATA_SET, FrameData};
pub use instance::{Instance, InstanceDesc, Validation, validation_messages};
pub use memory::{HeapReport, MemoryReport, texel_bytes};
pub use pipeline::{
    Blend, GraphicsDesc, PUSH_CONSTANT_BYTES, PUSH_CONSTANT_STAGES, PipelineCache, PipelineLayout,
    Pipelines,
};
pub use readback::Readback;
pub use screenshot::Screenshot;
pub use swapchain::{COLOR_RANGE, PresentPolicy};
pub use textures::{
    Arrival, MAX_MIP_LEVELS, SAMPLER_COUNT, SamplerId, TextureDesc, TextureId, TextureTable,
    TextureTableStats,
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
    /// Every slot of the texture table is taken, so no texture was created. `retiring` of
    /// them hold destroyed textures and come back once the frames that may still sample
    /// those have finished: with any, the caller can try again in a later frame.
    TextureTableFull { capacity: u32, retiring: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Vk { call, result } => write!(f, "{call} failed: {result}"),
            Error::Unsupported(why) => f.write_str(why),
            Error::Upload(e) => e.fmt(f),
            Error::TextureTableFull { capacity, retiring } => write!(
                f,
                "the texture table is full ({capacity} slots, {retiring} of them being freed)"
            ),
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

impl Error {
    /// Whether the same call can succeed in a later frame with nothing else changed: the
    /// upload ring was full for this frame, or the texture table has no free slot now
    /// (slots come back as destroyed textures retire, or as the owner of others lets go).
    pub fn try_again(&self) -> bool {
        matches!(
            self,
            Error::Upload(UploadError::Full) | Error::TextureTableFull { .. }
        )
    }
}

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
