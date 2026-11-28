//! One resource pack: the vanilla pack (client jar + asset index), a
//! folder, or a zip file.
//!
//! A pack is a stack of layers, highest priority first. Folder and zip packs
//! put their applicable overlay directories over the pack root (the last
//! listed overlay wins, `server/packs/OverlayedPackResources.java`); the
//! vanilla pack puts the asset-index objects over the jar's `assets/`
//! (`client/resources/ClientPackSource.java`).

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use regex::{Regex, RegexBuilder};
use serde_json::Value;

use crate::id::{valid_namespace, valid_path};
use crate::limits::{Budget, Limits};
use crate::zip::Zip;

/// The resource pack format of 26.3: 97.1 (`SharedConstants.java:27-29`).
pub const PACK_FORMAT: PackFormat = PackFormat {
    major: 97,
    minor: 1,
};

/// Namespaces the vanilla pack exposes.
const VANILLA_NAMESPACES: [&str; 2] = ["minecraft", "realms"];

/// A pack format version, `major.minor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackFormat {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug)]
pub enum PackError {
    Io(PathBuf, io::Error),
    /// `pack.mcmeta` is missing or has no `pack` section.
    NoMetadata(PathBuf),
    BadMetadata(PathBuf, String),
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            PackError::NoMetadata(p) => {
                write!(f, "{}: no pack.mcmeta with a \"pack\" section", p.display())
            }
            PackError::BadMetadata(p, why) => write!(f, "{}: pack.mcmeta: {why}", p.display()),
        }
    }
}

impl std::error::Error for PackError {}

enum Layer {
    /// Entries under `prefix` (ends in `assets/`).
    Zip { zip: Arc<Zip>, prefix: String },
    /// A folder holding `<namespace>/<path>`.
    Dir(PathBuf),
    /// Asset-index names (`<namespace>/<path>`, sorted) and their object hashes.
    Index {
        objects: PathBuf,
        names: Vec<(Box<str>, Box<str>)>,
    },
}

/// Where a file is inside a pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Loc {
    Zip(usize, usize),
    File(PathBuf),
}

/// `filter.block` of a pack's metadata (`server/packs/resources/ResourceFilterSection.java`).
#[derive(Debug)]
pub struct Filter {
    /// How many patterns it has.
    count: usize,
    /// Some pattern has no namespace regex: it filters every namespace.
    every_namespace: bool,
    /// The namespace regexes, compiled as small ASCII patterns (namespaces are ASCII).
    namespaces: regex::bytes::RegexSet,
    /// Some pattern has no path regex: it filters every path.
    every_path: bool,
    /// The path regexes, tried in one pass: a path is asked about on every lookup below
    /// the pack.
    paths: regex::bytes::RegexSet,
}

impl Filter {
    /// How many patterns it has.
    pub fn patterns(&self) -> usize {
        self.count
    }

    /// Whether any block pattern's namespace regex matches. A pattern without
    /// one matches every namespace.
    pub fn filters_namespace(&self, namespace: &str) -> bool {
        self.every_namespace || self.namespaces.is_match(namespace.as_bytes())
    }

    /// Whether any block pattern's path regex matches. Vanilla does not pair
    /// a pattern's namespace with its path.
    pub fn filters_path(&self, path: &str) -> bool {
        self.every_path || self.paths.is_match(path.as_bytes())
    }
}

/// The longest regular expression a pack may give (filters, atlas `filter` sources):
/// the game's are a few dozen characters.
pub const MAX_PATTERN_BYTES: usize = 1024;
/// What one compiled expression may take, and its lazy DFA's cache: a pattern such as
/// `a{1000}{1000}` compiles to gigabytes otherwise (the regex crate's defaults are 10 and
/// 2 MiB).
const REGEX_BYTES: usize = 1 << 20;
/// What one filter pattern (namespace, path, or atlas filter) may compile to: 16 KiB of
/// ASCII program takes any real pattern (`^textures/block/[a-z_]+\.png$` is under 1 KiB),
/// and the worst one that fits costs under a microsecond per string.
const FILTER_REGEX_BYTES: usize = 1 << 14;

