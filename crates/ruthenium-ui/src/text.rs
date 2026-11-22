//! Text for the modern menus: Nunito (bundled, SIL OFL 1.1), shaped with rustybuzz (kerning
//! and the font's default features), drawn from a multi-channel signed distance field
//! (MSDF) atlas built with fdsm, so one atlas renders crisp text at any size.
//!
//! Each weight is a static instance of Nunito with overlaps removed (distance fields need
//! that; see `fonts/README.md`); each has its own atlas entries.

use std::collections::HashMap;

use fdsm::bezier::scanline::FillRule;
use fdsm::correct_error::{ErrorCorrectionConfig, correct_error_msdf};
use fdsm::generate::generate_msdf;
use fdsm::render::correct_sign_msdf;
use fdsm::shape::Shape;
use fdsm::transform::Transform;
use image::Rgb32FImage;
use rayon::prelude::*;
use rustybuzz::ttf_parser::GlyphId;
use rustybuzz::{Face, UnicodeBuffer};

/// The bundled fonts, by weight (`fonts/README.md`).
const FONTS: [(Weight, &[u8]); 4] = [
    (
        Weight::REGULAR,
        include_bytes!("../fonts/Nunito-Regular.ttf"),
    ),
    (Weight::MEDIUM, include_bytes!("../fonts/Nunito-Medium.ttf")),
    (
        Weight::SEMIBOLD,
        include_bytes!("../fonts/Nunito-SemiBold.ttf"),
    ),
    (
        Weight::EXTRABOLD,
        include_bytes!("../fonts/Nunito-ExtraBold.ttf"),
    ),
];

/// Atlas texels per em.
const EM_TEXELS: f64 = 48.0;
/// Distance range of the field in atlas texels: 0..1 in the texture spans -3..+3 texels.
const RANGE: f64 = 6.0;
/// Texels kept around each glyph's outline (at least half the range, plus filtering).
const PAD: f64 = 4.0;
/// Atlas side in texels.
pub const ATLAS_SIZE: u32 = 1024;

/// A font weight (100 to 900). Only the bundled weights exist; others use the nearest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Weight(pub u16);

impl Weight {
    pub const REGULAR: Weight = Weight(400);
    pub const MEDIUM: Weight = Weight(500);
    pub const SEMIBOLD: Weight = Weight(600);
    pub const EXTRABOLD: Weight = Weight(800);
}

/// The menus' typeface at a set of weights.
pub struct Fonts {
    faces: Vec<(Weight, Face<'static>)>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let w: Vec<u16> = self.faces.iter().map(|(w, _)| w.0).collect();
        f.debug_struct("Fonts").field("weights", &w).finish()
    }
}

/// How a run of text looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Em size in pixels.
    pub size: f32,
    pub weight: Weight,
    /// Extra space after each glyph but the last, in ems.
    pub tracking: f32,
}

/// A shaped line: glyphs and their pen positions in pixels from the line's origin (the
/// baseline's left end).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub weight: Weight,
    pub size: f32,
    pub glyphs: Vec<(u16, f32, f32)>,
    /// Advance width in pixels.
    pub width: f32,
}

impl Fonts {
    /// The typeface at each of `weights` (each the nearest bundled weight, under the asked one).
    pub fn new(weights: &[Weight]) -> Fonts {
        let faces = weights
            .iter()
            .map(|&w| {
                let (_, bytes) = FONTS
                    .iter()
                    .min_by_key(|(b, _)| b.0.abs_diff(w.0))
                    .expect("bundled fonts");
                (
                    w,
                    Face::from_slice(bytes, 0).expect("the bundled font parses"),
                )
            })
            .collect();
        Fonts { faces }
    }

