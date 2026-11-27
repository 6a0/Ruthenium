//! The game's fonts: which picture and how much room each character has.
//!
//! A font (`font/<name>.json`) lists providers. Those of a higher pack come first, and
//! within a file the first; the first that has a character gives its glyph. A provider is
//! a bitmap (a PNG cut into cells), spaces (advances without pictures), a unihex file
//! (GNU Unifont's format, zipped) or a reference to another font. Lengths are in the
//! font's units: a line of text is nine high.
//!
//! TrueType providers are left out: no font of the game's own has one, and a pack's is
//! skipped with a warning (no font file of a pack is ever parsed).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, Weak};

use serde_json::Value;

use crate::id::Id;
use crate::image::{Image, alpha, argb, red};
use crate::limits::{Budget, Limits};
use crate::resources::Resources;
use crate::zip::Zip;

/// The baseline's distance from the top of a line.
const BASELINE: f32 = 7.0;
/// Rows of a unihex glyph.
const UNIHEX_ROWS: u32 = 16;
/// Fonts of up to this many providers ask each in turn for a character (the game's have
/// at most six); larger ones (plugin-made packs have thousands) look it up.
const LINEAR_PROVIDERS: usize = 16;
/// The largest advance, height or ascent a provider may give, either way: the text of a
/// line adds advances up, and a pack's `f32::MAX` would make its width infinite. Plugin
/// packs' "negative space" characters go to a few thousand.
const MAX_ADVANCE: f32 = 16384.0;

/// A pack's length clamped to [`MAX_ADVANCE`].
fn clamp_advance(v: f32) -> f32 {
    v.clamp(-MAX_ADVANCE, MAX_ADVANCE)
}
/// Size overrides of a unihex provider: every glyph read is looked up in them, so a pack
/// with a million could make reading Unifont take hours. The game's have 2 and 9.
const MAX_SIZE_OVERRIDES: usize = 1024;

/// What a character takes up and where its picture goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub advance: f32,
    /// How much wider, and how much further right its second copy lies, when bold.
    pub bold_offset: f32,
    /// How far from it its outline lies.
    pub shadow_offset: f32,
    /// The picture's box from the pen's place at the line's top; nothing to draw without
    /// one.
    pub picture: Option<Picture>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Picture {
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    /// Its size in texels.
    pub width: u32,
    pub height: u32,
}

impl Glyph {
    pub fn advance(&self, bold: bool) -> f32 {
        self.advance + if bold { self.bold_offset } else { 0.0 }
    }

    /// The hollow box that stands for a character no provider has.
    pub const MISSING: Glyph = Glyph {
        advance: 6.0,
        bold_offset: 1.0,
        shadow_offset: 1.0,
        picture: Some(Picture {
            right: 5.0,
            top: 0.0,
            bottom: 8.0,
            width: 5,
            height: 8,
        }),
    };
}

struct Bitmap {
    /// Pixels as they are drawn: white times alpha for a grey picture.
    image: Image,
    cell: [u32; 2],
    height: f32,
    ascent: f32,
    /// The cell of each character, and its advance.
    cells: HashMap<u32, ([u32; 2], f32)>,
}

/// A unihex file's glyphs: each character's rows (leftmost column in the highest bit),
/// and the first and last column drawn.
struct Unihex {
    /// Glyphs at most 16 columns wide (every one of the game's), in half the room.
    narrow: HashMap<u32, ([u16; UNIHEX_ROWS as usize], i32, i32)>,
    /// 24 and 32 columns wide.
    wide: HashMap<u32, ([u32; UNIHEX_ROWS as usize], i32, i32)>,
}

impl Unihex {
    /// A character's rows (as wide ones), and its first and last column drawn.
    fn get(&self, c: u32) -> Option<([u32; UNIHEX_ROWS as usize], i32, i32)> {
        if let Some((rows, left, right)) = self.narrow.get(&c) {
            return Some((rows.map(|r| u32::from(r) << 16), *left, *right));
        }
        self.wide.get(&c).copied()
    }

    fn characters(&self) -> impl Iterator<Item = u32> + '_ {
        self.narrow.keys().chain(self.wide.keys()).copied()
    }

    /// Bytes its tables take up, roughly: entries and their control bytes.
    fn heap_bytes(&self) -> usize {
        let table = |capacity: usize, entry: usize| capacity * (entry + 1);
        table(
            self.narrow.capacity(),
            size_of::<(u32, ([u16; UNIHEX_ROWS as usize], i32, i32))>(),
        ) + table(
            self.wide.capacity(),
            size_of::<(u32, ([u32; UNIHEX_ROWS as usize], i32, i32))>(),
        )
    }
}

enum Provider {
    Bitmap(Box<Bitmap>),
    Space(HashMap<u32, f32>),
    /// Shared by every font that reads the same file ([`shared_unihex`]).
    Unihex(Arc<Unihex>),
    Fixed(HashMap<u32, Glyph>),
}

