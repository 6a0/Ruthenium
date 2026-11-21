//! `cargo xtask fetch-compile-deps`: download the official client jar and
//! its class-path libraries, pinned by SHA-1, so CI can compile the exporters
//! and check the generated tables (`cargo xtask codegen --check`).
//!
//! These files are build-time inputs only. They go to `cache/` (git-ignored)
//! and are never committed, cached by CI or put in a build artifact. Ruthenium
//! itself downloads nothing: players' copies come from their own launcher.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha1::{Digest, Sha1};

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let mut out = root.join("cache/compile-deps");
    match args {
        [] => {}
        [flag, value] if flag == "--out" => out = PathBuf::from(value),
        _ => {
            eprintln!("usage: cargo xtask fetch-compile-deps [--out <folder>]");
            return ExitCode::FAILURE;
        }
    }
    let version = ruthenium_prepare::supported_version();
    let mut files = vec![(
        format!(
            "https://piston-data.mojang.com/v1/objects/{}/client.jar",
            version.client_jar.sha1
        ),
        out.join("client.jar"),
        version.client_jar.sha1,
    )];
    for lib in version.libraries {
        files.push((
            format!("https://libraries.minecraft.net/{}", lib.path),
            out.join("libraries").join(lib.rel_path()),
            lib.sha1,
        ));
    }

    let mut fetched = 0;
    for (url, dest, sha1) in &files {
        if sha1_of(dest).as_deref() == Some(*sha1) {
            continue;
        }
        if let Err(e) = fetch(url, dest, sha1) {
            eprintln!("fetch-compile-deps: {url}: {e}");
            return ExitCode::FAILURE;
        }
        fetched += 1;
    }
    println!(
        "fetch-compile-deps: {} files in {} ({fetched} downloaded); compile-only, never commit or redistribute",
        files.len(),
        out.display()
    );
    ExitCode::SUCCESS
}

fn sha1_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        Sha1::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

fn fetch(url: &str, dest: &Path, sha1: &str) -> Result<(), String> {
    let parent = dest.parent().expect("file has a parent");
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let partial = dest.with_extension("partial");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "3",
            "--output",
        ])
        .arg(&partial)
        .arg(url)
        .status()
        .map_err(|e| format!("running curl: {e}"))?;
    if !status.success() {
        return Err(format!("curl exited with {status}"));
    }
    match sha1_of(&partial) {
        Some(got) if got == sha1 => std::fs::rename(&partial, dest).map_err(|e| e.to_string()),
        got => {
            let _ = std::fs::remove_file(&partial);
            Err(format!("SHA-1 {got:?}, expected {sha1}"))
        }
    }
}
