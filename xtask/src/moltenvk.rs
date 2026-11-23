//! `cargo xtask moltenvk` and `cargo xtask bundle`: the macOS application bundle with its
//! own Vulkan implementation.
//!
//! On a Mac, Vulkan is MoltenVK over Metal, and a Mac has none unless a Vulkan SDK is
//! installed. Ruthenium ships one MoltenVK release, pinned by the SHA-256 of Khronos'
//! release archive, as `Contents/Frameworks/libMoltenVK.dylib`; the client hands that path
//! to SDL's Vulkan loader when it runs from a bundle (`ruthenium-platform`). MoltenVK is
//! Apache-2.0; its licence goes into the bundle beside Ruthenium's.
//!
//! `moltenvk` downloads and unpacks the pinned release into `cache/moltenvk/` (works
//! anywhere). `bundle` builds the client and assembles and signs
//! `target/bundle/Ruthenium.app` (on macOS; with `--binary`, any executable is wrapped,
//! which is how the layout is tested elsewhere).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha2::{Digest, Sha256};

const VERSION: &str = "1.4.2";
const URL: &str =
    "https://github.com/KhronosGroup/MoltenVK/releases/download/v1.4.2/MoltenVK-macos.tar";
const SHA256: &str = "f95765a6229cb7b915990a2890ce12ebe36a730b021545d3d52ae69ce4c4024e";
/// The universal (arm64 and x86-64) library and the licence inside the archive.
const DYLIB_MEMBER: &str = "MoltenVK/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib";
const LICENSE_MEMBER: &str = "MoltenVK/LICENSE";

/// The bundle's identifier: the one the client gives SDL (`ruthenium-platform`).
const BUNDLE_ID: &str = "dev.ruthenium.Ruthenium";
/// The oldest macOS the bundle claims to run on: the first with Apple Silicon.
const MINIMUM_MACOS: &str = "11.0";