/// A pack's regular expression, no longer than [`MAX_PATTERN_BYTES`] and compiled into at
/// most [`REGEX_BYTES`].
#[cfg(test)]
pub(crate) fn pack_regex(pattern: &str) -> Result<Regex, String> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(format!("a pattern longer than {MAX_PATTERN_BYTES} bytes"));
    }
    RegexBuilder::new(pattern)
        .size_limit(REGEX_BYTES)
        .dfa_size_limit(REGEX_BYTES)
        .build()
        .map_err(|e| e.to_string())
}

/// A pack's regular expression for ASCII resource ids (atlas `filter` sources), no longer
/// than [`MAX_PATTERN_BYTES`] and compiled into at most [`FILTER_REGEX_BYTES`] with ASCII
/// character classes.
pub(crate) fn filter_regex(pattern: &str) -> Result<regex::bytes::Regex, String> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(format!("a pattern longer than {MAX_PATTERN_BYTES} bytes"));
    }
    // Over bytes: without Unicode, the str regex refuses `.` and negated classes (they
    // could match inside a UTF-8 sequence), and ids are ASCII anyway.
    regex::bytes::RegexBuilder::new(pattern)
        .unicode(false)
        .size_limit(FILTER_REGEX_BYTES)
        .dfa_size_limit(FILTER_REGEX_BYTES * 4)
        .build()
        .map_err(|e| e.to_string())
}

/// A path/namespace pattern as Java reads it on ASCII identifiers: classes such as `\w` are
/// ASCII ones (Java's default), so that a pattern compiles small.
fn ascii_pattern(pattern: &str, bytes: usize) -> Result<regex::bytes::Regex, String> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(format!("a pattern longer than {MAX_PATTERN_BYTES} bytes"));
    }
    regex::bytes::RegexBuilder::new(pattern)
        .unicode(false)
        .size_limit(bytes)
        .dfa_size_limit(FILTER_REGEX_BYTES * 4)
        .build()
        .map_err(|e| e.to_string())
}

/// A loaded pack.
pub struct Pack {
    name: String,
    layers: Vec<Layer>,
    namespaces: BTreeSet<String>,
    filter: Option<Filter>,
    /// The whole `pack.mcmeta` (built-in metadata for the vanilla pack).
    metadata: Value,
    /// What its files are read under, and what they have cost this load.
    limits: Limits,
    budget: Arc<Budget>,
    /// A server's pack: what it could run is hidden ([`HIDDEN`]).
    untrusted: bool,
}

/// Folders of a namespace an untrusted pack never shows: shaders (core shaders, their
/// includes and post-processing programs) and post-effect chains. The client compiles its
/// own and runs no code a pack brings; vanilla's core-shader overrides are unsupported.
pub const HIDDEN: [&str; 2] = ["shaders/", "post_effect/"];

fn hidden(path: &str) -> bool {
    HIDDEN.iter().any(|h| path.starts_with(h))
}

impl fmt::Debug for Pack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pack")
            .field("name", &self.name)
            .field("layers", &self.layers.len())
            .field("namespaces", &self.namespaces)
            .finish()
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> PackError + use<> {
    let path = path.to_owned();
    move |e| PackError::Io(path, e)
}

/// `int` or `[major]` or `[major, minor]` (`PackFormat.fullCodec`).
fn parse_format(v: &Value, default_minor: u32) -> Option<PackFormat> {
    let n = |v: &Value| {
        v.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| n <= i32::MAX as u32)
    };
    match v {
        Value::Array(a) if !a.is_empty() && a.len() <= 256 => Some(PackFormat {
            major: n(&a[0])?,
            minor: a.get(1).map_or(Some(default_minor), n)?,
        }),
        Value::Array(_) => None,
        v => Some(PackFormat {
            major: n(v)?,
            minor: default_minor,
        }),
    }
}

