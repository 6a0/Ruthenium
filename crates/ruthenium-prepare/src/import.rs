//! Importing a verified install into Ruthenium's own cache.
//!
//! Files are hard-linked when the cache is on the same file system as the
//! launcher (no extra disk space), otherwise copied. Every file is checked
//! against its pinned or indexed SHA-1 on the way in, and the asset index
//! against its pinned digest ([`crate::asset_index`]), so a damaged launcher
//! download never reaches the cache. Layout:
//!
//! ```text
//! <cache>/versions/<id>/client.jar
//! <cache>/versions/<id>/libraries/<maven path>
//! <cache>/versions/<id>/import.ok        key of the completed import
//! <cache>/assets/indexes/<index>.json    shared between versions
//! <cache>/assets/objects/<xx>/<sha1>
//! ```

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::asset_index;
use crate::discover::Install;
use crate::hash::sha1_file;
use crate::step;
use crate::version::{AssetIndex, GameVersion};

const MARKER: &str = "import.ok";

/// A completed import.
#[derive(Debug, Clone)]
pub struct Imported {
    pub version: GameVersion,
    /// `<cache>/versions/<id>`.
    pub dir: PathBuf,
    pub client_jar: PathBuf,
    pub libraries: PathBuf,
    /// `<cache>/assets`.
    pub assets: PathBuf,
}

impl Imported {
    fn at(version: &GameVersion, cache: &Path) -> Imported {
        // Absolute, because tools run with a different working directory.
        let cache = std::path::absolute(cache).unwrap_or_else(|_| cache.to_owned());
        let dir = cache.join("versions").join(version.id);
        Imported {
            version: *version,
            client_jar: dir.join("client.jar"),
            libraries: dir.join("libraries"),
            assets: cache.join("assets"),
            dir,
        }
    }

    /// The client jar followed by every class-path library.
    pub fn class_path(&self) -> Vec<PathBuf> {
        std::iter::once(self.client_jar.clone())
            .chain(
                self.version
                    .libraries
                    .iter()
                    .map(|lib| self.libraries.join(lib.rel_path())),
            )
            .collect()
    }

    pub fn asset_index(&self) -> PathBuf {
        self.assets
            .join("indexes")
            .join(format!("{}.json", self.version.asset_index.id))
    }

    /// Identifies the imported inputs; later steps include it in their keys.
    pub fn key(&self) -> String {
        import_key(&self.version)
    }
}

fn import_key(version: &GameVersion) -> String {
    let mut parts = vec![
        "import-v1",
        version.id,
        version.client_jar.sha1,
        version.asset_index.pinned_sha256,
    ];
    parts.extend(version.libraries.iter().map(|l| l.sha1));
    step::key(&parts)
}

/// The import in `cache`, if one has completed for this version.
pub fn open(version: &GameVersion, cache: &Path) -> Option<Imported> {
    let imported = Imported::at(version, cache);
    let marker = std::fs::read_to_string(imported.dir.join(MARKER)).ok()?;
    (marker.trim() == import_key(version)).then_some(imported)
}

/// What an import did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    pub linked: usize,
    pub copied: usize,
    /// Already in the cache with the right hash.
    pub present: usize,
    /// Distinct asset objects in the index.
    pub objects: usize,
}

#[derive(Debug)]
pub enum ImportError {
    Io {
        path: PathBuf,
        error: io::Error,
    },
    /// A file's contents do not match its expected SHA-1.
    Mismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    BadAssetIndex {
        path: PathBuf,
        reason: String,
    },
    /// The launcher has not downloaded every asset object.
    MissingObjects {
        missing: usize,
        total: usize,
        examples: Vec<String>,
    },
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            ImportError::Mismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "{} is damaged: SHA-1 {actual}, expected {expected}",
                path.display()
            ),
            ImportError::BadAssetIndex { path, reason } => {
                write!(f, "{}: {reason}", path.display())
            }
            ImportError::MissingObjects {
                missing,
                total,
                examples,
            } => write!(
                f,
                "{missing} of {total} asset files are missing (e.g. {}); play the version once in your launcher so it finishes downloading",
                examples.join(", ")
            ),
        }
    }
}

