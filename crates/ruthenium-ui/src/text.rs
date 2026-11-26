//! Text for the modern menus: Nunito (bundled, SIL OFL 1.1), shaped with rustybuzz (kerning
//! and the font's default features), drawn from a multi-channel signed distance field
//! (MSDF) atlas built with fdsm, so one atlas renders crisp text at any size.
//!
//! Each weight is a static instance of Nunito with overlaps removed (distance fields need
//! that; see `fonts/README.md`); each has its own atlas entries.
//!
//! Characters Nunito lacks come from fonts already on the system ([`fallback`]), and
//! after them from the game's own GNU Unifont ([`unifont`]). A word that needs one is set
//! wholly in it where it can be, so a word in small capitals does not mix two typefaces. The atlas starts with what the menus' own text needs and takes in
//! the glyphs of other text (server names and messages, typed text) as it is drawn:
//! [`TextList`] notes the glyphs it had to leave out, and they are rendered and added.

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

pub mod fallback;
mod unifont;

use fallback::{Chain, SystemFonts, has_outline};
use fdsm::shape::Contour;

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
/// Atlas side in texels at first.
pub const ATLAS_SIZE: u32 = 1024;
/// The largest the atlas grows to as it takes in glyphs.
pub const MAX_ATLAS_SIZE: u32 = 2048;

/// A font weight (100 to 900). Only the bundled weights exist; others use the nearest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Weight(pub u16);

impl Weight {
    pub const REGULAR: Weight = Weight(400);
    pub const MEDIUM: Weight = Weight(500);
    pub const SEMIBOLD: Weight = Weight(600);
    pub const EXTRABOLD: Weight = Weight(800);
}

/// Which font a glyph is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FaceId {
    /// Nunito at this weight (the nearest bundled one draws it).
    Nunito(Weight),
    /// A system font, by its file's place in the fallback list.
    System(u16),
    /// The game's Unifont: the plane of the character, whose place in it is the glyph id.
    Unifont(u8),
}

/// One glyph of a shaped line: its font, its id there, and its pen position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub face: FaceId,
    pub id: u16,
    pub x: f32,
    pub y: f32,
}

impl Glyph {
    /// The glyph's atlas key.
    pub fn key(&self) -> (FaceId, u16) {
        (self.face, self.id)
    }
}

/// The menus' typeface at a set of weights, and the fallback fonts behind it.
pub struct Fonts {
    faces: Vec<(Weight, Face<'static>)>,
    fallback: Fallback,
}

/// Where the characters Nunito lacks come from.
#[derive(Clone, Copy)]
enum Fallback {
    None,
    /// The process's: the system's fonts (found the first time a character needs them)
    /// and Unifont (once it has come), as [`fallback::set_mode`] says.
    System,
    Given(Chain),
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
    pub size: f32,
    pub glyphs: Vec<Glyph>,
    /// Advance width in pixels.
    pub width: f32,
}

impl Fonts {
    /// The typeface at each of `weights` (each the nearest bundled weight, under the asked
    /// one), with the system's fonts for what it lacks (unless they are turned off).
    pub fn new(weights: &[Weight]) -> Fonts {
        Fonts {
            fallback: Fallback::System,
            ..Fonts::nunito(weights)
        }
    }

    /// Like [`Fonts::new`], with these fallback fonts instead of the system's.
    pub fn with_fallback(weights: &[Weight], fallback: &'static SystemFonts) -> Fonts {
        Fonts::with_chain(
            weights,
            Chain {
                system: Some(fallback),
                unifont: None,
            },
        )
    }

    /// Like [`Fonts::new`], with these fallback fonts instead of the process's.
    pub fn with_chain(weights: &[Weight], chain: Chain) -> Fonts {
        Fonts {
            fallback: Fallback::Given(chain),
            ..Fonts::nunito(weights)
        }
    }

    /// Nunito alone.
    pub fn nunito(weights: &[Weight]) -> Fonts {
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
        Fonts {
            faces,
            fallback: Fallback::None,
        }
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

    /// The fallback fonts, if there are any.
    fn fallback(&self) -> Option<Chain> {
        match self.fallback {
            Fallback::None => None,
            Fallback::System => Chain::current(),
            Fallback::Given(c) => Some(c),
        }
    }

    /// The face a glyph of `id` comes from (none for Unifont's, which have no face).
    fn face_of(&self, id: FaceId) -> Option<&Face<'static>> {
        match id {
            FaceId::Nunito(w) => Some(self.face(w)),
            FaceId::System(file) => self.fallback()?.system?.face(file),
            FaceId::Unifont(_) => None,
        }
    }

