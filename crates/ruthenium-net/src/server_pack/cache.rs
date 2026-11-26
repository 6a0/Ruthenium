//! The pack cache: `<cache>/<pack id>/<sha1 hex>`, as vanilla's `DownloadQueue` keeps it.
//!
//! Every name in it comes from a validated pack id (a UUID we format) or a SHA-1 we
//! format as hex, never from the URL or the server's headers, so nothing outside the cache
//! can be named. A cache directory that is a symlink, or isn't a directory, is refused;
//! cleaning walks without following symlinks and deletes only regular files it found
//! inside the cache. Files are created owner-only where the platform has permissions.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha1::{Digest, Sha1};

/// `UUID.toString`: lower-case hex in 8-4-4-4-12.
pub(crate) fn uuid_string(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// `HashCode.toString`: lower-case hex.
pub(crate) fn hex(hash: &[u8; 20]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// The directory must be a real directory (not a symlink to one); it is made if missing.
pub(crate) fn ensure_dir(dir: &Path, recursive: bool) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => Err(io::Error::other(format!(
            "{} is a symbolic link",
            dir.display()
        ))),
        Ok(m) if !m.is_dir() => Err(io::Error::other(format!(
            "{} is not a directory",
            dir.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut b = fs::DirBuilder::new();
            b.recursive(recursive);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b.create(dir)
        }
        Err(e) => Err(e),
    }
}

/// SHA-1 of a cached file, or None when it isn't a regular file or is larger than
/// `max_size` (then it can't be a pack we'd have kept).
pub(crate) fn hash_file(path: &Path, max_size: u64) -> io::Result<Option<[u8; 20]>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && m.len() <= max_size => {}
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut file = File::open(path)?.take(max_size + 1);
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_size {
            return Ok(None);
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(hasher.finalize().into()))
}

/// `HttpUtil.checkExistingFile`: whether the file is there with the expected hash.
pub(crate) fn matches(path: &Path, expected: &[u8; 20], max_size: u64) -> io::Result<bool> {
    Ok(hash_file(path, max_size)?.is_some_and(|h| &h == expected))
}

/// `Files.deleteIfExists`, for what we may find at a cache file's name: a file or a
/// symlink (removed itself, never followed); a directory there is an error.
pub(crate) fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => Err(io::Error::other(format!(
            "{} is a directory",
            path.display()
        ))),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `HttpUtil.updateModificationTime`: a reused pack counts as new for cleaning.
pub(crate) fn touch(path: &Path) {
    let r = OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
    if let Err(e) = r {
        eprintln!(
            "[packs] couldn't update the time of {}: {e}",
            path.display()
        );
    }
}

/// A download in progress: deleted unless [`Temp::persist`] moves it into place.
#[derive(Debug)]
pub(crate) struct Temp {
    path: PathBuf,
    file: Option<File>,
}

impl Temp {
    /// A new file `download-<random>.tmp` in `dir`, owner-only.
    pub fn new(dir: &Path) -> io::Result<Temp> {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = dir.join(format!("download-{name}.tmp"));
        let mut o = OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let file = o.open(&path)?;
        Ok(Temp {
            path,
            file: Some(file),
        })
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match &mut self.file {
            Some(f) => f.write_all(bytes),
            None => Err(io::Error::other("closed")),
        }
    }

    /// Moves the finished file to `target` (replacing what is there).
    pub fn persist(mut self, target: &Path) -> io::Result<()> {
        if let Some(f) = self.file.take() {
            f.sync_all()?;
        }
        fs::rename(&self.path, target)?;
        // Moved: nothing left to delete.
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        self.file.take();
        if !self.path.as_os_str().is_empty()
            && let Err(e) = fs::remove_file(&self.path)
            && e.kind() != io::ErrorKind::NotFound
        {
            eprintln!("[packs] couldn't remove {}: {e}", self.path.display());
        }
    }
}

/// How deep and how many entries cleaning looks at: the cache we write is two levels deep
/// and holds a few dozen files; anything far beyond that isn't ours to walk.
const VACUUM_MAX_DEPTH: usize = 8;
const VACUUM_MAX_ENTRIES: usize = 100_000;

/// `DownloadCacheCleaner.vacuumCacheDir`: keeps the newest `max_files` files below the
/// cache's top level, removing old files from directories with several first, then the
/// oldest; then removes directories that emptied. Errors are logged, as vanilla does.
pub(crate) fn vacuum(cache_dir: &Path, max_files: usize) {
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    let mut stack = vec![(cache_dir.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                if depth == 0 && e.kind() == io::ErrorKind::NotFound {
                    return;
                }
                eprintln!("[packs] couldn't list {}: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > VACUUM_MAX_ENTRIES {
                break;
            }
            // file_type() doesn't follow symlinks.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth + 1 < VACUUM_MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file() && depth > 0 {
                let time = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((path, time));
            }
        }
    }
    let Some(to_remove) = files.len().checked_sub(max_files).filter(|&n| n > 0) else {
        return;
    };
    // Newest first; each file's priority is how many newer files share its directory.
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut counts: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();
    let mut ordered: Vec<(PathBuf, usize)> = files
        .into_iter()
        .map(|(path, _)| {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let count = counts.entry(parent).or_insert(0);
            let priority = *count;
            *count += 1;
            (path, priority)
        })
        .collect();
    // Oldest first, then the highest priority first (a stable sort, as Java's).
    ordered.reverse();
    ordered.sort_by_key(|o| std::cmp::Reverse(o.1));
    let mut empty_candidates: Vec<PathBuf> = Vec::new();
    for (path, priority) in ordered.into_iter().take(to_remove) {
        match fs::remove_file(&path) {
            Ok(()) => {
                if priority == 0
                    && let Some(parent) = path.parent()
                    && parent != cache_dir
                    && !empty_candidates.iter().any(|p| p == parent)
                {
                    empty_candidates.push(parent.to_path_buf());
                }
            }
            Err(e) => eprintln!("[packs] couldn't delete {}: {e}", path.display()),
        }
    }
    for dir in empty_candidates {
        // Fails, as it should, when the directory isn't empty.
        let _ = fs::remove_dir(&dir);
    }
}
