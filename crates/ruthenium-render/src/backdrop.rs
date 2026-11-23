//! The menus' backdrop: the background drawn into an off-screen image the size of the
//! window, and a blurred quarter-size copy, which glass shapes sample (sharp near their
//! rims, frosted inside). The background is static, so both are redrawn only when the
//! window size, the swapchain format or the background changes.

use ruthenium_gpu::{Encoder, Error, Pipelines, TextureId, TextureTable, vk};

use crate::Background;
use crate::pipelines::PipelineId;

/// Horizontal + vertical blur rounds over the quarter-size image.
const BLUR_ROUNDS: usize = 2;

#[derive(Debug, Clone, Copy)]
struct Target {
    id: TextureId,
    extent: vk::Extent2D,
}

impl Target {
    fn new(
        textures: &mut TextureTable,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<Target, Error> {
        let id = textures.create_target(format, extent)?;
        Ok(Target { id, extent })
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

    /// Draws the background and blurs it, if it isn't drawn already, in passes of their
    /// own (so outside any other pass), in a frame begun after these images were created.
    /// Every image ends up ready for sampling. `pipelines` are for this backdrop's format.
    pub fn record(
        &mut self,
        encoder: &mut Encoder<'_>,
        pipelines: &Pipelines,
        textures: &TextureTable,
        background: &Background,
        stars: TextureId,
    ) {
        if self.ready(background) {
            return;
        }
        let mut pass = |dst: &Target, pipeline: PipelineId, push: &[u8]| {
            let mut pass = encoder.texture_pass(pipelines, textures, dst.id);
            // SAFETY: the push constants are those of background.frag and blur.frag, which
            // read no frame data; the textures they name are the starfield and the
            // backdrop's other images, never the one drawn into.
            unsafe { pass.draw(pipeline.index(), push, 3, 1) };
            pass.finish();
        };
        pass(
            &self.scene,
            PipelineId::Background,
            &background.push_constants(stars),
        );
        pass(
            &self.a,
            PipelineId::Blur,
            &blur_push(&self.scene, [0.0, 0.0], 0),
        );
        for _ in 0..BLUR_ROUNDS {
            pass(
                &self.b,
                PipelineId::Blur,
                &blur_push(&self.a, [1.0, 0.0], 1),
            );
            pass(
                &self.a,
                PipelineId::Blur,
                &blur_push(&self.b, [0.0, 1.0], 1),
            );
        }
        self.drawn = Some(*background);
    }
}
