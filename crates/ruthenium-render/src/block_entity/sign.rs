//! The text on signs: what a sign's data says, where each of its glyphs lies, and the
//! pictures the glyphs are drawn from.

use std::collections::{HashMap, VecDeque};

use ruthenium_assets::Font;
use ruthenium_assets::font::Glyph;
use ruthenium_gpu::{Arrival, ImageRegion, TextureDesc, TextureId, TextureTable, UploadRing};
use ruthenium_protocol::nbt;
use ruthenium_protocol::text::{Lang, Style, StyledText};
use ruthenium_shaders::interface::GLYPH;

use super::place;

/// How many characters of a width obfuscated text is drawn with: evenly from all of them.
const OBFUSCATED: usize = 512;
/// Lines on one side of a sign.
const LINES: usize = 4;
/// How far bold glyphs reach over their box on every side.
const BOLD_EDGE: f32 = 0.1;
/// How much nearer the second copy of a bold glyph lies, and how much nearer a bar.
const BOLD_NEARER: f32 = 0.001;
const BAR_NEARER: f32 = 0.01;

/// The colour each dye gives a sign's text, as 0xRRGGBB, in the order of their ids.
const TEXT_COLOURS: [u32; 16] = [
    0xFFFFFF, 0xFF681F, 0xFF00FF, 0x9AC0CD, 0xFFFF00, 0xBFFF00, 0xFF69B4, 0x808080, 0xD3D3D3,
    0x00FFFF, 0xA020F0, 0x0000FF, 0x8B4513, 0x00FF00, 0xFF0000, 0x000000,
];
const BLACK: u8 = 15;
/// What surrounds black text that glows.
const BLACK_OUTLINE: u32 = 0xF0EBCC;

/// One side of a sign as its data says it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SignText {
    lines: [Vec<(char, Style)>; LINES],
    dye: u8,
    pub glowing: bool,
}

impl SignText {
    /// The text under `key` of a sign's data. Nothing if it is not there, is not four lines
    /// or has nothing to draw.
    pub fn read(tag: &nbt::Tag, key: &str, lang: &dyn Lang) -> Option<SignText> {
        let side = tag.as_compound()?.get(key)?.as_compound()?;
        let messages: &[nbt::Tag; LINES] = side.list("messages").try_into().ok()?;
        let dye = match side.get("color") {
            None => BLACK,
            Some(_) => place::dye(side.string("color")?)?,
        };
        let lines = messages.each_ref().map(|message| {
            StyledText::from_nbt_in(message, lang)
                .spans
                .iter()
                .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
                .collect::<Vec<_>>()
        });
        (!lines.iter().all(Vec::is_empty)).then(|| SignText {
            lines,
            dye,
            glowing: side.boolean("has_glowing_text").unwrap_or(false),
        })
    }

    /// The colour its text is in where a component has none of its own, and the colour of
    /// what surrounds it.
    fn colours(&self) -> (u32, u32) {
        let own = TEXT_COLOURS[usize::from(self.dye)];
        let dark = if self.glowing && self.dye == BLACK {
            BLACK_OUTLINE
        } else {
            [16, 8, 0].iter().fold(0, |rgb, shift| {
                rgb << 8 | ((own >> shift & 0xFF) as f32 * 0.4) as u32
            })
        };
        (if self.glowing { own } else { dark }, dark)
    }

    /// The colour of what surrounds it where it glows, as 0xRRGGBB.
    pub fn outline(&self) -> u32 {
        self.colours().1
    }

    /// Whether its glow has an outline, for a player `near` it (less than 16 blocks).
    pub fn outlined(&self, near: bool) -> bool {
        self.glowing && (near || self.dye == BLACK)
    }
}

/// What a glyph is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ink {
    Char(char),
    /// Filled: underlines and strikes.
    Bar,
}

/// One quad of a side's text, in the text's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextQuad {
    /// Left, top, right, bottom.
    pub place: [f32; 4],
    /// How far the top and the bottom are moved to the right.
    pub slant: [f32; 2],
    pub nearer: f32,
    /// 0xRRGGBB, and how much of what is behind it it covers, of 255.
    pub colour: u32,
    pub alpha: u8,
    pub ink: Ink,
    /// How far from it its outline lies; nothing for bars, which have none.
    pub around: f32,
}

