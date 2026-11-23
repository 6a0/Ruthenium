//! A development check of the colour contract (`--exercise-colour`): textures, blending
//! and the window carry display-encoded colours in UNORM, byte for byte, with no sRGB
//! decode on sampling and no encode on writing.
//!
//! A texture of one known mid-grey byte is drawn unlit with the GUI pipeline, and beside
//! it the same grey with a translucent swatch blended over it. Both patches are read
//! back and compared: the grey must come out as the byte that went in, and the blend
//! must be the one computed on the stored bytes. An sRGB view of the texture, an sRGB
//! target (hardware encode, blending in linear light) or both at once (double gamma)
//! each move one of the two by tens of steps.
//!
//! The patches are drawn into an off-screen target of the window's format every frame,
//! and every few frames into a corner of the window itself, read back from the
//! swapchain image before it is presented: the view a presentation path really draws
//! through (what a UNORM view of an sRGB-only surface would have to pass).

use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, Encoder, Error, FrameData, Gpu, Pipelines, Readback, Screenshot, TextureDesc,
    TextureId, TextureTable, UploadRing, vk,
};
use ruthenium_ui::{NO_TEXTURE, Quad};

use crate::check_pass;

/// The texture's byte in every channel but alpha: mid-grey.
const GREY: u8 = 0x80;
/// The swatch blended over it: RGBA, straight alpha, half transparent.
const SWATCH: [u8; 4] = [0xFF, 0x40, 0x00, 0x80];
/// Side of one patch, in pixels.
const PATCH: u32 = 8;
/// The two patches side by side: unlit grey, then the swatch over grey.
pub(crate) const WIDTH: u32 = 2 * PATCH;
pub(crate) const HEIGHT: u32 = PATCH;
/// Frames between two checks of the window itself.
const WINDOW_EVERY: u64 = 16;
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;

/// What the unlit patch must read: the bytes that were uploaded.
const UNLIT: [u8; 4] = [GREY, GREY, GREY, 0xFF];

/// What the blended patch must read: `a * swatch + (1 - a) * grey` on the stored bytes
/// (the GUI pipeline's straight-alpha blend), rounded; alpha stays opaque.
fn blended() -> [u8; 4] {
    let a = f64::from(SWATCH[3]) / 255.0;
    let mix = |s: u8| (a * f64::from(s) + (1.0 - a) * f64::from(GREY)).round() as u8;
    [mix(SWATCH[0]), mix(SWATCH[1]), mix(SWATCH[2]), 0xFF]
}

