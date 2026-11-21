//! Finding a verified game install in the player's launchers.
//!
//! Every launcher stores the client jar somewhere different, but they all keep
//! class-path libraries in a Maven-style `libraries` folder and assets in
//! Mojang's `assets/indexes` + `assets/objects` layout. Candidates are accepted
//! only if the client jar and asset index hash to the pinned values and every
//! class-path library is present at its pinned size; [`crate::import`] then
//! checks the libraries and asset objects by hash as it copies them.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::hash::sha1_file;
use crate::version::GameVersion;

/// Launchers Ruthenium knows how to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Launcher {
    Official,
    Prism,
    MultiMc,
    Modrinth,
    AtLauncher,
    GdLauncher,
}

impl Launcher {
    pub const ALL: [Launcher; 6] = [
        Launcher::Official,
        Launcher::Prism,
        Launcher::MultiMc,
        Launcher::Modrinth,
        Launcher::AtLauncher,
        Launcher::GdLauncher,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Launcher::Official => "Minecraft Launcher",
            Launcher::Prism => "Prism Launcher",
            Launcher::MultiMc => "MultiMC",
            Launcher::Modrinth => "Modrinth App",
            Launcher::AtLauncher => "ATLauncher",
            Launcher::GdLauncher => "GDLauncher",
        }
    }
}

impl fmt::Display for Launcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The environment variables discovery reads, captured once so tests can
/// supply their own.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Env {
        // Empty values count as unset, as the XDG spec says.
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Env {
            home: var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
            xdg_data_home: var("XDG_DATA_HOME"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            appdata: var("APPDATA"),
            local_appdata: var("LOCALAPPDATA"),
        }
    }

    fn data_home(&self) -> Option<PathBuf> {
        self.xdg_data_home
            .clone()
            .or_else(|| Some(self.home.as_ref()?.join(".local/share")))
    }

    fn config_home(&self) -> Option<PathBuf> {
        self.xdg_config_home
            .clone()
            .or_else(|| Some(self.home.as_ref()?.join(".config")))
    }
}

/// Launcher folders searched by default, in order.
pub fn default_roots(env: &Env) -> Vec<(Launcher, PathBuf)> {
    use Launcher::*;
    let mut roots = Vec::new();
    let mut add = |l: Launcher, p: Option<PathBuf>| {
        if let Some(p) = p {
            roots.push((l, p));
        }
    };
    if cfg!(windows) {
        let appdata = |rel: &str| Some(env.appdata.as_ref()?.join(rel));
        add(Official, appdata(".minecraft"));
        add(Prism, appdata("PrismLauncher"));
        add(Modrinth, appdata("ModrinthApp"));
        add(AtLauncher, appdata("ATLauncher"));
        add(GdLauncher, appdata("gdlauncher_carbon/data"));
        add(GdLauncher, appdata("gdlauncher_next/datastore"));
        // MultiMC on Windows is portable only; use --from.
    } else if cfg!(target_os = "macos") {
        let support = |rel: &str| {
            Some(
                env.home
                    .as_ref()?
                    .join("Library/Application Support")
                    .join(rel),
            )
        };
        add(Official, support("minecraft"));
        add(Prism, support("PrismLauncher"));
        add(Modrinth, support("ModrinthApp"));
        add(Modrinth, support("com.modrinth.theseus"));
        add(AtLauncher, support("ATLauncher"));
        add(GdLauncher, support("gdlauncher_carbon/data"));
        add(GdLauncher, support("gdlauncher_next/datastore"));
        // MultiMC on macOS is portable only; use --from.
    } else {
        let home = |rel: &str| Some(env.home.as_ref()?.join(rel));
        let data = |rel: &str| Some(env.data_home()?.join(rel));
        let flatpak = |app: &str, rel: &str| home(&format!(".var/app/{app}/{rel}"));
        add(Official, home(".minecraft"));
        add(Official, flatpak("com.mojang.Minecraft", ".minecraft"));
        add(Prism, data("PrismLauncher"));
        add(
            Prism,
            flatpak("org.prismlauncher.PrismLauncher", "data/PrismLauncher"),
        );
        add(MultiMc, data("multimc"));
        add(Modrinth, data("ModrinthApp"));
        add(
            Modrinth,
            flatpak("com.modrinth.ModrinthApp", "data/ModrinthApp"),
        );
        add(Modrinth, data("com.modrinth.theseus"));
        add(AtLauncher, data("atlauncher"));
        add(
            AtLauncher,
            flatpak("com.atlauncher.ATLauncher", "data/atlauncher"),
        );
        add(GdLauncher, data("gdlauncher_carbon/data"));
        add(
            GdLauncher,
            env.config_home()
                .map(|c| c.join("gdlauncher_next/datastore")),
        );
    }
    roots
}

