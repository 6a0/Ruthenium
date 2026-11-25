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

/// Bytes of `blur.frag`'s push constants.
const PUSH: usize = 40;

/// `blur.frag`'s push constants: what every mode has, with nothing graded.
fn blur_push(src: &Target, dir: [f32; 2], mode: u32) -> [u8; PUSH] {
    let mut b = [0u8; PUSH];
    let [tx, ty] = src.texel();
    for (i, v) in [tx, ty, dir[0], dir[1]].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
    }
    b[16..20].copy_from_slice(&src.id.index().to_ne_bytes());
    b[20..24].copy_from_slice(&mode.to_ne_bytes());
    b[24..28].copy_from_slice(&1f32.to_ne_bytes());
    b[28..32].copy_from_slice(&1f32.to_ne_bytes());
    b
}

/// How the world looks behind a menu, and the menu over it while it comes and goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Veil {
    /// How much of the world's colour is left, and how bright it is (1: as it is).
    pub saturation: f32,
    pub brightness: f32,
    /// How far the world is blurred, in pixels.
    pub blur: f32,
    /// How far the menu is blurred, in pixels, and how opaque it is.
    pub menu_blur: f32,
    pub menu_opacity: f32,
}

impl Veil {
    /// What the shapes' glass and the world's copy are graded by.
    pub fn grade(&self) -> [f32; 2] {
        [self.saturation, self.brightness]
    }

    /// Whether the menu has to be drawn apart, into a layer of its own.
    pub fn wants_layer(&self) -> bool {
        self.menu_blur > 0.0 || self.menu_opacity < 1.0
    }
}

/// Rounds of the layer's blur over its half-size image, and how wide one of them blurs
/// in that image's texels, at a step of one.
const LAYER_ROUNDS: usize = 2;
const LAYER_WIDTH: f32 = 2.0;
/// Below this many pixels the half-size image is too coarse: the sharp layer shows
/// through it, all of it at none.
const LAYER_COARSE: f32 = 4.0;

/// A menu drawn apart while it comes or goes: over nothing, and then blurred.
#[derive(Debug)]
pub(crate) struct Layer {
    full: Target,
    a: Target,
    b: Target,
    format: vk::Format,
    extent: vk::Extent2D,
}

