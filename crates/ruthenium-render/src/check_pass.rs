//! What the development checks share: GUI quads drawn with the real GUI pipeline into an
//! off-screen target that is then read back.

use ruthenium_gpu::{Encoder, FrameData, Pipelines, Readback};
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
/// Every quad's texture is `NO_TEXTURE` or an index inside the texture table.
///
/// # Panics
/// If a copy of `target` is pending, or `pipelines` are for another format than its.
pub(crate) unsafe fn gui_quads_into(
    encoder: &mut Encoder<'_>,
    pipelines: &Pipelines,
    target: &mut Readback,
    frame_data: &mut FrameData,
    quads: &[Quad],
) -> bool {
    let Some((first, bytes)) = frame_data.alloc(quads.len() as u32, gui::QUAD_BYTES) else {
        return false;
    };
    gui::write_quads(quads, bytes);
    let extent = target.extent();
    let push = pixel_quads_push(extent.width, extent.height, first);
    let mut pass = encoder.readback_pass(pipelines, target);
    // SAFETY: the push constants are gui.vert's; the quads they point at were written
    // just above; the caller guarantees the texture indices.
    unsafe { pass.draw(PipelineId::Gui.index(), &push, 6, quads.len() as u32) };
    pass.finish();
    true
}
