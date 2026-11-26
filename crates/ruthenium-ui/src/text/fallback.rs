//! Fonts already on the system, for the characters Nunito does not have: small capitals
//! and phonetic letters (servers write their names with them), CJK, symbols, other
//! scripts, monochrome emoji.
//!
//! Only well-known families with wide coverage are looked for, by file name, in the
//! usual font folders. Which characters a file has is read from its `cmap` table alone;
//! the whole file is read only once text is set in it, and kept for the rest of the run.
//! A character no font has is drawn with Nunito's own missing-glyph box; private-use
//! characters (a resource pack's pictures) are never looked for.
//!
//! Distance fields need outlines without overlaps, so static files come before variable
//! ones, and a face without outlines (colour bitmaps, colour layers) never counts as
//! having a character.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rustybuzz::Face;

/// Font files looked for, best first: a regular file and, where the family has one, a
/// bold one. Stems are matched without case.
const FAMILIES: &[(&str, Option<&str>)] = &[
    // Sans faces with Latin, Greek, Cyrillic, IPA and phonetic small capitals.
    ("notosans-regular", Some("notosans-bold")),
    ("dejavusans", Some("dejavusans-bold")),
    ("segoeui", Some("segoeuib")),
    ("opensans-regular", Some("opensans-bold")),
    ("liberationsans-regular", Some("liberationsans-bold")),
    ("arial", Some("arialbd")),
    ("lucidagrande", None),
];

/// After those and the other scripts' Noto Sans files ([`other_script`]): symbols, CJK,
/// emoji and variable files. Mathematical fonts come after the scripts' own, as they
/// have some of their letters, unshaped.
const MORE_FAMILIES: &[(&str, Option<&str>)] = &[
    // Symbols, arrows, maths, box drawing.
    ("notosanssymbols-regular", None),
    ("notosanssymbols2-regular", None),
    ("notosansmath-regular", None),
    ("seguisym", None),
    ("segoeuihistoric", None),
    ("applesymbols", None),
    ("dejavusansmono", None),
    // Chinese, Japanese, Korean.
    ("notosanscjk-regular", Some("notosanscjk-bold")),
    ("notosanscjksc-regular", Some("notosanscjksc-bold")),
    ("notosanscjkjp-regular", Some("notosanscjkjp-bold")),
    ("sourcehansans-regular", Some("sourcehansans-bold")),
    ("msyh", Some("msyhbd")),
    ("yugothr", Some("yugothb")),
    ("malgun", Some("malgunbd")),
    ("notosanscjk-vf.otf", None),
    ("notosanscjk-vf", None),
    ("droidsansfallbackfull", None),
    ("droidsansfallback", None),
    ("wqy-microhei", None),
    ("wqy-zenhei", None),
    // Monochrome emoji (colour ones have no outlines).
    ("notoemoji-regular", None),
    ("seguiemj", None),
    ("symbola", None),
    // Variable files, after the static ones.
    ("notosans[wdth,wght]", None),
    ("notoemoji[wght]", None),
];

/// A Noto Sans file of one script (`NotoSansThai-Regular.ttf`, or a variable
/// `NotoSansArabic[wght].ttf`): 1 for a static file, 2 for a variable one.
fn other_script(stem: &str) -> Option<u8> {
    let rest = stem.strip_prefix("notosans")?;
    if rest.is_empty()
        || rest.starts_with(['-', '['])
        || [
            "mono",
            "display",
            "condensed",
            "ui",
            "cjk",
            "symbols",
            "math",
        ]
        .iter()
        .any(|w| rest.contains(w))
    {
        return None;
    }
    if rest.ends_with("-regular") {
        Some(1)
    } else if rest.ends_with("[wght]") || rest.ends_with("[wdth,wght]") {
        Some(2)
    } else {
        None
    }
}

static ENABLED: AtomicBool = AtomicBool::new(true);
static SYSTEM: OnceLock<SystemFonts> = OnceLock::new();

/// Turns the system fonts off (or on) for every `Fonts` made afterwards (`--font-fallback`).
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Finds the system's fonts and reads the first family's regular face, so the first text
/// that needs them does not wait (call it off the main thread).
pub fn warm() {
    if let Some(s) = system()
        && let Some(f) = s.families.first()
    {
        s.face(f.regular);
    }
}

/// The system's fonts, if they are on; found the first time they are asked for.
pub(crate) fn system() -> Option<&'static SystemFonts> {
    ENABLED
        .load(Ordering::Relaxed)
        .then(|| SYSTEM.get_or_init(SystemFonts::discover))
}

struct File {
    path: PathBuf,
    face: OnceLock<Option<Face<'static>>>,
    /// The file's `cmap` table, read on its own.
    cmap: OnceLock<Option<Vec<u8>>>,
}