/// Applicable overlay directories, in listed order.
fn overlays(meta: &Value) -> Result<Vec<String>, String> {
    let Some(section) = meta.get("overlays") else {
        return Ok(Vec::new());
    };
    let entries = section
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("overlays: no \"entries\" list")?;
    let mut out = Vec::new();
    for e in entries {
        let dir = e
            .get("directory")
            .and_then(Value::as_str)
            .filter(|d| {
                !d.is_empty()
                    && *d != "."
                    && *d != ".."
                    && d.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            })
            .ok_or("overlays: bad \"directory\"")?;
        let min = e.get("min_format").map(|v| parse_format(v, 0));
        let max = e
            .get("max_format")
            .map(|v| parse_format(v, i32::MAX as u32));
        match (min, max) {
            (Some(Some(min)), Some(Some(max))) => {
                if min > max {
                    return Err(format!(
                        "overlay {dir}: min_format is greater than max_format"
                    ));
                }
                if (min..=max).contains(&PACK_FORMAT) {
                    // Deduplicate keeping the last (highest-priority) occurrence.
                    out.retain(|existing| existing != dir);
                    out.push(dir.to_owned());
                }
            }
            (None, None) => {
                // Old `formats` only: ranges end at or below format 64, so
                // they never apply to 97.1. Neither: vanilla skips the entry.
            }
            _ => {
                return Err(format!(
                    "overlay {dir}: needs both min_format and max_format"
                ));
            }
        }
    }
    Ok(out)
}

/// The `filter` section, of at most `max` patterns ([`Limits::pack_filters`]).
fn filter(meta: &Value, max: usize) -> Result<Option<Filter>, String> {
    let Some(section) = meta.get("filter") else {
        return Ok(None);
    };
    let blocks = section
        .get("block")
        .and_then(Value::as_array)
        .ok_or("filter: no \"block\" list")?;
    if blocks.len() > max {
        return Err(format!("filter: more than {max} patterns"));
    }
    let text = |p: &'_ Value, key: &str| -> Result<Option<String>, String> {
        match p.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if s.len() > MAX_PATTERN_BYTES => Err(format!(
                "filter: a pattern longer than {MAX_PATTERN_BYTES} bytes"
            )),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("filter: \"{key}\" is not a string")),
        }
    };
    let mut namespaces = Vec::new();
    let mut every_namespace = false;
    let mut paths = Vec::new();
    let mut every_path = false;
    for b in blocks {
        match text(b, "namespace")? {
            Some(ns) => {
                ascii_pattern(&ns, FILTER_REGEX_BYTES).map_err(|e| format!("filter: {e}"))?;
                namespaces.push(ns);
            }
            None => every_namespace = true,
        }
        match text(b, "path")? {
            // Each within its own limit, before the set of them is built.
            Some(p) => {
                ascii_pattern(&p, FILTER_REGEX_BYTES).map_err(|e| format!("filter: {e}"))?;
                paths.push(p);
            }
            None => every_path = true,
        }
    }
    let namespaces = regex::bytes::RegexSetBuilder::new(&namespaces)
        .unicode(false)
        .size_limit(FILTER_REGEX_BYTES * namespaces.len().max(1))
        .dfa_size_limit(REGEX_BYTES)
        .build()
        .map_err(|e| format!("filter: {e}"))?;
    let paths = regex::bytes::RegexSetBuilder::new(&paths)
        .unicode(false)
        .size_limit(FILTER_REGEX_BYTES * paths.len().max(1))
        .dfa_size_limit(REGEX_BYTES)
        .build()
        .map_err(|e| format!("filter: {e}"))?;
    Ok(Some(Filter {
        count: blocks.len(),
        every_namespace,
        namespaces,
        every_path,
        paths,
    }))
}