/// One side's quads.
#[derive(Debug, Default)]
pub(crate) struct TextLayout {
    pub text: Vec<TextQuad>,
    /// Some of its characters change every frame.
    pub obfuscated: bool,
}

/// The part of `line` that fits in `width`, which is all of it that is drawn.
fn first_piece<'a>(line: &'a [(char, Style)], font: &Font, width: f32) -> &'a [(char, Style)] {
    let (mut taken, mut wide) = (0.0f32, false);
    let mut space = None;
    for (i, (c, style)) in line.iter().enumerate() {
        if *c == '\n' {
            return &line[..i];
        }
        if *c == ' ' {
            space = Some(i);
        }
        let advance = font.glyph(*c).advance(style.bold);
        taken += advance;
        // What has no width never begins an overflow, and one character always fits.
        if wide && taken > width {
            return &line[..space.unwrap_or(i)];
        }
        wide |= advance != 0.0;
    }
    line
}

/// Which character is drawn: for obfuscated ones another of the same width, by `pick`
/// (a number below the given one).
fn drawn(c: char, style: Style, font: &Font, pick: &mut dyn FnMut(usize) -> usize) -> char {
    if !style.obfuscated || c == ' ' {
        return c;
    }
    let same = font.of_advance(font.glyph(c).advance.ceil() as i32);
    if same.is_empty() {
        return char::REPLACEMENT_CHARACTER;
    }
    // Not all of them: their pictures are kept for good.
    let step = same.len().div_ceil(OBFUSCATED);
    let of = same.len().div_ceil(step);
    char::from_u32(same[pick(of) * step]).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// Adds a glyph's quads with the pen at `x`, `y`.
fn put(out: &mut Vec<TextQuad>, ink: char, glyph: &Glyph, style: Style, at: [f32; 2], colour: u32) {
    let Some(p) = glyph.picture else {
        return;
    };
    let [x, y] = at;
    let edge = if style.bold { BOLD_EDGE } else { 0.0 };
    let slant = if style.italic {
        [1.0 - 0.25 * p.top, 1.0 - 0.25 * p.bottom]
    } else {
        [0.0; 2]
    };
    let mut quad = |right: f32, nearer| {
        let x = x + right;
        out.push(TextQuad {
            place: [
                x - edge,
                y + p.top - edge,
                x + p.right + edge,
                y + p.bottom + edge,
            ],
            slant,
            nearer,
            colour,
            alpha: u8::MAX,
            ink: Ink::Char(ink),
            around: glyph.shadow_offset,
        });
    };
    quad(0.0, 0.0);
    if style.bold {
        quad(glyph.bold_offset, BOLD_NEARER);
    }
}

/// How wide a line is: what its characters move the pen by.
pub(crate) fn width(line: &[(char, Style)], font: &Font) -> f32 {
    line.iter()
        .map(|(c, s)| font.glyph(*c).advance(s.bold))
        .sum()
}

/// Lays out one line with the pen starting at `at`, in the colour `own` where a character
/// has none of its own: its glyphs, then its bars. Returns where the pen ends.
pub(crate) fn line(
    out: &mut TextLayout,
    line: &[(char, Style)],
    font: &Font,
    pick: &mut dyn FnMut(usize) -> usize,
    [left, top]: [f32; 2],
    own: u32,
) -> f32 {
    let mut x = left;
    let mut bars = Vec::new();
    let mut before = None;
    for &(c, style) in line {
        out.obfuscated |= style.obfuscated && c != ' ';
        let c = drawn(c, style, font, pick);
        let glyph = font.glyph(c);
        let advance = glyph.advance(style.bold);
        let colour = style.color.unwrap_or(own);
        put(&mut out.text, c, &glyph, style, [x, top], colour);
        // A run's bars begin a little before it.
        let from = if before == Some(style) { x } else { x - 1.0 };
        let mut bar = |y0: f32, y1: f32| {
            bars.push(TextQuad {
                place: [from, top + y0, x + advance, top + y1],
                slant: [0.0; 2],
                nearer: BAR_NEARER,
                colour,
                alpha: u8::MAX,
                ink: Ink::Bar,
                around: 0.0,
            });
        };
        if style.strikethrough {
            bar(3.5, 4.5);
        }
        if style.underlined {
            bar(8.0, 9.0);
        }
        before = Some(style);
        x += advance;
    }
    out.text.append(&mut bars);
    x
}

/// Lays out one side of a sign: a hanging one or not.
pub(crate) fn layout(
    text: &SignText,
    hanging: bool,
    font: &Font,
    pick: &mut dyn FnMut(usize) -> usize,
) -> TextLayout {
    let (width, height) = if hanging { (60.0, 9) } else { (90.0, 10) };
    let (own, _) = text.colours();
    let mut out = TextLayout::default();
    for (i, line) in text.lines.iter().enumerate() {
        let piece = first_piece(line, font, width);
        let left = -((self::width(piece, font).ceil() as i32) / 2) as f32;
        let top = (i as i32 * height - LINES as i32 * height / 2) as f32;
        self::line(&mut out, piece, font, pick, [left, top], own);
    }
    out
}

/// Words of a glyph's record, without its texture and its sign.
pub(crate) const GLYPH_WORDS: usize = GLYPH.words() as usize;

/// A side's quads as records, and the newest picture they need: their texture word holds
/// their page's number until they are written for a frame. With `arrived_only` the quads
/// whose picture is not there yet are left out.
pub(crate) fn records(
    quads: &[TextQuad],
    pages: &mut GlyphPages,
    font: &Font,
    arrived_only: bool,
) -> (Vec<[u32; GLYPH_WORDS]>, u64) {
    let mut newest = 0;
    let mut out = Vec::with_capacity(quads.len());
    for q in quads {
        let Some(slot) = pages.slot(q.ink, font) else {
            continue;
        };
        if arrived_only && !pages.arrived(slot.serial) {
            continue;
        }
        newest = newest.max(slot.serial);
        let mut r = [0u32; GLYPH_WORDS];
        let mut put = |field: &str, words: &[u32]| {
            let at = GLYPH.offset(field) as usize;
            r[at..at + words.len()].copy_from_slice(words);
        };
        put("box", &q.place.map(f32::to_bits));
        put("slant", &q.slant.map(f32::to_bits));
        put("nearer", &[q.nearer.to_bits()]);
        put("texels", &[slot.x | slot.y << 16]);
        put("size", &[slot.width | slot.height << 16]);
        let colour = super::renderer::record_colour(q.colour) & 0x00FF_FFFF;
        put("colour", &[colour | u32::from(q.alpha) << 24]);
        put("texture", &[slot.page as u32]);
        put("around", &[q.around.to_bits()]);
        out.push(r);
    }
    (out, newest)
}

/// Side of a page of glyphs' pictures, in texels.
const PAGE: u32 = 512;
/// Pages at most; characters beyond them are not drawn.
const MAX_PAGES: usize = 16;
/// Pictures staged at once.
const STAGED_AT_ONCE: usize = 256;

/// Where a glyph's picture is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slot {
    pub page: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Pictures are staged in the order of their serials.
    pub serial: u64,
}