/// Whether the two patches read right: the unlit one byte for byte, the blended one
/// within one step (a blend may round either way).
fn judge(unlit: [u8; 4], blend: [u8; 4]) -> bool {
    unlit == UNLIT
        && blend
            .iter()
            .zip(blended())
            .all(|(&got, want)| got.abs_diff(want) <= 1)
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColourCheckReport {
    /// Off-screen targets and window corners read back and compared.
    pub offscreen: u64,
    pub window: u64,
    /// How many of them read wrong.
    pub mismatches: u64,
    /// Frames whose window could not be read back (the swapchain cannot be copied from).
    pub window_unreadable: u64,
}

impl ColourCheckReport {
    /// Whether the run shows the contract holding: patches compared without a mismatch,
    /// in the window too wherever it can be read.
    pub fn passed(&self) -> bool {
        self.offscreen > 0
            && self.mismatches == 0
            && (self.window > 0 || self.window_unreadable > 0)
    }
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct ColourCheck {
    gpu: Arc<Gpu>,
    target: Readback,
    grey: TextureId,
    /// A copy of the window on its way back.
    shot: Option<Screenshot>,
    report: ColourCheckReport,
}

impl ColourCheck {
    pub fn new(
        gpu: Arc<Gpu>,
        format: vk::Format,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
    ) -> Result<ColourCheck, Error> {
        let target = Readback::new(gpu.clone(), format, extent())?;
        let texels: [u8; 64] = std::array::from_fn(|i| UNLIT[i % 4]);
        let grey = textures.create_with(
            uploads,
            TextureDesc::rgba8(4, 4),
            &[&texels],
            Arrival::NextFrame,
        )?;
        Ok(ColourCheck {
            gpu,
            target,
            grey,
            shot: None,
            report: ColourCheckReport::default(),
        })
    }

    pub fn report(&self) -> ColourCheckReport {
        self.report
    }

    /// The patches, in pixels from the top left of what they are drawn into: the grey
    /// texture under both, the untextured swatch over the right one.
    pub fn quads(&self) -> [Quad; 2] {
        let [r, g, b, a] = SWATCH.map(u32::from);
        [
            Quad {
                rect: [0.0, 0.0, WIDTH as f32, HEIGHT as f32],
                uv: [0.5; 4],
                color: 0xFFFF_FFFF,
                texture: self.grey.index(),
            },
            Quad {
                rect: [PATCH as f32, 0.0, WIDTH as f32, HEIGHT as f32],
                uv: [0.0; 4],
                color: a << 24 | r << 16 | g << 8 | b,
                texture: NO_TEXTURE,
            },
        ]
    }

    /// Compares what has come back and draws the patches off screen again. `completed`
    /// is a frame the GPU has finished; call it after `textures.begin_frame` for the
    /// frame `encoder` records.
    pub fn step(
        &mut self,
        encoder: &mut Encoder<'_>,
        completed: u64,
        textures: &TextureTable,
        frame_data: &mut FrameData,
        pipelines: &Pipelines,
    ) -> Result<(), Error> {
        if let Some((frame, pixels)) = self.target.take(completed) {
            let ok = compare("off screen", frame, pixels, WIDTH, &mut self.report);
            self.report.offscreen += 1;
            self.report.mismatches += u64::from(!ok);
        }
        if self.shot.as_ref().is_some_and(|s| s.frame() <= completed) {
            let shot = self.shot.take().expect("checked");
            let frame = shot.frame();
            let (width, _, pixels) = shot.into_rgba();
            let ok = compare("in the window", frame, &pixels, width, &mut self.report);
            self.report.window += 1;
            self.report.mismatches += u64::from(!ok);
        }
        if self.target.format() != pipelines.format() {
            self.target = Readback::new(self.gpu.clone(), pipelines.format(), extent())?;
        }
        if textures.is_ready(self.grey) && !self.target.pending() {
            let quads = self.quads();
            // SAFETY: the quads sample the check's own texture or none.
            unsafe {
                check_pass::gui_quads_into(encoder, pipelines, &mut self.target, frame_data, &quads)
            };
        }
        Ok(())
    }

    /// Whether this frame should draw the patches into the window and copy it.
    pub fn wants_window(&self, textures: &TextureTable, frame: u64) -> bool {
        self.shot.is_none() && frame.is_multiple_of(WINDOW_EVERY) && textures.is_ready(self.grey)
    }

    /// The window with the patches in its top left corner was copied.
    pub fn window_recorded(&mut self, shot: Screenshot) {
        self.shot = Some(shot);
    }

    /// The window with the patches could not be copied.
    pub fn window_unreadable(&mut self) {
        self.report.window_unreadable += 1;
    }
}

fn extent() -> vk::Extent2D {
    vk::Extent2D {
        width: WIDTH,
        height: HEIGHT,
    }
}

/// Judges every pixel of the two patches in the top left corner of an RGBA image
/// `width` pixels wide, and says so if one is wrong.
fn compare(
    place: &str,
    frame: u64,
    pixels: &[u8],
    width: u32,
    report: &mut ColourCheckReport,
) -> bool {
    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        pixels[i..i + 4].try_into().expect("4 bytes")
    };
    for y in 0..HEIGHT {
        for x in 0..PATCH {
            let (unlit, blend) = (at(x, y), at(x + PATCH, y));
            if !judge(unlit, blend) {
                if report.mismatches < PRINTED {
                    eprintln!(
                        "[colour-check] {place}, frame {frame}, pixel ({x}, {y}): grey reads {unlit:02x?} (must be {UNLIT:02x?}), the swatch over it {blend:02x?} (must be {:02x?} within one step)",
                        blended()
                    );
                }
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(v: f64) -> f64 {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn encode(v: f64) -> f64 {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }

    fn byte(v: f64) -> u8 {
        (v * 255.0).round() as u8
    }

    fn unit(b: u8) -> f64 {
        f64::from(b) / 255.0
    }

    /// The two patches as a pipeline would produce them: `sample` turns a stored texture
    /// byte into the shader's value, `store` a blended value into the target's byte and
    /// `load` reads it back for blending.
    fn render(
        sample: fn(f64) -> f64,
        store: fn(f64) -> f64,
        load: fn(f64) -> f64,
    ) -> ([u8; 4], [u8; 4]) {
        let grey = sample(unit(GREY));
        let unlit = byte(store(grey));
        let a = unit(SWATCH[3]);
        let dst = load(unit(unlit));
        let blend: [u8; 3] =
            std::array::from_fn(|c| byte(store(a * unit(SWATCH[c]) + (1.0 - a) * dst)));
        (
            [unlit, unlit, unlit, 0xFF],
            [blend[0], blend[1], blend[2], 0xFF],
        )
    }

    #[test]
    fn the_blend_is_computed_on_stored_bytes() {
        assert_eq!(blended(), [192, 96, 64, 255]);
        let same = |v| v;
        let (unlit, blend) = render(same, same, same);
        assert_eq!((unlit, blend), (UNLIT, blended()));
        assert!(judge(unlit, blend));
        // A blend may round either way, a sample may not.
        assert!(judge(UNLIT, [191, 97, 64, 255]));
        assert!(!judge(UNLIT, [190, 96, 64, 255]));
        assert!(!judge([0x81, 0x80, 0x80, 0xFF], blended()));
    }

    #[test]
    fn an_srgb_view_or_target_fails_the_fixture() {
        let same = |v| v;
        // The texture viewed as sRGB: decoded on sampling, stored as it is.
        let (unlit, blend) = render(decode, same, same);
        assert_eq!(unlit[0], 55);
        assert!(!judge(unlit, blend));
        // An sRGB target: encoded on writing, blended in linear light.
        let (unlit, blend) = render(same, encode, decode);
        assert_eq!(unlit[0], 188);
        assert!(!judge(unlit, blend));
        // Both: the grey survives the round trip, the blend does not.
        let (unlit, blend) = render(decode, encode, decode);
        assert_eq!(unlit, UNLIT);
        assert!(blend[0] > blended()[0] + 8, "{blend:?}");
        assert!(!judge(unlit, blend));
    }
}