impl Provider {
    fn glyph(&self, c: u32) -> Option<Glyph> {
        let plain = |advance| Glyph {
            advance,
            bold_offset: 1.0,
            shadow_offset: 1.0,
            picture: None,
        };
        match self {
            Provider::Fixed(glyphs) => glyphs.get(&c).copied(),
            Provider::Space(advances) => advances.get(&c).map(|a| plain(*a)),
            // Height 0: drawn 0 by 0 in vanilla, so nothing to draw (see `bitmap`).
            Provider::Bitmap(b) if b.height == 0.0 => {
                b.cells.get(&c).map(|&(_, advance)| plain(advance))
            }
            Provider::Bitmap(b) => b.cells.get(&c).map(|&(_, advance)| {
                let [width, height] = b.cell;
                // Texels per unit.
                let oversample = 1.0 / (b.height / height as f32);
                let top = BASELINE - b.ascent;
                Glyph {
                    picture: Some(Picture {
                        right: width as f32 / oversample,
                        top,
                        bottom: top + height as f32 / oversample,
                        width,
                        height,
                    }),
                    ..plain(advance)
                }
            }),
            Provider::Unihex(u) => u.get(c).map(|(_, left, right)| {
                let width = right - left + 1;
                Glyph {
                    advance: (width / 2 + 1) as f32,
                    bold_offset: 0.5,
                    shadow_offset: 0.5,
                    picture: (width > 0).then_some(Picture {
                        right: width as f32 / 2.0,
                        top: 0.0,
                        bottom: UNIHEX_ROWS as f32 / 2.0,
                        width: width.max(0) as u32,
                        height: UNIHEX_ROWS,
                    }),
                }
            }),
        }
    }

    fn texels(&self, c: u32) -> Option<Image> {
        match self {
            Provider::Space(_) | Provider::Fixed(_) => None,
            Provider::Bitmap(b) => {
                let ([x, y], _) = *b.cells.get(&c)?;
                b.image
                    .sub_image(x * b.cell[0], y * b.cell[1], b.cell[0], b.cell[1])
            }
            Provider::Unihex(u) => {
                let (rows, left, right) = u.get(c)?;
                let width = u32::try_from(right - left + 1).ok().filter(|w| *w > 0)?;
                let mut image = Image::new(width, UNIHEX_ROWS);
                for (y, row) in (0..).zip(rows) {
                    for x in 0..width {
                        let column = left + x as i32;
                        if (0..32).contains(&column) && row >> (31 - column) & 1 != 0 {
                            image.set(x, y, u32::MAX);
                        }
                    }
                }
                Some(image)
            }
        }
    }

    fn characters(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match self {
            Provider::Bitmap(b) => Box::new(b.cells.keys().copied()),
            Provider::Space(s) => Box::new(s.keys().copied()),
            Provider::Fixed(f) => Box::new(f.keys().copied()),
            Provider::Unihex(u) => Box::new(u.characters()),
        }
    }
}

/// A font, its providers in the order they are asked.
pub struct Font {
    /// Shared: a font referred to again gives the same providers, read once.
    providers: Vec<Arc<Provider>>,
    /// What it is held to (the strictest of the packs'), the glyphs its providers
    /// have so far, and decoded bitmap texels charged across this font.
    limits: Limits,
    glyphs: usize,
    bitmap_texels: u64,
    /// The characters by their advance rounded up, for obfuscated text.
    by_advance: HashMap<i32, Vec<u32>>,
    /// For a font of more than [`LINEAR_PROVIDERS`] providers, the provider of each
    /// character (`FontSet`'s glyph cache, made at load): asking thousands of providers
    /// for every character of every frame's text would be the frame's whole time.
    first: Option<HashMap<u32, u32>>,
    /// What could not be read.
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font")
            .field("providers", &self.providers.len())
            .finish_non_exhaustive()
    }
}

impl Font {
    /// The font `id` (`minecraft:default`), as it is with no font option set.
    pub fn load(res: &Resources, id: &Id) -> Font {
        let mut font = Font {
            providers: Vec::new(),
            limits: *res.limits(),
            glyphs: 0,
            bitmap_texels: 0,
            by_advance: HashMap::new(),
            first: None,
            warnings: Vec::new(),
        };
        font.add(res, id, &mut Vec::new(), &mut HashMap::new());
        // Deduplicate shared provider Arcs (preserving first occurrence) so reference DAGs
        // cannot multiply indexing or lookup work.
        let mut seen_ptrs: std::collections::HashSet<*const Provider> =
            std::collections::HashSet::with_capacity(font.providers.len());
        font.providers.retain(|p| seen_ptrs.insert(Arc::as_ptr(p)));
        if !res.cancelled() {
            font.index_checked(&|| res.cancelled());
        }
        font
    }

    /// Lists the characters by their advance.
    /// With more than [`LINEAR_PROVIDERS`] providers, also which provider has each
    /// character.
    fn index(&mut self) {
        self.index_checked(&|| false);
    }

    fn index_checked(&mut self, cancelled: &dyn Fn() -> bool) {
        let mut seen: HashMap<u32, u32> = HashMap::new();
        for (i, p) in self.providers.iter().enumerate() {
            if cancelled() {
                return;
            }
            for (step, c) in p.characters().enumerate() {
                if step & 1023 == 0 && cancelled() {
                    return;
                }
                if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(c)
                    && let Some(g) = p.glyph(c)
                {
                    e.insert(i as u32);
                    let width = g.advance.ceil() as i32;
                    self.by_advance.entry(width).or_default().push(c);
                }
            }
        }
        if self.providers.len() > LINEAR_PROVIDERS {
            seen.shrink_to_fit();
            self.first = Some(seen);
        }
        // In an order that does not change from run to run.
        for list in self.by_advance.values_mut() {
            list.sort_unstable();
        }
    }