struct Page {
    texture: Option<TextureId>,
    /// The shelf being filled: its top and height, and how much of it is taken.
    top: u32,
    height: u32,
    taken: u32,
}

struct Pending {
    serial: u64,
    page: usize,
    region: ImageRegion,
    texels: Vec<u8>,
}

/// The pictures of the glyphs drawn so far, in pages made as they are needed. None is ever
/// given up.
#[derive(Default)]
pub(crate) struct GlyphPages {
    pages: Vec<Page>,
    slots: HashMap<Ink, Option<Slot>>,
    pending: VecDeque<Pending>,
    serial: u64,
    /// Every picture up to these has been staged, and can be sampled.
    staged: u64,
    ready: u64,
}

impl GlyphPages {
    /// Room for a picture, with a free texel after it.
    fn room(&mut self, width: u32, height: u32) -> Option<(usize, u32, u32)> {
        let (w, h) = (width + 1, height + 1);
        if w > PAGE || h > PAGE {
            return None;
        }
        for (i, p) in self.pages.iter_mut().enumerate() {
            if h > p.height || p.taken + w > PAGE {
                // A new shelf, under the last.
                let top = p.top + p.height;
                if top + h > PAGE {
                    continue;
                }
                (p.top, p.height, p.taken) = (top, h, 0);
            }
            let x = p.taken;
            p.taken += w;
            return Some((i, x, p.top));
        }
        if self.pages.len() == MAX_PAGES {
            return None;
        }
        self.pages.push(Page {
            texture: None,
            top: 0,
            height: h,
            taken: w,
        });
        Some((self.pages.len() - 1, 0, 0))
    }

