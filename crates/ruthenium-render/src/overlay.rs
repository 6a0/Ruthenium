//! The development overlay (F3, `--overlay`): frame rate, 1% and 0.1% lows, CPU and GPU
//! frame times and a graph of the last frames, drawn as solid rectangles from the frame's
//! data. Its text uses a built-in 3×5 pixel font (Ruthenium's own, uppercase ASCII), so it
//! works before any game files are imported; the vanilla font comes with `ruthenium-ui`.
//!
//! Nothing allocates per frame: the text changes once a second, into reused strings.

use std::fmt::Write as _;

/// Lines of text.
pub const LINES: usize = 4;
/// Frames in the graph, one pixel column each.
const GRAPH_FRAMES: usize = 240;
const GRAPH_HEIGHT: f32 = 64.0;
/// Size of one font pixel in window pixels.
const SCALE: f32 = 2.0;
const GLYPH_ADVANCE: f32 = 4.0 * SCALE;
const LINE_HEIGHT: f32 = 7.0 * SCALE;
const MARGIN: f32 = 8.0;
const PADDING: f32 = 6.0;

const PANEL: [f32; 4] = [0.0, 0.0, 0.0, 0.62];
const TEXT: [f32; 4] = [0.92, 0.94, 1.0, 1.0];
const GOOD: [f32; 4] = [0.35, 0.85, 0.45, 0.9];
const SLOW: [f32; 4] = [0.95, 0.80, 0.25, 0.9];
const HITCH: [f32; 4] = [0.95, 0.30, 0.30, 0.95];
const GUIDE: [f32; 4] = [1.0, 1.0, 1.0, 0.35];

/// One rectangle as `overlay.vert` reads it: x, y, width, height, then RGBA.
pub type Rect = [f32; 8];
/// Bytes per [`Rect`] in the frame's data.
pub const RECT_BYTES: u32 = 32;

/// The 3×5 font for ASCII 32 to 127 (lowercase drawn as uppercase; unknown glyphs as
/// `?`): bit `row * 3 + column` is lit, row 0 at the top.
const FONT: [u16; 96] = [
    0x0000, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x52a5, 0x21a7, 0x21a7, 0x4494, 0x1491, 0x21a7, 0x21a7,
    0x21a7, 0x01c0, 0x2000, 0x12a4, 0x7b6f, 0x749a, 0x73e7, 0x79a7, 0x49ed, 0x79cf, 0x7bcf, 0x2527,
    0x7bef, 0x79ef, 0x0410, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x5bea, 0x3aeb, 0x624e,
    0x3b6b, 0x72cf, 0x12cf, 0x6b4e, 0x5bed, 0x7497, 0x2b24, 0x5aed, 0x7249, 0x5bfd, 0x5b6b, 0x2b6a,
    0x12eb, 0x676a, 0x5aeb, 0x388e, 0x2497, 0x7b6d, 0x2b6d, 0x5fed, 0x5aad, 0x24ad, 0x72a7, 0x21a7,
    0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x5bea, 0x3aeb, 0x624e, 0x3b6b, 0x72cf, 0x12cf, 0x6b4e,
    0x5bed, 0x7497, 0x2b24, 0x5aed, 0x7249, 0x5bfd, 0x5b6b, 0x2b6a, 0x12eb, 0x676a, 0x5aeb, 0x388e,
    0x2497, 0x7b6d, 0x2b6d, 0x5fed, 0x0aa8, 0x24ad, 0x72a7, 0x21a7, 0x21a7, 0x21a7, 0x21a7, 0x21a7,
];

fn glyph(c: char) -> u16 {
    let i = c as u32;
    if (32..128).contains(&i) {
        FONT[(i - 32) as usize]
    } else {
        FONT[(b'?' - 32) as usize]
    }
}

/// See the module docs.
#[derive(Debug)]
pub struct Overlay {
    visible: bool,
    /// Frame intervals in milliseconds, a ring.
    history: [f32; GRAPH_FRAMES],
    head: usize,
    filled: usize,
    lines: [String; LINES],
    rects: Vec<Rect>,
}

impl Default for Overlay {
    fn default() -> Overlay {
        Overlay::new()
    }
}

