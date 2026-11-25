//! The frame encoder: what the renderer records a frame's passes through. It issues the
//! Vulkan commands and owns what they need to be right (command buffer, pipeline layout,
//! image layouts, barriers, attachments, viewport), so the renderer says which target to
//! draw into and which pipeline draws what, and holds no raw handle.
//!
//! It wraps what the menus and the world draw: full-target passes into the window, into a
//! render-target texture of the table or into a read-back target, each with a depth
//! attachment or without; draws that pull everything from push constants, the frame's
//! data and the texture table; indexed draws from a vertex buffer; and many of those in
//! one call, their commands read from the frame's data (multi-draw indirect, where the
//! GPU has it).

use std::sync::Arc;

use ash::vk;

use crate::Error;
use crate::depth::{DEPTH_CLEAR, DEPTH_RANGE, DepthTarget};
use crate::device::Gpu;
use crate::frame::{FrameCtx, Target};
use crate::frame_data::FrameData;
use crate::pipeline::{PUSH_CONSTANT_BYTES, PUSH_CONSTANT_STAGES, PipelineLayout, Pipelines};
use crate::readback::Readback;
use crate::screenshot::Screenshot;
use crate::swapchain::COLOR_RANGE;
use crate::textures::{TextureId, TextureTable};

/// The stages that read and write a depth attachment.
pub(crate) const DEPTH_STAGES: vk::PipelineStageFlags2 = vk::PipelineStageFlags2::from_raw(
    vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS.as_raw()
        | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS.as_raw(),
);

