//! Game-file import and preparation.
//!
//! Ruthenium ships no Mojang files and downloads none. This crate finds a
//! 26.3 install the player already has (official launcher, Prism, MultiMC,
//! Modrinth App, ATLauncher, GDLauncher), verifies the client jar against a
//! pinned SHA-1, and builds per-version caches from it on the player's machine.

#![forbid(unsafe_code)]

/// SHA-1 of the 26.3 client jar, as published in Mojang's version manifest.
pub const CLIENT_JAR_SHA1_26_3: &str = "e877b6a07acd633fb3bb475002175cec036e7b87";
