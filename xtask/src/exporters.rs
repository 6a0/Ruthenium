//! `cargo xtask exporters`: compile Ruthenium's Java exporters into
//! `target/java/ruthenium-exporters.jar`, and refuse any bundle holding a class
//! that is not Ruthenium's own.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::jvm;

const SOURCES: &str = "tools/java/exporters";
/// Everything in the bundle must live under this prefix (or be the manifest).
const OWN_PREFIX: &str = "dev/ruthenium/";
const MANIFEST: &str = "META-INF/MANIFEST.MF";

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let mut javac = jvm::tool("javac");
    let mut classpath: Option<OsString> = None;
    let mut deps: Option<PathBuf> = None;
    let mut out = root.join("target/java/ruthenium-exporters.jar");
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(value) = it.next() else {
            eprintln!("exporters: {arg} needs a value");
            return ExitCode::FAILURE;
        };
        match arg.as_str() {
            "--javac" => javac = PathBuf::from(value),
            "--classpath" => classpath = Some(OsString::from(value)),
            "--deps" => deps = Some(PathBuf::from(value)),
            "--out" => out = PathBuf::from(value),
            other => {
                eprintln!("exporters: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let classpath = match classpath.map_or_else(|| jvm::game_classpath(root, deps.as_deref()), Ok) {
        Ok(cp) => cp,
        Err(e) => {
            eprintln!("exporters: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut sources = Vec::new();
    collect(&root.join(SOURCES), "java", &mut sources);
    sources.sort();
    let classes = root.join("target/java/exporters/classes");
    let _ = std::fs::remove_dir_all(&classes);
    let compiled = Command::new(&javac)
        .args([
            "--release",
            "25",
            "-encoding",
            "UTF-8",
            "-Xlint:all,-classfile",
            "-Werror",
            "-d",
        ])
        .arg(&classes)
        .arg("-cp")
        .arg(&classpath)
        .args(&sources)
        .status();
    if !matches!(compiled, Ok(s) if s.success()) {
        eprintln!("exporters: compiling with {} failed", javac.display());
        return ExitCode::FAILURE;
    }

    if let Err(e) = write_bundle(&classes, &out) {
        eprintln!("exporters: writing {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    match check_bundle(&out) {
        Ok(n) => {
            println!(
                "exporters: {} ({n} classes, all {OWN_PREFIX})",
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            eprintln!("exporters: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cargo xtask check-bundle <jar>`.
pub fn run_check(args: &[String]) -> ExitCode {
    let [jar] = args else {
        eprintln!("usage: cargo xtask check-bundle <jar>");
        return ExitCode::FAILURE;
    };
    match check_bundle(Path::new(jar)) {
        Ok(n) => {
            println!("check-bundle: {jar}: {n} classes, all {OWN_PREFIX}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("check-bundle: {e}");
            ExitCode::FAILURE
        }
    }
}

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// A stored (uncompressed) jar with sorted entries and fixed timestamps, so the
/// same classes always give the same bytes.
fn write_bundle(classes: &Path, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    collect(classes, "class", &mut files);
    let mut names: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(classes).expect("under classes");
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            (name, p)
        })
        .collect();
    names.sort();

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut zip = ZipWriter::new(std::fs::File::create(out)?);
    zip.start_file(MANIFEST, options)?;
    zip.write_all(b"Manifest-Version: 1.0\r\nCreated-By: Ruthenium xtask\r\n\r\n")?;
    for (name, path) in names {
        zip.start_file(name, options)?;
        zip.write_all(&std::fs::read(path)?)?;
    }
    zip.finish()?;
    Ok(())
}

/// Checks that every entry is Ruthenium's own; returns the class count.
pub fn check_bundle(jar: &Path) -> Result<usize, String> {
    let file = std::fs::File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut classes = 0;
    let mut foreign = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry
            .name()
            .map_or_else(|_| format!("entry #{i}"), |n| n.into_owned());
        if name == MANIFEST || name == "META-INF/" || is_own_prefix_dir(&name) {
            continue;
        }
        if name.starts_with(OWN_PREFIX) && !name.contains("..") {
            classes += usize::from(name.ends_with(".class"));
        } else {
            foreign.push(name);
        }
    }
    if foreign.is_empty() {
        Ok(classes)
    } else {
        Err(format!(
            "{} contains entries that are not Ruthenium's own: {}",
            jar.display(),
            foreign.join(", ")
        ))
    }
}

/// `dev/` itself, the parent folder entry of the allowed prefix.
fn is_own_prefix_dir(name: &str) -> bool {
    OWN_PREFIX.starts_with(name) && name.ends_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jar_with(names: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bundle.jar");
        let mut zip = ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in names {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        (tmp, path)
    }

    #[test]
    fn bundle_check_accepts_only_ruthenium_classes() {
        let (_ok, ok) = jar_with(&[MANIFEST, "dev/ruthenium/exporters/Main.class"]);
        assert_eq!(check_bundle(&ok), Ok(1));
        let (_bad, bad) = jar_with(&[
            MANIFEST,
            "dev/ruthenium/exporters/Main.class",
            "net/minecraft/world/entity/EntityType.class",
        ]);
        assert!(check_bundle(&bad).unwrap_err().contains("EntityType.class"));
        let (_sneaky, sneaky) = jar_with(&["dev/ruthenium/../../net/minecraft/A.class", "x", "y"]);
        assert!(check_bundle(&sneaky).is_err());
    }
}