/// The most draws one indirect call may issue: what every GPU with multi-draw indirect
/// takes at least (`maxDrawIndirectCount`'s required minimum).
pub const MAX_INDIRECT_DRAWS: u32 = (1 << 16) - 1;

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
            None,
            None,
            End::Window(*target),
        )
    }

    /// A pass over the whole window image, cleared to `clear` (a display-encoded
    /// colour), with `depth` as its depth attachment, cleared to the far distance and
    /// not kept after the pass. Only pipelines built for a depth attachment draw in it.
    ///
    /// # Panics
    /// If `pipelines` were built for another format than the window's or the depth
    /// image's, or `depth` is not the window's size.
    pub fn window_pass_with_depth<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        target: &Target,
        depth: &DepthTarget,
        clear: [f32; 4],
    ) -> Pass<'e> {
        assert_eq!(
            pipelines.format(),
            target.format,
            "pipelines for the window"
        );
        self.pass(
            pipelines,
            target.image,
            target.view,
            target.extent,
            vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
            Some(clear),
            Some(depth),
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
            None,
            None,
            End::Sampled(image),
        )
    }

    /// [`Self::texture_pass`] cleared to `clear` first.
    ///
    /// # Panics
    /// As [`Self::texture_pass`].
    pub fn texture_pass_cleared<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        textures: &TextureTable,
        id: TextureId,
        clear: [f32; 4],
    ) -> Pass<'e> {
        let (image, view, desc) = textures.target(id);
        assert_eq!(pipelines.format(), desc.format, "pipelines for {id:?}");
        let extent = vk::Extent2D {
            width: desc.width,
            height: desc.height,
        };
        self.pass(
            pipelines,
            image,
            view,
            extent,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            Some(clear),
            None,
            End::Sampled(image),
        )
    }

    /// [`Self::texture_pass`] cleared to `clear` first, with `depth` as its depth
    /// attachment (see [`Self::window_pass_with_depth`]).
    ///
    /// # Panics
    /// As [`Self::texture_pass`]; or if `depth` is not the texture's size or the
    /// pipelines were built for another depth format.
    pub fn texture_pass_with_depth<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        textures: &TextureTable,
        id: TextureId,
        depth: &DepthTarget,
        clear: [f32; 4],
    ) -> Pass<'e> {
        let (image, view, desc) = textures.target(id);
        assert_eq!(pipelines.format(), desc.format, "pipelines for {id:?}");
        let extent = vk::Extent2D {
            width: desc.width,
            height: desc.height,
        };
        self.pass(
            pipelines,
            image,
            view,
            extent,
            vk::PipelineStageFlags2::FRAGMENT_SHADER,
            Some(clear),
            Some(depth),
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
            Some([0.0; 4]),
            None,
            End::Readback(target),
        )
    }

    /// [`Self::readback_pass`] with `depth` as its depth attachment (see
    /// [`Self::window_pass_with_depth`]).
    ///
    /// # Panics
    /// As [`Self::readback_pass`]; or if `depth` is not the target's size or the
    /// pipelines were built for another depth format.
    pub fn readback_pass_with_depth<'e>(
        &'e mut self,
        pipelines: &'e Pipelines,
        target: &'e mut Readback,
        depth: &DepthTarget,
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
        self.pass(
            pipelines,
            image,
            view,
            target.extent(),
            vk::PipelineStageFlags2::COPY,
            Some([0.0; 4]),
            Some(depth),
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
        // The colour the target is cleared to; none: its old contents are not kept
        // and every pixel is drawn.
        clear: Option<[f32; 4]>,
        depth: Option<&DepthTarget>,
        end: End<'e>,
    ) -> Pass<'e> {
        if let Some(depth) = depth {
            assert_eq!(
                depth.extent(),
                extent,
                "the depth image is the target's size"
            );
            assert_eq!(
                depth.format(),
                pipelines.depth_format(),
                "pipelines for the depth image"
            );
        }
        let mut barriers = [vk::ImageMemoryBarrier2::default(); 2];
        barriers[0] = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(after)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE);
        if let Some(depth) = depth {
            // After the pass that used the depth image last; its old contents go.
            barriers[1] = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(DEPTH_STAGES)
                .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_stage_mask(DEPTH_STAGES)
                .dst_access_mask(
                    vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
                        | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
                .image(depth.image().0)
                .subresource_range(DEPTH_RANGE);
        }
        let to_attachment = &barriers[..1 + usize::from(depth.is_some())];
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(match clear {
                Some(_) => vk::AttachmentLoadOp::CLEAR,
                None => vk::AttachmentLoadOp::DONT_CARE,
            })
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: clear.unwrap_or_default(),
                },
            })
            .store_op(vk::AttachmentStoreOp::STORE)];
        let depth_attachment = depth.map(|depth| {
            vk::RenderingAttachmentInfo::default()
                .image_view(depth.image().1)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::DONT_CARE)
                .clear_value(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: DEPTH_CLEAR,
                        stencil: 0,
                    },
                })
        });
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        let mut rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments);
        if let Some(a) = &depth_attachment {
            rendering = rendering.depth_attachment(a);
        }
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
        // encoder until it is finished); the images and their views are alive, as their
        // owners (a window target of this frame, the texture table, the read-back
        // target, the depth target, which waits for the GPU before it goes) keep them at
        // least until the frame has finished; leaving UNDEFINED is valid whatever layout
        // an image is in.
        unsafe {
            gpu.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().image_memory_barriers(to_attachment),
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
            depth: depth.is_some(),
            extent,
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
    /// The pass has a depth attachment.
    depth: bool,
    /// The target's size.
    extent: vk::Extent2D,
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
    /// Clears the whole depth attachment to the far distance, as the pass's start did:
    /// what is drawn from here on is in front of everything drawn before, whatever its
    /// depth (the player's own hands over the world).
    ///
    /// # Panics
    /// If the pass has no depth attachment.
    pub fn clear_depth(&mut self) {
        assert!(self.depth, "a pass without a depth attachment");
        let attachment = vk::ClearAttachment {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            color_attachment: 0,
            clear_value: vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: DEPTH_CLEAR,
                    stencil: 0,
                },
            },
        };
        let rect = vk::ClearRect {
            rect: vk::Rect2D {
                offset: vk::Offset2D::default(),
                extent: self.extent,
            },
            base_array_layer: 0,
            layer_count: 1,
        };
        // SAFETY: inside the pass this value began, whose depth attachment (checked
        // above) covers `extent` and has the depth aspect.
        unsafe {
            self.gpu
                .device
                .cmd_clear_attachments(self.ctx.cmd, &[attachment], &[rect]);
        }
    }

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
        assert!(
            !self.pipelines.has_vertex_input(pipeline),
            "pipeline {pipeline} reads a vertex buffer: draw_indexed"
        );
        // SAFETY: inside the pass this value began; the pipeline was built with the
        // layout for the target's formats (checked when the pass began and in `bind`);
        // the caller guarantees what the shaders read.
        unsafe {
            self.bind(pipeline, push);
            self.gpu
                .device
                .cmd_draw(self.ctx.cmd, vertices, instances, 0, 0);
        }
    }

    /// Draws `vertices` vertices from `first` of the bound vertex buffer, in order, with
    /// pipeline `pipeline` and `push` as its push constants.
    ///
    /// # Safety
    /// As [`Self::draw`]; a vertex buffer is bound, and every vertex the draw reads lies
    /// inside what was written to it.
    ///
    /// # Panics
    /// As [`Self::draw`]; or if `pipeline` reads no vertex buffer.
    pub unsafe fn draw_vertices(
        &mut self,
        pipeline: usize,
        push: &[u8],
        first: u32,
        vertices: u32,
    ) {
        assert!(
            push.len() <= PUSH_CONSTANT_BYTES as usize && push.len().is_multiple_of(4),
            "{} bytes of push constants",
            push.len()
        );
        assert!(
            self.pipelines.has_vertex_input(pipeline),
            "pipeline {pipeline} reads no vertex buffer: draw"
        );
        // SAFETY: as in `draw`; the caller guarantees the vertices.
        unsafe {
            self.bind(pipeline, push);
            self.gpu
                .device
                .cmd_draw(self.ctx.cmd, vertices, 1, first, 0);
        }
    }

    /// Draws `instances` instances of `vertices` vertices, the instances from `first` of
    /// the bound vertex buffer, with pipeline `pipeline`, whose vertex input is per
    /// instance, and `push` as its push constants.
    ///
    /// # Safety
    /// As [`Self::draw`]; a vertex buffer is bound, and every instance the draw reads lies
    /// inside what was written to it.
    ///
    /// # Panics
    /// As [`Self::draw`]; or if `pipeline` reads no vertex buffer.
    pub unsafe fn draw_instances(
        &mut self,
        pipeline: usize,
        push: &[u8],
        vertices: u32,
        first: u32,
        instances: u32,
    ) {
        assert!(
            push.len() <= PUSH_CONSTANT_BYTES as usize && push.len().is_multiple_of(4),
            "{} bytes of push constants",
            push.len()
        );
        assert!(
            self.pipelines.has_vertex_input(pipeline),
            "pipeline {pipeline} reads no vertex buffer: draw"
        );
        // SAFETY: as in `draw`; the caller guarantees the instances.
        unsafe {
            self.bind(pipeline, push);
            self.gpu
                .device
                .cmd_draw(self.ctx.cmd, vertices, instances, 0, first);
        }
    }

    /// Binds `pipeline` if it is not bound and sets `push` if there is any.
    ///
    /// # Safety
    /// Inside the pass; `push` is what the pipeline's shaders expect.
    unsafe fn bind(&mut self, pipeline: usize, push: &[u8]) {
        assert_eq!(
            self.pipelines.has_depth(pipeline),
            self.depth,
            "pipeline {pipeline} and the pass disagree about a depth attachment"
        );
        let raw = self.pipelines.get(pipeline);
        let d = &self.gpu.device;
        let cmd = self.ctx.cmd;
        // SAFETY: the caller's.
        unsafe {
            if self.bound != Some(pipeline) {
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, raw);
                self.bound = Some(pipeline);
            }
            if !push.is_empty() {
                d.cmd_push_constants(cmd, self.layout, PUSH_CONSTANT_STAGES, 0, push);
            }
        }
    }

    /// Binds `buffer` from `offset` as the vertex buffer of the draws that follow.
    ///
    /// # Safety
    /// `buffer` is alive until the frame has finished and was created for vertices; the
    /// bytes the draws read from it were written by copies that this frame may read
    /// ([`crate::UploadRing::is_ready`]).
    pub unsafe fn bind_vertex_buffer(&mut self, buffer: vk::Buffer, offset: u64) {
        // SAFETY: inside the pass; the rest is the caller's guarantee.
        unsafe {
            self.gpu
                .device
                .cmd_bind_vertex_buffers(self.ctx.cmd, 0, &[buffer], &[offset]);
        }
    }

    /// Binds `buffer` from `offset` as the 16-bit index buffer of the draws that follow.
    ///
    /// # Safety
    /// As [`Self::bind_vertex_buffer`], for indices.
    pub unsafe fn bind_index_buffer(&mut self, buffer: vk::Buffer, offset: u64) {
        // SAFETY: the caller's.
        unsafe { self.bind_index_buffer_of(buffer, offset, false) }
    }

    /// Binds `buffer` from `offset` as the index buffer of the draws that follow: of
    /// 32-bit indices if `wide`, else of 16-bit ones.
    ///
    /// # Safety
    /// As [`Self::bind_vertex_buffer`], for indices.
    pub unsafe fn bind_index_buffer_of(&mut self, buffer: vk::Buffer, offset: u64, wide: bool) {
        let kind = if wide {
            vk::IndexType::UINT32
        } else {
            vk::IndexType::UINT16
        };
        // SAFETY: inside the pass; the rest is the caller's guarantee.
        unsafe {
            self.gpu
                .device
                .cmd_bind_index_buffer(self.ctx.cmd, buffer, offset, kind);
        }
    }

    /// Draws `indices` indices from `first_index` of the bound index buffer, each added
    /// to `vertex_offset` to give a vertex of the bound vertex buffer, with pipeline
    /// `pipeline` and `push` as its push constants (none: those set last stay). The one
    /// instance drawn has the number `instance`, which the shaders may read to find the
    /// draw's own data.
    ///
    /// # Safety
    /// As [`Self::draw`]; a vertex and an index buffer are bound, and every index and
    /// vertex the draw reads lies inside what was written to them.
    ///
    /// # Panics
    /// As [`Self::draw`]; or if `pipeline` reads no vertex buffer.
    pub unsafe fn draw_indexed(
        &mut self,
        pipeline: usize,
        push: &[u8],
        indices: u32,
        first_index: u32,
        vertex_offset: i32,
        instance: u32,
    ) {
        // SAFETY: the caller's.
        unsafe {
            self.draw_indexed_instances(
                pipeline,
                push,
                indices,
                first_index,
                vertex_offset,
                instance,
                1,
            );
        }
    }

    /// [`Self::draw_indexed`] `count` times over, as instances numbered from `instance`.
    ///
    /// # Safety
    /// As [`Self::draw_indexed`], for every instance.
    ///
    /// # Panics
    /// As [`Self::draw_indexed`].
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw_indexed_instances(
        &mut self,
        pipeline: usize,
        push: &[u8],
        indices: u32,
        first_index: u32,
        vertex_offset: i32,
        instance: u32,
        count: u32,
    ) {
        assert!(
            push.len() <= PUSH_CONSTANT_BYTES as usize && push.len().is_multiple_of(4),
            "{} bytes of push constants",
            push.len()
        );
        assert!(
            self.pipelines.has_vertex_input(pipeline),
            "pipeline {pipeline} reads no vertex buffer"
        );
        // SAFETY: as in `draw`.
        unsafe {
            self.bind(pipeline, push);
            self.gpu.device.cmd_draw_indexed(
                self.ctx.cmd,
                indices,
                count,
                first_index,
                vertex_offset,
                instance,
            );
        }
    }

    /// Draws `count` indexed draws in one call, their commands read from `commands` at
    /// `offset` ([`crate::FrameData::alloc_draw_commands`]), with pipeline `pipeline` and
    /// `push` as its push constants (none: those set last stay). Each command is one
    /// [`Self::draw_indexed`]: its first instance is the number the shaders read.
    ///
    /// # Safety
    /// As [`Self::draw_indexed`], for every command; the `count` commands were written
    /// this frame and `commands` is alive until the frame has finished.
    ///
    /// # Panics
    /// As [`Self::draw_indexed`]; or if the GPU cannot draw several commands in a call,
    /// or commands that start at another instance than the first
    /// ([`crate::OptionalFeatures`]); or if `count` is more than every GPU that can
    /// takes ([`MAX_INDIRECT_DRAWS`]).
    pub unsafe fn draw_indexed_indirect(
        &mut self,
        pipeline: usize,
        push: &[u8],
        commands: vk::Buffer,
        offset: u64,
        count: u32,
    ) {
        assert!(
            push.len() <= PUSH_CONSTANT_BYTES as usize && push.len().is_multiple_of(4),
            "{} bytes of push constants",
            push.len()
        );
        assert!(
            self.pipelines.has_vertex_input(pipeline),
            "pipeline {pipeline} reads no vertex buffer"
        );
        let optional = self.gpu.caps().optional;
        assert!(
            optional.multi_draw_indirect && optional.draw_indirect_first_instance,
            "this GPU has no multi-draw indirect with a first instance"
        );
        assert!(count <= MAX_INDIRECT_DRAWS, "{count} draws in one call");
        // SAFETY: as in `draw`; the commands are the caller's guarantee.
        unsafe {
            self.bind(pipeline, push);
            self.gpu.device.cmd_draw_indexed_indirect(
                self.ctx.cmd,
                commands,
                offset,
                count,
                crate::DRAW_COMMAND_BYTES,
            );
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
