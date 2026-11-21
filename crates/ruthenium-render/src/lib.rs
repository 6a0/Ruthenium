//! Ruthenium's renderer. For now: GPU start-up for a window, and a frame that clears the
//! swapchain image, recorded and presented through [`ruthenium_gpu::FrameLoop`].

mod stats;

pub use stats::{FrameTimes, Summary};

use std::sync::Arc;

use ruthenium_gpu::{
    COLOR_RANGE, FrameLoop, Gpu, GpuOptions, GpuTime, Instance, InstanceDesc, PresentPolicy,
    Presented, Validation, vk,
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

/// Draws frames into the window.
#[derive(Debug)]
pub struct Renderer {
    frames: FrameLoop,
}

impl Renderer {
    pub fn new(
        gpu: Arc<Gpu>,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
    ) -> Result<Renderer, ruthenium_gpu::Error> {
        Ok(Renderer {
            frames: FrameLoop::new(gpu, pixel_size, policy)?,
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

    /// Records, submits and presents one frame that clears the window to `clear` (linear
    /// UNORM RGBA, as written to the swapchain).
    pub fn render(&mut self, clear: [f32; 4]) -> Result<FrameReport, ruthenium_gpu::Error> {
        profiling::scope!("render");
        let (ctx, gpu_time) = self.frames.begin()?;
        // Off-screen passes will be recorded here, before the late acquire.
        let target = self.frames.acquire(&ctx)?;
        if let Some(t) = target {
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
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(t.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue { float32: clear },
                })];
            let rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent: t.extent,
                })
                .layer_count(1)
                .color_attachments(&attachments);
            // SAFETY: cmd is recording; the image was acquired this frame and its acquire
            // semaphore is waited at COLOR_ATTACHMENT_OUTPUT, the first barrier's source stage.
            unsafe {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
                );
                gpu.cmd_begin_rendering(cmd, &rendering);
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