/// The `cmap` table of a font file's first face, read without the rest of the file.
fn read_cmap(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let mut f = std::fs::File::open(path)?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head)?;
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let start = if &head[..4] == b"ttcf" {
        // A collection: the first face's offset follows the count.
        let mut first = [0u8; 4];
        f.read_exact(&mut first)?;
        let at = u64::from(be32(&first));
        f.seek(SeekFrom::Start(at))?;
        f.read_exact(&mut head)?;
        at
    } else {
        0
    };
    let tables = usize::from(u16::from_be_bytes([head[4], head[5]]));
    let mut records = vec![0u8; tables * 16];
    f.seek(SeekFrom::Start(start + 12))?;
    f.read_exact(&mut records)?;
    let Some(r) = records
        .as_chunks::<16>()
        .0
        .iter()
        .find(|r| &r[..4] == b"cmap")
    else {
        return Ok(None);
    };
    let (offset, len) = (be32(&r[8..12]), be32(&r[12..16]));
    let mut cmap = vec![0u8; len as usize];
    f.seek(SeekFrom::Start(u64::from(offset)))?;
    f.read_exact(&mut cmap)?;
    Ok(Some(cmap))
}

impl File {
    fn new(path: PathBuf) -> File {
        File {
            path,
            face: OnceLock::new(),
            cmap: OnceLock::new(),
        }
    }

    /// Whether the file has `c`: from the face if it is read, else from its `cmap`.
    fn has(&self, c: char) -> bool {
        if let Some(face) = self.face.get() {
            return face.as_ref().is_some_and(|f| has_outline(f, c));
        }
        let cmap = self
            .cmap
            .get_or_init(|| read_cmap(&self.path).ok().flatten());
        cmap.as_deref()
            .and_then(rustybuzz::ttf_parser::cmap::Table::parse)
            .is_some_and(|t| {
                t.subtables
                    .into_iter()
                    .any(|s| s.is_unicode() && s.glyph_index(u32::from(c)).is_some())
            })
    }

    /// The face, read and parsed the first time; kept for the rest of the run.
    fn face(&self) -> Option<&Face<'static>> {
        self.face
            .get_or_init(|| {
                let bytes = std::fs::read(&self.path).ok()?;
                // Faces borrow their bytes; a font, once read, lives as long as the game.
                let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
                let Some(mut face) = Face::from_slice(bytes, 0) else {
                    eprintln!("[ui] couldn't read the font {}", self.path.display());
                    return None;
                };
                // A variable font's default may be its lightest: take the regular weight.
                if face.is_variable() {
                    face.set_variations(&[rustybuzz::Variation {
                        tag: rustybuzz::ttf_parser::Tag::from_bytes(b"wght"),
                        value: 400.0,
                    }]);
                }
                Some(face)
            })
            .as_ref()
    }
}

/// One family: its regular file and maybe a bold one.
struct Family {
    regular: u16,
    bold: Option<u16>,
}

/// Fallback fonts and which of them has which character.
pub struct SystemFonts {
    files: Vec<File>,
    families: Vec<Family>,
    /// Characters looked up so far: the family that has each, if any.
    chars: Mutex<HashMap<char, Option<u16>>>,
}

impl std::fmt::Debug for SystemFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemFonts")
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

/// Adds the files of `families` that were found, taking them out of `found`.
fn take(
    fonts: &mut SystemFonts,
    found: &mut HashMap<String, PathBuf>,
    families: &[(&str, Option<&str>)],
) {
    for (regular, bold) in families {
        if let Some(r) = found.remove(*regular) {
            let b = bold.and_then(|b| found.remove(b));
            fonts.push(r, b);
        }
    }
}

/// Folders fonts are installed in.
fn font_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let home = env("HOME");
    let mut dirs = Vec::new();
    if cfg!(target_os = "windows") {
        if let Some(w) = env("WINDIR").or_else(|| env("SystemRoot")) {
            dirs.push(w.join("Fonts"));
        }
        if let Some(l) = env("LOCALAPPDATA") {
            dirs.push(l.join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
    } else {
        match env("XDG_DATA_HOME") {
            Some(d) => dirs.push(d.join("fonts")),
            None => {
                if let Some(h) = &home {
                    dirs.push(h.join(".local/share/fonts"));
                }
            }
        }
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
        }
        let data = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        dirs.extend(data.split(':').map(|d| Path::new(d).join("fonts")));
        // Flatpak sandboxes see the host's fonts here.
        dirs.push("/run/host/fonts".into());
        dirs.push("/run/host/user-fonts".into());
    }
    dirs
}

/// Every font file under `dir` (a few levels deep) by lowercase stem; the first found wins.
fn walk(dir: &Path, depth: u32, found: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, found);
            }
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(ext.as_deref(), Some("ttf" | "otf" | "ttc" | "otc")) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            found.entry(stem.to_ascii_lowercase()).or_insert(path);
        }
    }
}

impl SystemFonts {
    fn discover() -> SystemFonts {
        let mut found = HashMap::new();
        for dir in font_dirs() {
            walk(&dir, 4, &mut found);
        }
        let mut fonts = SystemFonts::new(Vec::new());
        take(&mut fonts, &mut found, FAMILIES);
        // The scripts' own fonts, static ones first.
        let mut scripts: Vec<(u8, String)> = found
            .keys()
            .filter_map(|stem| Some((other_script(stem)?, stem.clone())))
            .collect();
        scripts.sort();
        for (_, stem) in scripts {
            if let Some(path) = found.remove(&stem) {
                fonts.push(path, None);
            }
        }
        take(&mut fonts, &mut found, MORE_FAMILIES);
        fonts
    }