/// A server pack's `pack.mcmeta`, parsed and checked as `Pack.readPackMetadata` does:
/// the metadata, and the overlays that apply.
fn untrusted_metadata(
    path: &Path,
    bytes: &[u8],
    limits: &Limits,
) -> Result<(Value, Vec<String>), PackError> {
    let bad = |why: String| PackError::BadMetadata(path.to_owned(), why);
    let metadata: Value = crate::meta::parse_json(bytes).map_err(bad)?;
    let pack = metadata.get("pack").filter(|p| p.is_object());
    if pack.is_none_or(|p| p.get("description").is_none()) {
        return Err(PackError::NoMetadata(path.to_owned()));
    }
    let entries = metadata
        .get("overlays")
        .and_then(|s| s.get("entries"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if entries > limits.pack_overlays {
        return Err(bad(format!("more than {} overlays", limits.pack_overlays)));
    }
    let overlays = overlays(&metadata).map_err(bad)?;
    Ok((metadata, overlays))
}

/// The pack's filter, or none if the section does not read: vanilla logs "Failed to get
/// filter section" and loads the pack without one (`MultiPackResourceManager`).
fn filter_or_none(path: &Path, meta: &Value, max: usize) -> Option<Filter> {
    filter(meta, max).unwrap_or_else(|e| {
        eprintln!(
            "[assets] {}: {e}; the pack is loaded without its filter",
            path.display()
        );
        None
    })
}

impl Pack {
    /// The vanilla pack: the client jar's `assets/`, with the asset-index
    /// objects over it.
    pub fn vanilla(
        client_jar: &Path,
        asset_index: &Path,
        objects: &Path,
    ) -> Result<Pack, PackError> {
        let zip = Arc::new(Zip::open(client_jar).map_err(io_err(client_jar))?);
        let text = std::fs::read_to_string(asset_index).map_err(io_err(asset_index))?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|e| PackError::BadMetadata(asset_index.to_owned(), e.to_string()))?;
        let mut names: Vec<(Box<str>, Box<str>)> = json
            .get("objects")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(name, obj)| {
                let hash = obj.get("hash")?.as_str()?;
                (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                    .then(|| (name.as_str().into(), hash.into()))
            })
            .collect();
        names.sort();
        Ok(Pack {
            name: "vanilla".into(),
            layers: vec![
                Layer::Index {
                    objects: objects.to_owned(),
                    names,
                },
                Layer::Zip {
                    zip,
                    prefix: "assets/".into(),
                },
            ],
            namespaces: VANILLA_NAMESPACES.iter().map(|s| s.to_string()).collect(),
            filter: None,
            metadata: serde_json::json!({
                "pack": {
                    "description": {"translate": "resourcePack.vanilla.description"},
                    "min_format": [PACK_FORMAT.major, PACK_FORMAT.minor],
                    "max_format": [PACK_FORMAT.major, i32::MAX],
                }
            }),
            limits: Limits::trusted(),
            budget: Arc::default(),
            untrusted: false,
        })
    }

    /// A resource pack folder or zip file. Fails if it has no valid
    /// `pack.mcmeta` (vanilla does not load such packs).
    pub fn open(path: &Path) -> Result<Pack, PackError> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let (meta_bytes, root): (Option<Vec<u8>>, Root) = if path.is_dir() {
            let meta = match std::fs::read(path.join("pack.mcmeta")) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(io_err(&path.join("pack.mcmeta"))(e)),
            };
            (meta, Root::Dir(path.to_owned()))
        } else {
            let zip = Arc::new(Zip::open(path).map_err(io_err(path))?);
            let meta = match zip.find("pack.mcmeta") {
                Some(i) => Some(zip.read(i).map_err(io_err(path))?.into_owned()),
                None => None,
            };
            (meta, Root::Zip(zip))
        };
        let meta_bytes = meta_bytes.ok_or_else(|| PackError::NoMetadata(path.to_owned()))?;
        let bad = |why: String| PackError::BadMetadata(path.to_owned(), why);
        let metadata: Value = crate::meta::parse_json(&meta_bytes).map_err(bad)?;
        if !metadata.get("pack").is_some_and(Value::is_object) {
            return Err(PackError::NoMetadata(path.to_owned()));
        }
        let filter = filter_or_none(path, &metadata, usize::MAX);
        let overlays = overlays(&metadata).map_err(bad)?;

        // Highest priority first: the last applicable overlay, ..., the root.
        let mut layers = Vec::new();
        for dir in overlays
            .iter()
            .rev()
            .map(|d| format!("{d}/"))
            .chain([String::new()])
        {
            layers.push(match &root {
                Root::Dir(p) => Layer::Dir(p.join(&dir).join("assets")),
                Root::Zip(z) => Layer::Zip {
                    zip: z.clone(),
                    prefix: format!("{dir}assets/"),
                },
            });
        }
        let mut namespaces = BTreeSet::new();
        for layer in &layers {
            match layer {
                Layer::Dir(p) => {
                    for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                        if e.file_type().is_ok_and(|t| t.is_dir())
                            && let Some(n) = e.file_name().to_str().filter(|n| valid_namespace(n))
                        {
                            namespaces.insert(n.to_owned());
                        }
                    }
                }
                Layer::Zip { zip, prefix } => {
                    for i in zip.with_prefix(prefix) {
                        if let Some((ns, _)) = zip.name(i)[prefix.len()..].split_once('/')
                            && valid_namespace(ns)
                        {
                            namespaces.insert(ns.to_owned());
                        }
                    }
                }
                Layer::Index { .. } => {}
            }
        }
        Ok(Pack {
            name,
            layers,
            namespaces,
            filter,
            metadata,
            limits: Limits::trusted(),
            budget: Arc::default(),
            untrusted: false,
        })
    }

    /// A pack whose every byte is hostile: a zip file only, read under `limits`. Fails as
    /// vanilla's `Pack.readPackMetadata` does when `pack.mcmeta` is missing or invalid (no
    /// `pack` section, or one without a `description`), and when the archive itself is
    /// broken or over a limit (vanilla's `ZipFile` refuses a broken one; the pack then has
    /// no metadata). Its shader and post-effect folders are hidden ([`HIDDEN`]).
    pub fn open_untrusted(path: &Path, limits: &Limits) -> Result<Pack, PackError> {
        Pack::open_untrusted_with(path, limits, Arc::new(Budget::default()))
    }

    /// Like [`Self::open_untrusted`], sharing `budget` (bytes, texels, and cancel flag)
    /// across every untrusted pack in one load.
    pub fn open_untrusted_with(
        path: &Path,
        limits: &Limits,
        budget: Arc<Budget>,
    ) -> Result<Pack, PackError> {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if !path.is_file() {
            return Err(PackError::Io(
                path.to_owned(),
                io::Error::new(io::ErrorKind::InvalidInput, "not a zip file"),
            ));
        }
        let zip =
            Arc::new(Zip::open_untrusted(path, limits, budget.clone()).map_err(io_err(path))?);
        let i = zip
            .find("pack.mcmeta")
            .ok_or_else(|| PackError::NoMetadata(path.to_owned()))?;
        let meta_bytes = zip.read(i).map_err(io_err(path))?;
        if meta_bytes.len() as u64 > limits.json_bytes {
            let why = format!("{} bytes is too large", meta_bytes.len());
            return Err(PackError::BadMetadata(path.to_owned(), why));
        }
        let (metadata, overlays) = untrusted_metadata(path, &meta_bytes, limits)?;
        let filter = filter_or_none(path, &metadata, limits.pack_filters);
        let mut layers = Vec::new();
        for dir in overlays
            .iter()
            .rev()
            .map(|d| format!("{d}/"))
            .chain([String::new()])
        {
            layers.push(Layer::Zip {
                zip: zip.clone(),
                prefix: format!("{dir}assets/"),
            });
        }
        let mut namespaces = BTreeSet::new();
        for layer in &layers {
            if budget.cancelled() {
                return Err(PackError::Io(
                    path.to_owned(),
                    io::Error::new(io::ErrorKind::Interrupted, "the load was cancelled"),
                ));
            }
            if let Layer::Zip { zip, prefix } = layer {
                for (step, i) in zip.with_prefix(prefix).enumerate() {
                    if step & 255 == 0 && budget.cancelled() {
                        return Err(PackError::Io(
                            path.to_owned(),
                            io::Error::new(io::ErrorKind::Interrupted, "the load was cancelled"),
                        ));
                    }
                    if let Some((ns, _)) = zip.name(i)[prefix.len()..].split_once('/')
                        && valid_namespace(ns)
                        && ns != "."
                        && ns != ".."
                        && !namespaces.contains(ns)
                    {
                        if namespaces.len() >= limits.pack_namespaces {
                            return Err(PackError::BadMetadata(
                                path.to_owned(),
                                format!(
                                    "more than {} namespaces in one pack",
                                    limits.pack_namespaces
                                ),
                            ));
                        }
                        namespaces.insert(ns.to_owned());
                    }
                }
            }
        }
        // Don't permanently retain arbitrary unused JSON sections from untrusted pack.mcmeta.
        Ok(Pack {
            name,
            layers,
            namespaces,
            filter,
            metadata: Value::Null,
            limits: *limits,
            budget,
            untrusted: true,
        })
    }

    /// Whether a server's downloaded pack can be loaded, as cheaply as can be: what
    /// vanilla's `Pack.readPackMetadata` checks (a `pack.mcmeta` with a `pack` section
    /// and a `description`, and readable overlays), read under `limits`. Only
    /// `pack.mcmeta` is read ([`Zip::read_one_untrusted`]): no other entry, no filter's
    /// regular expressions (vanilla reads the filter later, and ignores a broken one).
    /// [`Self::open_untrusted`] can still refuse an archive this passes.
    pub fn check_untrusted(path: &Path, limits: &Limits) -> Result<(), PackError> {
        if !path.is_file() {
            return Err(PackError::Io(
                path.to_owned(),
                io::Error::new(io::ErrorKind::InvalidInput, "not a zip file"),
            ));
        }
        let limits = Limits {
            entry_bytes: limits
                .entry_bytes
                .min(limits.mcmeta_bytes)
                .min(limits.json_bytes),
            ..*limits
        };
        let bytes = Zip::read_one_untrusted(path, "pack.mcmeta", &limits)
            .map_err(io_err(path))?
            .ok_or_else(|| PackError::NoMetadata(path.to_owned()))?;
        untrusted_metadata(path, &bytes, &limits).map(|_| ())
    }

    /// What the pack's files are read under.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// What the pack has cost the current load.
    pub fn budget(&self) -> &Arc<Budget> {
        &self.budget
    }

    /// Gives the pack's load a cancel flag ([`Budget::cancel_with`]).
    pub fn cancel_with(&self, flag: Arc<std::sync::atomic::AtomicBool>) {
        self.budget.cancel_with(flag);
    }

    /// A new load of the same pack begins: what it costs is counted from nothing again.
    pub fn reset_budget(&self) {
        self.budget.reset();
    }

    /// A server's pack, opened by [`Pack::open_untrusted`].
    pub fn is_untrusted(&self) -> bool {
        self.untrusted
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn namespaces(&self) -> &BTreeSet<String> {
        &self.namespaces
    }

    pub fn filter(&self) -> Option<&Filter> {
        self.filter.as_ref()
    }

    /// Loads the pack without its filter (over the stack's limit of patterns).
    pub(crate) fn drop_filter(&mut self) {
        self.filter = None;
    }

    /// The pack's `pack.mcmeta`.
    pub fn metadata(&self) -> &Value {
        &self.metadata
    }

    pub(crate) fn find(&self, namespace: &str, path: &str) -> Option<Loc> {
        if !plain_relative(namespace) || !plain_relative(path) {
            return None;
        }
        if self.untrusted && hidden(path) {
            return None;
        }
        for (i, layer) in self.layers.iter().enumerate() {
            match layer {
                Layer::Zip { zip, prefix } => {
                    if let Some(e) = zip.find(&format!("{prefix}{namespace}/{path}")) {
                        return Some(Loc::Zip(i, e));
                    }
                }
                Layer::Dir(root) => {
                    let p = root.join(namespace).join(path);
                    if p.is_file() {
                        return Some(Loc::File(p));
                    }
                }
                Layer::Index { objects, names } => {
                    let key = format!("{namespace}/{path}");
                    if let Ok(k) = names.binary_search_by(|(n, _)| (**n).cmp(&key)) {
                        return Some(Loc::File(object_path(objects, &names[k].1)));
                    }
                }
            }
        }
        None
    }

    /// Every file under `<namespace>/<dir>/`, with paths relative to the
    /// namespace. The highest layer wins per path; the order is unspecified.
    pub(crate) fn list(&self, namespace: &str, dir: &str, out: &mut dyn FnMut(String, Loc)) {
        if !plain_relative(namespace) || (!dir.is_empty() && !plain_relative(dir)) {
            return;
        }
        if self.untrusted && hidden(&format!("{dir}/")) {
            return;
        }
        let mut found: HashMap<String, Loc> = HashMap::new();
        for (i, layer) in self.layers.iter().enumerate() {
            match layer {
                Layer::Zip { zip, prefix } => {
                    let start = format!("{prefix}{namespace}/");
                    for e in zip.with_prefix(&format!("{start}{dir}/")) {
                        let path = &zip.name(e)[start.len()..];
                        if valid_path(path) && !found.contains_key(path) {
                            found.insert(path.to_owned(), Loc::Zip(i, e));
                        }
                    }
                }
                Layer::Dir(root) => {
                    let base = root.join(namespace);
                    walk(&base.join(dir), &mut |p| {
                        let Ok(rel) = p.strip_prefix(&base) else {
                            return;
                        };
                        let Some(rel) = rel.to_str() else { return };
                        let rel = rel.replace(std::path::MAIN_SEPARATOR, "/");
                        if valid_path(&rel) && !found.contains_key(&rel) {
                            found.insert(rel, Loc::File(p.to_owned()));
                        }
                    });
                }
                Layer::Index { objects, names } => {
                    let start = format!("{namespace}/");
                    let prefix = format!("{start}{dir}/");
                    let from = names.partition_point(|(n, _)| **n < *prefix);
                    for (name, hash) in names[from..]
                        .iter()
                        .take_while(|(n, _)| n.starts_with(&prefix))
                    {
                        let path = &name[start.len()..];
                        if valid_path(path) && !found.contains_key(path) {
                            found.insert(path.to_owned(), Loc::File(object_path(objects, hash)));
                        }
                    }
                }
            }
        }
        for (path, loc) in found {
            out(path, loc);
        }
    }

    /// A file's contents, taken from the load's budget.
    pub(crate) fn read(&self, loc: &Loc) -> io::Result<Cow<'_, [u8]>> {
        self.read_inner(loc, true)
    }

    /// A file's contents, not taken from the load's budget: for files read while playing.
    pub(crate) fn read_on_demand(&self, loc: &Loc) -> io::Result<Cow<'_, [u8]>> {
        self.read_inner(loc, false)
    }

    fn read_inner(&self, loc: &Loc, budgeted: bool) -> io::Result<Cow<'_, [u8]>> {
        match loc {
            Loc::Zip(layer, e) => match self.layers.get(*layer) {
                Some(Layer::Zip { zip, .. }) if budgeted => zip.read(*e),
                Some(Layer::Zip { zip, .. }) => zip.read_on_demand(*e),
                _ => Err(io::Error::other("zip location in a non-zip layer")),
            },
            Loc::File(p) => std::fs::read(p).map(Cow::Owned),
        }
    }
}

