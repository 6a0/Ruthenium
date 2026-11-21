//! Ruthenium's renderer. For now: GPU start-up for a window, the pipeline set, and a frame
//! that draws the menu background, recorded and presented through
//! [`ruthenium_gpu::FrameLoop`].

mod pipelines;
mod stats;

pub use pipelines::PipelineId;
pub use stats::{FrameTimes, Summary};

use std::path::Path;
use std::sync::Arc;

use ruthenium_gpu::{
    COLOR_RANGE, FrameLoop, Gpu, GpuOptions, GpuTime, Instance, InstanceDesc, PUSH_CONSTANT_STAGES,
    PipelineCache, Pipelines, PresentPolicy, Presented, Validation, vk,
};
use ruthenium_platform::{Platform, Window};

/// Creates the Vulkan instance, the window's surface and the device.
///
/// `window` must outlive the returned Gpu (and everything created from it).
pub fn create_gpu(
    platform: &Platform,
    window: &Window,
    validation: Validation,
    options: &GpuOptions,
) -> Result<Arc<Gpu>, Box<dyn std::error::Error>> {
    let surface_extensions = platform.vulkan_instance_extensions();
    let instance = Instance::new(&InstanceDesc {
        get_instance_proc_addr: platform.vk_get_instance_proc_addr()?,
        surface_extensions: &surface_extensions,
        validation,
    })?;
    // SAFETY: the instance was created with SDL's surface extensions; Gpu takes ownership
    // of the surface and destroys it before the instance; the caller keeps the window alive.
    let surface = unsafe { window.create_surface(instance.handle())? };
    Ok(Arc::new(Gpu::new(instance, surface, options)?))
}

/// What one rendered frame reports back to the loop.
#[derive(Clone, Copy, Debug)]
pub struct FrameReport {
    pub presented: Presented,
    /// GPU time of an earlier frame that has just been read back.
    pub gpu_time: Option<GpuTime>,
}

/// The menu background: a vertical gradient between two display-encoded RGBA colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub top: [f32; 4],
    pub bottom: [f32; 4],
}

impl Background {
    fn push_constants(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        for (i, v) in self.top.iter().chain(&self.bottom).enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
        b
    }
}

/// Draws frames into the window.
#[derive(Debug)]
pub struct Renderer {
    // Fields drop in order: the frame loop first (its Drop waits for the GPU), then the
    // pipelines it used, then the cache, which saves itself as it goes.
    frames: FrameLoop,
    pipelines: Pipelines,
    cache: PipelineCache,
}

impl Renderer {
    /// Creates the frame loop and builds every pipeline through the on-disk pipeline cache
    /// in `cache_dir` (`None`: no cache file).
    pub fn new(
        gpu: Arc<Gpu>,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
        cache_dir: Option<&Path>,
    ) -> Result<Renderer, ruthenium_gpu::Error> {
        let frames = FrameLoop::new(gpu.clone(), pixel_size, policy)?;
        let mut cache = PipelineCache::open(gpu.clone(), cache_dir)?;
        let pipelines =
            Pipelines::build(gpu, &mut cache, &pipelines::descs(), frames.color_format()?)?;
        Ok(Renderer {
            frames,
            pipelines,
            cache,
        })
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        self.frames.gpu()
    }

    pub fn frames(&self) -> &FrameLoop {
        &self.frames
    }

    pub fn frames_mut(&mut self) -> &mut FrameLoop {
        &mut self.frames
    }

    /// Rebuilds the pipelines if the swapchain's format changed (rare: a different monitor
    /// or surface). Waits for earlier frames, which may still use the old pipelines.
    fn ensure_format(&mut self, format: vk::Format) -> Result<(), ruthenium_gpu::Error> {
        if format == self.pipelines.format() {
            return Ok(());
        }
        self.frames.wait_for_gpu(1)?;
        let gpu = self.frames.gpu().clone();
        self.pipelines = Pipelines::build(gpu, &mut self.cache, &pipelines::descs(), format)?;
        Ok(())
    }

    /// Records, submits and presents one frame showing `background`.
    pub fn render(&mut self, background: &Background) -> Result<FrameReport, ruthenium_gpu::Error> {
        profiling::scope!("render");
        let (ctx, gpu_time) = self.frames.begin()?;
        // Off-screen passes will be recorded here, before the late acquire.
        let target = self.frames.acquire(&ctx)?;
        if let Some(t) = target {
            self.ensure_format(t.format)?;
            let gpu = self.frames.gpu();
            let cmd = ctx.cmd;
            let to_attachment = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(t.image)
                .subresource_range(COLOR_RANGE)];
            let to_present = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                .dst_access_mask(vk::AccessFlags2::NONE)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(t.image)
                .subresource_range(COLOR_RANGE)];
            // The background covers every pixel, so the old contents are never loaded.
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(t.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)];
            let area = vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: t.extent,
            };
            let rendering = vk::RenderingInfo::default()
                .render_area(area)
                .layer_count(1)
                .color_attachments(&attachments);
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: t.extent.width as f32,
                height: t.extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let push = background.push_constants();
            let d = gpu.device();
            // SAFETY: cmd is recording; the image was acquired this frame and its acquire
            // semaphore is waited at COLOR_ATTACHMENT_OUTPUT, the first barrier's source stage;
            // the pipeline was built for the target's format with the shared layout.
            unsafe {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
                );
                gpu.cmd_begin_rendering(cmd, &rendering);
                d.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipelines.get(PipelineId::Background.index()),
                );
                d.cmd_set_viewport(cmd, 0, &[viewport]);
                d.cmd_set_scissor(cmd, 0, &[area]);
                d.cmd_push_constants(cmd, self.pipelines.layout(), PUSH_CONSTANT_STAGES, 0, &push);
                d.cmd_draw(cmd, 3, 1, 0, 0);
                gpu.cmd_end_rendering(cmd);
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                );
            }
        }
        let presented = self.frames.submit(ctx, target)?;
        Ok(FrameReport {
            presented,
            gpu_time,
        })
    }
}