    /// The outline of a glyph, in its font's units, its box (`x_min, y_min, x_max, y_max`)
    /// and the units to the em.
    fn outline(
        &self,
        (id, glyph): (FaceId, u16),
    ) -> Option<(fdsm::shape::Shape<Contour>, [f64; 4], f64)> {
        if let FaceId::Unifont(plane) = id {
            let c = unifont::char_of(plane, glyph)?;
            let (shape, bbox) = unifont::outline(self.fallback()?.unifont?, c)?;
            return Some((shape, bbox, unifont::ROWS_PER_EM));
        }
        let face = self.face_of(id)?;
        let gid = GlyphId(glyph);
        // A variable font's set weight moves the outline from its stored box: measure it.
        let bbox = if face.is_variable() {
            face.outline_glyph(gid, &mut NoOutline)?
        } else {
            face.glyph_bounding_box(gid)?
        };
        let shape = fdsm_ttf_parser::load_shape_from_face(face, gid)?;
        let bbox = [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max].map(f64::from);
        Some((shape, bbox, f64::from(face.units_per_em())))
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

    /// Shapes one line of text: in runs of one font each, set end to end.
    pub fn shape(&self, text: &str, style: TextStyle) -> Line {
        let track = style.tracking * style.size;
        let mut glyphs = Vec::with_capacity(text.len());
        let mut x = 0.0f32;
        for (range, id) in self.runs(text, style.weight) {
            if let FaceId::Unifont(_) = id {
                let Some(font) = self.fallback().and_then(|c| c.unifont) else {
                    continue;
                };
                // Pictures, not outlines to shape: each at vanilla's advance.
                for c in text[range].chars() {
                    if unifont::invisible(c) || !font.has(c) {
                        continue;
                    }
                    if !glyphs.is_empty() {
                        x += track;
                    }
                    let (plane, id) = unifont::key(c);
                    glyphs.push(Glyph {
                        face: FaceId::Unifont(plane),
                        id,
                        x,
                        y: 0.0,
                    });
                    x += unifont::advance(font, c) * style.size;
                }
                continue;
            }
            let Some(face) = self.face_of(id) else {
                continue;
            };
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str(&text[range]);
            // Nunito has no right-to-left script: its runs read left to right, whatever
            // stray character comes first. Other runs are guessed (a Hebrew word reads
            // right to left).
            if matches!(id, FaceId::Nunito(_)) {
                buffer.set_direction(rustybuzz::Direction::LeftToRight);
            }
            let out = rustybuzz::shape(face, &[], buffer);
            let scale = style.size / face.units_per_em() as f32;
            for (info, pos) in out.glyph_infos().iter().zip(out.glyph_positions()) {
                if !glyphs.is_empty() {
                    x += track;
                }
                glyphs.push(Glyph {
                    face: id,
                    id: info.glyph_id as u16,
                    x: x + pos.x_offset as f32 * scale,
                    y: -(pos.y_offset as f32) * scale,
                });
                x += pos.x_advance as f32 * scale;
            }
        }
        Line {
            size: style.size,
            glyphs,
            width: x,
        }
    }

    /// `text` in runs of one font each: Nunito wherever it has the characters; a word it
    /// can't set in the first fallback font that has all of the word, else character by
    /// character in the first that has each (Nunito's missing glyph when none has it).
    /// Unifont's runs are `FaceId::Unifont(0)`; [`Fonts::shape`] gives each glyph its plane.
    fn runs(&self, text: &str, weight: Weight) -> Vec<(std::ops::Range<usize>, FaceId)> {
        let nunito = FaceId::Nunito(weight);
        let primary = self.face(weight);
        let all_nunito = || {
            text.chars()
                .all(|c| ignorable(c) || c.is_control() || has_outline(primary, c))
        };
        let fallback = match self.fallback {
            Fallback::None => None,
            // The system's fonts are only looked for once some text needs them.
            Fallback::System if all_nunito() => None,
            _ => self.fallback(),
        };
        let Some(fallback) = fallback else {
            return vec![(0..text.len(), nunito)];
        };
        let bold = weight.0 >= 600;
        let coverage = |c: char| has_outline(primary, c);
        itemize(text, coverage, &fallback)
            .into_iter()
            .map(|(range, font)| {
                let id = match (font, fallback.system) {
                    (Some(f), _) if fallback.is_unifont(f) => FaceId::Unifont(0),
                    (Some(f), Some(system)) => FaceId::System(system.file(f, bold)),
                    _ => nunito,
                };
                (range, id)
            })
            .collect()
    }
}

/// Which fonts have which characters (the fallback fonts, or a stand-in in tests).
pub(crate) trait Coverage {
    /// How many families there are.
    fn families(&self) -> u16;
    /// The first family with `c`.
    fn covering(&self, c: char) -> Option<u16>;
    /// Whether `family` has `c`.
    fn family_has(&self, family: u16, c: char) -> bool;
}

/// `text` in runs of one font each (`None`: Nunito, which has `primary(c)`): Nunito for a
/// word it has every character of; else the first fallback that has the whole word
/// (looking first at the one with the first character Nunito lacks); else character by
/// character, each in Nunito if it has it, or the first fallback that does (Nunito's
/// missing glyph when none has it). Joiners and marks go with what they follow.
fn itemize(
    text: &str,
    primary: impl Fn(char) -> bool,
    fallback: &impl Coverage,
) -> Vec<(std::ops::Range<usize>, Option<u16>)> {
    let mut runs: Vec<(std::ops::Range<usize>, Option<u16>)> = Vec::new();
    let mut push = |range: std::ops::Range<usize>, id: Option<u16>| match runs.last_mut() {
        Some((r, last)) if *last == id && r.end == range.start => r.end = range.end,
        _ => runs.push((range, id)),
    };
    for word in words(text) {
        let chars = || text[word.clone()].chars().filter(|&c| !ignorable(c));
        if chars().all(&primary) {
            push(word, None);
            continue;
        }
        let has_all = |f: u16| chars().all(|c| fallback.family_has(f, c));
        let first = chars()
            .find(|&c| !primary(c))
            .and_then(|c| fallback.covering(c));
        let whole = match first {
            Some(f) if has_all(f) => Some(f),
            Some(_) => (0..fallback.families()).find(|&f| has_all(f)),
            None => None,
        };
        if let Some(f) = whole {
            push(word, Some(f));
            continue;
        }
        let mut current = None;
        for (i, c) in text[word.clone()].char_indices() {
            let at = word.start + i;
            let id = if ignorable(c) {
                current
            } else if primary(c) {
                None
            } else {
                fallback.covering(c)
            };
            current = id;
            push(at..at + c.len_utf8(), id);
        }
    }
    runs
}

/// `text` cut into words and the spaces between them (byte ranges).
fn words(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut space = None;
    for (i, c) in text.char_indices() {
        let s = c.is_whitespace();
        if space.is_some_and(|was| was != s) {
            out.push(start..i);
            start = i;
        }
        space = Some(s);
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

/// Characters that only join or modify their neighbours (joiners, variation selectors,
/// tags, combining marks): they go with the font of what they follow.
fn ignorable(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{E0000}'..='\u{E007F}' | '\u{E0100}'..='\u{E01EF}'
        | '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}'
        | '\u{1F3FB}'..='\u{1F3FF}')
}

/// Where a glyph is in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    /// `u0, v0, u1, v1`.
    pub uv: [f32; 4],
    /// The field's quad relative to the pen, in ems, y down: `x0, y0, x1, y1`.
    pub plane: [f32; 4],
}

/// A packed glyph: its block in texels and its quad.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    at: [u32; 2],
    size: [u32; 2],
    plane: [f32; 4],
}

/// What the atlas knows of a glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Known {
    /// In the atlas.
    Slot(GlyphSlot),
    /// Nothing to draw: no outline (a space), or no room for it.
    Blank,
    /// Not rendered yet.
    Unknown,
}