/// Where one launcher keeps things, relative to its root.
#[derive(Debug)]
struct Layout {
    client_jars: Vec<PathBuf>,
    libraries: PathBuf,
    assets: PathBuf,
    java_roots: Vec<PathBuf>,
}

fn layout(launcher: Launcher, root: &Path, version: &GameVersion) -> Layout {
    let id = version.id;
    match launcher {
        Launcher::Official => Layout {
            client_jars: version_folder_jars(&root.join("versions"), id),
            libraries: root.join("libraries"),
            assets: root.join("assets"),
            java_roots: vec![root.join("runtime")],
        },
        Launcher::Prism | Launcher::MultiMc => Layout {
            client_jars: vec![root.join(format!(
                "libraries/com/mojang/minecraft/{id}/minecraft-{id}-client.jar"
            ))],
            libraries: root.join("libraries"),
            assets: root.join("assets"),
            java_roots: vec![root.join("java")],
        },
        Launcher::Modrinth => Layout {
            client_jars: version_folder_jars(&root.join("meta/versions"), id),
            libraries: root.join("meta/libraries"),
            assets: root.join("meta/assets"),
            java_roots: vec![root.join("meta/java_versions")],
        },
        Launcher::AtLauncher | Launcher::GdLauncher => Layout {
            client_jars: vec![root.join(format!(
                "libraries/net/minecraft/client/{id}/client-{id}.jar"
            ))],
            libraries: root.join("libraries"),
            assets: root.join("assets"),
            java_roots: vec![root.join("runtimes"), root.join("java")],
        },
    }
}

/// `<versions>/<id>/<id>.jar`, then any `<versions>/<id>-<suffix>/<same>.jar`
/// (loader profiles that carry their own copy of the vanilla jar).
fn version_folder_jars(versions: &Path, id: &str) -> Vec<PathBuf> {
    let mut jars = vec![versions.join(id).join(format!("{id}.jar"))];
    let prefix = format!("{id}-");
    if let Ok(entries) = std::fs::read_dir(versions) {
        let mut extra: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.starts_with(&prefix)
                    .then(|| e.path().join(format!("{name}.jar")))
            })
            .collect();
        extra.sort();
        jars.extend(extra);
    }
    jars
}

/// A launcher install whose files match the pinned version.
#[derive(Debug, Clone)]
pub struct Install {
    pub launcher: Launcher,
    pub root: PathBuf,
    pub client_jar: PathBuf,
    /// Folder holding Maven-style library paths.
    pub libraries: PathBuf,
    /// Folder holding `indexes/` and `objects/`.
    pub assets: PathBuf,
    /// Folders the launcher keeps Java runtimes in (may not exist).
    pub java_roots: Vec<PathBuf>,
}

impl Install {
    pub fn asset_index(&self, version: &GameVersion) -> PathBuf {
        self.assets
            .join("indexes")
            .join(format!("{}.json", version.asset_index.id))
    }
}