    /// A font of made-up glyphs without pictures: for tests of what is laid out.
    #[doc(hidden)]
    pub fn of_glyphs(glyphs: impl IntoIterator<Item = (char, Glyph)>) -> Font {
        let mut font = Font {
            providers: vec![Arc::new(Provider::Fixed(
                glyphs.into_iter().map(|(c, g)| (u32::from(c), g)).collect(),
            ))],
            limits: Limits::trusted(),
            glyphs: 0,
            bitmap_texels: 0,
            by_advance: HashMap::new(),
            first: None,
            warnings: Vec::new(),
        };
        font.index();
        font
    }

    /// Adds the providers of the font `id`; `read` holds those of every font read so
    /// far, so that a font referred to many times is read once.
    fn add(
        &mut self,
        res: &Resources,
        id: &Id,
        within: &mut Vec<Id>,
        read: &mut HashMap<Id, Vec<Arc<Provider>>>,
    ) {
        if res.cancelled() {
            return;
        }
        if within.contains(id) {
            self.warnings.push(format!("font {id} refers to itself"));
            return;
        }
        if let Some(providers) = read.get(id) {
            for p in providers {
                if res.cancelled() {
                    return;
                }
                if self.providers.len() >= self.limits.font_providers {
                    self.warnings.push(format!(
                        "font {id}: more providers than a font may have ({})",
                        self.limits.font_providers
                    ));
                    return;
                }
                self.providers.push(p.clone());
            }
            return;
        }
        let first = self.providers.len();
        if within.len() >= self.limits.font_references {
            self.warnings.push(format!(
                "font {id}: references nest deeper than {}",
                self.limits.font_references
            ));
            return;
        }
        within.push(id.clone());
        // Highest pack first.
        for file in res.stack(&id.wrap("font/", ".json")).iter().rev() {
            if res.cancelled() {
                break;
            }
            match file.json() {
                Ok(json) => {
                    for p in json["providers"].as_array().map_or(&[][..], Vec::as_slice) {
                        if res.cancelled() {
                            break;
                        }
                        if self.providers.len() >= self.limits.font_providers {
                            self.warnings.push(format!(
                                "font {id}: more providers than a font may have ({})",
                                self.limits.font_providers
                            ));
                            break;
                        }
                        if let Err(e) = self.add_provider(res, p, within, read) {
                            self.warnings.push(format!("font {id}: {e}"));
                        }
                    }
                }
                Err(e) => self.warnings.push(format!("font {id}: {e}")),
            }
        }
        within.pop();
        read.insert(id.clone(), self.providers[first..].to_vec());
    }

    fn add_provider(
        &mut self,
        res: &Resources,
        p: &Value,
        within: &mut Vec<Id>,
        read: &mut HashMap<Id, Vec<Arc<Provider>>>,
    ) -> Result<(), String> {
        // Every option is off.
        let wanted = p["filter"]
            .as_object()
            .is_none_or(|f| f.values().all(|v| v.as_bool() == Some(false)));
        if !wanted {
            return Ok(());
        }
        let id = |key: &str| {
            p[key]
                .as_str()
                .and_then(Id::parse)
                .ok_or_else(|| format!("no {key}"))
        };
        match p["type"].as_str() {
            Some("reference") => {
                self.add(res, &id("id")?, within, read);
                return Ok(());
            }
            Some("space") => {
                let advances = p["advances"].as_object().ok_or("no advances")?;
                self.take_glyphs(advances.len())?;
                self.providers.push(Arc::new(Provider::Space(
                    advances
                        .iter()
                        .filter_map(|(c, a)| {
                            let mut chars = c.chars();
                            let (c, None) = (chars.next()?, chars.next()) else {
                                return None;
                            };
                            // A number past `f32::MAX` would be an infinite advance.
                            let a = a.as_f64()? as f32;
                            a.is_finite().then_some((u32::from(c), clamp_advance(a)))
                        })
                        .collect(),
                )));
            }
            Some("bitmap") => {
                let file = id("file")?;
                let texture = file.wrap("textures/", "");
                let picture = res.get(&texture).ok_or_else(|| format!("no {texture}"))?;
                let bytes = picture.read().map_err(|e| format!("{texture}: {e}"))?;
                let limits = picture.limits().min(&self.limits);
                let budget = picture.pack().budget().clone();
                let grid: Vec<Vec<u32>> = p["chars"]
                    .as_array()
                    .ok_or("no chars")?
                    .iter()
                    .map(|row| row.as_str().map(|r| r.chars().map(u32::from).collect()))
                    .collect::<Option<_>>()
                    .ok_or("chars that are not text")?;
                let height = clamp_advance(p["height"].as_i64().unwrap_or(8) as f32);
                let ascent = clamp_advance(p["ascent"].as_i64().ok_or("no ascent")? as f32);
                // Vanilla takes a height of 0 (BitmapProvider.java's `Definition` checks
                // only the ascent): see `bitmap`.
                if height < 0.0 {
                    return Err("height must not be negative".into());
                }
                if ascent > height {
                    return Err(format!("ascent {ascent} is greater than height {height}"));
                }
                self.take_glyphs(grid.iter().map(Vec::len).sum())?;
                let max_font_texels = self.limits.font_bitmap_texels;
                let mut font_texels = self.bitmap_texels;
                let image = Image::decode_png_with(
                    &bytes,
                    limits.image_side,
                    limits.image_texels,
                    &mut |n| {
                        let Some(next) = font_texels.checked_add(n) else {
                            return false;
                        };
                        if next > max_font_texels {
                            return false;
                        }
                        if !budget.take_texels(n, &limits) {
                            return false;
                        }
                        font_texels = next;
                        true
                    },
                )
                .map_err(|e| format!("{file}: {e}"))?;
                self.bitmap_texels = font_texels;
                self.providers.push(Arc::new(Provider::Bitmap(Box::new(
                    bitmap(image, &bytes, &grid, height, ascent)
                        .map_err(|e| format!("{file}: {e}"))?,
                ))));
            }
            Some("unihex") => {
                let file = id("hex_file")?;
                let zip = res.get(&file).ok_or_else(|| format!("no {file}"))?;
                let bytes = zip.read().map_err(|e| format!("{file}: {e}"))?.into_owned();
                // The zip inside the zip is read as its pack is.
                let pack = zip.pack();
                let guard = pack
                    .is_untrusted()
                    .then(|| (*pack.limits(), pack.budget().clone()));
                let max = self.limits.font_glyphs.saturating_sub(self.glyphs);
                let unihex =
                    shared_unihex(bytes, &p["size_overrides"], guard, max, &|| res.cancelled())
                        .map_err(|e| format!("{file}: {e}"))?;
                self.take_glyphs(unihex.narrow.len() + unihex.wide.len())?;
                self.providers.push(Arc::new(Provider::Unihex(unihex)));
            }
            other => return Err(format!("a {} provider", other.unwrap_or("nameless"))),
        }
        Ok(())
    }

