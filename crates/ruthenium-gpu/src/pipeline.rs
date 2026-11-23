//! Pipelines: one shared layout, a closed table built in parallel at start-up, and a
//! `VkPipelineCache` kept on disk so later launches skip shader compilation. The cache
//! file is named after the GPU, the driver's cache id and a hash of everything Ruthenium
//! builds pipelines from ([`interface_hash`]): a build with other shaders or another
//! interface starts a cache of its own, cold, and the same build always finds its own,
//! warm.
//!
//! Nothing here runs per frame. A frame never creates a pipeline (vanilla compiles a missing
//! pipeline in the middle of a frame).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use ash::vk::TaggedStructure;

use crate::device::Gpu;
use crate::memory::Image;
use crate::textures::TextureTable;
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

/// Part of [`interface_hash`]: raise it when what [`build_one`] makes of a
/// [`GraphicsDesc`] changes (its fixed state, its attachments) in a way the hashed values
/// don't show.
const PIPELINE_STATE_VERSION: u32 = 1;

/// Cache files of other builds kept beside the current one, newest first: enough to
/// switch between a few builds (an A/B comparison) without any of them starting cold.
const OTHER_CACHES_KEPT: usize = 3;

/// A hash that is the same on every platform, toolchain and run (64-bit FNV-1a), for
/// naming files after what they were made from.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceHash(u64);

impl Default for InterfaceHash {
    fn default() -> InterfaceHash {
        InterfaceHash(0xcbf2_9ce4_8422_2325)
    }
}