    /// Fallbacks from these files, best first (regular and maybe bold): for tests.
    pub fn from_files(files: Vec<(PathBuf, Option<PathBuf>)>) -> SystemFonts {
        let mut fonts = SystemFonts::new(Vec::new());
        for (r, b) in files {
            fonts.push(r, b);
        }
        fonts
    }

    fn new(files: Vec<File>) -> SystemFonts {
        SystemFonts {
            files,
            families: Vec::new(),
            chars: Mutex::new(HashMap::new()),
        }
    }

    fn push(&mut self, regular: PathBuf, bold: Option<PathBuf>) {
        let mut add = |p: PathBuf| {
            self.files.push(File::new(p));
            (self.files.len() - 1) as u16
        };
        let regular = add(regular);
        let bold = bold.map(add);
        self.families.push(Family { regular, bold });
    }

    /// How many families there are.
    pub fn len(&self) -> usize {
        self.families.len()
    }

    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    /// The file `i`'s face (see [`FaceId::System`](super::FaceId::System)).
    pub(crate) fn face(&self, file: u16) -> Option<&Face<'static>> {
        self.files.get(usize::from(file))?.face()
    }

    /// The file to draw family `family` with at `bold`ness.
    pub(crate) fn file(&self, family: u16, bold: bool) -> u16 {
        let f = &self.families[usize::from(family)];
        match f.bold {
            Some(b) if bold && self.face(b).is_some() => b,
            _ => f.regular,
        }
    }
}

impl super::Coverage for SystemFonts {
    fn families(&self) -> u16 {
        self.families.len() as u16
    }

    /// Whether family `family` has `c`.
    fn family_has(&self, family: u16, c: char) -> bool {
        let file = self.families[usize::from(family)].regular;
        self.files[usize::from(file)].has(c)
    }

    /// The first family with `c`; none for private-use characters.
    fn covering(&self, c: char) -> Option<u16> {
        if private_use(c) {
            return None;
        }
        if let Some(&known) = self.chars.lock().expect("fallback lock").get(&c) {
            return known;
        }
        let found = (0..self.families.len() as u16).find(|&f| self.family_has(f, c));
        self.chars.lock().expect("fallback lock").insert(c, found);
        found
    }
}

/// Characters whose pictures only a server's own resource pack has.
fn private_use(c: char) -> bool {
    matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// Whether `face` draws `c` with an outline (a blank glyph counts for spaces).
pub(crate) fn has_outline(face: &Face<'_>, c: char) -> bool {
    face.glyph_index(c)
        .is_some_and(|g| c.is_whitespace() || face.glyph_bounding_box(g).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_fonts_are_known_by_name() {
        assert_eq!(other_script("notosansthai-regular"), Some(1));
        assert_eq!(other_script("notosansarabic[wght]"), Some(2));
        assert_eq!(other_script("notosansarabic[wdth,wght]"), Some(2));
        for not in [
            "notosans-regular",
            "notosans[wdth,wght]",
            "notosansmono-regular",
            "notosansdisplay-regular",
            "notosanscjkjp-regular",
            "notosanssymbols-regular",
            "notosansmath-regular",
            "notosansthai-bold",
            "notoserifthai-regular",
        ] {
            assert_eq!(other_script(not), None, "{not}");
        }
    }

    /// The `cmap` read alone says what the whole face says (the bundled Nunito).
    #[test]
    fn coverage_comes_from_the_cmap_alone() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts/Nunito-Regular.ttf");
        let alone = File::new(path.clone());
        let whole = File::new(path);
        assert!(whole.face().is_some());
        // (Control characters: a Macintosh subtable maps those codes to letters.)
        for c in [
            'a', 'é', 'ѕ', '\u{1D07}', '日', ' ', '\u{E000}', '\u{8E}', '\u{87}',
        ] {
            assert_eq!(alone.has(c), whole.has(c), "{c:?}");
        }
        assert!(alone.face.get().is_none(), "only the cmap was read");
        assert!(alone.has('ѕ') && !alone.has('\u{1D07}'));
        for c in ['\u{E000}', '\u{F8FF}', '\u{F0000}', '\u{10FFFD}'] {
            assert!(private_use(c), "{c:?}");
        }
        assert!(!private_use('a') && !private_use('\u{F900}') && !private_use('\u{EFFFF}'));
        // A glyph without an outline doesn't count, unless the character is a space.
        let face = whole.face().unwrap();
        assert!(face.glyph_index('\u{200B}').is_some());
        assert!(!has_outline(face, '\u{200B}') && has_outline(face, ' '));
    }

    #[test]
    fn missing_files_cover_nothing() {
        use crate::text::Coverage;
        let fonts = SystemFonts::from_files(vec![("/nowhere/font.ttf".into(), None)]);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts.covering('a'), None);
        assert!(fonts.face(0).is_none());
    }
}