/// Why a launcher folder was not used.
#[derive(Debug)]
pub enum Rejection {
    /// The folder does not exist or is not this launcher's layout.
    NotFound,
    /// The launcher is there, but the version was never installed.
    NoClientJar,
    /// A client jar exists but is not the pinned build (modified or a
    /// different version).
    ClientJarMismatch {
        path: PathBuf,
        sha1: String,
    },
    /// The asset index is missing: the version was installed but not played.
    NoAssetIndex(PathBuf),
    AssetIndexMismatch {
        path: PathBuf,
        sha1: String,
    },
    /// Class-path libraries missing or of the wrong size.
    MissingLibraries(Vec<&'static str>),
    Io {
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::NotFound => f.write_str("not found"),
            Rejection::NoClientJar => f.write_str("version not installed"),
            Rejection::ClientJarMismatch { path, sha1 } => write!(
                f,
                "{} is not the official client jar (SHA-1 {sha1})",
                path.display()
            ),
            Rejection::NoAssetIndex(p) => write!(
                f,
                "{} is missing; play the version once so the launcher downloads its assets",
                p.display()
            ),
            Rejection::AssetIndexMismatch { path, sha1 } => {
                write!(f, "{} has the wrong SHA-1 ({sha1})", path.display())
            }
            Rejection::MissingLibraries(names) => {
                write!(f, "{} libraries missing: {}", names.len(), names.join(", "))
            }
            Rejection::Io { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

/// One folder that was searched and what was found there.
#[derive(Debug)]
pub struct Candidate {
    pub launcher: Launcher,
    pub root: PathBuf,
    pub outcome: Result<Install, Rejection>,
}

/// Checks one launcher folder.
pub fn check(launcher: Launcher, root: &Path, version: &GameVersion) -> Result<Install, Rejection> {
    let layout = layout(launcher, root, version);
    if !layout.libraries.is_dir() && !layout.client_jars.iter().any(|j| j.is_file()) {
        return Err(Rejection::NotFound);
    }

    let io_err = |path: &Path| {
        let path = path.to_owned();
        move |error| Rejection::Io { path, error }
    };

    // The first jar with the pinned hash wins; otherwise report the first
    // mismatch, which is the most likely to be the one the player meant.
    let mut mismatch = None;
    let mut client_jar = None;
    for jar in &layout.client_jars {
        let Ok(meta) = std::fs::metadata(jar) else {
            continue;
        };
        let sha1 = if meta.len() == version.client_jar.size {
            sha1_file(jar).map_err(io_err(jar))?
        } else {
            format!("<size {}>", meta.len())
        };
        if sha1 == version.client_jar.sha1 {
            client_jar = Some(jar.clone());
            break;
        }
        mismatch.get_or_insert(Rejection::ClientJarMismatch {
            path: jar.clone(),
            sha1,
        });
    }
    let Some(client_jar) = client_jar else {
        return Err(mismatch.unwrap_or(Rejection::NoClientJar));
    };

    let index = layout
        .assets
        .join("indexes")
        .join(format!("{}.json", version.asset_index.id));
    if !index.is_file() {
        return Err(Rejection::NoAssetIndex(index));
    }
    let sha1 = sha1_file(&index).map_err(io_err(&index))?;
    if sha1 != version.asset_index.sha1 {
        return Err(Rejection::AssetIndexMismatch { path: index, sha1 });
    }

    let missing: Vec<&'static str> = version
        .libraries
        .iter()
        .filter(|lib| {
            std::fs::metadata(layout.libraries.join(lib.rel_path()))
                .map_or(true, |m| m.len() != lib.size)
        })
        .map(|lib| lib.name)
        .collect();
    if !missing.is_empty() {
        return Err(Rejection::MissingLibraries(missing));
    }

    Ok(Install {
        launcher,
        root: root.to_owned(),
        client_jar,
        libraries: layout.libraries,
        assets: layout.assets,
        java_roots: layout.java_roots,
    })
}

/// Checks every folder in `roots`, in order.
pub fn search(roots: &[(Launcher, PathBuf)], version: &GameVersion) -> Vec<Candidate> {
    roots
        .iter()
        .map(|(launcher, root)| Candidate {
            launcher: *launcher,
            root: root.clone(),
            outcome: check(*launcher, root, version),
        })
        .collect()
}

/// Checks a folder the player picked. It may be a launcher root or anything
/// inside one (an instance folder, say), so the folder and its ancestors are
/// tried against every launcher layout. Folders matching no layout at all are
/// left out of the result.
pub fn search_folder(folder: &Path, version: &GameVersion) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for dir in folder.ancestors().take(4) {
        for launcher in Launcher::ALL {
            // Prism and MultiMC share a layout; check the folder once.
            if launcher == Launcher::MultiMc
                && out
                    .iter()
                    .any(|c| c.launcher == Launcher::Prism && c.root == dir)
            {
                continue;
            }
            let outcome = check(launcher, dir, version);
            if !matches!(outcome, Err(Rejection::NotFound)) {
                out.push(Candidate {
                    launcher,
                    root: dir.to_owned(),
                    outcome,
                });
            }
        }
        if out.iter().any(|c| c.outcome.is_ok()) {
            // Other layouts that merely share folder names are noise now.
            out.retain(|c| c.outcome.is_ok());
            break;
        }
    }
    out
}

/// The first verified install among the candidates.
pub fn first_install(candidates: &[Candidate]) -> Option<&Install> {
    candidates.iter().find_map(|c| c.outcome.as_ref().ok())
}