    /// Counts a provider's glyphs against [`Limits::font_glyphs`].
    fn take_glyphs(&mut self, n: usize) -> Result<(), String> {
        match self.glyphs.checked_add(n) {
            Some(total) if total <= self.limits.font_glyphs => {
                self.glyphs = total;
                Ok(())
            }
            _ => Err(format!(
                "more glyphs than a font may have ({})",
                self.limits.font_glyphs
            )),
        }
    }

    fn provider(&self, c: u32) -> Option<&Provider> {
        match &self.first {
            Some(first) => first
                .get(&c)
                .and_then(|&i| self.providers.get(i as usize))
                .map(|p| &**p),
            None => self
                .providers
                .iter()
                .find(|p| p.glyph(c).is_some())
                .map(|p| &**p),
        }
    }

    /// Whether some provider has a glyph for `c`.
    pub fn has(&self, c: char) -> bool {
        self.provider(u32::from(c)).is_some()
    }

    /// Bytes its unihex glyphs take up, roughly (each file once, however many fonts
    /// share it).
    pub fn unihex_bytes(&self) -> usize {
        let mut seen = Vec::new();
        let mut bytes = 0;
        for p in &self.providers {
            if let Provider::Unihex(u) = &**p
                && !seen.iter().any(|s| Arc::ptr_eq(s, u))
            {
                bytes += u.heap_bytes();
                seen.push(u.clone());
            }
        }
        bytes
    }

    /// A font of one unihex provider read from `hex` (the lines of a `.hex` file), with no
    /// size overrides: for tests of what draws its glyphs.
    #[doc(hidden)]
    pub fn of_unihex(hex: &str) -> Result<Font, String> {
        let unihex = parse_unihex(
            std::iter::once(Ok(std::borrow::Cow::Borrowed(hex.as_bytes()))),
            &Value::Null,
            usize::MAX,
            &|| false,
        )?;
        let mut font = Font {
            providers: vec![Arc::new(Provider::Unihex(Arc::new(unihex)))],
            limits: Limits::trusted(),
            glyphs: 0,
            bitmap_texels: 0,
            by_advance: HashMap::new(),
            first: None,
            warnings: Vec::new(),
        };
        font.index();
        Ok(font)
    }

    /// The glyph of a character: [`Glyph::MISSING`'s if the font has none.
    pub fn glyph(&self, c: char) -> Glyph {
        self.provider(u32::from(c))
            .and_then(|p| p.glyph(u32::from(c)))
            .unwrap_or(Glyph::MISSING)
    }

    /// The picture of a character's glyph.
    pub fn texels(&self, c: char) -> Option<Image> {
        match self.provider(u32::from(c)) {
            Some(p) => p.texels(u32::from(c)),
            None => Some(missing()),
        }
    }

    /// The characters whose advance, rounded up, is `advance`: what obfuscated text draws
    /// for one that has it.
    pub fn of_advance(&self, advance: i32) -> &[u32] {
        self.by_advance.get(&advance).map_or(&[], Vec::as_slice)
    }
}

/// A hollow box.
fn missing() -> Image {
    let mut image = Image::new(5, 8);
    for y in 0..8 {
        for x in 0..5 {
            if x == 0 || x == 4 || y == 0 || y == 7 {
                image.set(x, y, u32::MAX);
            }
        }
    }
    image
}

