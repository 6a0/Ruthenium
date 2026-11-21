//! Pipelines: one shared layout, a closed table built in parallel at start-up, and a
//! `VkPipelineCache` kept on disk so later launches skip shader compilation.
//!
//! Nothing here runs per frame. A frame never creates a pipeline (vanilla compiles a missing
//! pipeline in the middle of a frame).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use ash::vk::TaggedStructure;

use crate::device::Gpu;
use crate::{Error, VkContext};

/// Bytes in the single push-constant range of the shared layout: the minimum every Vulkan
/// device supports (`maxPushConstantsSize`).
pub const PUSH_CONSTANT_BYTES: u32 = 128;

/// Stages that may read push constants. `vkCmdPushConstants` must use exactly these.
pub const PUSH_CONSTANT_STAGES: vk::ShaderStageFlags = vk::ShaderStageFlags::from_raw(
    vk::ShaderStageFlags::VERTEX.as_raw()
        | vk::ShaderStageFlags::FRAGMENT.as_raw()
        | vk::ShaderStageFlags::COMPUTE.as_raw(),
);

/// `VkPipelineCacheHeaderVersionOne`: 16 bytes of ids, then the cache UUID.
const HEADER_BYTES: usize = 16 + vk::UUID_SIZE;

/// The driver's pipeline cache, loaded from and saved to one file per GPU and driver.
pub struct PipelineCache {
    gpu: Arc<Gpu>,
    raw: vk::PipelineCache,
    path: Option<PathBuf>,
    saved_len: usize,
}

impl std::fmt::Debug for PipelineCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineCache")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PipelineCache {
    /// Opens the cache for this GPU in `dir` (`None`: in memory only, nothing saved).
    /// A missing, unreadable or foreign file just starts an empty cache.
    pub fn open(gpu: Arc<Gpu>, dir: Option<&Path>) -> Result<PipelineCache, Error> {
        let caps = gpu.caps();
        let uuid: String = caps
            .pipeline_cache_uuid
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let path = dir.map(|d| {
            d.join(format!(
                "pipelines-{:04x}-{:04x}-{uuid}.bin",
                caps.vendor_id, caps.device_id
            ))
        });
        let data = match &path {
            Some(p) => match std::fs::read(p) {
                Ok(bytes)
                    if header_matches(
                        &bytes,
                        caps.vendor_id,
                        caps.device_id,
                        &caps.pipeline_cache_uuid,
                    ) =>
                {
                    bytes
                }
                Ok(_) => {
                    eprintln!(
                        "[gpu] ignoring {}: made by another GPU or driver",
                        p.display()
                    );
                    Vec::new()
                }
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        let info = vk::PipelineCacheCreateInfo::default().initial_data(&data);
        // SAFETY: the data is empty or carries this device's cache header (the driver
        // validates the rest).
        let raw = unsafe { gpu.device.create_pipeline_cache(&info, None) }
            .ctx("vkCreatePipelineCache")?;
        Ok(PipelineCache {
            gpu,
            raw,
            path,
            saved_len: data.len(),
        })
    }

    pub fn raw(&self) -> vk::PipelineCache {
        self.raw
    }

    /// Writes the cache to disk if it grew since it was loaded or last saved. The file is
    /// replaced atomically (written beside it, then renamed).
    pub fn save(&mut self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        // SAFETY: live cache; the driver synchronises access internally.
        let data = unsafe { self.gpu.device.get_pipeline_cache_data(self.raw) }
            .map_err(|e| e.to_string())?;
        if data.len() == self.saved_len {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &data).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.saved_len = data.len();
        Ok(())
    }
}

impl Drop for PipelineCache {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            eprintln!("[gpu] could not save the pipeline cache: {e}");
        }
        // SAFETY: pipeline creation that used the cache has returned.
        unsafe { self.gpu.device.destroy_pipeline_cache(self.raw, None) };
    }
}

fn header_matches(data: &[u8], vendor: u32, device: u32, uuid: &[u8; vk::UUID_SIZE]) -> bool {
    if data.len() < HEADER_BYTES {
        return false;
    }
    let word = |i: usize| u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    word(0) as usize >= HEADER_BYTES
        && word(1) == vk::PipelineCacheHeaderVersion::ONE.as_raw() as u32
        && word(2) == vendor
        && word(3) == device
        && data[16..HEADER_BYTES] == uuid[..]
}

/// How a pipeline's colour output combines with the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// Overwrite.
    Opaque,
    /// Straight alpha: `src.a * src + (1 - src.a) * dst`; alpha accumulates likewise.
    Alpha,
}

/// One graphics pipeline of the closed set: what differs between pipelines. Everything
/// else is shared: the layout, dynamic viewport and scissor, no depth yet, one colour
/// attachment in the target's format.
#[derive(Clone, Copy, Debug)]
pub struct GraphicsDesc {
    pub name: &'static str,
    pub vertex: &'static [u32],
    pub fragment: &'static [u32],
    pub topology: vk::PrimitiveTopology,
    pub blend: Blend,
}

/// The shared pipeline layout and every pipeline, built for one colour format.
pub struct Pipelines {
    gpu: Arc<Gpu>,
    layout: vk::PipelineLayout,
    pipelines: Vec<vk::Pipeline>,
    format: vk::Format,
}

impl std::fmt::Debug for Pipelines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipelines")
            .field("count", &self.pipelines.len())
            .field("format", &self.format)
            .finish()
    }
}

