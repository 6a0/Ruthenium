/// Where, and how strongly, a shape or glyph may draw, in window pixels, with optional fades: within
/// `fade_top` pixels of the top edge (and `fade_bottom` of the bottom one) it becomes
/// transparent, so a scrolled list melts away at its ends instead of being cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    /// `x0, y0, x1, y1`.
    pub rect: [f32; 4],
    pub fade_top: f32,
    pub fade_bottom: f32,
    /// Multiplies coverage (fading a whole screen in).
    pub opacity: f32,
}

impl Clip {
    /// Draws everywhere.
    pub const NONE: Clip = Clip {
        rect: [-1.0e9, -1.0e9, 1.0e9, 1.0e9],
        fade_top: 0.0,
        fade_bottom: 0.0,
        opacity: 1.0,
    };

    /// A hard clip to `rect`.
    pub fn rect(rect: [f32; 4]) -> Clip {
        Clip { rect, ..Clip::NONE }
    }
}

impl Default for Clip {
    fn default() -> Clip {
        Clip::NONE
    }
}