    /// The face for `weight`, or the nearest one loaded.
    fn face(&self, weight: Weight) -> &Face<'static> {
        &self
            .faces
            .iter()
            .min_by_key(|(w, _)| w.0.abs_diff(weight.0))
            .expect("at least one weight")
            .1
    }

    pub fn weights(&self) -> impl Iterator<Item = Weight> + '_ {
        self.faces.iter().map(|(w, _)| *w)
    }

    /// Capital height in ems (for centring text on a line).
    pub fn cap_height(&self, weight: Weight) -> f32 {
        let f = self.face(weight);
        let cap = f
            .capital_height()
            .unwrap_or((f.units_per_em() as f32 * 0.7) as i16);
        f32::from(cap) / f.units_per_em() as f32
    }

    /// Shapes one line of text.
    pub fn shape(&self, text: &str, style: TextStyle) -> Line {
        let face = self.face(style.weight);
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        let out = rustybuzz::shape(face, &[], buffer);
        let scale = style.size / face.units_per_em() as f32;
        let track = style.tracking * style.size;
        let n = out.len();
        let mut glyphs = Vec::with_capacity(n);
        let mut x = 0.0f32;
        for (i, (info, pos)) in out
            .glyph_infos()
            .iter()
            .zip(out.glyph_positions())
            .enumerate()
        {
            let gx = x + pos.x_offset as f32 * scale;
            let gy = -(pos.y_offset as f32) * scale;
            glyphs.push((info.glyph_id as u16, gx, gy));
            x += pos.x_advance as f32 * scale;
            if i + 1 < n {
                x += track;
            }
        }
        Line {
            weight: style.weight,
            size: style.size,
            glyphs,
            width: x,
        }
    }
}

/// Where a glyph is in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    /// `u0, v0, u1, v1`.
    pub uv: [f32; 4],
    /// The field's quad relative to the pen, in ems, y down: `x0, y0, x1, y1`.
    pub plane: [f32; 4],
}

/// The distance-field atlas: RGBA8 texels (MSDF in RGB), packed in shelves.
pub struct GlyphAtlas {
    pub size: u32,
    pub pixels: Vec<u8>,
    /// `None` for glyphs with no outline (spaces).
    glyphs: HashMap<(Weight, u16), Option<GlyphSlot>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("size", &self.size)
            .field("glyphs", &self.glyphs.len())
            .finish()
    }
}

impl GlyphAtlas {
    pub fn new(size: u32) -> GlyphAtlas {
        GlyphAtlas {
            size,
            pixels: vec![0; (size * size * 4) as usize],
            glyphs: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
        }
    }

