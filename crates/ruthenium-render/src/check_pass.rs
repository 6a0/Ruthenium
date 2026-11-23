//! What the development checks share: GUI quads drawn with the real GUI pipeline into an
//! off-screen target that is then read back.

use ruthenium_gpu::{
    COLOR_RANGE, FrameCtx, FrameData, Gpu, PUSH_CONSTANT_STAGES, PipelineLayout, Pipelines,
    Readback, vk,
};
use ruthenium_ui::Quad;

use crate::gui;
use crate::pipelines::PipelineId;

/// `gui.vert`'s push constants for quads laid out in pixels of a `width` × `height`
/// target: its size, a GUI scale of one, the first quad in the frame's data.
pub(crate) fn pixel_quads_push(width: u32, height: u32, first: u32) -> [u8; 16] {
    let mut push = [0u8; 16];
    push[0..4].copy_from_slice(&(width as f32).to_ne_bytes());
    push[4..8].copy_from_slice(&(height as f32).to_ne_bytes());
    push[8..12].copy_from_slice(&1.0f32.to_ne_bytes());
    push[12..16].copy_from_slice(&first.to_ne_bytes());
    push
}

/// Clears `target` to transparent black, draws `quads` (in pixels from its top left) in
/// one batch and records the copy back. `false`, with nothing recorded, if the frame's
/// data has no room for the quads: try in the next frame.
///
/// # Safety
/// `ctx.cmd` is recording a frame on the graphics queue, outside a render pass, after
/// the texture table's `begin_frame`, with the shared layout's sets bound; `pipelines`
/// were built with `layout` for the target's format; no copy of `target` is pending.
pub(crate) unsafe fn gui_quads_into(
    gpu: &Gpu,
    ctx: &FrameCtx,
    target: &mut Readback,
    frame_data: &mut FrameData,
    pipelines: &Pipelines,
    layout: &PipelineLayout,
    quads: &[Quad],
) -> bool {
    let Some((first, bytes)) = frame_data.alloc(quads.len() as u32, gui::QUAD_BYTES) else {
        return false;
    };
    gui::write_quads(quads, bytes);
    let (image, view) = target.image();
    let extent = target.extent();
    // After the copy that read the target last; what it held is not needed any more.
    let to_attachment = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COPY)
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
        .load_op(vk::AttachmentLoadOp::CLEAR)
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
    let push = pixel_quads_push(extent.width, extent.height, first);
    let d = gpu.device();
    let cmd = ctx.cmd;
    // SAFETY: guaranteed by the caller; the target is alive.
    unsafe {
        gpu.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
        );
        gpu.cmd_begin_rendering(cmd, &rendering);
        d.cmd_bind_pipeline(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            pipelines.get(PipelineId::Gui.index()),
        );
        d.cmd_set_viewport(cmd, 0, &[viewport]);
        d.cmd_set_scissor(cmd, 0, &[area]);
        d.cmd_push_constants(cmd, layout.raw(), PUSH_CONSTANT_STAGES, 0, &push);
        d.cmd_draw(cmd, 6, quads.len() as u32, 0, 0);
        gpu.cmd_end_rendering(cmd);
        target.record_copy(ctx);
    }
    true
}