impl InterfaceHash {
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut InterfaceHash {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut InterfaceHash {
        self.bytes(&v.to_le_bytes())
    }

    /// A string or a blob, with its length, so that two in a row can't run together.
    pub fn blob(&mut self, bytes: &[u8]) -> &mut InterfaceHash {
        self.u32(bytes.len() as u32).bytes(bytes)
    }

    /// SPIR-V words, with their count.
    pub fn words(&mut self, words: &[u32]) -> &mut InterfaceHash {
        self.u32(words.len() as u32);
        for &w in words {
            self.u32(w);
        }
        self
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// A hash of everything the pipelines in `descs` are built from: their compiled SPIR-V
/// and state, and the interface they share with the renderer (the push-constant range,
/// the texture table's bindings, size and samplers, the frame data's binding; vertex
/// formats once a pipeline has one). The driver's own cache id covers the rest.
pub fn interface_hash(descs: &[GraphicsDesc], textures: &TextureTable) -> u64 {
    let mut h = InterfaceHash::default();
    h.u32(PIPELINE_STATE_VERSION)
        .u32(PUSH_CONSTANT_BYTES)
        .u32(PUSH_CONSTANT_STAGES.as_raw());
    textures.hash_interface(&mut h);
    crate::frame_data::hash_interface(&mut h);
    h.u32(descs.len() as u32);
    for d in descs {
        h.blob(d.name.as_bytes())
            .u32(d.topology.as_raw() as u32)
            .u32(match d.blend {
                Blend::Opaque => 0,
                Blend::Alpha => 1,
            })
            .words(d.vertex)
            .words(d.fragment);
    }
    h.finish()
}

/// The start of the name of every cache file of one GPU model.
fn cache_file_prefix(vendor: u32, device: u32) -> String {
    format!("pipelines-{vendor:04x}-{device:04x}-")
}

/// The cache file of one GPU, driver cache id and interface hash.
fn cache_file_name(vendor: u32, device: u32, uuid: &[u8; vk::UUID_SIZE], interface: u64) -> String {
    let uuid: String = uuid.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}{uuid}-{interface:016x}.bin",
        cache_file_prefix(vendor, device)
    )
}

/// Which of `files` (name and modification time) to delete after saving `own`: cache
/// files of the same GPU model for other drivers or other builds, except the
/// [`OTHER_CACHES_KEPT`] used last.
fn stale_caches(
    mut files: Vec<(String, std::time::SystemTime)>,
    own: &str,
    prefix: &str,
) -> Vec<String> {
    files.retain(|(name, _)| name != own && name.starts_with(prefix) && name.ends_with(".bin"));
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    files
        .into_iter()
        .skip(OTHER_CACHES_KEPT)
        .map(|(name, _)| name)
        .collect()
}

/// The driver's pipeline cache, loaded from and saved to one file per GPU, driver and
/// interface hash.
pub struct PipelineCache {
    gpu: Arc<Gpu>,
    raw: vk::PipelineCache,
    path: Option<PathBuf>,
    saved_len: usize,
    interface: u64,
    loaded_len: usize,
}

impl std::fmt::Debug for PipelineCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineCache")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PipelineCache {
    /// Opens the cache of this GPU and `interface` ([`interface_hash`]) in `dir` (`None`:
    /// in memory only, nothing saved). A missing, unreadable or foreign file just starts
    /// an empty cache.
    pub fn open(gpu: Arc<Gpu>, dir: Option<&Path>, interface: u64) -> Result<PipelineCache, Error> {
        let caps = gpu.caps();
        let path = dir.map(|d| {
            d.join(cache_file_name(
                caps.vendor_id,
                caps.device_id,
                &caps.pipeline_cache_uuid,
                interface,
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
                    // Used now: pruning keeps the caches used last, and a warm cache
                    // may never be written again.
                    if let Ok(f) = std::fs::File::options().append(true).open(p) {
                        let _ = f.set_modified(std::time::SystemTime::now());
                    }
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
        match &path {
            Some(p) => eprintln!(
                "[gpu] pipeline cache {}: {}",
                p.file_name().unwrap_or_default().to_string_lossy(),
                match data.len() {
                    0 => "new (cold)".to_owned(),
                    n => format!("{:.1} KiB loaded (warm)", n as f64 / 1024.0),
                }
            ),
            None => eprintln!("[gpu] pipeline cache: off"),
        }
        Ok(PipelineCache {
            gpu,
            raw,
            path,
            saved_len: data.len(),
            interface,
            loaded_len: data.len(),
        })
    }

    pub fn raw(&self) -> vk::PipelineCache {
        self.raw
    }

    /// The interface hash in the cache's file name.
    pub fn interface(&self) -> u64 {
        self.interface
    }

    /// Bytes loaded from the file when the cache was opened: none on a cold start.
    pub fn loaded_bytes(&self) -> usize {
        self.loaded_len
    }

    /// Whether the cache is kept in a file.
    pub fn on_disk(&self) -> bool {
        self.path.is_some()
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
        self.prune(path);
        Ok(())
    }

    /// Deletes the cache files of this GPU model that other drivers and builds left
    /// behind, but for the few used last. Failures don't matter: they are only a cache.
    fn prune(&self, path: &Path) {
        let (Some(dir), Some(own)) = (path.parent(), path.file_name()) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let files = entries
            .flatten()
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((e.file_name().to_string_lossy().into_owned(), modified))
            })
            .collect();
        let caps = self.gpu.caps();
        let prefix = cache_file_prefix(caps.vendor_id, caps.device_id);
        for name in stale_caches(files, &own.to_string_lossy(), &prefix) {
            let _ = std::fs::remove_file(dir.join(name));
        }
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

/// The one pipeline layout every pipeline uses: the descriptor sets (set 0 is the texture
/// table) and the push-constant range. Created once and never rebuilt, so descriptor sets
/// bound in a frame stay valid when the pipelines are rebuilt.
pub struct PipelineLayout {
    gpu: Arc<Gpu>,
    raw: vk::PipelineLayout,
}

impl std::fmt::Debug for PipelineLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineLayout").finish_non_exhaustive()
    }
}

impl PipelineLayout {
    pub fn new(
        gpu: Arc<Gpu>,
        set_layouts: &[vk::DescriptorSetLayout],
    ) -> Result<PipelineLayout, Error> {
        let ranges = [vk::PushConstantRange::default()
            .stage_flags(PUSH_CONSTANT_STAGES)
            .size(PUSH_CONSTANT_BYTES)];
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(set_layouts)
            .push_constant_ranges(&ranges);
        // SAFETY: plain object creation; destroyed in Drop.
        let raw = unsafe { gpu.device.create_pipeline_layout(&info, None) }
            .ctx("vkCreatePipelineLayout")?;
        Ok(PipelineLayout { gpu, raw })
    }

    pub fn raw(&self) -> vk::PipelineLayout {
        self.raw
    }
}

impl Drop for PipelineLayout {
    fn drop(&mut self) {
        // SAFETY: the owner drops the layout after the pipelines built with it and once no
        // frame that bound it is pending (the frame loop waits in its Drop).
        unsafe { self.gpu.device.destroy_pipeline_layout(self.raw, None) };
    }
}

/// Every pipeline, built for one colour format with the shared layout.
pub struct Pipelines {
    gpu: Arc<Gpu>,
    pipelines: Vec<vk::Pipeline>,
    format: vk::Format,
    /// The 1×1 target of the warm-up draws, until they are recorded.
    warm_up: Option<Image>,
    warmed: bool,
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
    /// Builds every pipeline in `descs` (index = id) for `format` with `layout`, in
    /// parallel, through `cache`; then saves the cache if it grew.
    pub fn build(
        gpu: Arc<Gpu>,
        cache: &mut PipelineCache,
        layout: &PipelineLayout,
        descs: &[GraphicsDesc],
        format: vk::Format,
    ) -> Result<Pipelines, Error> {
        let started = Instant::now();
        let layout = layout.raw;
        let mut set = Pipelines {
            gpu,
            pipelines: Vec::with_capacity(descs.len()),
            format,
            warm_up: None,
            warmed: false,
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

    pub fn format(&self) -> vk::Format {
        self.format
    }

    /// Whether [`Self::record_warm_up`] has run for these pipelines.
    pub fn warmed_up(&self) -> bool {
        self.warmed
    }

    /// Records one draw with every pipeline into a 1×1 scratch target, with zeroed push
    /// constants (texture 0: the missing texture). Some drivers finish compiling, or upload
    /// shader code, only at a pipeline's first draw; this moves that to the first frame
    /// instead of the frame that first needs the pipeline (a HUD element, the world).
    /// Call it once, outside a render pass, before the frame's passes; later calls do
    /// nothing.
    ///
    /// # Safety
    /// `cmd` is recording a frame on the graphics queue, outside a render pass, with every
    /// descriptor set of `layout` (the layout these pipelines were built with) bound: the
    /// texture table, with its missing texture ready, and the frame's data.
    pub unsafe fn record_warm_up(
        &mut self,
        cmd: vk::CommandBuffer,
        layout: &PipelineLayout,
    ) -> Result<(), Error> {
        if self.warmed {
            return Ok(());
        }
        let started = Instant::now();
        let one = vk::Extent2D {
            width: 1,
            height: 1,
        };
        let target = Image::new_2d(
            &self.gpu,
            self.format,
            one,
            1,
            vk::ImageUsageFlags::COLOR_ATTACHMENT,
        )?;
        let gpu = &*self.gpu;
        let d = &gpu.device;
        let to_attachment = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::NONE)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .image(target.raw)
            .subresource_range(crate::swapchain::COLOR_RANGE)];
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(target.view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)];
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent: one,
        };
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let push = [0u8; PUSH_CONSTANT_BYTES as usize];
        // SAFETY: guaranteed by the caller; the target is alive until these pipelines are
        // dropped, which happens only once no submitted frame uses them.
        unsafe {
            gpu.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
            );
            gpu.cmd_begin_rendering(cmd, &rendering);
            d.cmd_set_viewport(cmd, 0, &[viewport]);
            d.cmd_set_scissor(cmd, 0, &[area]);
            d.cmd_push_constants(cmd, layout.raw, PUSH_CONSTANT_STAGES, 0, &push);
            for &p in &self.pipelines {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, p);
                d.cmd_draw(cmd, 3, 1, 0, 0);
            }
            gpu.cmd_end_rendering(cmd);
        }
        self.warm_up = Some(target);
        self.warmed = true;
        eprintln!(
            "[gpu] warm-up draws for {} pipeline(s) recorded in {:.2} ms",
            self.pipelines.len(),
            started.elapsed().as_secs_f64() * 1e3
        );
        Ok(())
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
            if let Some(t) = self.warm_up.take() {
                t.destroy(&self.gpu);
            }
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
    fn the_interface_hash_is_fnv_1a() {
        // Published test vectors of 64-bit FNV-1a.
        assert_eq!(InterfaceHash::default().finish(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(
            InterfaceHash::default().bytes(b"a").finish(),
            0xaf63_dc4c_8601_ec8c
        );
        assert_eq!(
            InterfaceHash::default().bytes(b"foobar").finish(),
            0x8594_4171_f739_67e8
        );
        // Lengths are part of it: the same bytes split differently hash differently.
        let split = |a: &[u8], b: &[u8]| InterfaceHash::default().blob(a).blob(b).finish();
        assert_ne!(split(b"ab", b"c"), split(b"a", b"bc"));
        let words = |w: &[u32]| InterfaceHash::default().words(w).finish();
        assert_ne!(words(&[1, 2]), words(&[1, 3]));
        assert_ne!(words(&[1]), words(&[1, 0]));
    }

    #[test]
    fn cache_files_are_named_after_gpu_driver_and_interface() {
        let uuid = [0xABu8; 16];
        let name = cache_file_name(0x10de, 0x28e0, &uuid, 0x0123_4567_89ab_cdef);
        assert_eq!(
            name,
            "pipelines-10de-28e0-abababababababababababababababab-0123456789abcdef.bin"
        );
        assert!(name.starts_with(&cache_file_prefix(0x10de, 0x28e0)));
        assert_ne!(name, cache_file_name(0x10de, 0x28e0, &uuid, 1));
    }

    #[test]
    fn old_caches_of_the_same_gpu_are_pruned() {
        let at = |s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let prefix = cache_file_prefix(0x10de, 0x28e0);
        let files = vec![
            (format!("{prefix}aa-5.bin"), at(5)),
            (format!("{prefix}aa-1.bin"), at(1)),
            (format!("{prefix}aa-own.bin"), at(9)),
            (format!("{prefix}aa-3.bin"), at(3)),
            // Before the hash was part of the name.
            (format!("{prefix}aa.bin"), at(2)),
            (format!("{prefix}aa-4.bin"), at(4)),
            // Another GPU's, and other files: never touched.
            ("pipelines-8086-468b-bb-1.bin".to_owned(), at(0)),
            (format!("{prefix}aa-0.tmp"), at(0)),
            ("servers.dat".to_owned(), at(0)),
        ];
        let mut stale = stale_caches(files, &format!("{prefix}aa-own.bin"), &prefix);
        stale.sort();
        assert_eq!(
            stale,
            vec![format!("{prefix}aa-1.bin"), format!("{prefix}aa.bin")]
        );
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
