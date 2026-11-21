//! What a supported game version is, as far as preparation is concerned.

/// A file identified by its SHA-1 and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    pub sha1: &'static str,
    pub size: u64,
}

/// The asset index a version uses (`assets/indexes/<id>.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetIndex {
    pub id: &'static str,
    pub sha1: &'static str,
    pub size: u64,
}

/// The Java runtime Mojang ships for the version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JavaRequirement {
    /// Mojang's runtime component name, e.g. `java-runtime-epsilon`.
    pub component: &'static str,
    /// Lowest acceptable feature release (`java.specification.version`).
    pub major: u32,
}

/// A class-path library, located by its Maven path under a launcher's
/// `libraries` folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Library {
    pub name: &'static str,
    /// Relative path with `/` separators.
    pub path: &'static str,
    pub sha1: &'static str,
    pub size: u64,
}

/// Everything Ruthenium pins for one game version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameVersion {
    pub id: &'static str,
    pub client_jar: Pinned,
    pub asset_index: AssetIndex,
    pub java: JavaRequirement,
    pub libraries: &'static [Library],
}

impl Library {
    /// `path` as a relative `PathBuf` for the host OS.
    pub fn rel_path(&self) -> std::path::PathBuf {
        self.path.split('/').collect()
    }
}