impl Layer {
    /// Creates the images, as [`Backdrop::new`].
    pub fn new(
        textures: &mut TextureTable,
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> Result<Layer, Error> {
        let half = vk::Extent2D {
            width: extent.width.div_ceil(2),
            height: extent.height.div_ceil(2),
        };
        let mut made: Vec<Target> = Vec::with_capacity(3);
        for size in [extent, half, half] {
            match Target::new(textures, format, size) {
                Ok(t) => made.push(t),
                Err(e) => {
                    for t in made {
                        textures.destroy(t.id);
                    }
                    return Err(e);
                }
            }
        }
        Ok(Layer {
            full: made[0],
            a: made[1],
            b: made[2],
            format,
            extent,
        })
    }

    pub fn destroy(self, textures: &mut TextureTable) {
        for t in [self.full, self.a, self.b] {
            textures.destroy(t.id);
        }
    }

    pub fn matches(&self, format: vk::Format, extent: vk::Extent2D) -> bool {
        self.format == format && self.extent == extent
    }

    /// What the menu is drawn into, cleared to nothing first.
    pub fn full(&self) -> TextureId {
        self.full.id
    }

    /// Blurs the menu, which has just been drawn, over `pixels`, in passes of its own.
    pub fn blur(
        &self,
        encoder: &mut Encoder<'_>,
        pipelines: &Pipelines,
        textures: &TextureTable,
        pixels: f32,
    ) {
        if pixels <= 0.0 {
            return;
        }
        // Pixels are half as many texels, spread over the rounds' widths.
        let step = pixels / (2.0 * LAYER_WIDTH * (LAYER_ROUNDS as f32).sqrt());
        let mut pass = |dst: &Target, push: &[u8]| {
            let mut pass = encoder.texture_pass(pipelines, textures, dst.id);
            // SAFETY: the push constants are blur.frag's, which reads no frame data; the
            // textures they name are the layer's other images, never the one drawn into.
            unsafe { pass.draw(PipelineId::Blur.index(), push, 3, 1) };
            pass.finish();
        };
        // Mode 1 with no step is one bilinear tap: at half the size, the mean of four.
        pass(&self.a, &blur_push(&self.full, [0.0, 0.0], 1));
        for _ in 0..LAYER_ROUNDS {
            pass(&self.b, &blur_push(&self.a, [step, 0.0], 1));
            pass(&self.a, &blur_push(&self.b, [0.0, step], 1));
        }
    }

    /// The push constants that lay it over the window ([`PipelineId::Layer`]).
    pub fn push(&self, veil: &Veil) -> [u8; PUSH] {
        let mut b = blur_push(&self.full, [veil.menu_opacity, 0.0], 4);
        let soft = (veil.menu_blur / LAYER_COARSE).clamp(0.0, 1.0);
        b[32..36].copy_from_slice(&self.a.id.index().to_ne_bytes());
        b[36..40].copy_from_slice(&soft.to_ne_bytes());
        b
    }
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

    /// The full-size image: what is behind the menus. The caller may draw it itself
    /// (the world) and then call [`Self::blur`].
    pub fn scene(&self) -> TextureId {
        self.scene.id
    }

    /// The push constants that copy the full-size image into the window, behind a menu
    /// as `veil` says.
    pub fn copy_push(&self, veil: Option<&Veil>) -> [u8; PUSH] {
        let Some(veil) = veil else {
            return blur_push(&self.scene, [0.0, 0.0], 2);
        };
        let mut b = blur_push(&self.scene, [veil.blur, 0.0], 3);
        b[24..28].copy_from_slice(&veil.saturation.to_ne_bytes());
        b[28..32].copy_from_slice(&veil.brightness.to_ne_bytes());
        b
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
        let mut pass = encoder.texture_pass(pipelines, textures, self.scene.id);
        // SAFETY: the push constants are background.frag's, which reads no frame data;
        // the texture it names is the starfield.
        unsafe {
            pass.draw(
                PipelineId::Background.index(),
                &background.push_constants(stars),
                3,
                1,
            )
        };
        pass.finish();
        self.blur(encoder, pipelines, textures);
        self.drawn = Some(*background);
    }

    /// Blurs the full-size image, which has just been drawn into (and is ready for
    /// sampling), into the quarter-size one, in passes of their own. What the images
    /// hold afterwards is no background the menus know: the next static one is drawn
    /// anew.
    pub fn blur(
        &mut self,
        encoder: &mut Encoder<'_>,
        pipelines: &Pipelines,
        textures: &TextureTable,
    ) {
        self.drawn = None;
        let mut pass = |dst: &Target, pipeline: PipelineId, push: &[u8]| {
            let mut pass = encoder.texture_pass(pipelines, textures, dst.id);
            // SAFETY: the push constants are blur.frag's, which reads no frame data; the
            // textures they name are the backdrop's other images, never the one drawn
            // into.
            unsafe { pass.draw(pipeline.index(), push, 3, 1) };
            pass.finish();
        };
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
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Veil = Veil {
        saturation: 1.0,
        brightness: 1.0,
        blur: 0.0,
        menu_blur: 0.0,
        menu_opacity: 1.0,
    };

    #[test]
    fn a_menu_is_drawn_apart_only_while_it_comes_or_goes() {
        assert!(!PLAIN.wants_layer());
        let coming = Veil {
            menu_blur: 3.0,
            ..PLAIN
        };
        let faint = Veil {
            menu_opacity: 0.5,
            ..PLAIN
        };
        assert!(coming.wants_layer() && faint.wants_layer());
    }

    #[test]
    fn push_constants_are_laid_out_as_the_shader_says() {
        let target = |index: u32| Target {
            id: TextureId::MISSING,
            extent: vk::Extent2D {
                width: 4 * index,
                height: 2,
            },
        };
        let layer = Layer {
            full: target(2),
            a: target(1),
            b: target(1),
            format: vk::Format::B8G8R8A8_UNORM,
            extent: vk::Extent2D {
                width: 8,
                height: 2,
            },
        };
        let float = |b: &[u8], at: usize| f32::from_ne_bytes(b[at..at + 4].try_into().unwrap());
        let mode = |b: &[u8]| u32::from_ne_bytes(b[20..24].try_into().unwrap());
        let b = layer.push(&Veil {
            menu_blur: 1.0,
            menu_opacity: 0.5,
            ..PLAIN
        });
        assert_eq!((float(&b, 0), float(&b, 4)), (0.125, 0.5));
        assert_eq!((float(&b, 8), mode(&b), float(&b, 36)), (0.5, 4, 0.25));
        // As blurred as it gets, and not at all.
        let soft = |menu_blur| float(&layer.push(&Veil { menu_blur, ..PLAIN }), 36);
        assert_eq!([soft(0.0), soft(20.0)], [0.0, 1.0]);
        let backdrop = Backdrop {
            scene: target(1),
            a: target(1),
            b: target(1),
            format: vk::Format::B8G8R8A8_UNORM,
            extent: vk::Extent2D {
                width: 4,
                height: 2,
            },
            drawn: None,
        };
        assert_eq!(mode(&backdrop.copy_push(None)), 2);
        let b = backdrop.copy_push(Some(&Veil {
            saturation: 0.25,
            brightness: 0.75,
            blur: 2.0,
            ..PLAIN
        }));
        assert_eq!(
            (mode(&b), float(&b, 8), float(&b, 24), float(&b, 28)),
            (3, 2.0, 0.25, 0.75)
        );
    }

    #[test]
    fn push_constants_fit_what_the_shaders_declare() {
        let block = |source: &str| {
            let at = source.find("uniform Push").unwrap();
            let body = &source[at..];
            let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
            body.lines()
                .filter_map(|l| l.split_whitespace().next())
                .map(|t| match t {
                    "vec2" => 8,
                    "uint" | "float" => 4,
                    _ => 0,
                })
                .sum::<usize>()
        };
        let blur = include_str!("../../ruthenium-shaders/glsl/blur.frag");
        assert_eq!(block(blur), PUSH);
        let shape = include_str!("../../ruthenium-shaders/glsl/shape.frag");
        assert_eq!(block(shape), crate::SHAPE_PUSH);
    }
}