/// The distance-field atlas: RGBA8 texels (MSDF in RGB), packed in shelves. It grows (up to
/// [`MAX_ATLAS_SIZE`]) when a glyph no longer fits.
#[derive(Clone)]
pub struct GlyphAtlas {
    pub size: u32,
    pub pixels: Vec<u8>,
    /// `None` for glyphs with nothing to draw.
    glyphs: HashMap<(FaceId, u16), Option<Placed>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// The largest it may grow to.
    max: u32,
    /// Some glyph found no room (said once).
    full: bool,
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("size", &self.size)
            .field("glyphs", &self.glyphs.len())
            .finish()
    }
}

/// Glyphs' fields, rendered (on any thread) to be put in an atlas.
pub struct Rendered(Vec<((FaceId, u16), Option<Field>)>);

impl std::fmt::Debug for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Rendered").field(&self.0.len()).finish()
    }
}

impl Rendered {
    /// Renders the fields of `glyphs` in parallel.
    pub fn new(fonts: &Fonts, glyphs: &[(FaceId, u16)]) -> Rendered {
        let mut todo = glyphs.to_vec();
        todo.sort_unstable();
        todo.dedup();
        Rendered(
            todo.into_par_iter()
                .map(|k| (k, render_glyph(fonts, k)))
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl GlyphAtlas {
    pub fn new(size: u32) -> GlyphAtlas {
        GlyphAtlas::growing(size, size)
    }

    /// An atlas `size` texels wide that may grow to `max`.
    pub fn growing(size: u32, max: u32) -> GlyphAtlas {
        GlyphAtlas {
            size,
            pixels: vec![0; (size * size * 4) as usize],
            glyphs: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            max: max.max(size),
            full: false,
        }
    }

    /// What the atlas knows of a glyph.
    pub fn known(&self, key: (FaceId, u16)) -> Known {
        match self.glyphs.get(&key) {
            Some(Some(p)) => Known::Slot(self.slot(p)),
            Some(None) => Known::Blank,
            None => Known::Unknown,
        }
    }

    pub fn get(&self, key: (FaceId, u16)) -> Option<GlyphSlot> {
        match self.known(key) {
            Known::Slot(s) => Some(s),
            _ => None,
        }
    }

    fn slot(&self, p: &Placed) -> GlyphSlot {
        let size = self.size as f32;
        GlyphSlot {
            uv: [
                p.at[0] as f32 / size,
                p.at[1] as f32 / size,
                (p.at[0] + p.size[0]) as f32 / size,
                (p.at[1] + p.size[1]) as f32 / size,
            ],
            plane: p.plane,
        }
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
        let glyphs: Vec<(FaceId, u16)> = line.glyphs.iter().map(Glyph::key).collect();
        self.add_many(fonts, &glyphs);
    }

    /// Adds glyphs, generating their fields in parallel and packing them tallest first.
    /// Returns false if some did not fit.
    pub fn add_many(&mut self, fonts: &Fonts, glyphs: &[(FaceId, u16)]) -> bool {
        let todo: Vec<(FaceId, u16)> = glyphs
            .iter()
            .copied()
            .filter(|k| !self.glyphs.contains_key(k))
            .collect();
        self.insert(Rendered::new(fonts, &todo))
    }

    /// Puts rendered glyphs in, tallest first, growing the atlas when they don't fit.
    /// Returns false if some found no room even so (they are left blank).
    pub fn insert(&mut self, rendered: Rendered) -> bool {
        let mut fields: Vec<((FaceId, u16), Option<Field>)> = rendered
            .0
            .into_iter()
            .filter(|(k, _)| !self.glyphs.contains_key(k))
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
        loop {
            let (mut x, mut y, mut shelf) = (self.shelf_x, self.shelf_y, self.shelf_h);
            if x + w > self.size {
                y += shelf + 1;
                x = 0;
                shelf = 0;
            }
            if y + h <= self.size && w <= self.size {
                self.shelf_x = x + w + 1;
                self.shelf_y = y;
                self.shelf_h = shelf.max(h);
                return Some((x, y));
            }
            if !self.grow() {
                return None;
            }
        }
    }

    /// Doubles the atlas's side, keeping what is in it where it is. False at the most.
    fn grow(&mut self) -> bool {
        if self.size >= self.max {
            return false;
        }
        let (old, new) = (self.size as usize, (self.size * 2).min(self.max) as usize);
        let mut pixels = vec![0; new * new * 4];
        for y in 0..old {
            pixels[y * new * 4..(y * new + old) * 4]
                .copy_from_slice(&self.pixels[y * old * 4..(y + 1) * old * 4]);
        }
        self.pixels = pixels;
        self.size = new as u32;
        true
    }

    /// Generates one glyph's field, if it isn't there yet. Returns false if it did not fit.
    pub fn add(&mut self, fonts: &Fonts, key: (FaceId, u16)) -> bool {
        self.add_many(fonts, &[key])
    }

    fn place(&mut self, key: (FaceId, u16), field: Option<Field>) -> bool {
        let Some(f) = field else {
            self.glyphs.insert(key, None);
            return true;
        };
        let Some((ax, ay)) = self.pack(f.w, f.h) else {
            if !self.full {
                eprintln!("[ui] the glyph atlas is full; some text is left out");
                self.full = true;
            }
            self.glyphs.insert(key, None);
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
        self.glyphs.insert(
            key,
            Some(Placed {
                at: [ax, ay],
                size: [f.w, f.h],
                plane: f.plane,
            }),
        );
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

/// Takes an outline in and keeps none of it (for its bounding box).
struct NoOutline;

impl rustybuzz::ttf_parser::OutlineBuilder for NoOutline {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// The MSDF of one glyph, or `None` if it has no outline.
fn render_glyph(fonts: &Fonts, key: (FaceId, u16)) -> Option<Field> {
    let (mut shape, [x_min, y_min, x_max, y_max], upem) = fonts.outline(key)?;
    let s = EM_TEXELS / upem;
    let w = ((x_max - x_min) * s + 2.0 * PAD).ceil() as u32;
    let h = ((y_max - y_min) * s + 2.0 * PAD).ceil() as u32;
    // Font units (y up) to field texels (y down), with the padding.
    let m = nalgebra::Matrix3::new(
        s,
        0.0,
        PAD - x_min * s,
        0.0,
        -s,
        PAD + y_max * s,
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
    let x0 = (x_min / upem - PAD / EM_TEXELS) as f32;
    let y0 = (-y_max / upem - PAD / EM_TEXELS) as f32;
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
    pub clip: crate::Clip,
}

/// Glyph quads for one frame.
#[derive(Debug, Default)]
pub struct TextList {
    quads: Vec<GlyphQuad>,
    clip: crate::Clip,
    /// Glyphs left out because the atlas doesn't have them yet.
    missing: Vec<(FaceId, u16)>,
}

impl TextList {
    pub fn new() -> TextList {
        TextList {
            quads: Vec::with_capacity(256),
            clip: crate::Clip::NONE,
            missing: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.quads.clear();
        self.clip = crate::Clip::NONE;
        self.missing.clear();
    }

    /// The glyphs left out since the last clear because the atlas doesn't have them yet
    /// (each once), for the atlas to take in.
    pub fn take_missing(&mut self) -> Vec<(FaceId, u16)> {
        let mut m = std::mem::take(&mut self.missing);
        m.sort_unstable();
        m.dedup();
        m
    }

    /// Multiplies the opacity of every glyph so far by `k`.
    pub fn fade_all(&mut self, k: f32) {
        for q in &mut self.quads {
            q.clip.opacity *= k;
        }
    }

    /// The clip for glyphs added from now on.
    pub fn set_clip(&mut self, clip: crate::Clip) {
        self.clip = clip;
    }

    /// The clip glyphs are added with.
    pub fn clip(&self) -> crate::Clip {
        self.clip
    }

    pub fn quads(&self) -> &[GlyphQuad] {
        &self.quads
    }

    /// Draws `line` with its baseline's left end at `(x, y)` pixels. Glyphs missing from
    /// the atlas are skipped (and noted, see [`TextList::take_missing`]).
    pub fn line(&mut self, atlas: &GlyphAtlas, line: &Line, x: f32, y: f32, color: [f32; 4]) {
        self.line_scaled(atlas, line, x, y, 1.0, color);
    }

    /// Like [`TextList::line`], `scale` times the size it was shaped at (for small
    /// animated changes; shape it again for a new size).
    pub fn line_scaled(
        &mut self,
        atlas: &GlyphAtlas,
        line: &Line,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
    ) {
        let size = line.size * scale;
        let range = (RANGE / EM_TEXELS) as f32 * size;
        for g in &line.glyphs {
            let slot = match atlas.known(g.key()) {
                Known::Slot(s) => s,
                Known::Blank => continue,
                Known::Unknown => {
                    self.missing.push(g.key());
                    continue;
                }
            };
            let (gx, gy) = (g.x, g.y);
            let [px0, py0, px1, py1] = slot.plane.map(|v| v * size);
            let (ox, oy) = (x + gx * scale, y + gy * scale);
            self.quads.push(GlyphQuad {
                rect: [ox + px0, oy + py0, ox + px1, oy + py1],
                uv: slot.uv,
                color,
                range,
                clip: self.clip,
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
        let o = atlas.get(line.glyphs[0].key()).expect("O has a field");
        assert_eq!(line.glyphs[0].face, FaceId::Nunito(Weight::MEDIUM));
        assert_eq!(
            atlas.known(line.glyphs[1].key()),
            Known::Blank,
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

    /// A stand-in for the system's fonts: family 0 has some small capitals, family 1 has
    /// those, `ɢ` and Cyrillic, family 2 has CJK.
    struct Fake;

    impl Coverage for Fake {
        fn families(&self) -> u16 {
            3
        }

        fn covering(&self, c: char) -> Option<u16> {
            (0..3).find(|&f| self.family_has(f, c))
        }

        fn family_has(&self, family: u16, c: char) -> bool {
            let small = ('\u{1D00}'..='\u{1D2B}').contains(&c) || c == '\u{26A}' || c == '.';
            match family {
                0 => small,
                1 => small || c == '\u{262}' || ('\u{400}'..='\u{4FF}').contains(&c),
                _ => ('\u{4E00}'..='\u{9FFF}').contains(&c),
            }
        }
    }

    fn itemized(text: &str) -> Vec<(&str, Option<u16>)> {
        // Nunito, here, is ASCII and Cyrillic.
        let primary = |c: char| c.is_ascii() || ('\u{400}'..='\u{4FF}').contains(&c);
        itemize(text, primary, &Fake)
            .into_iter()
            .map(|(r, f)| (&text[r], f))
            .collect()
    }

    #[test]
    fn words_nunito_lacks_take_one_fallback_font() {
        assert_eq!(itemized("Hello there"), [("Hello there", None)]);
        // The word in small capitals, its Cyrillic `ѕ` too, from the first font that has
        // all of it (not the first with `ɪ`); the spaces and ASCII words stay Nunito's.
        assert_eq!(
            itemized("#1 ѕɪᴇɢᴇ ᴛᴇꜱᴛ"),
            [
                ("#1 ", None),
                ("ѕɪᴇɢᴇ", Some(1)),
                (" ", None),
                ("ᴛᴇ", Some(0)),
                ("ꜱ", None),
                ("ᴛ", Some(0))
            ]
        );
        // No font has the whole word: character by character, Nunito first.
        assert_eq!(
            itemized("aᴀ日b"),
            [("a", None), ("ᴀ", Some(0)), ("日", Some(2)), ("b", None)]
        );
        // Joiners and marks go with what they follow.
        assert_eq!(
            itemized("ᴀ\u{301}\u{200D}日\u{FE0F}"),
            [("ᴀ\u{301}\u{200D}", Some(0)), ("日\u{FE0F}", Some(2))]
        );
        assert_eq!(itemized(""), []);
        assert_eq!(words(" ab  c "), [0..1, 1..3, 3..5, 5..6, 6..7]);
    }

    #[test]
    fn the_atlas_grows_and_keeps_what_it_had() {
        let fonts = Fonts::nunito(&[Weight::REGULAR]);
        let mut atlas = GlyphAtlas::growing(64, 256);
        // "O" goes second, away from the corner, where a size change shows.
        atlas.add_text(&fonts, Weight::REGULAR, "l");
        atlas.add_text(&fonts, Weight::REGULAR, "O");
        let key = fonts
            .shape(
                "O",
                TextStyle {
                    size: 1.0,
                    weight: Weight::REGULAR,
                    tracking: 0.0,
                },
            )
            .glyphs[0]
            .key();
        let texels = |a: &GlyphAtlas| {
            let s = a.get(key).unwrap();
            let x = (s.uv[0] * a.size as f32).round() as usize;
            let y = (s.uv[1] * a.size as f32).round() as usize;
            let w = ((s.uv[2] - s.uv[0]) * a.size as f32).round() as usize;
            (0..w)
                .map(|i| a.pixels[(y * a.size as usize + x + i) * 4 + 4 * a.size as usize * 10])
                .collect::<Vec<u8>>()
        };
        let at = |a: &GlyphAtlas| a.get(key).unwrap().uv.map(|v| v * a.size as f32);
        let before = (texels(&atlas), atlas.get(key).unwrap().plane, at(&atlas));
        assert!(before.2[0] > 0.0);
        let keys = |s: &str| -> Vec<(FaceId, u16)> {
            fonts
                .shape(
                    s,
                    TextStyle {
                        size: 1.0,
                        weight: Weight::REGULAR,
                        tracking: 0.0,
                    },
                )
                .glyphs
                .iter()
                .map(Glyph::key)
                .collect()
        };
        assert!(atlas.add_many(&fonts, &keys("ABCDEFGHIJKLMNOPQRSTUVWXYZ")));
        assert!(atlas.size > 64 && atlas.size <= 256, "{}", atlas.size);
        assert_eq!(
            (texels(&atlas), atlas.get(key).unwrap().plane, at(&atlas)),
            before
        );
        // At the most, what doesn't fit is left blank, and said to be.
        assert!(!atlas.add_many(&fonts, &keys("abcdefghijklmnopqrstuvwxyz0123456789")));
        assert_eq!(atlas.size, 256);
        let line = fonts.shape(
            "z9",
            TextStyle {
                size: 1.0,
                weight: Weight::REGULAR,
                tracking: 0.0,
            },
        );
        assert!(
            line.glyphs
                .iter()
                .any(|g| atlas.known(g.key()) == Known::Blank)
        );
    }

    #[test]
    fn text_notes_the_glyphs_the_atlas_lacks() {
        let fonts = Fonts::nunito(&[Weight::REGULAR]);
        let style = TextStyle {
            size: 20.0,
            weight: Weight::REGULAR,
            tracking: 0.0,
        };
        let mut atlas = GlyphAtlas::new(256);
        atlas.add_text(&fonts, Weight::REGULAR, "ab");
        let mut list = TextList::new();
        list.line(&atlas, &fonts.shape("abba c", style), 0.0, 0.0, [1.0; 4]);
        assert_eq!(list.quads().len(), 4);
        let missing = list.take_missing();
        // `c` once; the space is not asked for: it is known to have nothing to draw...
        // once it has been rendered.
        assert!(missing.len() <= 2 && !missing.is_empty(), "{missing:?}");
        atlas.insert(Rendered::new(&fonts, &missing));
        list.clear();
        list.line(&atlas, &fonts.shape("abba c", style), 0.0, 0.0, [1.0; 4]);
        assert_eq!(list.quads().len(), 5);
        assert!(list.take_missing().is_empty());
    }

    /// GNU Unifont's lines for a ring 'A' (columns 1 to 6, rows 2 to 11), 'a', a small
    /// capital, the zero-width joiner's box, a block in plane 1 that no other font has and
    /// a private-use character.
    const HEX: &str = "0041:00007E42424242424242427E00000000
0061:00000000000000003C043C44443C0000
1D00:00000000000018242442427E42420000
200D:000000007E7E7E7E7E7E7E7E00000000
01E5D0:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
E000:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
";

    fn unifont() -> &'static ruthenium_assets::Font {
        static FONT: std::sync::OnceLock<ruthenium_assets::Font> = std::sync::OnceLock::new();
        FONT.get_or_init(|| ruthenium_assets::Font::of_unihex(HEX).expect("the test's unihex"))
    }

    fn nunito_as_system() -> &'static SystemFonts {
        static FONTS: std::sync::OnceLock<SystemFonts> = std::sync::OnceLock::new();
        FONTS.get_or_init(|| {
            let path =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts/Nunito-Regular.ttf");
            SystemFonts::from_files(vec![(path, None)])
        })
    }

    /// Unifont is one family more, after the system's, and never has private-use
    /// characters.
    #[test]
    fn unifont_comes_after_the_system_fonts() {
        let both = Chain {
            system: Some(nunito_as_system()),
            unifont: Some(unifont()),
        };
        assert_eq!(both.families(), 2);
        assert_eq!(both.covering('a'), Some(0));
        assert_eq!(both.covering('\u{1E5D0}'), Some(1));
        assert_eq!(both.covering('\u{1D00}'), Some(1));
        assert_eq!(both.covering('\u{E000}'), None);
        assert_eq!(both.covering('\u{1E5D1}'), None);
        assert!(both.family_has(1, 'a') && !both.family_has(0, '\u{1E5D0}'));
        assert!(!both.family_has(1, '\u{E000}'));
        assert!(both.is_unifont(1) && !both.is_unifont(0));
        let alone = Chain {
            system: None,
            unifont: Some(unifont()),
        };
        assert_eq!(alone.families(), 1);
        assert_eq!(alone.covering('\u{1E5D0}'), Some(0));
        assert!(alone.is_unifont(0));
        let system = Chain {
            system: Some(nunito_as_system()),
            unifont: None,
        };
        assert_eq!((system.families(), system.covering('\u{1E5D0}')), (1, None));
        assert!(!system.is_unifont(1) && !system.is_unifont(0));
    }

    #[test]
    fn unifont_sets_words_at_vanillas_advances() {
        let fonts = Fonts::with_chain(
            &[Weight::REGULAR],
            Chain {
                system: Some(nunito_as_system()),
                unifont: Some(unifont()),
            },
        );
        let style = TextStyle {
            size: 16.0,
            weight: Weight::REGULAR,
            tracking: 0.0,
        };
        let faces = |s: &str| -> Vec<(FaceId, u16)> {
            fonts
                .shape(s, style)
                .glyphs
                .iter()
                .map(Glyph::key)
                .collect()
        };
        let line = fonts.shape("x \u{1E5D0}\u{1E5D0}", style);
        let unifont_glyph = (FaceId::Unifont(1), 0xE5D0);
        assert_eq!(line.glyphs.len(), 4);
        assert_eq!(line.glyphs[0].face, FaceId::Nunito(Weight::REGULAR));
        assert_eq!(line.glyphs[2].key(), unifont_glyph);
        // 16 columns: vanilla's 9 units, 18 rows; 16 rows to the em, here 16 pixels.
        let start = line.glyphs[2].x;
        assert_eq!(line.glyphs[3].x - start, 18.0);
        assert_eq!(line.width - start, 36.0);
        assert_eq!(line.glyphs[2].y, 0.0);
        // The whole word in Unifont, which has its 'a' too; joiners are left out.
        assert_eq!(
            faces("a\u{1E5D0}\u{200D}\u{FE0F}"),
            [(FaceId::Unifont(0), 0x61), unifont_glyph]
        );
        // A word Unifont can't set whole: each character where it is.
        assert_eq!(
            faces("b\u{1E5D0}"),
            [
                (FaceId::Nunito(Weight::REGULAR), faces("b")[0].1),
                unifont_glyph
            ]
        );
        // Without it, Nunito's missing glyph; tracking goes between Unifont's glyphs too.
        let none = Fonts::with_fallback(&[Weight::REGULAR], nunito_as_system());
        assert_eq!(
            none.shape("\u{1E5D0}", style).glyphs[0].key(),
            (FaceId::Nunito(Weight::REGULAR), 0)
        );
        let tracked = fonts.shape(
            "\u{1E5D0}\u{1E5D0}",
            TextStyle {
                tracking: 0.5,
                ..style
            },
        );
        assert_eq!(tracked.width, 36.0 + 8.0);
    }

    #[test]
    fn unifont_glyphs_are_drawn_from_their_pixels() {
        let fonts = Fonts::with_chain(
            &[Weight::REGULAR],
            Chain {
                system: None,
                unifont: Some(unifont()),
            },
        );
        let mut atlas = GlyphAtlas::new(256);
        let ring = (FaceId::Unifont(0), u16::from(b'A'));
        assert!(atlas.add_many(&fonts, &[ring, (FaceId::Unifont(0), 0x20)]));
        let slot = atlas.get(ring).expect("the ring has a field");
        // Columns 1 to 6 from the pen (the picture starts at its ink), rows 2 to 11 of
        // 16 with 14 above the baseline: 3 texels a row, 4 of padding around.
        let pad = (PAD / EM_TEXELS) as f32;
        let expected = [
            -pad,
            -12.0 / 16.0 - pad,
            6.0 / 16.0 + pad,
            -2.0 / 16.0 + pad,
        ];
        for (a, b) in slot.plane.iter().zip(expected) {
            assert!((a - b).abs() < 1e-3, "{:?} vs {expected:?}", slot.plane);
        }
        let [u0, v0, u1, v1] = slot.uv.map(|v| v * 256.0);
        assert_eq!((u1 - u0, v1 - v0), (26.0, 38.0));
        let texel = |x: f32, y: f32| {
            let i = (((v0 + y) as usize) * 256 + (u0 + x) as usize) * 4;
            let mut c = [atlas.pixels[i], atlas.pixels[i + 1], atlas.pixels[i + 2]];
            c.sort();
            c[1]
        };
        // In the ring's left column, its top row; the hole; the padding.
        assert!(texel(5.5, 19.0) > 160, "{}", texel(5.5, 19.0));
        assert!(texel(13.0, 5.5) > 160, "{}", texel(13.0, 5.5));
        assert!(texel(13.0, 19.0) < 100, "{}", texel(13.0, 19.0));
        assert!(texel(1.0, 1.0) < 100 && texel(24.0, 36.0) < 100);
        // Its corners are sharp: just inside one is inside.
        assert!(texel(4.5, 4.5) > 128, "{}", texel(4.5, 4.5));
        assert!(texel(3.5, 3.5) < 128, "{}", texel(3.5, 3.5));
        // A character without ink (not in this font) has nothing to draw.
        assert_eq!(atlas.known((FaceId::Unifont(0), 0x20)), Known::Blank);
    }

    /// With the system's fonts (where there are any), small capitals are drawn from one.
    #[test]
    fn system_fonts_draw_what_nunito_lacks() {
        let fonts = Fonts::new(&[Weight::REGULAR]);
        let Some(system) = fallback::system().filter(|s| s.covering('\u{1D07}').is_some()) else {
            eprintln!("no system font with small capitals; skipped");
            return;
        };
        let style = TextStyle {
            size: 20.0,
            weight: Weight::REGULAR,
            tracking: 0.0,
        };
        let line = fonts.shape("ѕɪᴇɢᴇ x", style);
        assert!(
            matches!(line.glyphs[0].face, FaceId::System(_)),
            "{:?}",
            line.glyphs
        );
        assert_eq!(
            line.glyphs.last().unwrap().face,
            FaceId::Nunito(Weight::REGULAR)
        );
        let mut atlas = GlyphAtlas::new(512);
        atlas.add_many(
            &fonts,
            &line.glyphs.iter().map(Glyph::key).collect::<Vec<_>>(),
        );
        assert!(atlas.get(line.glyphs[1].key()).is_some());
        assert!(!system.is_empty());
        // Heavy text takes the family's bold file, where it has one.
        let heavy = Fonts::new(&[Weight::REGULAR, Weight::EXTRABOLD]);
        let bold = heavy.shape(
            "ᴇ",
            TextStyle {
                weight: Weight::EXTRABOLD,
                ..style
            },
        );
        let regular = heavy.shape("ᴇ", style);
        let family = system.covering('\u{1D07}').unwrap();
        if system.file(family, true) != system.file(family, false) {
            assert_ne!(bold.glyphs[0].face, regular.glyphs[0].face);
        }
        // Off, they are Nunito's missing glyph.
        let plain = Fonts::nunito(&[Weight::REGULAR]).shape("ᴇ", style);
        assert_eq!(plain.glyphs[0].key(), (FaceId::Nunito(Weight::REGULAR), 0));
    }
}
