//! A development launcher for the vanilla client, used by `cargo xtask bench`
//! and `cargo xtask trace`.
//!
//! It runs the 26.3 install the player already has (found and verified by
//! ruthenium-prepare), with Mojang's Java runtime from that launcher, the
//! launcher's own JVM arguments, and Fabric Loader for Ruthenium's development
//! mods. It runs in offline developer mode (`--offlineDeveloperMode`), so it
//! is limited to singleplayer and never touches the player's account.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use ruthenium_prepare::discover::{self, Env};
use ruthenium_prepare::{java, supported_version};

use crate::fabric;
use crate::fetch_compile_deps::sha1_of;
use crate::launch_pins::{JVM_ARGS, PLATFORM_LIBRARIES, PlatformLibrary};

pub const USERNAME: &str = "Ruthenium";

#[derive(Debug)]
pub struct Game {
    pub java: PathBuf,
    pub java_description: String,
    pub launcher: String,
    /// The client jar, its libraries and this platform's natives.
    pub class_path: Vec<PathBuf>,
    pub assets: PathBuf,
}

/// What to run: Fabric Loader with `mods`, in `game_dir`.
#[derive(Debug, Default)]
pub struct Launch {
    pub game_dir: PathBuf,
    pub mods: Vec<PathBuf>,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    /// Environment changes: `Some` sets a variable, `None` removes it.
    pub env: Vec<(String, Option<String>)>,
    pub width: u32,
    pub height: u32,
}

fn platform() -> (&'static str, &'static str) {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    (os, arch)
}

/// Finds the player's verified install, a Java runtime and the natives.
pub fn locate(java_override: Option<&Path>) -> Result<Game, String> {
    let version = supported_version();
    let candidates = discover::search(&discover::default_roots(&Env::from_process()), version);
    let install = discover::first_install(&candidates).ok_or_else(|| {
        format!(
            "no verified {} install found; play it once in a launcher (see `cargo run -p ruthenium -- locate`)",
            version.id
        )
    })?;
    let java =
        java::find_java(Some(install), &version.java, java_override).map_err(|e| e.to_string())?;

    let mut class_path = vec![install.client_jar.clone()];
    class_path.extend(
        version
            .libraries
            .iter()
            .map(|l| install.libraries.join(l.rel_path())),
    );
    let (os, arch) = platform();
    for lib in PLATFORM_LIBRARIES
        .iter()
        .filter(|l| l.os == os && (l.arch.is_empty() || l.arch == arch))
    {
        match find_platform_library(&install.libraries, lib) {
            Some(p) => class_path.push(p),
            None => eprintln!(
                "warning: {} (SHA-1 {}) not found in {}",
                lib.name,
                lib.sha1,
                install.libraries.display()
            ),
        }
    }
    Ok(Game {
        java: java.path,
        java_description: java.description,
        launcher: format!("{} ({})", install.launcher.name(), install.root.display()),
        class_path,
        assets: install.assets.clone(),
    })
}

/// Natives are under Mojang's path in the official launcher, and under an
/// artifact named `<artifact>-<classifier>` in Prism and MultiMC. Either way
/// the file must have the pinned SHA-1.
fn find_platform_library(libraries: &Path, lib: &PlatformLibrary) -> Option<PathBuf> {
    let mut candidates = vec![libraries.join(lib.path)];
    let parts: Vec<&str> = lib.name.split(':').collect();
    if let [group, artifact, version, classifier] = parts[..] {
        let name = format!("{artifact}-{classifier}");
        let mut p: PathBuf = group.split('.').collect();
        p.push(&name);
        p.push(version);
        p.push(format!("{name}-{version}.jar"));
        candidates.push(libraries.join(p));
    }
    candidates.into_iter().find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.len() == lib.size)
            && sha1_of(p).as_deref() == Some(lib.sha1)
    })
}

impl Game {
    /// Starts the client. Its output goes to `<game_dir>/launch.log`.
    pub fn spawn(&self, root: &Path, launch: &Launch) -> Result<Child, String> {
        let version = supported_version();
        let game_dir = std::path::absolute(&launch.game_dir).map_err(|e| e.to_string())?;
        let mods_dir = game_dir.join("mods");
        let _ = std::fs::remove_dir_all(&mods_dir);
        std::fs::create_dir_all(&mods_dir).map_err(|e| format!("{}: {e}", mods_dir.display()))?;
        for m in &launch.mods {
            let name = m.file_name().ok_or("mod path has no file name")?;
            std::fs::copy(m, mods_dir.join(name)).map_err(|e| format!("{}: {e}", m.display()))?;
        }

        let mut class_path = fabric::fetch_all(root, fabric::LOADER)?;
        class_path.extend(self.class_path.iter().cloned());
        let class_path = std::env::join_paths(&class_path).map_err(|e| e.to_string())?;

        let natives = game_dir.join("natives");
        let (os, _) = platform();
        let mut cmd = Command::new(&self.java);
        for (arg_os, arg) in JVM_ARGS {
            if *arg_os == os {
                cmd.arg(
                    arg.replace("${natives_directory}", &natives.to_string_lossy())
                        .replace("${launcher_name}", "ruthenium-xtask")
                        .replace("${launcher_version}", "0"),
                );
            }
        }
        cmd.args(&launch.jvm_args)
            .arg("-cp")
            .arg(class_path)
            .arg(fabric::KNOT_CLIENT)
            .args(["--username", USERNAME, "--version", version.id])
            .arg("--gameDir")
            .arg(&game_dir)
            .arg("--assetsDir")
            .arg(&self.assets)
            .args(["--assetIndex", version.asset_index.id])
            .args(["--accessToken", "0", "--versionType", "release"])
            .arg("--offlineDeveloperMode")
            .args(["--width", &launch.width.to_string()])
            .args(["--height", &launch.height.to_string()])
            .args(&launch.game_args);

        for (key, value) in &launch.env {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            };
        }

        let log_path = game_dir.join("launch.log");
        let log =
            std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
        cmd.current_dir(&game_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        cmd.spawn()
            .map_err(|e| format!("running {}: {e}", self.java.display()))
    }
}

/// Writes `options.txt` with only these keys; the game fills in its defaults
/// for everything else.
pub fn write_options(game_dir: &Path, options: &[(&str, String)]) -> Result<(), String> {
    std::fs::create_dir_all(game_dir).map_err(|e| e.to_string())?;
    let text: String = options.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let path = game_dir.join("options.txt");
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}