pub fn run_fetch(root: &Path, args: &[String]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("moltenvk: takes no arguments");
        return ExitCode::FAILURE;
    }
    match fetch(root) {
        Ok(dir) => {
            println!("{}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("moltenvk: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_bundle(root: &Path, args: &[String]) -> ExitCode {
    match bundle(root, args) {
        Ok(app) => {
            println!("{}", app.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bundle: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The folder holding the pinned `libMoltenVK.dylib` and its `LICENSE`, downloading and
/// checking the release archive first if they are not there yet.
fn fetch(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("cache/moltenvk").join(VERSION);
    let (dylib, license) = (dir.join("libMoltenVK.dylib"), dir.join("LICENSE"));
    if dylib.is_file() && license.is_file() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let archive = dir.join("MoltenVK-macos.tar");
    println!("moltenvk: downloading MoltenVK {VERSION} (about 60 MB, once)");
    run(Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "3",
            "--output",
        ])
        .arg(&archive)
        .arg(URL))?;
    let bytes = std::fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if sha != SHA256 {
        let _ = std::fs::remove_file(&archive);
        return Err(format!("{URL}: SHA-256 {sha}, expected {SHA256}"));
    }
    for (member, depth) in [(DYLIB_MEMBER, 5), (LICENSE_MEMBER, 1)] {
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&dir)
            .arg(format!("--strip-components={depth}"))
            .arg(member))?;
    }
    let _ = std::fs::remove_file(&archive);
    if !(dylib.is_file() && license.is_file()) {
        return Err(format!(
            "the MoltenVK archive has no {DYLIB_MEMBER} or {LICENSE_MEMBER}"
        ));
    }
    Ok(dir)
}

fn bundle(root: &Path, args: &[String]) -> Result<PathBuf, String> {
    let mut binary: Option<PathBuf> = None;
    let mut out = root.join("target/bundle");
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--binary" => binary = Some(it.next().ok_or("--binary needs a path")?.into()),
            "--out" => out = it.next().ok_or("--out needs a folder")?.into(),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let binary = match binary {
        Some(b) => b,
        None if cfg!(target_os = "macos") => {
            run(Command::new(env!("CARGO")).current_dir(root).args([
                "build",
                "--release",
                "-p",
                "ruthenium",
            ]))?;
            root.join("target/release/ruthenium")
        }
        None => {
            return Err(
                "the client is built for the bundle on macOS only; elsewhere pass --binary <executable>"
                    .into(),
            );
        }
    };
    let moltenvk = fetch(root)?;

    let app = out.join("Ruthenium.app");
    if app.exists() {
        std::fs::remove_dir_all(&app).map_err(|e| format!("{}: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let copies = [
        (binary, contents.join("MacOS/ruthenium")),
        (
            moltenvk.join("libMoltenVK.dylib"),
            contents.join("Frameworks/libMoltenVK.dylib"),
        ),
        (
            moltenvk.join("LICENSE"),
            contents.join("Resources/licenses/MoltenVK-LICENSE.txt"),
        ),
        (
            root.join("LICENSE"),
            contents.join("Resources/licenses/Ruthenium-LICENSE.txt"),
        ),
    ];
    for (from, to) in &copies {
        let dir = to.parent().expect("inside the bundle");
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::copy(from, to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    // The icon needs macOS's own tools; a bundle without one still runs.
    let icon = cfg!(target_os = "macos")
        && make_icon(
            &root.join("branding/ruthenium.png"),
            &contents.join("Resources/ruthenium.icns"),
        )
        .map_err(|e| eprintln!("bundle: no icon: {e}"))
        .is_ok();
    let plist = contents.join("Info.plist");
    std::fs::write(&plist, info_plist(env!("CARGO_PKG_VERSION"), icon))
        .map_err(|e| format!("{}: {e}", plist.display()))?;
    if cfg!(target_os = "macos") {
        // Ad hoc: enough to run on the machine that built it and in CI. A bundle for
        // players needs a Developer ID signature and notarisation on top.
        run(Command::new("codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app))?;
    }
    Ok(app)
}

/// `ruthenium.icns` from the 1024-pixel PNG, with `sips` and `iconutil`.
fn make_icon(png: &Path, icns: &Path) -> Result<(), String> {
    let set = icns.with_extension("iconset");
    std::fs::create_dir_all(&set).map_err(|e| format!("{}: {e}", set.display()))?;
    for size in [16u32, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let side = (size * scale).to_string();
            run(Command::new("sips")
                .args(["-z", &side, &side])
                .arg(png)
                .arg("--out")
                .arg(set.join(format!("icon_{size}x{size}{suffix}.png")))
                .stdout(std::process::Stdio::null()))?;
        }
    }
    run(Command::new("iconutil")
        .args(["--convert", "icns", "--output"])
        .arg(icns)
        .arg(&set))?;
    std::fs::remove_dir_all(&set).map_err(|e| format!("{}: {e}", set.display()))
}

fn info_plist(version: &str, icon: bool) -> String {
    let icon = if icon {
        "\t<key>CFBundleIconFile</key>\n\t<string>ruthenium</string>\n"
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>CFBundleName</key>\n\t<string>Ruthenium</string>\n\
\t<key>CFBundleDisplayName</key>\n\t<string>Ruthenium</string>\n\
\t<key>CFBundleIdentifier</key>\n\t<string>{BUNDLE_ID}</string>\n\
\t<key>CFBundleExecutable</key>\n\t<string>ruthenium</string>\n\
\t<key>CFBundlePackageType</key>\n\t<string>APPL</string>\n\
\t<key>CFBundleInfoDictionaryVersion</key>\n\t<string>6.0</string>\n\
\t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n\
\t<key>CFBundleVersion</key>\n\t<string>{version}</string>\n\
{icon}\
\t<key>LSMinimumSystemVersion</key>\n\t<string>{MINIMUM_MACOS}</string>\n\
\t<key>LSApplicationCategoryType</key>\n\t<string>public.app-category.games</string>\n\
\t<key>NSHighResolutionCapable</key>\n\t<true/>\n\
\t<key>NSSupportsAutomaticGraphicsSwitching</key>\n\t<true/>\n\
</dict>\n\
</plist>\n"
    )
}

fn run(command: &mut Command) -> Result<(), String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .map_err(|e| format!("running {name}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_names_the_executable_and_the_bundle() {
        let p = info_plist("1.2.3", true);
        for needle in [
            "<key>CFBundleExecutable</key>\n\t<string>ruthenium</string>",
            "<key>CFBundleIdentifier</key>\n\t<string>dev.ruthenium.Ruthenium</string>",
            "<key>CFBundleShortVersionString</key>\n\t<string>1.2.3</string>",
            "<key>CFBundleIconFile</key>\n\t<string>ruthenium</string>",
            "<key>NSHighResolutionCapable</key>\n\t<true/>",
        ] {
            assert!(p.contains(needle), "{needle}");
        }
        assert!(p.starts_with("<?xml"));
        assert!(p.trim_end().ends_with("</plist>"));
        assert!(!info_plist("1.2.3", false).contains("CFBundleIconFile"));
        // Well formed as far as tags go: every key has its value.
        assert_eq!(
            p.matches("<key>").count(),
            p.matches("<string>").count() + p.matches("<true/>").count()
        );
    }

    #[test]
    fn the_pin_is_one_release() {
        assert!(URL.contains(&format!("/v{VERSION}/")));
        assert_eq!(SHA256.len(), 64);
        assert!(DYLIB_MEMBER.ends_with("macOS/libMoltenVK.dylib"));
    }
}