impl Pipelines {
    /// Builds every pipeline in `descs` (index = id) for `format`, in parallel, through
    /// `cache`; then saves the cache if it grew.
    pub fn build(
        gpu: Arc<Gpu>,
        cache: &mut PipelineCache,
        descs: &[GraphicsDesc],
        format: vk::Format,
    ) -> Result<Pipelines, Error> {
        let started = Instant::now();
        let ranges = [vk::PushConstantRange::default()
            .stage_flags(PUSH_CONSTANT_STAGES)
            .size(PUSH_CONSTANT_BYTES)];
        let info = vk::PipelineLayoutCreateInfo::default().push_constant_ranges(&ranges);
        // SAFETY: plain object creation; destroyed in Drop.
        let layout = unsafe { gpu.device.create_pipeline_layout(&info, None) }
            .ctx("vkCreatePipelineLayout")?;
        let mut set = Pipelines {
            gpu,
            layout,
            pipelines: Vec::with_capacity(descs.len()),
            format,
        };

        let cache_raw = cache.raw();
        let results: Vec<Result<vk::Pipeline, Error>> = std::thread::scope(|s| {
            let handles: Vec<_> = descs
                .iter()
                .map(|d| {
                    let gpu = &set.gpu;
                    s.spawn(move || build_one(gpu, cache_raw, layout, d, format))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("pipeline build thread panicked"))
                .collect()
        });
        // Keep what was built so Drop frees it even if another one failed.
        let mut first_error = None;
        for r in results {
            match r {
                Ok(p) => set.pipelines.push(p),
                Err(e) => {
                    set.pipelines.push(vk::Pipeline::null());
                    first_error.get_or_insert(e);
                }
            }
        }
        if let Some(e) = first_error {
            return Err(e);
        }
        if let Err(e) = cache.save() {
            eprintln!("[gpu] could not save the pipeline cache: {e}");
        }
        eprintln!(
            "[gpu] {} pipeline(s) for {format:?} in {:.1} ms",
            descs.len(),
            started.elapsed().as_secs_f64() * 1e3
        );
        Ok(set)
    }

    pub fn get(&self, index: usize) -> vk::Pipeline {
        self.pipelines[index]
    }

    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }

    pub fn format(&self) -> vk::Format {
        self.format
    }
}

impl Drop for Pipelines {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        // SAFETY: the owner drops Pipelines only once no submitted frame uses them (the
        // frame loop waits for the GPU before it is dropped or before a rebuild).
        unsafe {
            for &p in &self.pipelines {
                if p != vk::Pipeline::null() {
                    d.destroy_pipeline(p, None);
                }
            }
            d.destroy_pipeline_layout(self.layout, None);
        }
    }
}

fn shader_module(gpu: &Gpu, name: &str, code: &[u32]) -> Result<vk::ShaderModule, Error> {
    let info = vk::ShaderModuleCreateInfo::default().code(code);
    // SAFETY: the code is SPIR-V compiled for Vulkan 1.2 by `cargo xtask shaders`.
    unsafe { gpu.device.create_shader_module(&info, None) }.map_err(|result| {
        eprintln!("[gpu] shader module for {name} failed");
        Error::Vk {
            call: "vkCreateShaderModule",
            result,
        }
    })
}

fn build_one(
    gpu: &Gpu,
    cache: vk::PipelineCache,
    layout: vk::PipelineLayout,
    d: &GraphicsDesc,
    format: vk::Format,
) -> Result<vk::Pipeline, Error> {
    let vs = shader_module(gpu, d.name, d.vertex)?;
    let fs = match shader_module(gpu, d.name, d.fragment) {
        Ok(m) => m,
        Err(e) => {
            // SAFETY: just created, unused.
            unsafe { gpu.device.destroy_shader_module(vs, None) };
            return Err(e);
        }
    };
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs)
            .name(c"main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs)
            .name(c"main"),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(d.topology);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    let attachment = match d.blend {
        Blend::Opaque => vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA),
        Blend::Alpha => vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA),
    };
    let attachments = [attachment];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let formats = [format];
    let mut rendering =
        vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .push(&mut rendering);
    // SAFETY: every pointer in info outlives the call; the cache is internally synchronised,
    // so threads may build through it at once.
    let result = unsafe {
        gpu.device
            .create_graphics_pipelines(cache, std::slice::from_ref(&info), None)
    };
    // SAFETY: modules may be destroyed once the pipeline is created.
    unsafe {
        gpu.device.destroy_shader_module(vs, None);
        gpu.device.destroy_shader_module(fs, None);
    }
    match result {
        Ok(p) => Ok(p[0]),
        Err((_, result)) => {
            eprintln!("[gpu] pipeline {} failed", d.name);
            Err(Error::Vk {
                call: "vkCreateGraphicsPipelines",
                result,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(vendor: u32, device: u32, uuid: [u8; 16]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&vendor.to_le_bytes());
        h.extend_from_slice(&device.to_le_bytes());
        h.extend_from_slice(&uuid);
        h.extend_from_slice(&[0xAB; 64]);
        h
    }

    #[test]
    fn cache_header_must_match_the_device() {
        let uuid = [7u8; 16];
        let h = header(0x10de, 0x28e0, uuid);
        assert!(header_matches(&h, 0x10de, 0x28e0, &uuid));
        assert!(!header_matches(&h, 0x8086, 0x28e0, &uuid));
        assert!(!header_matches(&h, 0x10de, 0x468b, &uuid));
        assert!(!header_matches(&h, 0x10de, 0x28e0, &[8u8; 16]));
        assert!(!header_matches(&h[..20], 0x10de, 0x28e0, &uuid));
        assert!(!header_matches(&[], 0x10de, 0x28e0, &uuid));
    }
}