    fn slot(&mut self, ink: Ink, font: &Font) -> Option<Slot> {
        if let Some(slot) = self.slots.get(&ink) {
            return *slot;
        }
        let (width, height, texels) = match ink {
            Ink::Bar => (5, 8, vec![255; 5 * 8 * 4]),
            Ink::Char(c) => match font.texels(c) {
                Some(i) => (i.width, i.height, i.to_rgba8()),
                None => (0, 0, Vec::new()),
            },
        };
        let slot = (width > 0 && height > 0)
            .then(|| self.room(width, height))
            .flatten()
            .map(|(page, x, y)| {
                self.serial += 1;
                self.pending.push_back(Pending {
                    serial: self.serial,
                    page,
                    region: ImageRegion {
                        mip: 0,
                        x,
                        y,
                        width,
                        height,
                    },
                    texels,
                });
                Slot {
                    page,
                    x,
                    y,
                    width,
                    height,
                    serial: self.serial,
                }
            });
        self.slots.insert(ink, slot);
        slot
    }

    /// Makes the pages and stages the pictures that wait for it: before the frame's
    /// `UploadRing::record`.
    pub fn stage(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) {
        // What was staged a frame ago has arrived.
        self.ready = self.staged;
        for page in &mut self.pages {
            if page.texture.is_none() {
                let empty = vec![0; (PAGE * PAGE * 4) as usize];
                page.texture = textures
                    .create_with(
                        uploads,
                        TextureDesc::rgba8(PAGE, PAGE),
                        &[&empty],
                        Arrival::NextFrame,
                    )
                    .ok();
            }
        }
        while let Some(first) = self.pending.front() {
            let page = first.page;
            let Some(texture) = self.pages[page].texture.filter(|t| textures.is_ready(*t)) else {
                break;
            };
            let run: Vec<(ImageRegion, &[u8])> = self
                .pending
                .iter()
                .take_while(|p| p.page == page)
                .take(STAGED_AT_ONCE)
                .map(|p| (p.region, p.texels.as_slice()))
                .collect();
            let count = run.len();
            if textures.update(uploads, texture, &run).is_err() {
                break;
            }
            self.staged = self.pending[count - 1].serial;
            self.pending.drain(..count);
        }
    }

    /// One page whose texture is not there yet.
    #[cfg(test)]
    pub fn with_a_page() -> GlyphPages {
        GlyphPages {
            pages: vec![Page {
                texture: None,
                top: 0,
                height: 0,
                taken: 0,
            }],
            ..GlyphPages::default()
        }
    }

    /// Whether the pictures up to `serial` can be sampled.
    pub fn arrived(&self, serial: u64) -> bool {
        serial <= self.ready
    }