impl std::error::Error for ImportError {}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ImportError + use<> {
    let path = path.to_owned();
    move |error| ImportError::Io { path, error }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placed {
    Linked,
    Copied,
    Present,
}

/// Puts `src` at `dst`, checking the result against `sha1`.
fn place(src: &Path, dst: &Path, sha1: &str) -> Result<Placed, ImportError> {
    if dst.is_file() && sha1_file(dst).map_err(io_err(dst))? == sha1 {
        return Ok(Placed::Present);
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    match std::fs::remove_file(dst) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(dst)(e)),
        _ => {}
    }
    let placed = if std::fs::hard_link(src, dst).is_ok() {
        Placed::Linked
    } else {
        let tmp = dst.with_extension("partial");
        std::fs::copy(src, &tmp).map_err(io_err(src))?;
        std::fs::rename(&tmp, dst).map_err(io_err(dst))?;
        Placed::Copied
    };
    let actual = sha1_file(dst).map_err(io_err(dst))?;
    if actual != sha1 {
        let _ = std::fs::remove_file(dst);
        return Err(ImportError::Mismatch {
            path: src.to_owned(),
            expected: sha1.to_owned(),
            actual,
        });
    }
    Ok(placed)
}

/// Puts the launcher's asset index at `dst` and returns the `(sha1, size)` of
/// every distinct object in it. The bytes written are the bytes checked
/// against the pin. Always a copy: a launcher replaces its index when Mojang
/// republishes it, and one that wrote in place would change a hard link too.
fn place_index(
    src: &Path,
    dst: &Path,
    pin: &AssetIndex,
) -> Result<(Placed, Vec<(String, u64)>), ImportError> {
    let bytes = std::fs::read(src).map_err(io_err(src))?;
    let entries = asset_index::verify(&bytes, pin).map_err(|e| ImportError::BadAssetIndex {
        path: src.to_owned(),
        reason: e.to_string(),
    })?;
    let placed = if std::fs::read(dst).is_ok_and(|have| have == bytes) {
        Placed::Present
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let tmp = dst.with_extension("partial");
        std::fs::write(&tmp, &bytes).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, dst).map_err(io_err(dst))?;
        Placed::Copied
    };
    let mut objects: Vec<(String, u64)> = entries.into_iter().map(|e| (e.hash, e.size)).collect();
    objects.sort();
    objects.dedup();
    Ok((placed, objects))
}

/// `objects/<first two hex digits>/<sha1>`.
fn object_path(assets: &Path, hash: &str) -> PathBuf {
    assets.join("objects").join(&hash[..2]).join(hash)
}

/// Imports `install` into `cache`. Safe to re-run: files already present
/// with the right hash are kept.
pub fn import(
    install: &Install,
    version: &GameVersion,
    cache: &Path,
) -> Result<(Imported, ImportStats), ImportError> {
    let imported = Imported::at(version, cache);
    let mut stats = ImportStats::default();
    let mut count = |p: Placed| match p {
        Placed::Linked => stats.linked += 1,
        Placed::Copied => stats.copied += 1,
        Placed::Present => stats.present += 1,
    };
    let _ = std::fs::remove_file(imported.dir.join(MARKER));

    count(place(
        &install.client_jar,
        &imported.client_jar,
        version.client_jar.sha1,
    )?);
    for lib in version.libraries {
        let rel = lib.rel_path();
        count(place(
            &install.libraries.join(&rel),
            &imported.libraries.join(&rel),
            lib.sha1,
        )?);
    }
    let (placed, objects) = place_index(
        &install.asset_index(version),
        &imported.asset_index(),
        &version.asset_index,
    )?;
    count(placed);

    let next = AtomicUsize::new(0);
    let placed: Mutex<[usize; 3]> = Mutex::new([0; 3]);
    let missing: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let failure: Mutex<Option<ImportError>> = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(16));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let mut local = [0usize; 3];
                while let Some((hash, _)) = objects.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let src = object_path(&install.assets, hash);
                    if !src.is_file() {
                        missing.lock().unwrap().push(hash.clone());
                        continue;
                    }
                    match place(&src, &object_path(&imported.assets, hash), hash) {
                        Ok(p) => local[p as usize] += 1,
                        Err(e) => {
                            failure.lock().unwrap().get_or_insert(e);
                            next.store(usize::MAX / 2, Ordering::Relaxed);
                        }
                    }
                }
                let mut total = placed.lock().unwrap();
                for (t, l) in total.iter_mut().zip(local) {
                    *t += l;
                }
            });
        }
    });
    if let Some(e) = failure.into_inner().unwrap() {
        return Err(e);
    }
    let mut missing = missing.into_inner().unwrap();
    if !missing.is_empty() {
        missing.sort();
        return Err(ImportError::MissingObjects {
            missing: missing.len(),
            total: objects.len(),
            examples: missing.into_iter().take(3).collect(),
        });
    }
    let [linked, copied, present] = placed.into_inner().unwrap();
    stats.linked += linked;
    stats.copied += copied;
    stats.present += present;
    stats.objects = objects.len();

    let marker = imported.dir.join(MARKER);
    std::fs::write(&marker, import_key(version)).map_err(io_err(&marker))?;
    Ok((imported, stats))
}
