//! Writes ruthenium-ui's quads into the frame's data for `gui.vert`.

use ruthenium_ui::Quad;
use ruthenium_ui::modern::Shape;

/// Bytes per quad in the frame's data (std430: two vec4s and four uints).
pub const QUAD_BYTES: u32 = 48;

/// Bytes per shape in the frame's data (std430: three vec4s).
pub const SHAPE_BYTES: u32 = 48;

/// One frame's GUI: vanilla-style quads (with the GUI scale they were laid out for) and
/// the modern menus' shapes, drawn after the quads.
#[derive(Clone, Copy, Debug, Default)]
pub struct GuiFrame<'a> {
    pub scale: u32,
    pub quads: &'a [Quad],
    pub shapes: &'a [Shape],
}

pub fn write_shapes(shapes: &[Shape], out: &mut [u8]) {
    for (s, chunk) in shapes.iter().zip(out.as_chunks_mut::<48>().0) {
        let params = [
            s.radius,
            if s.glass { 1.0 } else { 0.0 },
            s.highlight,
            s.refraction,
        ];
        let words = chunk.as_chunks_mut::<4>().0;
        for (w, v) in words
            .iter_mut()
            .zip(s.rect.iter().chain(&s.fill).chain(&params))
        {
            *w = v.to_ne_bytes();
        }
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
        words[11] = [0; 4];
    }
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
        assert_eq!(&out[40..], &[0; 8]);
    }
}
