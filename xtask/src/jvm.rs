//! Shared helpers for tasks that run javac/java against the game.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `$JAVA_HOME/bin/<name>`, or `<name>` on PATH.
pub fn tool(name: &str) -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join("bin").join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

pub fn join(parts: &[PathBuf]) -> OsString {
    std::env::join_paths(parts).expect("class path entries contain no separator")
}

pub fn push(cp: &mut OsString, entry: &Path) {
    if !cp.is_empty() {
        cp.push(if cfg!(windows) { ";" } else { ":" });
    }
    cp.push(entry);
}

/// The client jar and its class-path libraries. With `deps` (a folder made by
/// `cargo xtask fetch-compile-deps`), the pinned libraries in it; otherwise the
/// local reference copy, then `cache/compile-deps` if it exists.
pub fn game_classpath(root: &Path, deps: Option<&Path>) -> Result<OsString, String> {
    let fetched = root.join("cache/compile-deps");
    let deps = match deps {
        Some(d) => Some(d.to_owned()),
        None if root.join("reference/26.3/jars/client.jar").is_file() => None,
        None if fetched.join("client.jar").is_file() => Some(fetched),
        None => {
            return Err(
                "no client jar: run `cargo xtask fetch-compile-deps` or pass --deps/--classpath"
                    .into(),
            );
        }
    };
    Ok(match deps {
        Some(d) => {
            let mut parts = vec![d.join("client.jar")];
            parts.extend(
                ruthenium_prepare::supported_version()
                    .libraries
                    .iter()
                    .map(|l| d.join("libraries").join(l.rel_path())),
            );
            join(&parts)
        }
        // Java expands a trailing "*" class-path entry to every jar in that folder.
        None => join(&[
            root.join("reference/26.3/jars/client.jar"),
            root.join("reference/26.3/libs/*"),
        ]),
    })
}
