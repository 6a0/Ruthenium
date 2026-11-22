//! The menus' backdrop: the background drawn into an off-screen image the size of the
//! window, and a blurred quarter-size copy, which glass shapes sample (sharp near their
//! rims, frosted inside). The background is static, so both are redrawn only when the
//! window size, the swapchain format or the background changes.

use ruthenium_gpu::{
    COLOR_RANGE, Error, Gpu, PUSH_CONSTANT_STAGES, PipelineLayout, Pipelines, TextureId,
    TextureTable, vk,
};

use crate::Background;
use crate::pipelines::PipelineId;

/// Horizontal + vertical blur rounds over the quarter-size image.
const BLUR_ROUNDS: usize = 2;

#[derive(Debug, Clone, Copy)]
struct Target {
    id: TextureId,
    image: vk::Image,
    view: vk::ImageView,
    extent: vk::Extent2D,
}

impl Target {
    fn new(
        textures: &mut TextureTable,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<Target, Error> {
        let (id, image, view) = textures.create_target(format, extent)?;
        Ok(Target {
            id,
            image,
            view,
            extent,
        })
    }

    fn texel(&self) -> [f32; 2] {
        [
            1.0 / self.extent.width as f32,
            1.0 / self.extent.height as f32,
        ]
    }
}

#[derive(Debug)]
pub(crate) struct Backdrop {
    scene: Target,
    a: Target,
    b: Target,
    format: vk::Format,
    extent: vk::Extent2D,
    /// The background the images hold, once drawn.
    drawn: Option<Background>,
}

fn quarter(e: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D {
        width: (e.width / 4).max(1),
        height: (e.height / 4).max(1),
    }
}

/// `blur.frag`'s push constants.
pub(crate) fn blur_push_copy(scene: TextureId) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[16..20].copy_from_slice(&scene.index().to_ne_bytes());
    b[20..24].copy_from_slice(&2u32.to_ne_bytes());
    b
}

/// `blur.frag`'s push constants.
fn blur_push(src: &Target, dir: [f32; 2], mode: u32) -> [u8; 24] {
    let mut b = [0u8; 24];
    let [tx, ty] = src.texel();
    for (i, v) in [tx, ty, dir[0], dir[1]].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
    }
    b[16..20].copy_from_slice(&src.id.index().to_ne_bytes());
    b[20..24].copy_from_slice(&mode.to_ne_bytes());
    b
}

impl Backdrop {
    /// Creates the images. Call before the texture table's `begin_frame`, so this frame's
    /// set already holds them.
    pub fn new(
        textures: &mut TextureTable,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<Backdrop, Error> {
        let scene = Target::new(textures, format, extent)?;
        let small = quarter(extent);
        let a = Target::new(textures, format, small);
        let b = a
            .as_ref()
            .ok()
            .map(|_| Target::new(textures, format, small));
        match (a, b) {
            (Ok(a), Some(Ok(b))) => Ok(Backdrop {
                scene,
                a,
                b,
                format,
                extent,
                drawn: None,
            }),
            (a, b) => {
                textures.destroy(scene.id);
                if let Ok(a) = &a {
                    textures.destroy(a.id);
                }
                Err(match (a, b) {
                    (Err(e), _) | (_, Some(Err(e))) => e,
                    _ => unreachable!(),
                })
            }
        }
    }

    pub fn destroy(self, textures: &mut TextureTable) {
        for t in [self.scene, self.a, self.b] {
            textures.destroy(t.id);
        }
    }

    pub fn matches(&self, format: vk::Format, extent: vk::Extent2D) -> bool {
        self.format == format && self.extent == extent
    }

    /// Whether the images hold `background`, so shapes may sample them.
    pub fn ready(&self, background: &Background) -> bool {
        self.drawn.as_ref() == Some(background)
    }

    /// `(scene, blurred)` texture indices.
    pub fn textures(&self) -> (TextureId, TextureId) {
        (self.scene.id, self.a.id)
    }

    /// Draws the background and blurs it, if it isn't drawn already. Leaves every image in
    /// `SHADER_READ_ONLY_OPTIMAL`.
    ///
    /// # Safety
    /// `cmd` is recording outside a render pass, with the shared layout's sets bound, in a
    /// frame begun after these images were created; earlier reads of them are by fragment
    /// shaders of earlier commands.
    pub unsafe fn record(
        &mut self,
        gpu: &Gpu,
        cmd: vk::CommandBuffer,
        pipelines: &Pipelines,
        layout: &PipelineLayout,
        background: &Background,
        stars: TextureId,
    ) {
        if self.ready(background) {
            return;
        }
        let background_push = background.push_constants(stars);
        // SAFETY: guaranteed by the caller.
        unsafe {
            pass(
                gpu,
                cmd,
                &self.scene,
                pipelines.get(PipelineId::Background.index()),
                layout,
                &background_push,
            );
            let blur = pipelines.get(PipelineId::Blur.index());
            pass(
                gpu,
                cmd,
                &self.a,
                blur,
                layout,
                &blur_push(&self.scene, [0.0, 0.0], 0),
            );
            for _ in 0..BLUR_ROUNDS {
                pass(
                    gpu,
                    cmd,
                    &self.b,
                    blur,
                    layout,
                    &blur_push(&self.a, [1.0, 0.0], 1),
                );
                pass(
                    gpu,
                    cmd,
                    &self.a,
                    blur,
                    layout,
                    &blur_push(&self.b, [0.0, 1.0], 1),
                );
            }
        }
        self.drawn = Some(*background);
    }
}

/// One full-screen triangle into `dst`, which ends up ready for sampling.
///
/// # Safety
/// As [`Backdrop::record`].
unsafe fn pass(
    gpu: &Gpu,
    cmd: vk::CommandBuffer,
    dst: &Target,
    pipeline: vk::Pipeline,
    layout: &PipelineLayout,
    push: &[u8],
) {
    // Earlier fragment-shader reads of the image finish before it is overwritten; its old
    // contents are not needed.
    let to_attachment = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
        .src_access_mask(vk::AccessFlags2::NONE)
        .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .image(dst.image)
        .subresource_range(COLOR_RANGE)];
    let to_read = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
        .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
        .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image(dst.image)
        .subresource_range(COLOR_RANGE)];
    let attachments = [vk::RenderingAttachmentInfo::default()
        .image_view(dst.view)
        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .load_op(vk::AttachmentLoadOp::DONT_CARE)
        .store_op(vk::AttachmentStoreOp::STORE)];
    let area = vk::Rect2D {
        offset: vk::Offset2D::default(),
        extent: dst.extent,
    };
    let rendering = vk::RenderingInfo::default()
        .render_area(area)
        .layer_count(1)
        .color_attachments(&attachments);
    let viewport = vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: dst.extent.width as f32,
        height: dst.extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let d = gpu.device();
    // SAFETY: guaranteed by the caller.
    unsafe {
        gpu.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&to_attachment),
        );
        gpu.cmd_begin_rendering(cmd, &rendering);
        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
        d.cmd_set_viewport(cmd, 0, &[viewport]);
        d.cmd_set_scissor(cmd, 0, &[area]);
        d.cmd_push_constants(cmd, layout.raw(), PUSH_CONSTANT_STAGES, 0, push);
        d.cmd_draw(cmd, 3, 1, 0, 0);
        gpu.cmd_end_rendering(cmd);
        gpu.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&to_read),
        );
    }
}
