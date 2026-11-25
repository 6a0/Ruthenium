//! Writes ruthenium-ui's quads into the frame's data for `gui.vert`.

use ruthenium_gpu::SamplerId;
use ruthenium_ui::Quad;
use ruthenium_ui::modern::Shape;
use ruthenium_ui::text::GlyphQuad;

/// Bytes per quad in the frame's data (std430: two vec4s and four uints).
pub const QUAD_BYTES: u32 = 48;

/// Bytes per shape in the frame's data (std430: four vec4s, a uvec4, the clip vec4 and
/// a vec4 of the opacity and the bevel's width).
pub const SHAPE_BYTES: u32 = 112;

/// One frame's GUI: vanilla-style quads (with the GUI scale they were laid out for) and
/// the modern menus' shapes, drawn after the quads.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuiFrame<'a> {
    pub scale: u32,
    pub quads: &'a [Quad],
    pub shapes: &'a [Shape],
    /// Text, drawn after the shapes, from the glyph atlas texture `font_atlas`.
    pub glyphs: &'a [GlyphQuad],
    pub font_atlas: u32,
    /// How the world is seen behind the menu, and the menu while it comes or goes.
    pub veil: Option<crate::Veil>,
}

/// Bytes per glyph in the frame's data (std430: six vec4s).
pub const GLYPH_BYTES: u32 = 96;

pub fn write_glyphs(glyphs: &[GlyphQuad], out: &mut [u8]) {
    for (g, chunk) in glyphs.iter().zip(out.as_chunks_mut::<96>().0) {
        let params = [g.range, g.clip.fade_top, g.clip.fade_bottom, g.clip.opacity];
        let sides = [g.clip.fade_left, g.clip.fade_right, 0.0, 0.0];
        let words = chunk.as_chunks_mut::<4>().0;
        for (w, v) in words.iter_mut().zip(
            g.rect
                .iter()
                .chain(&g.uv)
                .chain(&g.color)
                .chain(&params)
                .chain(&g.clip.rect)
                .chain(&sides),
        ) {
            *w = v.to_ne_bytes();
        }
    }
}

pub fn write_shapes(shapes: &[Shape], out: &mut [u8]) {
    for (s, chunk) in shapes.iter().zip(out.as_chunks_mut::<112>().0) {
        // params.y: 0 flat, 1 glass, 2 image.
        let kind = match (s.image, s.glass) {
            (Some(_), _) => 2.0,
            (None, true) => 1.0,
            (None, false) => 0.0,
        };
        let params = [s.radius, kind, s.highlight, s.refraction];
        let uv = s.image.map_or([0.0; 4], |i| i.uv);
        let words = chunk.as_chunks_mut::<4>().0;
        for (w, v) in words
            .iter_mut()
            .zip(s.rect.iter().chain(&s.fill).chain(&params).chain(&uv))
        {
            *w = v.to_ne_bytes();
        }
        words[16] = s.image.map_or(0, |i| i.texture).to_ne_bytes();
        // tex.y: ring width in pixels, as float bits (flat shapes; 0 fills).
        words[17] = s.ring.to_ne_bytes();
        // tex.z, tex.w: the clip's top and bottom fades in pixels, as float bits.
        words[18] = s.clip.fade_top.to_ne_bytes();
        words[19] = s.clip.fade_bottom.to_ne_bytes();
        for (w, v) in words[20..24].iter_mut().zip(&s.clip.rect) {
            *w = v.to_ne_bytes();
        }
        words[24] = s.clip.opacity.to_ne_bytes();
        words[25] = s.bevel.to_ne_bytes();
        words[26] = s.clip.fade_left.to_ne_bytes();
        words[27] = s.clip.fade_right.to_ne_bytes();
    }
}

pub fn write_quads(quads: &[Quad], out: &mut [u8]) {
    for (q, chunk) in quads.iter().zip(out.as_chunks_mut::<48>().0) {
        let words = chunk.as_chunks_mut::<4>().0;
        for (w, v) in words.iter_mut().zip(q.rect.iter().chain(&q.uv)) {
            *w = v.to_ne_bytes();
        }
        words[8] = q.color.to_ne_bytes();
        words[9] = q.texture.to_ne_bytes();
        words[10] = [0; 4];
        let [top, bottom] = q.slant.map(|s| u32::from(half(s)));
        words[11] = (top | bottom << 16).to_ne_bytes();
    }
}

/// A float as the nearest half-precision one below it in size (`unpackHalf2x16` reads it
/// back): exact for the slants of text, which are quarters and eighths. Nothing for what
/// is too small to be one.
fn half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = (bits >> 16 & 0x8000) as u16;
    let exponent = (bits >> 23 & 0xFF) as i32 - 127 + 15;
    if exponent <= 0 {
        return sign;
    }
    // (Nothing in a GUI reaches the largest.)
    let exponent = exponent.min(30) as u16;
    sign | exponent << 10 | (bits >> 13 & 0x3FF) as u16
}

/// Writes one quad whose texture is read with `sampler`.
pub fn write_quad_with(quad: Quad, sampler: SamplerId, out: &mut [u8]) {
    write_quads(&[quad], out);
    out[40..44].copy_from_slice(&sampler.index().to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_shader() {
        let q = Quad {
            rect: [1.0, 2.0, 3.0, 4.0],
            uv: [0.5, 0.25, 0.75, 1.0],
            color: 0x80FF0000,
            texture: 9,
            slant: [0.0; 2],
        };
        let mut out = [0xAAu8; 48];
        write_quads(&[q], &mut out);
        assert_eq!(f32::from_ne_bytes(out[12..16].try_into().unwrap()), 4.0);
        assert_eq!(f32::from_ne_bytes(out[16..20].try_into().unwrap()), 0.5);
        assert_eq!(
            u32::from_ne_bytes(out[32..36].try_into().unwrap()),
            0x80FF0000
        );
        assert_eq!(u32::from_ne_bytes(out[36..40].try_into().unwrap()), 9);
        // Sprites' edges are clamped, unless said otherwise.
        assert_eq!(&out[40..], &[0; 8]);
        assert_eq!(SamplerId::NearestClamp.index(), 0);
        write_quad_with(q, SamplerId::NearestRepeat, &mut out);
        assert_eq!(u32::from_ne_bytes(out[36..40].try_into().unwrap()), 9);
        assert_eq!(u32::from_ne_bytes(out[40..44].try_into().unwrap()), 1);
        assert_eq!(&out[44..], &[0; 4]);
        // Slants as two halves: 1 and -0.75.
        let slanted = Quad {
            slant: [1.0, -0.75],
            ..q
        };
        write_quads(&[slanted], &mut out);
        assert_eq!(
            u32::from_ne_bytes(out[44..].try_into().unwrap()),
            0xBA00_3C00
        );
        let halves = [0.0, 0.25, 4.0, -1.25, 1e-6].map(half);
        assert_eq!(halves, [0, 0x3400, 0x4400, 0xBD00, 0]);
    }
}