impl Overlay {
    pub fn new() -> Overlay {
        Overlay {
            visible: false,
            history: [0.0; GRAPH_FRAMES],
            head: 0,
            filled: 0,
            lines: std::array::from_fn(|_| String::with_capacity(64)),
            rects: Vec::with_capacity(4096),
        }
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Adds one frame interval to the graph.
    pub fn push_frame(&mut self, ms: f32) {
        self.history[self.head] = ms;
        self.head = (self.head + 1) % GRAPH_FRAMES;
        self.filled = (self.filled + 1).min(GRAPH_FRAMES);
    }

    /// Replaces line `i` with what `write` writes (into the line's reused string).
    pub fn set_line(&mut self, i: usize, write: impl FnOnce(&mut String) -> std::fmt::Result) {
        let line = &mut self.lines[i];
        line.clear();
        let _ = write(line);
    }

    /// Writes the standard lines from one second's statistics.
    pub fn set_stats(&mut self, s: &OverlayStats<'_>) {
        self.set_line(0, |l| {
            write!(
                l,
                "{:.1} FPS  1% {:.1}  0.1% {:.1}",
                s.fps, s.low_1, s.low_01
            )
        });
        self.set_line(1, |l| {
            write!(
                l,
                "FRAME {:.2} MS  CPU {:.2}  GPU {:.2}",
                s.frame_ms, s.cpu_ms, s.gpu_ms
            )
        });
        self.set_line(2, |l| {
            write!(
                l,
                "0.1%/MEDIAN {:.2}  MAX {:.1} MS",
                s.p999_over_median, s.max_ms
            )
        });
        self.set_line(3, |l| write!(l, "{}  {}", s.present, s.gpu));
    }

    /// Where the overlay's panel ends, in pixels from the target's top: text laid out
    /// below it does not run into it.
    pub const BOTTOM: f32 =
        MARGIN + 2.0 * PADDING + LINES as f32 * LINE_HEIGHT + PADDING + GRAPH_HEIGHT;

    /// The overlay's rectangles for a `width` × `height` target, back to front.
    pub fn build(&mut self, width: u32, height: u32) -> &[Rect] {
        self.rects.clear();
        if !self.visible || width == 0 || height == 0 {
            return &self.rects;
        }
        let text_columns = self
            .lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let inner_w = (text_columns as f32 * GLYPH_ADVANCE).max(GRAPH_FRAMES as f32);
        let inner_h = LINES as f32 * LINE_HEIGHT + PADDING + GRAPH_HEIGHT;
        self.rects.push(rect(
            MARGIN,
            MARGIN,
            inner_w + 2.0 * PADDING,
            inner_h + 2.0 * PADDING,
            PANEL,
        ));
        let x0 = MARGIN + PADDING;
        let mut y = MARGIN + PADDING;
        for i in 0..LINES {
            let mut x = x0;
            for c in self.lines[i].chars() {
                let bits = glyph(c);
                for row in 0..5 {
                    // Runs of lit pixels in a row become one rectangle.
                    let mut col = 0;
                    while col < 3 {
                        if bits & (1 << (row * 3 + col)) == 0 {
                            col += 1;
                            continue;
                        }
                        let start = col;
                        while col < 3 && bits & (1 << (row * 3 + col)) != 0 {
                            col += 1;
                        }
                        self.rects.push(rect(
                            x + start as f32 * SCALE,
                            y + row as f32 * SCALE,
                            (col - start) as f32 * SCALE,
                            SCALE,
                            TEXT,
                        ));
                    }
                }
                x += GLYPH_ADVANCE;
            }
            y += LINE_HEIGHT;
        }
        // The graph: one column per frame, oldest on the left, scaled to 4× the median;
        // green up to 1.5× the median (the menu gate), yellow up to 3×, red above.
        let top = y + PADDING;
        let median = self.median();
        if median > 0.0 {
            let full = 4.0 * median;
            let bottom = top + GRAPH_HEIGHT;
            let guide = bottom - (1.5 / 4.0) * GRAPH_HEIGHT;
            self.rects
                .push(rect(x0, guide, GRAPH_FRAMES as f32, 1.0, GUIDE));
            let start = (self.head + GRAPH_FRAMES - self.filled) % GRAPH_FRAMES;
            let left = x0 + (GRAPH_FRAMES - self.filled) as f32;
            for i in 0..self.filled {
                let ms = self.history[(start + i) % GRAPH_FRAMES];
                let h = (ms / full).min(1.0) * GRAPH_HEIGHT;
                let color = if ms <= 1.5 * median {
                    GOOD
                } else if ms <= 3.0 * median {
                    SLOW
                } else {
                    HITCH
                };
                self.rects
                    .push(rect(left + i as f32, bottom - h, 1.0, h.max(1.0), color));
            }
        }
        &self.rects
    }

    /// Median of the graph's frames (0 if there are none).
    fn median(&self) -> f32 {
        if self.filled == 0 {
            return 0.0;
        }
        let mut sorted = [0.0f32; GRAPH_FRAMES];
        let n = self.filled;
        let start = (self.head + GRAPH_FRAMES - n) % GRAPH_FRAMES;
        for (i, s) in sorted[..n].iter_mut().enumerate() {
            *s = self.history[(start + i) % GRAPH_FRAMES];
        }
        let (_, m, _) = sorted[..n].select_nth_unstable_by(n / 2, f32::total_cmp);
        *m
    }
}

/// One second of statistics, for [`Overlay::set_stats`].
#[derive(Clone, Copy, Debug)]
pub struct OverlayStats<'a> {
    pub fps: f64,
    pub low_1: f64,
    pub low_01: f64,
    pub frame_ms: f64,
    pub cpu_ms: f64,
    pub gpu_ms: f64,
    pub p999_over_median: f64,
    pub max_ms: f64,
    pub present: &'a str,
    pub gpu: &'a str,
}