    pub fn get(&self, weight: Weight, glyph: u16) -> Option<GlyphSlot> {
        self.glyphs.get(&(weight, glyph)).copied().flatten()
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Adds every glyph of `text` at `weight` (shaped as one line).
    pub fn add_text(&mut self, fonts: &Fonts, weight: Weight, text: &str) {
        let line = fonts.shape(
            text,
            TextStyle {
                size: 1.0,
                weight,
                tracking: 0.0,
            },
        );
        let glyphs: Vec<(Weight, u16)> = line.glyphs.iter().map(|&(g, _, _)| (weight, g)).collect();
        self.add_many(fonts, &glyphs);
    }

    /// Adds glyphs, generating their fields in parallel and packing them tallest first.
    /// Returns false if some did not fit.
    pub fn add_many(&mut self, fonts: &Fonts, glyphs: &[(Weight, u16)]) -> bool {
        let mut todo: Vec<(Weight, u16)> = glyphs
            .iter()
            .copied()
            .filter(|k| !self.glyphs.contains_key(k))
            .collect();
        todo.sort_unstable();
        todo.dedup();
        let mut fields: Vec<((Weight, u16), Option<Field>)> = todo
            .into_par_iter()
            .map(|(w, g)| ((w, g), render_glyph(fonts.face(w), g)))
            .collect();
        // Tallest first packs shelves tightly; ties by key keep the layout deterministic.
        fields.sort_by(|a, b| {
            let h = |f: &Option<Field>| f.as_ref().map_or(0, |f| f.h);
            h(&b.1).cmp(&h(&a.1)).then(a.0.cmp(&b.0))
        });
        let mut all = true;
        for (key, field) in fields {
            all &= self.place(key, field);
        }
        all
    }

    /// Room for a `w` x `h` block, or `None` if the atlas is full.
    fn pack(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.shelf_x + w > self.size {
            self.shelf_y += self.shelf_h + 1;
            self.shelf_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + h > self.size || w > self.size {
            return None;
        }
        let at = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        Some(at)
    }

    /// Generates one glyph's field, if it isn't there yet. Returns false if it did not fit.
    pub fn add(&mut self, fonts: &Fonts, weight: Weight, glyph: u16) -> bool {
        self.add_many(fonts, &[(weight, glyph)])
    }

    fn place(&mut self, key: (Weight, u16), field: Option<Field>) -> bool {
        let Some(f) = field else {
            self.glyphs.insert(key, None);
            return true;
        };
        let Some((ax, ay)) = self.pack(f.w, f.h) else {
            return false;
        };
        for y in 0..f.h {
            for x in 0..f.w {
                let i = (((ay + y) * self.size + ax + x) * 4) as usize;
                let p = f.texels[(y * f.w + x) as usize];
                self.pixels[i..i + 3].copy_from_slice(&p);
                self.pixels[i + 3] = 255;
            }
        }
        let size = self.size as f32;
        let slot = GlyphSlot {
            uv: [
                ax as f32 / size,
                ay as f32 / size,
                (ax + f.w) as f32 / size,
                (ay + f.h) as f32 / size,
            ],
            plane: f.plane,
        };
        self.glyphs.insert(key, Some(slot));
        true
    }
}

/// One glyph's distance field before packing.
struct Field {
    w: u32,
    h: u32,
    texels: Vec<[u8; 3]>,
    plane: [f32; 4],
}

/// The MSDF of one glyph, or `None` if it has no outline.
fn render_glyph(face: &Face<'_>, glyph: u16) -> Option<Field> {
    let gid = GlyphId(glyph);
    let bbox = face.glyph_bounding_box(gid)?;
    let mut shape = fdsm_ttf_parser::load_shape_from_face(face, gid)?;
    let upem = f64::from(face.units_per_em());
    let s = EM_TEXELS / upem;
    let w = ((f64::from(bbox.x_max) - f64::from(bbox.x_min)) * s + 2.0 * PAD).ceil() as u32;
    let h = ((f64::from(bbox.y_max) - f64::from(bbox.y_min)) * s + 2.0 * PAD).ceil() as u32;
    // Font units (y up) to field texels (y down), with the padding.
    let m = nalgebra::Matrix3::new(
        s,
        0.0,
        PAD - f64::from(bbox.x_min) * s,
        0.0,
        -s,
        PAD + f64::from(bbox.y_max) * s,
        0.0,
        0.0,
        1.0,
    );
    shape.transform(&nalgebra::Affine2::from_matrix_unchecked(m));
    let colored = Shape::edge_coloring_simple(shape, 0.03, 0x5275_7468);
    let prepared = colored.prepare();
    let mut field = Rgb32FImage::new(w, h);
    generate_msdf(&prepared, RANGE, &mut field);
    correct_error_msdf(
        &mut field,
        &colored,
        &prepared,
        RANGE,
        &ErrorCorrectionConfig::default(),
    );
    correct_sign_msdf(&mut field, &prepared, FillRule::Nonzero);
    let texels = field
        .pixels()
        .map(|p| p.0.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    let x0 = (f64::from(bbox.x_min) / upem - PAD / EM_TEXELS) as f32;
    let y0 = (-f64::from(bbox.y_max) / upem - PAD / EM_TEXELS) as f32;
    Some(Field {
        w,
        h,
        texels,
        plane: [
            x0,
            y0,
            x0 + (f64::from(w) / EM_TEXELS) as f32,
            y0 + (f64::from(h) / EM_TEXELS) as f32,
        ],
    })
}

/// One glyph to draw, in window pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// Straight-alpha RGBA.
    pub color: [f32; 4],
    /// The field's range in screen pixels at this size (for the edge's sharpness).
    pub range: f32,
}

/// Glyph quads for one frame.
#[derive(Debug, Default)]
pub struct TextList {
    quads: Vec<GlyphQuad>,
}

impl TextList {
    pub fn new() -> TextList {
        TextList {
            quads: Vec::with_capacity(256),
        }
    }

    pub fn clear(&mut self) {
        self.quads.clear();
    }

    pub fn quads(&self) -> &[GlyphQuad] {
        &self.quads
    }

    /// Draws `line` with its baseline's left end at `(x, y)` pixels. Glyphs missing from
    /// the atlas are skipped.
    pub fn line(&mut self, atlas: &GlyphAtlas, line: &Line, x: f32, y: f32, color: [f32; 4]) {
        let range = (RANGE / EM_TEXELS) as f32 * line.size;
        for &(g, gx, gy) in &line.glyphs {
            let Some(slot) = atlas.get(line.weight, g) else {
                continue;
            };
            let [px0, py0, px1, py1] = slot.plane.map(|v| v * line.size);
            let (ox, oy) = (x + gx, y + gy);
            self.quads.push(GlyphQuad {
                rect: [ox + px0, oy + py0, ox + px1, oy + py1],
                uv: slot.uv,
                color,
                range,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_with_kerning_and_tracking() {
        let fonts = Fonts::new(&[Weight::REGULAR]);
        let style = TextStyle {
            size: 100.0,
            weight: Weight::REGULAR,
            tracking: 0.0,
        };
        let av = fonts.shape("AV", style);
        let a = fonts.shape("A", style);
        let v = fonts.shape("V", style);
        assert_eq!(av.glyphs.len(), 2);
        // Kerning pulls A and V together.
        assert!(
            av.width < a.width + v.width,
            "{} vs {}",
            av.width,
            a.width + v.width
        );
        let tracked = fonts.shape(
            "AV",
            TextStyle {
                tracking: 0.1,
                ..style
            },
        );
        assert!((tracked.width - av.width - 10.0).abs() < 1e-3);
        assert!((0.6..0.8).contains(&fonts.cap_height(Weight::REGULAR)));
    }

    #[test]
    fn heavier_weights_are_wider() {
        let fonts = Fonts::new(&[Weight::REGULAR, Weight::EXTRABOLD]);
        let style = |weight| TextStyle {
            size: 100.0,
            weight,
            tracking: 0.0,
        };
        let thin = fonts.shape("RUTHENIUM", style(Weight::REGULAR));
        let bold = fonts.shape("RUTHENIUM", style(Weight::EXTRABOLD));
        assert!(bold.width > thin.width);
    }

    #[test]
    fn atlas_fields_have_inside_and_outside() {
        let fonts = Fonts::new(&[Weight::MEDIUM]);
        let mut atlas = GlyphAtlas::new(256);
        atlas.add_text(&fonts, Weight::MEDIUM, "O ");
        let line = fonts.shape(
            "O ",
            TextStyle {
                size: 1.0,
                weight: Weight::MEDIUM,
                tracking: 0.0,
            },
        );
        let o = atlas
            .get(Weight::MEDIUM, line.glyphs[0].0)
            .expect("O has a field");
        assert!(
            atlas.get(Weight::MEDIUM, line.glyphs[1].0).is_none(),
            "space has none"
        );
        // The plane spans the glyph's ink plus padding; the O's ring is inside (> 0.5) and
        // its centre outside (< 0.5), as is the padding.
        let texel = |u: f32, v: f32| {
            let (x, y) = ((u * 256.0) as usize, (v * 256.0) as usize);
            let i = (y * 256 + x) * 4;
            let p = &atlas.pixels[i..i + 3];
            let mut c = [p[0], p[1], p[2]];
            c.sort();
            c[1]
        };
        let [u0, v0, u1, v1] = o.uv;
        let (cu, cv) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
        assert!(texel(cu, cv) < 128, "centre of O is outside");
        assert!(
            texel(u0 + 1.0 / 256.0, v0 + 1.0 / 256.0) < 128,
            "corner padding is outside"
        );
        // Walk right from the centre: we must cross into the ring.
        let ring = (0..40).any(|i| texel(cu + i as f32 / 256.0, cv) > 160);
        assert!(ring, "the ring is inside");
        assert!(
            o.plane[1] < 0.0 && o.plane[3] > 0.0,
            "O sits on the baseline: {:?}",
            o.plane
        );
    }
}
