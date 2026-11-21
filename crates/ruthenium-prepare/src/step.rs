//! Preparation steps keyed by the hash of their inputs.
//!
//! A step writes into a fresh temporary folder; only when it succeeds is the
//! folder moved into place with a key file recording its inputs. A later run
//! with the same key skips the step, so normal launches never start a JVM.

use std::io;
use std::path::{Path, PathBuf};

use crate::hash::sha256_hex;

const KEY_FILE: &str = ".ruthenium-key";

/// Builds a step key from its inputs (step name and version first, then
/// hashes of input files).
pub fn key(parts: &[&str]) -> String {
    sha256_hex(parts.join("\n").as_bytes())
}

/// Whether `out` already holds the result for `key`.
pub fn is_current(out: &Path, key: &str) -> bool {
    std::fs::read_to_string(out.join(KEY_FILE)).is_ok_and(|k| k.trim() == key)
}

/// Runs `build` into a temporary sibling of `out` unless `out` is current.
/// Returns whether the step ran.
pub fn run<E: From<io::Error>>(
    out: &Path,
    key: &str,
    build: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<bool, E> {
    if is_current(out, key) {
        return Ok(false);
    }
    let tmp = sibling(out, "tmp");
    remove_dir_if_exists(&tmp)?;
    std::fs::create_dir_all(&tmp)?;
    build(&tmp)?;
    std::fs::write(tmp.join(KEY_FILE), key)?;
    remove_dir_if_exists(out)?;
    std::fs::rename(&tmp, out)?;
    Ok(true)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

pub(crate) fn remove_dir_if_exists(dir: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