fn rect(x: f32, y: f32, w: f32, h: f32, c: [f32; 4]) -> Rect {
    [x, y, w, h, c[0], c[1], c[2], c[3]]
}

/// Writes rectangles in the layout `overlay.vert` reads (std430: eight floats).
pub fn write_rects(rects: &[Rect], out: &mut [u8]) {
    for (r, chunk) in rects.iter().zip(out.as_chunks_mut::<32>().0) {
        for (v, b) in r.iter().zip(chunk.as_chunks_mut::<4>().0) {
            *b = v.to_ne_bytes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_and_letters_have_glyphs() {
        let unknown = glyph('?');
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ.%:/-()x".chars() {
            assert_ne!(glyph(c), unknown, "{c}");
            assert_ne!(glyph(c), 0, "{c}");
        }
        assert_eq!(glyph(' '), 0);
        assert_eq!(glyph('a'), glyph('A'));
        assert_eq!(glyph('é'), unknown);
        // Glyphs are distinct, so the overlay is readable.
        let set: std::collections::HashSet<u16> = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ.%:/-()"
            .chars()
            .map(glyph)
            .collect();
        assert_eq!(set.len(), 43);
    }

    #[test]
    fn hidden_overlay_draws_nothing() {
        let mut o = Overlay::new();
        o.push_frame(4.0);
        assert!(o.build(1280, 720).is_empty());
        o.set_visible(true);
        assert!(!o.build(1280, 720).is_empty());
    }

    #[test]
    fn graph_colours_follow_the_median() {
        let mut o = Overlay::new();
        o.set_visible(true);
        for _ in 0..100 {
            o.push_frame(4.0);
        }
        o.push_frame(5.0); // 1.25x: good
        o.push_frame(10.0); // 2.5x: slow
        o.push_frame(20.0); // 5x: hitch
        assert_eq!(o.median(), 4.0);
        let rects = o.build(1280, 720).to_vec();
        let last: Vec<[f32; 4]> = rects[rects.len() - 3..]
            .iter()
            .map(|r| [r[4], r[5], r[6], r[7]])
            .collect();
        assert_eq!(last, vec![GOOD, SLOW, HITCH]);
        // The hitch is clipped to the graph's height (4x the median).
        assert_eq!(rects[rects.len() - 1][3], GRAPH_HEIGHT);
    }

    #[test]
    fn rects_serialise_as_eight_floats() {
        let r = [rect(1.0, 2.0, 3.0, 4.0, [0.5, 0.25, 0.125, 1.0])];
        let mut out = [0u8; 32];
        write_rects(&r, &mut out);
        assert_eq!(f32::from_ne_bytes(out[8..12].try_into().unwrap()), 3.0);
        assert_eq!(f32::from_ne_bytes(out[28..32].try_into().unwrap()), 1.0);
    }
}
