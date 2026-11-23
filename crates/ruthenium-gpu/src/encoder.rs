//! The frame encoder: what the renderer records a frame's passes through. It issues the
//! Vulkan commands and owns what they need to be right (command buffer, pipeline layout,
//! image layouts, barriers, attachments, viewport), so the renderer says which target to
//! draw into and which pipeline draws what, and holds no raw handle.
//!
//! It wraps what the menus draw today: full-target passes into the window, into a
//! render-target texture of the table or into a read-back target, and draws that pull
//! everything from push constants, the frame's data and the texture table. Depth, vertex
//! and index buffers and indirect draws come with their first users.

use std::sync::Arc;

use ash::vk;

use crate::Error;
use crate::device::Gpu;
use crate::frame::{FrameCtx, Target};
use crate::frame_data::FrameData;
use crate::pipeline::{PUSH_CONSTANT_BYTES, PUSH_CONSTANT_STAGES, PipelineLayout, Pipelines};
use crate::readback::Readback;
use crate::screenshot::Screenshot;
use crate::swapchain::COLOR_RANGE;
use crate::textures::{TextureId, TextureTable};

/// Records the passes of one frame. See the module docs.
pub struct Encoder<'f> {
    gpu: Arc<Gpu>,
    ctx: &'f FrameCtx,
    layout: vk::PipelineLayout,
}

impl std::fmt::Debug for Encoder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encoder")
            .field("frame", &self.ctx.number)
            .finish_non_exhaustive()
    }
}

impl<'f> Encoder<'f> {
    /// Starts encoding the passes of the frame `ctx` records: binds the frame's copy of
    /// the texture table and the frame's data, which stay bound for every pass.
    ///
    /// # Safety
    /// `layout` was made of `textures.layout()` and `frame_data.layout()`, in that order;
    /// `textures.begin_frame` and `frame_data.begin_frame` have run for this frame; every
    /// [`Pipelines`] this encoder is given was built with `layout`.
    pub unsafe fn begin(
        gpu: &Arc<Gpu>,
        ctx: &'f FrameCtx,
        layout: &PipelineLayout,
        textures: &TextureTable,
        frame_data: &FrameData,
    ) -> Encoder<'f> {
        // SAFETY: ctx is a frame being recorded (only FrameLoop::begin makes one, and
        // submitting consumes it); the rest is the caller's guarantee.
        unsafe {
            textures.bind(ctx.cmd, ctx.slot(), layout.raw());
            frame_data.bind(ctx.cmd, layout.raw());
        }
        Encoder {
            gpu: gpu.clone(),
            ctx,
            layout: layout.raw(),
        }
    }

    /// Draws once with every pipeline into a scratch target, the first time it is called
    /// for `pipelines` (see [`Pipelines::warmed_up`]).
    pub fn warm_up(&mut self, pipelines: &mut Pipelines) -> Result<(), Error> {
        // SAFETY: outside a pass (one borrows the encoder), sets bound, and the pipelines
        // were built with the layout (begin's contract).
        unsafe { pipelines.record_warm_up(self.ctx.cmd, self.layout) }
    }

    /// A pass over the whole window image `target`, whose old contents are not kept (the
    /// draws cover it). Finishing the pass leaves the image ready to present.
    ///
    /// # Panics
    /// If `pipelines` were built for another format than the window's.
    pub fn window_pass<'e>(&'e mut self, pipelines: &'e Pipelines, target: &Target) -> Pass<'e> {
        assert_eq!(
            pipelines.format(),
            target.format,
            "pipelines for the window"
        );
        // The acquire semaphore is waited at COLOR_ATTACHMENT_OUTPUT, this barrier's
        // source stage, so the transition comes after the image is acquired.
        self.pass(
            pipelines,
            target.image,
            target.view,
            target.extent,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            vk::AttachmentLoadOp::DONT_CARE,
            End::Window(*target),
        )
    }

    /// A pass over the whole render-target texture `id` ([`TextureTable::create_target`]),
    /// whose old contents are not kept. Finishing the pass leaves it ready for sampling.
    ///
    /// # Panics
    /// If `id` is not a live render target, or `pipelines` were built for another format.
    pub fn texture_pass<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        textures: &TextureTable,
        id: TextureId,
    ) -> Pass<'e> {
        let (image, view, desc) = textures.target(id);
        assert_eq!(pipelines.format(), desc.format, "pipelines for {id:?}");
        let extent = vk::Extent2D {
            width: desc.width,
            height: desc.height,
        };
        // Earlier fragment-shader reads of the texture finish before it is overwritten.
        self.pass(
            pipelines,
            image,
            view,
            extent,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            vk::AttachmentLoadOp::DONT_CARE,
            End::Sampled(image),
        )
    }

    /// A pass over the whole read-back target, cleared to transparent black first.
    /// Finishing the pass records the copy back.
    ///
    /// # Panics
    /// If a copy of `target` is still pending, or `pipelines` were built for another
    /// format.
    pub fn readback_pass<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        target: &'e mut Readback,
    ) -> Pass<'e> {
        assert!(
            !target.pending(),
            "one copy of a read-back target at a time"
        );
        assert_eq!(
            pipelines.format(),
            target.format(),
            "pipelines for the target"
        );
        let (image, view) = target.image();
        // After the copy that read the target last.
        self.pass(
            pipelines,
            image,
            view,
            target.extent(),
            vk::PipelineStageFlags2::COPY,
            vk::AttachmentLoadOp::CLEAR,
            End::Readback(target),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pass<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        image: vk::Image,
        view: vk::ImageView,
        extent: vk::Extent2D,
        after: vk::PipelineStageFlags2,
        load: vk::AttachmentLoadOp,
        end: End<'e>,
    ) -> Pass<'e> {
        let to_attachment = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(after)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(load)
            .store_op(vk::AttachmentStoreOp::STORE)];
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let gpu = &*self.gpu;
        let cmd = self.ctx.cmd;
        // SAFETY: the frame is being recorded and no pass is open (a pass borrows the
        // encoder until it is finished); the image and its view are alive, as their
        // owners (a window target of this frame, the texture table, the read-back
        // target) keep them at least until the frame has finished; leaving UNDEFINED is
        // valid whatever layout the image is in.
        unsafe {
            gpu.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
            );
            gpu.cmd_begin_rendering(cmd, &rendering);
            gpu.device.cmd_set_viewport(cmd, 0, &[viewport]);
            gpu.device.cmd_set_scissor(cmd, 0, &[area]);
        }
        Pass {
            gpu: &self.gpu,
            ctx: self.ctx,
            layout: self.layout,
            pipelines,
            bound: None,
            end: Some(end),
        }
    }
}