    /// The pages' textures, as indices into the texture table.
    pub fn textures(&self) -> [u32; MAX_PAGES] {
        std::array::from_fn(|i| {
            self.pages
                .get(i)
                .and_then(|p| p.texture)
                .unwrap_or(TextureId::MISSING)
                .index()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_assets::font::Picture;

    const LINES_ON_A_SIDE: usize = super::LINES;

    fn compound(entries: &[(&str, nbt::Tag)]) -> nbt::Tag {
        let mut c = nbt::Compound::new();
        for (k, v) in entries {
            c.put(k, v.clone());
        }
        nbt::Tag::Compound(c)
    }

    fn text(s: &str) -> nbt::Tag {
        nbt::Tag::String(s.into())
    }

    fn sign(lines: &[&str], extra: &[(&str, nbt::Tag)]) -> nbt::Tag {
        let mut side = vec![(
            "messages",
            nbt::Tag::List(lines.iter().map(|l| text(l)).collect()),
        )];
        side.extend_from_slice(extra);
        compound(&[("front_text", compound(&side))])
    }

    fn read(tag: &nbt::Tag) -> Option<SignText> {
        SignText::read(tag, "front_text", &())
    }

    fn styled(s: &str) -> Vec<(char, Style)> {
        s.chars().map(|c| (c, Style::default())).collect()
    }

    /// Every character of the font six wide and eight high, a space four.
    fn font() -> Font {
        let space = Glyph {
            advance: 4.0,
            picture: None,
            ..Glyph::MISSING
        };
        Font::of_glyphs([(' ', space)])
    }

    /// What the game's own font makes of the same made-up lines: every corner of every
    /// quad and its colour.
    #[test]
    fn the_games_own_font_draws_the_same_quads() {
        use super::super::sign_golden::LINES;
        const ALPHABET: &str = "abcdefghij.";
        let mut glyphs: Vec<(char, Glyph)> = ALPHABET
            .chars()
            .enumerate()
            .map(|(k, c)| {
                let advance = if k == 9 { 0.0 } else { (2 + k % 7) as f32 };
                let other = if k % 4 == 3 { 0.5 } else { 1.0 };
                let top = (k % 3) as f32;
                let picture = Picture {
                    right: (advance - 1.0).max(1.0),
                    top,
                    bottom: top + 8.0,
                    width: 1,
                    height: 8,
                };
                let glyph = Glyph {
                    advance,
                    bold_offset: other,
                    shadow_offset: other,
                    picture: Some(picture),
                };
                (c, glyph)
            })
            .collect();
        let space = Glyph {
            advance: 4.0,
            picture: None,
            ..Glyph::MISSING
        };
        glyphs.push((' ', space));
        let font = Font::of_glyphs(glyphs);
        let mut state = 20_261_125u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % n
        };
        for (batch, want) in LINES.into_iter().enumerate() {
            let mut hash = 0xCBF2_9CE4_8422_2325u64;
            let mut word = |v: i64| {
                for byte in &v.to_le_bytes()[..4] {
                    hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01B3);
                }
            };
            for _ in 0..50 {
                let (hanging, line) = (next(2) == 1, next(4) as usize);
                let (dye, glowing) = (next(16) as u8, next(2) == 1);
                let runs = 1 + next(3);
                let mut parts = Vec::new();
                for _ in 0..runs {
                    let bits = next(16);
                    let colour = (next(3) == 0).then(|| next(16) as usize);
                    let mut s = String::new();
                    for _ in 0..next(12) {
                        match next(15) as usize {
                            c @ 0..11 => s.push_str(&ALPHABET[c..=c]),
                            11 | 12 => s.push(' '),
                            13 if next(4) == 0 => s.push('\n'),
                            13 => s.push('a'),
                            _ => {
                                s.push('§');
                                let code = next(5) as usize;
                                s.push_str(&"clonm"[code..=code]);
                            }
                        }
                    }
                    let mut part = serde_json::json!({
                        "text": s,
                        "bold": bits & 1 != 0,
                        "italic": bits & 2 != 0,
                        "underlined": bits & 4 != 0,
                        "strikethrough": bits & 8 != 0,
                    });
                    if let Some(c) = colour {
                        let name = ruthenium_protocol::text::NAMED_COLORS[c].0;
                        part["color"] = name.into();
                    }
                    parts.push(part);
                }
                let message =
                    StyledText::from_json(&serde_json::json!({"text": "", "extra": parts}));
                let mut lines: [Vec<(char, Style)>; LINES_ON_A_SIDE] = Default::default();
                lines[line] = message
                    .spans
                    .iter()
                    .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
                    .collect();
                let text = SignText {
                    lines,
                    dye,
                    glowing,
                };
                let laid = layout(&text, hanging, &font, &mut |_| 0);
                // What the outline's shader makes of the quads: each eight times.
                let mut quads = Vec::new();
                if glowing {
                    for (dx, dy) in (-1..=1)
                        .flat_map(|dx| (-1..=1).map(move |dy| (dx, dy)))
                        .filter(|d| *d != (0, 0))
                    {
                        for q in laid.text.iter().filter(|q| q.around != 0.0) {
                            let (dx, dy) = (dx as f32 * q.around, dy as f32 * q.around);
                            let [x0, y0, x1, y1] = q.place;
                            quads.push(TextQuad {
                                place: [x0 + dx, y0 + dy, x1 + dx, y1 + dy],
                                colour: text.outline(),
                                ..*q
                            });
                        }
                    }
                }
                quads.extend_from_slice(&laid.text);
                for q in &quads {
                    word(i64::from(q.colour));
                    let [x0, y0, x1, y1] = q.place;
                    let [top, bottom] = q.slant;
                    for corner in [
                        [x0 + top, y0, q.nearer],
                        [x0 + bottom, y1, q.nearer],
                        [x1 + bottom, y1, q.nearer],
                        [x1 + top, y0, q.nearer],
                    ] {
                        for c in corner {
                            word((f64::from(c) * 1000.0 + 0.5).floor() as i64);
                        }
                    }
                }
            }
            assert_eq!(hash, want, "batch {batch}");
        }
    }

    #[test]
    fn texts_are_turned_as_the_games() {
        use super::super::sign_golden::TRANSFORMS;
        use ruthenium_sim::direction::Direction;
        for (hanging, wall, key, back, want) in TRANSFORMS {
            let name = match (hanging, wall) {
                (false, false) => "minecraft:oak_sign",
                (false, true) => "minecraft:oak_wall_sign",
                (true, false) => "minecraft:oak_hanging_sign",
                (true, true) => "minecraft:oak_wall_hanging_sign",
            };
            let state = (0..ruthenium_data::v26_3::blocks::STATE_COUNT as u32)
                .find(|&id| {
                    let s = ruthenium_data::v26_3::BlockState::from_id(id).unwrap();
                    let value = if wall {
                        Direction::from_id(key).unwrap().name().to_owned()
                    } else {
                        key.to_string()
                    };
                    let property = if wall { "facing" } else { "rotation" };
                    s.block().name() == name && s.value(property) == Some(value.as_str())
                })
                .unwrap();
            let got = place::sign_text(state, hanging, wall, !back);
            for (g, w) in got.0.as_flattened().iter().zip(want) {
                assert!((g - w).abs() < 1e-6, "{name} {key} {back}: {got:?}");
            }
        }
    }

    fn at(quads: &[TextQuad]) -> Vec<[f32; 4]> {
        quads.iter().map(|q| q.place).collect()
    }

    fn lay(t: &SignText, hanging: bool) -> TextLayout {
        layout(t, hanging, &font(), &mut |_| 0)
    }

    #[test]
    fn a_side_is_four_lines_or_nothing() {
        let t = read(&sign(&["a", "", "", ""], &[])).unwrap();
        assert_eq!((t.dye, t.glowing), (BLACK, false));
        assert!(read(&sign(&["", "", "", ""], &[])).is_none());
        assert!(read(&sign(&["a", "b", "c"], &[])).is_none());
        assert!(read(&sign(&["a", "", "", ""], &[("color", text("sky"))])).is_none());
        assert!(SignText::read(&sign(&["a", "", "", ""], &[]), "back_text", &()).is_none());
        let lit = sign(
            &["a", "", "", ""],
            &[
                ("color", text("lime")),
                ("has_glowing_text", nbt::Tag::Byte(1)),
            ],
        );
        let t = read(&lit).unwrap();
        assert_eq!((t.dye, t.glowing), (5, true));
    }

    #[test]
    fn text_that_does_not_glow_is_darker() {
        let colours = |dye: &str, glow: i8| {
            read(&sign(
                &["a", "", "", ""],
                &[
                    ("color", text(dye)),
                    ("has_glowing_text", nbt::Tag::Byte(glow)),
                ],
            ))
            .unwrap()
            .colours()
        };
        assert_eq!(colours("white", 0), (0x666666, 0x666666));
        assert_eq!(colours("white", 1), (0xFFFFFF, 0x666666));
        assert_eq!(colours("orange", 0).0, 0x66290C);
        assert_eq!(colours("black", 1), (0x000000, BLACK_OUTLINE));
        assert_eq!(colours("black", 0), (0, 0));
    }

    #[test]
    fn outlines_show_near_by_or_around_black() {
        let t = |dye: &str, glow: i8| {
            read(&sign(
                &["a", "", "", ""],
                &[
                    ("color", text(dye)),
                    ("has_glowing_text", nbt::Tag::Byte(glow)),
                ],
            ))
            .unwrap()
        };
        assert!(t("red", 1).outlined(true) && !t("red", 1).outlined(false));
        assert!(t("black", 1).outlined(false));
        assert!(!t("black", 0).outlined(true));
    }

    #[test]
    fn lines_break_at_spaces_and_draw_their_first_piece() {
        let f = font();
        let piece = |s: &str, width: f32| -> String {
            first_piece(&styled(s), &f, width)
                .iter()
                .map(|(c, _)| *c)
                .collect()
        };
        assert_eq!(piece("abc", 18.0), "abc");
        assert_eq!(piece("abcd", 18.0), "abc");
        assert_eq!(piece("ab cd", 20.0), "ab");
        // The space itself is what does not fit.
        assert_eq!(piece("abc d", 18.0), "abc");
        assert_eq!(piece("a b c", 12.0), "a");
        assert_eq!(piece("ab\ncd", 90.0), "ab");
        // One character always fits.
        assert_eq!(piece("abc", 3.0), "a");
        assert_eq!(piece("", 3.0), "");
        // Characters without width before it do not count as one.
        let nothing = Glyph {
            advance: 0.0,
            ..Glyph::MISSING
        };
        let f = Font::of_glyphs([('z', nothing)]);
        assert_eq!(first_piece(&styled("zab"), &f, 3.0).len(), 2);
    }

    #[test]
    fn lines_are_centred_and_ten_or_nine_apart() {
        let t = read(&sign(&["abc", "a", "", "ab"], &[])).unwrap();
        let l = lay(&t, false);
        // 18, 6 and 12 wide.
        assert_eq!(
            at(&l.text).iter().map(|p| (p[0], p[1])).collect::<Vec<_>>(),
            [
                (-9.0, -20.0),
                (-3.0, -20.0),
                (3.0, -20.0),
                (-3.0, -10.0),
                (-6.0, 10.0),
                (0.0, 10.0)
            ]
        );
        assert_eq!(at(&l.text)[0], [-9.0, -20.0, -4.0, -12.0]);
        assert!(!l.obfuscated);
        let l = lay(&t, true);
        assert_eq!(at(&l.text)[3][1], -9.0);
        assert_eq!(at(&l.text)[0][1], -18.0);
    }

    #[test]
    fn glowing_text_is_in_its_dyes_own_colour() {
        let t = read(&sign(
            &["a", "", "", ""],
            &[
                ("has_glowing_text", nbt::Tag::Byte(1)),
                ("color", text("red")),
            ],
        ))
        .unwrap();
        let l = lay(&t, false);
        assert_eq!(l.text.len(), 1);
        assert_eq!((l.text[0].colour, l.text[0].around), (0xFF0000, 1.0));
        assert_eq!(t.outline(), 0x660000);
    }

    #[test]
    fn obfuscated_characters_are_others_of_their_width() {
        let six = (0x100..0x100 + 1300).filter_map(char::from_u32);
        let wide = Glyph {
            advance: 9.0,
            ..Glyph::MISSING
        };
        let f = Font::of_glyphs(six.map(|c| (c, Glyph::MISSING)).chain([('w', wide)]));
        let style = Style {
            obfuscated: true,
            ..Style::default()
        };
        let mut asked = 0;
        let first = drawn('\u{100}', style, &f, &mut |n| {
            asked = n;
            0
        });
        assert_eq!(first, '\u{100}');
        // Evenly from all of them, and no more of them than are kept.
        assert_eq!(asked, 434);
        let last = drawn('\u{100}', style, &f, &mut |n| n - 1);
        assert_eq!(u32::from(last), 0x100 + 433 * 3);
        assert_eq!(drawn('w', style, &f, &mut |n| n - 1), 'w');
        assert_eq!(drawn(' ', style, &f, &mut |_| unreachable!()), ' ');
        assert_eq!(
            drawn('w', Style::default(), &f, &mut |_| unreachable!()),
            'w'
        );
    }

    #[test]
    fn obfuscated_text_is_drawn_with_what_has_arrived() {
        let f = font();
        let mut pages = GlyphPages::with_a_page();
        let q = TextQuad {
            place: [0.0; 4],
            slant: [0.0; 2],
            nearer: 0.0,
            colour: 0,
            alpha: u8::MAX,
            ink: Ink::Char('a'),
            around: 1.0,
        };
        assert!(records(&[q], &mut pages, &f, true).0.is_empty());
        assert_eq!(records(&[q], &mut pages, &f, false).0.len(), 1);
        // Every picture so far is there.
        pages.ready = pages.serial;
        assert_eq!(records(&[q], &mut pages, &f, true).0.len(), 1);
    }

    #[test]
    fn styles_thicken_slant_and_underline() {
        let s = |json: serde_json::Value| SignText {
            lines: [
                StyledText::from_json(&json)
                    .spans
                    .iter()
                    .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
                    .collect(),
                vec![],
                vec![],
                vec![],
            ],
            dye: BLACK,
            glowing: false,
        };
        let l = lay(
            &s(serde_json::json!({"text":"a","bold":true,"italic":true,"color":"red"})),
            false,
        );
        assert_eq!(l.text.len(), 2);
        // Seven wide: the bold offset is part of the advance.
        assert_eq!(l.text[0].place[0], -3.0 - BOLD_EDGE);
        assert_eq!(l.text[1].place[0] - l.text[0].place[0], 1.0);
        assert_eq!(l.text[1].nearer, BOLD_NEARER);
        assert_eq!(l.text[0].slant, [1.0, -1.0]);
        assert_eq!(l.text[0].colour, 0xFF5555);
        let l = lay(
            &s(serde_json::json!({"text":"ab","underlined":true,"strikethrough":true})),
            false,
        );
        let bars: Vec<[f32; 4]> = l
            .text
            .iter()
            .filter(|q| q.ink == Ink::Bar)
            .map(|q| q.place)
            .collect();
        // The first bar of a run begins one before it, the second does not.
        assert_eq!(
            bars,
            [
                [-7.0, -16.5, 0.0, -15.5],
                [-7.0, -12.0, 0.0, -11.0],
                [0.0, -16.5, 6.0, -15.5],
                [0.0, -12.0, 6.0, -11.0],
            ]
        );
        // After the glyphs of its line.
        assert_eq!(l.text[..2].iter().filter(|q| q.ink == Ink::Bar).count(), 0);
        let l = lay(
            &s(serde_json::json!({"text":"a b","obfuscated":true})),
            false,
        );
        assert!(l.obfuscated);
    }

    #[test]
    fn every_glyph_of_a_page_has_its_own_texels() {
        let mut pages = GlyphPages::default();
        let mut taken: Vec<(usize, u32, u32, u32, u32)> = Vec::new();
        for i in 0..4000u32 {
            let (w, h) = (3 + i % 14, 8 + i % 9);
            let (page, x, y) = pages.room(w, h).unwrap();
            assert!(x + w <= PAGE && y + h <= PAGE);
            for &(p, ox, oy, ow, oh) in taken.iter().filter(|t| t.0 == page) {
                assert!(
                    x > ox + ow || ox > x + w || y >= oy + oh || oy >= y + h,
                    "{p}"
                );
            }
            taken.push((page, x, y, w, h));
        }
        assert!(pages.pages.len() > 1);
        assert!(pages.room(PAGE, 8).is_none());
        assert!((0..100_000).any(|_| pages.room(100, 100).is_none()));
        assert_eq!(pages.pages.len(), MAX_PAGES);
    }

    #[test]
    fn records_hold_the_fields_where_the_shader_reads_them() {
        let f = font();
        let mut pages = GlyphPages::default();
        let q = TextQuad {
            place: [1.0, 2.0, 3.0, 4.0],
            slant: [5.0, 6.0],
            nearer: 7.0,
            colour: 0x123456,
            alpha: u8::MAX,
            ink: Ink::Char('a'),
            around: 8.0,
        };
        let (r, newest) = records(&[q, q], &mut pages, &f, false);
        assert_eq!((r.len(), newest), (2, 1));
        for (i, v) in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]
            .into_iter()
            .enumerate()
        {
            assert_eq!(r[0][i], v.to_bits());
        }
        assert_eq!(
            r[0][7..],
            [0, 5 | 8 << 16, 0xFF56_3412, 0, 0, 8.0f32.to_bits()]
        );
        // The next picture is to the right of the first, then under it.
        let next = |pages: &mut GlyphPages, c| {
            let ink = Ink::Char(c);
            records(&[TextQuad { ink, ..q }], pages, &f, false).0[0][7]
        };
        assert_eq!(next(&mut pages, 'b'), 6);
        for c in (0x100..0x100 + 83).filter_map(char::from_u32) {
            next(&mut pages, c);
        }
        assert_eq!(next(&mut pages, 'c'), 9 << 16);
        assert!(!pages.arrived(newest));
    }
}
