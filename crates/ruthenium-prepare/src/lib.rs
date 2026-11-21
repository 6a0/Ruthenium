//! Game-file import and preparation.
//!
//! Ruthenium ships no Mojang files and downloads none. This crate finds a
//! 26.3 install the player already has (official launcher, Prism, MultiMC,
//! Modrinth App, ATLauncher, GDLauncher), verifies it against pinned SHA-1s,
//! imports it into Ruthenium's cache, and builds per-version data from it on
//! the player's machine.
//!
//! 1. [`discover`]: search launcher folders; accept only pinned files.
//! 2. [`import`]: hard-link or copy the jar, libraries and assets into the
//!    cache, checking every hash.
//! 3. [`java`] + [`datagen`] + [`exporters`]: run Mojang's data generator and
//!    Ruthenium's own exporters against the imported jar. Steps are keyed by
//!    their inputs ([`step`]), so they run once.

#![forbid(unsafe_code)]

pub mod datagen;
pub mod discover;
pub mod exporters;
mod hash;
pub mod import;
pub mod java;
mod pins;
pub mod step;
pub mod version;

use std::path::PathBuf;

pub use version::GameVersion;

/// The game version this build of Ruthenium plays.
pub fn supported_version() -> &'static GameVersion {
    &pins::V26_3
}

/// Ruthenium's per-user data folder: `$XDG_DATA_HOME/ruthenium` (default
/// `~/.local/share/ruthenium`), `~/Library/Application Support/Ruthenium`
/// on macOS, or `%LOCALAPPDATA%\Ruthenium`.
pub fn default_data_dir(env: &discover::Env) -> Option<PathBuf> {
    if cfg!(windows) {
        Some(env.local_appdata.as_ref()?.join("Ruthenium"))
    } else if cfg!(target_os = "macos") {
        Some(
            env.home
                .as_ref()?
                .join("Library/Application Support/Ruthenium"),
        )
    } else {
        Some(
            env.xdg_data_home
                .clone()
                .or_else(|| Some(env.home.as_ref()?.join(".local/share")))?
                .join("ruthenium"),
        )
    }
}