/// How a pass leaves its target.
enum End<'e> {
    /// Ready to present, or copied for a screenshot first.
    Window(Target),
    /// Ready for sampling.
    Sampled(vk::Image),
    /// Copied back.
    Readback(&'e mut Readback),
}

/// One pass being recorded: draws into the target it was begun on. Dropping it finishes
/// it, as [`Self::finish`] does.
pub struct Pass<'e> {
    gpu: &'e Arc<Gpu>,
    ctx: &'e FrameCtx,
    layout: vk::PipelineLayout,
    pipelines: &'e Pipelines,
    /// The pipeline bound last.
    bound: Option<usize>,
    /// `None` once finished.
    end: Option<End<'e>>,
}

impl std::fmt::Debug for Pass<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pass")
            .field("frame", &self.ctx.number)
            .finish_non_exhaustive()
    }
}

impl Pass<'_> {
    /// Draws `instances` instances of `vertices` vertices with pipeline `pipeline` (its
    /// index in the set) and `push` as its push constants.
    ///
    /// # Safety
    /// `push` is what the pipeline's shaders expect; every element of the frame's data
    /// they read through it was written this frame; every texture index they sample is
    /// inside the table and not the pass's own target.
    ///
    /// # Panics
    /// If `push` is larger than the layout's range or not whole words, or `pipeline` is
    /// not in the set.
    pub unsafe fn draw(&mut self, pipeline: usize, push: &[u8], vertices: u32, instances: u32) {
        assert!(
            push.len() <= PUSH_CONSTANT_BYTES as usize && push.len().is_multiple_of(4),
            "{} bytes of push constants",
            push.len()
        );
        let raw = self.pipelines.get(pipeline);
        let d = &self.gpu.device;
        let cmd = self.ctx.cmd;
        // SAFETY: inside the pass this value began; the pipeline was built with the
        // layout for the target's format (checked when the pass began); the caller
        // guarantees what the shaders read.
        unsafe {
            if self.bound != Some(pipeline) {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, raw);
                self.bound = Some(pipeline);
            }
            if !push.is_empty() {
                d.cmd_push_constants(cmd, self.layout, PUSH_CONSTANT_STAGES, 0, push);
            }
            d.cmd_draw(cmd, vertices, instances, 0, 0);
        }
    }

    /// Ends the pass and leaves its target as the pass's kind says.
    pub fn finish(mut self) {
        let _ = self.end_with(false);
    }

    /// Ends a window pass and copies the image before it is made ready to present:
    /// `None` if the swapchain cannot be read (or the pass is not a window's).
    pub fn finish_captured(mut self) -> Result<Option<Screenshot>, Error> {
        self.end_with(true)
    }

    fn end_with(&mut self, capture: bool) -> Result<Option<Screenshot>, Error> {
        let Some(end) = self.end.take() else {
            return Ok(None);
        };
        let gpu = self.gpu;
        let cmd = self.ctx.cmd;
        // SAFETY: the pass was begun by this value and is ended exactly once; its target
        // is in COLOR_ATTACHMENT_OPTIMAL, which the pass began in.
        unsafe {
            gpu.cmd_end_rendering(cmd);
            match end {
                End::Window(target) => {
                    let shot = if capture {
                        // Makes the transition to PRESENT_SRC itself when it copies.
                        Screenshot::record(gpu, self.ctx, &target)?
                    } else {
                        None
                    };
                    if shot.is_none() {
                        let to_present = [vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                            .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                            .dst_access_mask(vk::AccessFlags2::NONE)
                            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                            .image(target.image)
                            .subresource_range(COLOR_RANGE)];
                        gpu.cmd_pipeline_barrier2(
                            cmd,
                            &vk::DependencyInfo::default().image_memory_barriers(&to_present),
                        );
                    }
                    return Ok(shot);
                }
                End::Sampled(image) => {
                    let to_read = [vk::ImageMemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                        .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                        .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                        .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                        .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .image(image)
                        .subresource_range(COLOR_RANGE)];
                    gpu.cmd_pipeline_barrier2(
                        cmd,
                        &vk::DependencyInfo::default().image_memory_barriers(&to_read),
                    );
                }
                End::Readback(target) => target.record_copy(self.ctx),
            }
        }
        Ok(None)
    }
}

impl Drop for Pass<'_> {
    fn drop(&mut self) {
        let _ = self.end_with(false);
    }
}