fn bitmap(
    mut image: Image,
    png: &[u8],
    grid: &[Vec<u32>],
    height: f32,
    ascent: f32,
) -> Result<Bitmap, String> {
    let columns = grid.first().map_or(0, Vec::len);
    if columns == 0 || grid.iter().any(|r| r.len() != columns) {
        return Err("chars are not a grid".into());
    }
    // A height of 0 is vanilla's too: its glyphs' scale is 0 (BitmapProvider.java:150),
    // so each advances 1 (`(int)(0.5 + width * 0) + 1`) and is drawn 0 wide and 0 high
    // (oversample 1 / 0): nothing shows. Here such a glyph has no picture, and no
    // division by its height is made. A negative height (vanilla draws its glyphs
    // mirrored, with negative advances) is still refused.
    if !height.is_finite() || height < 0.0 {
        return Err("height must not be negative".into());
    }
    if ascent > height {
        return Err("the ascent is above the height".into());
    }
    // A picture without colours is drawn by its brightness alone.
    let grey = png.get(25) == Some(&0);
    if grey {
        for p in &mut image.pixels {
            let v = red(*p);
            *p = argb(v, v, v, v);
        }
    }
    let cell = [
        image.width / columns as u32,
        image.height / grid.len() as u32,
    ];
    if cell.contains(&0) {
        return Err("more characters than pixels".into());
    }
    let scale = height / cell[1] as f32;
    let mut cells = HashMap::new();
    for (y, row) in (0..).zip(grid) {
        for (x, &c) in (0..).zip(row) {
            if c == 0 {
                continue;
            }
            let ink = (0..cell[0]).rev().find(|column| {
                (0..cell[1]).any(|r| alpha(image.get(x * cell[0] + column, y * cell[1] + r)) != 0)
            });
            let width = ink.map_or(0, |column| column + 1);
            let advance = ((0.5 + f64::from(width as f32 * scale)) as i32).saturating_add(1);
            cells.insert(c, ([x, y], clamp_advance(advance as f32)));
        }
    }
    Ok(Bitmap {
        image,
        cell,
        height,
        ascent,
        cells,
    })
}

/// Unihex files read so far, by a hash of their bytes and size overrides. The game's
/// font and the menus' fallback read the same Unifont: the second gets the first's.
static READ: Mutex<Vec<(u64, Weak<Unihex>)>> = Mutex::new(Vec::new());

/// The glyphs of the zipped unihex file `zip`, shared with any font that has read it.
/// `guard`: the limits and budget of an untrusted pack, which the zip is read under;
/// `max_glyphs`: how many glyphs it may have.
fn shared_unihex(
    zip: Vec<u8>,
    overrides: &Value,
    guard: Option<(Limits, Arc<Budget>)>,
    max_glyphs: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<Unihex>, String> {
    let mut hasher = DefaultHasher::new();
    zip.hash(&mut hasher);
    overrides.to_string().hash(&mut hasher);
    let key = hasher.finish();
    // Check cache briefly without holding READ across zip inflation and hex parsing.
    {
        let mut read = READ
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read.retain(|(_, w)| w.strong_count() > 0);
        if let Some(u) = read
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, w)| w.upgrade())
        {
            return Ok(u);
        }
    }
    let zip = match guard {
        Some((limits, budget)) => Zip::from_vec_untrusted(zip, &limits, budget),
        None => Zip::from_vec(zip),
    }
    .map_err(|e| e.to_string())?;
    let files = (0..zip.len())
        .filter(|&i| zip.name(i).ends_with(".hex"))
        .map(|i| zip.read(i).map_err(|e| format!("{}: {e}", zip.name(i))));
    let unihex = Arc::new(parse_unihex(files, overrides, max_glyphs, cancelled)?);
    let mut read = READ
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    read.retain(|(_, w)| w.strong_count() > 0);
    if let Some(existing) = read
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, w)| w.upgrade())
    {
        return Ok(existing);
    }
    read.push((key, Arc::downgrade(&unihex)));
    Ok(unihex)
}

fn parse_unihex<'a>(
    files: impl Iterator<Item = Result<std::borrow::Cow<'a, [u8]>, String>>,
    overrides: &Value,
    max_glyphs: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Unihex, String> {
    // The character ranges whose sides are given: the first that has a character counts.
    let mut sized = Vec::new();
    let overrides = overrides.as_array().map_or(&[][..], Vec::as_slice);
    // Each glyph is looked up in them: the game's own provider has a few dozen.
    if overrides.len() > MAX_SIZE_OVERRIDES {
        return Err(format!("more than {MAX_SIZE_OVERRIDES} size overrides"));
    }
    for o in overrides {
        let character = |key: &str| o[key].as_str()?.chars().next().map(u32::from);
        // A glyph has at most 32 columns: sides past them are cut to just outside.
        let side = |key: &str| o[key].as_i64().map(|v| v.clamp(-1, 32) as i32);
        let (Some(from), Some(to), Some(left), Some(right)) = (
            character("from"),
            character("to"),
            side("left"),
            side("right"),
        ) else {
            return Err("a bad size override".into());
        };
        sized.push((from..=to, (left, right)));
    }
    let mut unihex = Unihex {
        narrow: HashMap::new(),
        wide: HashMap::new(),
    };
    let max_lines = max_glyphs.saturating_mul(2).max(1024);
    let mut seen_lines = 0usize;
    // Straight into the tables, sized beforehand: a table grown by doubling, or one read
    // first and copied, would leave a few MiB more of the heap taken for good (the
    // allocator keeps what a thread freed).
    for bytes in files {
        if cancelled() {
            return Err("the load was cancelled".into());
        }
        let bytes = bytes?;
        let lines = bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty());
        unihex.narrow.reserve(lines.clone().count().min(max_glyphs));
        for line in lines {
            seen_lines += 1;
            if seen_lines > max_lines {
                return Err(format!(
                    "more unihex lines than a font may have ({max_lines})"
                ));
            }
            if seen_lines & 1023 == 0 && cancelled() {
                return Err("the load was cancelled".into());
            }
            let (c, (rows, width)) = hex_line(line).ok_or("not a unihex line")?;
            let (left, right) = sized
                .iter()
                .find(|(range, _)| range.contains(&c))
                .map_or_else(
                    || {
                        let mask = rows.iter().fold(0, |m, r| m | r);
                        if mask == 0 {
                            (0, width as i32)
                        } else {
                            (
                                mask.leading_zeros() as i32,
                                31 - mask.trailing_zeros() as i32,
                            )
                        }
                    },
                    |&(_, sides)| sides,
                );
            if unihex.narrow.len() + unihex.wide.len() >= max_glyphs
                && !unihex.narrow.contains_key(&c)
                && !unihex.wide.contains_key(&c)
            {
                return Err(format!("more glyphs than a font may have ({max_glyphs})"));
            }
            // A later line for a character replaces an earlier one.
            if width <= 16 {
                let narrow = rows.map(|r| (r >> 16) as u16);
                unihex.narrow.insert(c, (narrow, left, right));
                unihex.wide.remove(&c);
            } else {
                unihex.wide.insert(c, (rows, left, right));
                unihex.narrow.remove(&c);
            }
        }
    }
    unihex.narrow.shrink_to_fit();
    unihex.wide.shrink_to_fit();
    Ok(unihex)
}