enum Root {
    Dir(PathBuf),
    Zip(Arc<Zip>),
}

/// No empty, `.` or `..` segments, so a path cannot leave the pack folder.
fn plain_relative(path: &str) -> bool {
    path.split('/')
        .all(|s| !s.is_empty() && s != "." && s != "..")
}

fn object_path(objects: &Path, hash: &str) -> PathBuf {
    objects.join(&hash[..2]).join(hash)
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&p, f),
            Ok(t) if t.is_file() => f(&p),
            // Symbolic links are not followed.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_ranges() {
        let meta = serde_json::json!({"overlays": {"entries": [
            {"directory": "old", "formats": [15, 64]},
            {"directory": "new", "min_format": 97, "max_format": [97, 5]},
            {"directory": "later", "min_format": [97, 2], "max_format": 120},
            {"directory": "any", "min_format": 80, "max_format": 100},
        ]}});
        assert_eq!(overlays(&meta).unwrap(), ["new", "any"]);
        let half =
            serde_json::json!({"overlays": {"entries": [{"directory": "x", "min_format": 97}]}});
        assert!(overlays(&half).is_err());
        let bad_dir = serde_json::json!({"overlays": {"entries": [{"directory": "a/b", "min_format": 1, "max_format": 2}]}});
        assert!(overlays(&bad_dir).is_err());
    }

    /// A short pattern that compiles to megabytes is refused, though the regex crate's
    /// own limits would take it; real patterns compile.
    #[test]
    fn pack_patterns_compile_small() {
        assert!(Regex::new("a{50000}").is_ok());
        assert!(pack_regex("a{50000}").is_err());
        assert!(pack_regex("^textures/block/(stone|dirt)$").is_ok());
        assert!(pack_regex(&"a".repeat(MAX_PATTERN_BYTES + 1)).is_err());
    }

    /// The worst filter a pack may bring (eight of the costliest patterns found) answers
    /// a path in microseconds: with Unicode classes and 1 MiB a pattern, eight of
    /// `(\w|\d|\s){20}x` took 5 ms a path.
    #[test]
    fn a_filter_answers_quickly() {
        let blocks = [r#"{"path": "(\\w|\\d|\\s){90}x"}"#; 8].join(",");
        let meta: Value =
            serde_json::from_str(&format!(r#"{{"filter": {{"block": [{blocks}]}}}}"#)).unwrap();
        let f = filter(&meta, 8).unwrap().unwrap();
        assert_eq!(f.patterns(), 8);
        let path = "textures/block/a_banana_and_a_papaya_on_a_tea_tray_at_a_bazaar/a.png";
        let started = std::time::Instant::now();
        for _ in 0..2000 {
            assert!(!f.filters_path(path));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        // Classes are Java's ASCII ones: \w is [A-Za-z0-9_].
        let meta: Value =
            serde_json::from_str(r#"{"filter": {"block": [{"path": "^\\w+$"}]}}"#).unwrap();
        let f = filter(&meta, 8).unwrap().unwrap();
        assert!(f.filters_path("stone_2") && !f.filters_path("a/b"));
        assert!(
            filter(
                &serde_json::json!({"filter": {"block": [{"path": "a"}, {"path": "b"}]}}),
                1
            )
            .is_err()
        );
        // A pattern of more than 16 KiB of program, though under the 1 MiB of others.
        assert!(pack_regex("(.|..){100}x").is_ok());
        let long = serde_json::json!({"filter": {"block": [{"path": "(.|..){100}x"}]}});
        assert!(filter(&long, 8).is_err());
    }

    #[test]
    #[ignore]
    fn filter_lookup_cost() {
        // Run with `--ignored --nocapture` in release: the worst patterns found, as many
        // as a pack may have (BENCH_PATTERNS), each repeated N times (BENCH_N).
        // The largest repetition of each template that compiles under the limit.
        let templates = [
            ".*a.{N}",
            ".*[aeiou_].{N}$",
            "(.*_.{N})|(.*e.{N})|(.*o.{N})",
            "(.|..){N}x",
            "(\\w*a){N}b",
            "([^/]*/){N}x",
            "(\\w|\\d|\\s){N}x",
            "(.*a.{5}){N}",
            "([a-z]+/)*[a-z]{N}\\.png",
            "(a|b|c|d|e|f|g|h)*a(a|b|c|d|e|f|g|h){N}",
            "\\pL{N}",
        ];
        let paths: Vec<String> = (0..2000).map(|i| format!("textures/block/a_banana_and_a_papaya_{i}_on_a_tea_tray_at_a_bazaar_in_alabama/a_{i}.png")).collect();
        for t in templates {
            let fits = |n: usize| {
                ascii_pattern(&t.replace('N', &n.to_string()), FILTER_REGEX_BYTES).is_ok()
            };
            let (mut lo, mut hi) = (1usize, 1 << 20);
            if !fits(lo) {
                eprintln!("BENCH {t}: nothing fits");
                continue;
            }
            while lo + 1 < hi {
                let m = (lo + hi) / 2;
                if fits(m) { lo = m } else { hi = m }
            }
            if let Some(n) = std::env::var("BENCH_N")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
            {
                lo = lo.min(n);
            }
            let pat = t.replace('N', &lo.to_string());
            let n = std::env::var("BENCH_PATTERNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(64);
            let blocks: Vec<String> = (0..n)
                .map(|i| {
                    format!(
                        r#"{{"path": "{}{}"}}"#,
                        pat.replace('\\', "\\\\"),
                        if std::env::var("BENCH_SAME").is_ok() {
                            String::new()
                        } else {
                            format!("(?:){i}")
                        }
                    )
                })
                .collect();
            let meta: Value = serde_json::from_str(&format!(
                r#"{{"filter": {{"block": [{}]}}}}"#,
                blocks.join(",")
            ))
            .unwrap();
            let f = match filter(&meta, usize::MAX) {
                Ok(Some(f)) => f,
                e => {
                    eprintln!("BENCH {t}: {:?}", e.err());
                    continue;
                }
            };
            let started = std::time::Instant::now();
            let hits = paths.iter().filter(|p| f.filters_path(p)).count();
            eprintln!(
                "BENCH {t} N={lo}: {n} patterns, {:?} for 2000 paths ({hits} hits)",
                started.elapsed()
            );
        }
    }

    #[test]
    fn filter_patterns_are_not_paired() {
        let meta = serde_json::json!({"filter": {"block": [
            {"namespace": "^minecraft$", "path": "^textures/block/"},
            {"namespace": "^other$"},
        ]}});
        let f = filter(&meta, usize::MAX).unwrap().unwrap();
        assert!(f.filters_namespace("minecraft"));
        assert!(f.filters_namespace("other"));
        assert!(!f.filters_namespace("third"));
        // The second pattern has no path regex, so it filters every path.
        assert!(f.filters_path("sounds.json"));
    }
}
