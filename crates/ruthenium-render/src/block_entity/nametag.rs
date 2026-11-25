//! The names over heads: where their glyphs lie, and how a name is turned to the camera.

use ruthenium_assets::Font;
use ruthenium_protocol::text::Style;

use super::affine::Affine;
use super::sign::{self, Ink, TextLayout, TextQuad};

/// How large a unit of the text is, in blocks (`EntityRenderer.NAMETAG_SCALE`).
const SCALE: f32 = 0.025;
/// How far over the top of its entity a name is, in blocks.
const ABOVE: f32 = 0.5;
/// How far a name is lifted over what stands below it, in blocks: a line and a bit.
pub(crate) const LINE: f32 = 9.0 * 1.15 * 0.025;
/// How much what lies behind a name covers, of 255: a quarter (the game's own measure
/// while its text background option is for the chat only).
pub(crate) const BACKGROUND_ALPHA: u8 = 63;
/// How much a name covers where it is seen through walls or is its sneaking owner's: a
/// half.
pub(crate) const TEXT_ALPHA: u8 = 127;
/// How far behind the text its background lies, in the text's units.
const BEHIND: f32 = -0.01;
const WHITE: u32 = 0xFF_FFFF;
/// The light a name that is seen through walls has at least where it is not.
pub(crate) const EMISSION: u8 = 2;
/// From how far a sneaking entity's name is seen, in blocks.
pub(crate) const DISCRETE_DISTANCE: f64 = 32.0;

/// One line over a head, laid out.
#[derive(Debug, Default)]
pub(crate) struct NameLine {
    /// Its glyphs and bars, whole.
    pub solid: Vec<TextQuad>,
    /// What lies behind it, then the same glyphs and bars, covering half.
    pub sheer: Vec<TextQuad>,
    /// Some of its characters change every frame.
    pub obfuscated: bool,
}

/// Lays out a line `offset` units below its place, centred.
pub(crate) fn layout(
    line: &[(char, Style)],
    offset: i32,
    font: &Font,
    pick: &mut dyn FnMut(usize) -> usize,
) -> NameLine {
    // (The width is a whole number; its half need not be.)
    let left = -sign::width(line, font).ceil() / 2.0;
    let top = offset as f32;
    let mut out = TextLayout::default();
    let end = sign::line(&mut out, line, font, pick, [left, top], WHITE);
    let background = TextQuad {
        place: [left - 1.0, top - 1.0, end, top + 9.0],
        slant: [0.0; 2],
        nearer: BEHIND,
        colour: 0,
        alpha: BACKGROUND_ALPHA,
        ink: Ink::Bar,
        around: 0.0,
    };
    let sheer = std::iter::once(background)
        .chain(out.text.iter().map(|q| TextQuad {
            alpha: TEXT_ALPHA,
            ..*q
        }))
        .collect();
    NameLine {
        solid: out.text,
        sheer,
        obfuscated: out.obfuscated,
    }
}

/// From a name's own space (y runs down) to blocks from the camera: over `top`, the top
/// of its entity, `lift` blocks higher, facing a camera that looks along `forward` and
/// has `right` and `up` as its other axes.
pub(crate) fn transform(top: [f32; 3], lift: f32, [forward, right, up]: [[f32; 3]; 3]) -> Affine {
    Affine(std::array::from_fn(|i| {
        let at = top[i] + if i == 1 { ABOVE + lift } else { 0.0 };
        [right[i] * SCALE, -up[i] * SCALE, -forward[i] * SCALE, at]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_assets::font::Glyph;

    /// Every character six wide, a space four and a half.
    fn font() -> Font {
        let space = Glyph {
            advance: 4.5,
            picture: None,
            ..Glyph::MISSING
        };
        Font::of_glyphs([(' ', space)])
    }

    fn styled(s: &str, style: Style) -> Vec<(char, Style)> {
        s.chars().map(|c| (c, style)).collect()
    }

    #[test]
    fn a_name_is_centred_on_its_whole_width() {
        // Three characters and a space: 22.5 wide, 23 as a whole number.
        let line = styled("ab c", Style::default());
        let laid = layout(&line, 0, &font(), &mut |_| 0);
        assert_eq!(laid.solid.len(), 3);
        assert_eq!(laid.solid[0].place[0], -11.5);
        assert_eq!(laid.solid[2].place[0], -11.5 + 6.0 + 6.0 + 4.5);
        // What lies behind reaches a unit to the left and above, to the pen's end and to
        // the line's foot.
        let background = laid.sheer[0];
        assert_eq!(background.place, [-12.5, -1.0, 11.0, 9.0]);
        assert_eq!((background.colour, background.alpha), (0, 63));
        assert_eq!((background.ink, background.nearer), (Ink::Bar, -0.01));
        assert_eq!(laid.sheer.len(), 4);
        for (whole, seen_through) in laid.solid.iter().zip(&laid.sheer[1..]) {
            assert_eq!((whole.alpha, seen_through.alpha), (255, 127));
            assert_eq!(whole.place, seen_through.place);
            assert_eq!(whole.colour, 0xFF_FFFF);
        }
        assert!(!laid.obfuscated);
    }

    #[test]
    fn a_name_keeps_its_colours_and_can_stand_higher() {
        let red = Style {
            color: Some(0xFF_5555),
            underlined: true,
            ..Style::default()
        };
        let mut line = styled("ab", red);
        line.extend(styled("c", Style::default()));
        let laid = layout(&line, -10, &font(), &mut |_| 0);
        // Three glyphs, then the bar under the two.
        assert_eq!(laid.solid.len(), 5);
        assert_eq!(laid.solid[0].colour, 0xFF_5555);
        assert_eq!(laid.solid[2].colour, 0xFF_FFFF);
        assert_eq!(laid.solid[3].ink, Ink::Bar);
        assert_eq!(laid.solid[3].place[1], -10.0 + 8.0);
        assert_eq!(laid.sheer[0].place, [-10.0, -11.0, 9.0, -1.0]);
        // Nothing to write is still something to lie behind.
        let empty = layout(&[], 0, &font(), &mut |_| 0);
        assert!(empty.solid.is_empty());
        assert_eq!(empty.sheer[0].place, [-1.0, -1.0, 0.0, 9.0]);
    }

    #[test]
    fn a_name_faces_the_camera() {
        // A camera that looks north (towards -z): east is to its right.
        let axes = [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let t = transform([3.0, 1.8, -5.0], 0.0, axes);
        assert_eq!(t.point([0.0; 3]), [3.0, 2.3, -5.0]);
        // The text's right is the camera's, its down is down, and its front faces back
        // at the camera.
        let moved = |p: [f32; 3]| {
            let q = t.point(p);
            [q[0] - 3.0, q[1] - 2.3, q[2] + 5.0]
        };
        assert_eq!(moved([40.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
        assert_eq!(moved([0.0, 40.0, 0.0]), [0.0, -1.0, 0.0]);
        assert_eq!(moved([0.0, 0.0, 40.0]), [0.0, 0.0, 1.0]);
        // Lifted over a line below it.
        let lifted = transform([3.0, 1.8, -5.0], LINE, axes);
        assert_eq!(lifted.point([0.0; 3])[1], 1.8 + 0.5 + 0.25875);
    }
}