/// `<code point>:<rows>` in hexadecimal: the character, its rows with the leftmost column in
/// the highest bit, and its width in columns.
fn hex_line(line: &[u8]) -> Option<(u32, ([u32; UNIHEX_ROWS as usize], u32))> {
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let (c, rows) = line.split_once(':')?;
    if !(4..=6).contains(&c.len()) || !rows.is_ascii() {
        return None;
    }
    let digits = match rows.len() {
        32 => 2,
        64 => 4,
        96 => 6,
        128 => 8,
        _ => return None,
    };
    let mut out = [0; UNIHEX_ROWS as usize];
    for (i, row) in out.iter_mut().enumerate() {
        let text = &rows[i * digits..(i + 1) * digits];
        *row = u32::from_str_radix(text, 16).ok()? << (32 - digits * 4);
    }
    Some((u32::from_str_radix(c, 16).ok()?, (out, digits as u32 * 4)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::Pack;

    /// Deleted when dropped.
    struct Folder(std::path::PathBuf);

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A folder pack with these files.
    fn pack(name: &str, files: &[(&str, &[u8])]) -> (Resources, Folder) {
        let dir =
            std::env::temp_dir().join(format!("ruthenium-font-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut all = vec![(
            "pack.mcmeta",
            &br#"{"pack":{"description":"","min_format":97,"max_format":97}}"#[..],
        )];
        all.extend_from_slice(files);
        for (path, bytes) in all {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let res = Resources::new(vec![Pack::open(&dir).unwrap()]);
        (res, Folder(dir))
    }

    fn default() -> Id {
        Id::parse("minecraft:default").unwrap()
    }

    #[test]
    fn a_higher_packs_providers_come_first() {
        let font = |name: &str, advance: &str| {
            let json = format!(
                r#"{{"providers":[{{"type":"space","advances":{{"A":{advance},"{name}":1}}}}]}}"#
            );
            let dir = std::env::temp_dir().join(format!(
                "ruthenium-font-{}-stack-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("assets/minecraft/font")).unwrap();
            std::fs::write(dir.join("assets/minecraft/font/default.json"), json).unwrap();
            std::fs::write(
                dir.join("pack.mcmeta"),
                r#"{"pack":{"description":"","min_format":97,"max_format":97}}"#,
            )
            .unwrap();
            (Pack::open(&dir).unwrap(), Folder(dir))
        };
        let ((low, _l), (high, _h)) = (font("L", "5"), font("H", "6"));
        let res = Resources::new(vec![low, high]);
        let font = Font::load(&res, &default());
        assert_eq!(font.glyph('A').advance, 6.0);
        assert_eq!(
            (font.glyph('L').advance, font.glyph('H').advance),
            (1.0, 1.0)
        );
    }

    /// 'A' is three columns wide in a 8 x 8 cell, 'B' empty.
    fn sheet() -> Vec<u8> {
        let mut image = Image::new(16, 8);
        for y in 1..8 {
            image.set(0, y, u32::MAX);
            // Black is ink; what has no alpha is not, whatever its colour.
            image.set(2, y, 0xFF00_0000);
            image.set(6, y, 0x00FF_FFFF);
        }
        image.encode_png()
    }

    #[test]
    fn bitmap_cells_have_the_room_their_ink_takes() {
        let png = sheet();
        let (res, _dir) = pack(
            "bitmap",
            &[
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/x.png","ascent":7,"chars":["AB"]}]}"#,
                ),
                ("assets/minecraft/textures/font/x.png", &png),
            ],
        );
        let font = Font::load(&res, &default());
        assert!(font.warnings.is_empty(), "{:?}", font.warnings);
        let a = font.glyph('A');
        assert_eq!(a.advance, 4.0);
        assert_eq!(a.advance(true), 5.0);
        let p = a.picture.unwrap();
        assert_eq!((p.right, p.top, p.bottom), (8.0, 0.0, 8.0));
        assert_eq!(font.glyph('B').advance, 1.0);
        assert_eq!(
            (font.glyph(' ').advance, font.glyph(' ').picture),
            (4.0, None)
        );
        assert_eq!(font.glyph('?'), Glyph::MISSING);
        assert_eq!(font.texels('?').unwrap().width, 5);
        let t = font.texels('A').unwrap();
        assert_eq!(
            (t.get(0, 1), t.get(1, 1), t.get(2, 1)),
            (u32::MAX, 0, 0xFF00_0000)
        );
        assert!(font.texels(' ').is_none());
        assert_eq!(font.of_advance(4), [u32::from(' '), u32::from('A')]);
    }

    #[test]
    fn taller_pictures_are_scaled_to_their_height() {
        let png = sheet();
        let (res, _dir) = pack(
            "height",
            &[
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[{"type":"bitmap","file":"minecraft:font/x.png","height":4,"ascent":3,"chars":["AB"]}]}"#,
                ),
                ("assets/minecraft/textures/font/x.png", &png),
            ],
        );
        let g = Font::load(&res, &default()).glyph('A');
        let p = g.picture.unwrap();
        // Three columns at half size are a column and a half, and half of one more.
        assert_eq!(g.advance, 3.0);
        assert_eq!((p.right, p.top, p.bottom), (4.0, 4.0, 8.0));
    }

    /// Vanilla takes a height of 0: each glyph advances 1 and shows nothing
    /// (BitmapProvider.java's scale of 0). A negative height is refused.
    #[test]
    fn a_height_of_0_advances_one_and_draws_nothing() {
        let png = sheet();
        let (res, _dir) = pack(
            "height0",
            &[
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[{"type":"bitmap","file":"minecraft:font/x.png","height":0,"ascent":-3,"chars":["AB"]},
                        {"type":"bitmap","file":"minecraft:font/x.png","height":-1,"ascent":-2,"chars":["CD"]}]}"#,
                ),
                ("assets/minecraft/textures/font/x.png", &png),
            ],
        );
        let font = Font::load(&res, &default());
        for c in ['A', 'B'] {
            let g = font.glyph(c);
            assert_eq!((g.advance, g.picture), (1.0, None), "{c}");
        }
        assert_eq!(font.glyph('C'), Glyph::MISSING);
        assert!(
            font.warnings.iter().any(|w| w.contains("negative")),
            "{:?}",
            font.warnings
        );
    }

    #[test]
    fn options_that_are_on_leave_a_provider_out_and_references_stay_in_order() {
        let png = sheet();
        let (res, _dir) = pack(
            "references",
            &[
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[
                        {"type":"reference","id":"minecraft:other"},
                        {"type":"space","advances":{"A":9,"C":7,"D":1}},
                        {"type":"space","filter":{"uniform":true},"advances":{"D":5,"E":5}},
                        {"type":"space","filter":{"uniform":false,"jp":false},"advances":{"F":2}}]}"#,
                ),
                (
                    "assets/minecraft/font/other.json",
                    br#"{"providers":[{"type":"bitmap","file":"minecraft:font/x.png","ascent":7,"chars":["AB"]},
                        {"type":"reference","id":"minecraft:default"}]}"#,
                ),
                ("assets/minecraft/textures/font/x.png", &png),
            ],
        );
        let font = Font::load(&res, &default());
        assert_eq!(font.glyph('A').advance, 4.0);
        assert_eq!(font.glyph('C').advance, 7.0);
        assert_eq!(font.glyph('D').advance, 1.0);
        assert_eq!(font.glyph('E'), Glyph::MISSING);
        assert_eq!(font.glyph('F').advance, 2.0);
        // The circle is said and cut.
        assert_eq!(font.warnings.len(), 1);
    }

    #[test]
    fn unihex_glyphs_are_as_wide_as_their_ink() {
        // Columns 2 to 5 of an 8-wide glyph.
        let eight = format!("0041:{}", "3C".repeat(16));
        // 16 wide, empty.
        let empty = format!("3000:{}", "0000".repeat(16));
        let wide = format!("04E00:{}", "8001".repeat(16));
        let text = [eight, empty, wide].join("\n") + "\n";
        let z = stored_zip("a.hex", text.as_bytes());
        let (res, _dir) = pack(
            "unihex",
            &[
                (
                    "assets/minecraft/font/default.json",
                    br#"{"providers":[{"type":"unihex","hex_file":"minecraft:font/u.zip","size_overrides":[{"from":"\u4e00","to":"\u4e01","left":1,"right":15}]}]}"#,
                ),
                ("assets/minecraft/font/u.zip", &z),
            ],
        );
        let font = Font::load(&res, &default());
        assert!(font.warnings.is_empty(), "{:?}", font.warnings);
        let a = font.glyph('A');
        assert_eq!((a.advance, a.bold_offset, a.shadow_offset), (3.0, 0.5, 0.5));
        assert_eq!(a.picture.unwrap().width, 4);
        assert_eq!(font.texels('A').unwrap().pixels, vec![u32::MAX; 4 * 16]);
        // Nothing in it: the whole width and a column.
        assert_eq!(font.glyph('\u{3000}').picture.unwrap().width, 17);
        let one = font.glyph('\u{4e00}');
        assert_eq!((one.advance, one.picture.unwrap().width), (8.0, 15));
        let t = font.texels('\u{4e00}').unwrap();
        assert_eq!((t.get(0, 0), t.get(14, 0)), (0, u32::MAX));
    }

    #[test]
    fn fonts_that_read_the_same_unihex_file_share_it() {
        let hex = |c: &str| format!("{c}:{}", "18".repeat(16));
        // 24 and 32 columns wide, kept apart from the narrow ones.
        let wide = format!(
            "0042:{}\n0043:{}\n",
            "800001".repeat(16),
            "80000001".repeat(16)
        );
        // 16 columns is narrow; a later line replaces an earlier one, of another width too.
        let sixteen = format!("0045:{}\n", "8001".repeat(16));
        let again = format!("0046:{}\n0046:{}\n", "800001".repeat(16), "81".repeat(16));
        let text = [hex("0041"), hex("0044")].join("\n") + "\n" + &wide + &sixteen + &again;
        let z = stored_zip("a.hex", text.as_bytes());
        let provider =
            br#"{"type":"unihex","hex_file":"minecraft:font/u.zip","size_overrides":[]}"#;
        let json = |p: &[u8]| [&br#"{"providers":["#[..], p, b"]}"].concat();
        let (default, other) = (json(provider), json(provider));
        // Of two size overrides that name a character, the first counts.
        let overridden = json(
            br#"{"type":"unihex","hex_file":"minecraft:font/u.zip","size_overrides":[{"from":"A","to":"A","left":0,"right":15},{"from":"A","to":"D","left":0,"right":3}]}"#,
        );
        let (res, _dir) = pack(
            "shared",
            &[
                ("assets/minecraft/font/default.json", &default),
                ("assets/minecraft/font/other.json", &other),
                ("assets/minecraft/font/sized.json", &overridden),
                ("assets/minecraft/font/u.zip", &z),
            ],
        );
        let load = |name: &str| Font::load(&res, &Id::minecraft(name).unwrap());
        let (a, b, sized) = (load("default"), load("other"), load("sized"));
        assert!(a.warnings.is_empty(), "{:?}", a.warnings);
        let unihex = |f: &Font| match &*f.providers[0] {
            Provider::Unihex(u) => u.clone(),
            _ => unreachable!(),
        };
        assert!(Arc::ptr_eq(&unihex(&a), &unihex(&b)));
        // Other size overrides are another reading.
        assert!(!Arc::ptr_eq(&unihex(&a), &unihex(&sized)));
        assert_eq!(sized.glyph('A').picture.unwrap().width, 16);
        assert_eq!(sized.glyph('B').picture.unwrap().width, 4);
        let u = unihex(&a);
        assert_eq!((u.narrow.len(), u.wide.len()), (4, 2));
        assert_eq!(a.glyph('E').picture.unwrap().width, 16);
        assert_eq!(a.glyph('F').picture.unwrap().width, 8);
        assert!(a.unihex_bytes() >= 2 * 44 + 2 * 76);
        // Narrow and wide glyphs read as they were written.
        assert_eq!(a.glyph('A').picture.unwrap().width, 2);
        let b24 = a.glyph('B');
        assert_eq!((b24.picture.unwrap().width, b24.advance), (24, 13.0));
        assert_eq!(a.glyph('C').picture.unwrap().width, 32);
        let t = a.texels('C').unwrap();
        assert_eq!(
            (t.get(0, 3), t.get(1, 3), t.get(31, 3)),
            (u32::MAX, 0, u32::MAX)
        );
        let t = a.texels('A').unwrap();
        assert_eq!((t.get(0, 0), t.get(1, 15)), (u32::MAX, u32::MAX));
        assert!(a.has('A') && a.has('C') && !a.has('G'));
        // Once no font holds it, it is read again.
        let first = Arc::downgrade(&u);
        drop((u, a, b));
        assert!(first.upgrade().is_none());
        assert_eq!(load("other").glyph('D').advance, 2.0);
    }

    /// A zip with one stored file.
    fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
        let crc = {
            let mut c = !0u32;
            for &b in data {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        c >> 1 ^ 0xEDB8_8320
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        };
        let (n, size) = (name.len() as u16, data.len() as u32);
        let common = |out: &mut Vec<u8>| {
            out.extend(20u16.to_le_bytes());
            out.extend([0; 2]);
            out.extend([0; 2]);
            out.extend([0; 4]);
            out.extend(crc.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend(size.to_le_bytes());
            out.extend(n.to_le_bytes());
            out.extend([0; 2]);
        };
        let mut z = Vec::new();
        z.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut z);
        z.extend(name.as_bytes());
        z.extend(data);
        let directory = z.len() as u32;
        z.extend(0x0201_4b50u32.to_le_bytes());
        z.extend(20u16.to_le_bytes());
        common(&mut z);
        z.extend([0; 2]); // comment length
        z.extend([0; 2]); // disk
        z.extend([0; 2]); // internal attributes
        z.extend([0; 4]); // external attributes
        z.extend([0; 4]); // the local header's offset
        z.extend(name.as_bytes());
        let length = z.len() as u32 - directory;
        z.extend(0x0605_4b50u32.to_le_bytes());
        z.extend([0; 4]);
        z.extend(1u16.to_le_bytes());
        z.extend(1u16.to_le_bytes());
        z.extend(length.to_le_bytes());
        z.extend(directory.to_le_bytes());
        z.extend([0; 2]);
        z
    }
}
