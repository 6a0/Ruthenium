//! Building Ruthenium's development Fabric mods (`tools/java/<name>`).
//!
//! 26.x ships with Mojang's names, so a mod compiles with plain javac against
//! the client jar and needs no remapping. The jar holds only Ruthenium's own
//! `dev/ruthenium/` classes and its metadata files.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::{fabric, jvm};

const OWN_PREFIX: &str = "dev/ruthenium/";

/// Compiles `tools/java/<name>/src` (+ `resources`) into
/// `target/java/ruthenium-<name>.jar` against the game and `extra` jars.
pub fn build(root: &Path, name: &str, extra: &[PathBuf]) -> Result<PathBuf, String> {
    let src = root.join("tools/java").join(name).join("src");
    let resources = root.join("tools/java").join(name).join("resources");
    let mut cp = jvm::game_classpath(root, None)?;
    for jar in fabric::fetch_all(root, fabric::LOADER)?.iter().chain(extra) {
        jvm::push(&mut cp, jar);
    }

    let mut sources = Vec::new();
    collect(&src, &mut sources);
    sources.retain(|p| p.extension().is_some_and(|e| e == "java"));
    sources.sort();
    if sources.is_empty() {
        return Err(format!("no sources in {}", src.display()));
    }
    let classes = root.join("target/java").join(name).join("classes");
    let _ = std::fs::remove_dir_all(&classes);
    let ok = Command::new(jvm::tool("javac"))
        .args([
            "--release",
            "25",
            "-encoding",
            "UTF-8",
            "-proc:none",
            "-Xlint:all,-classfile,-processing,-serial",
            "-Werror",
            "-d",
        ])
        .arg(&classes)
        .arg("-cp")
        .arg(&cp)
        .args(&sources)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return Err(format!("compiling {name} failed"));
    }

    let out = root
        .join("target/java")
        .join(format!("ruthenium-{name}.jar"));
    write_jar(&classes, &resources, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    check_jar(&out)?;
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(dir).expect("under dir");
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            (name, p)
        })
        .collect()
}

/// A stored jar with sorted entries and fixed timestamps.
fn write_jar(
    classes: &Path,
    resources: &Path,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut names = entries(classes);
    names.extend(entries(resources));
    names.sort();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut zip = ZipWriter::new(std::fs::File::create(out)?);
    zip.start_file("META-INF/MANIFEST.MF", options)?;
    zip.write_all(b"Manifest-Version: 1.0\r\nCreated-By: Ruthenium xtask\r\n\r\n")?;
    for (name, path) in names {
        zip.start_file(name, options)?;
        zip.write_all(&std::fs::read(path)?)?;
    }
    zip.finish()?;
    Ok(())
}

/// Every entry must be a `dev/ruthenium/` class or a top-level metadata file
/// (`fabric.mod.json`, a `*.mixins.json`).
pub fn check_jar(jar: &Path) -> Result<(), String> {
    let file = std::fs::File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut foreign = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry
            .name()
            .map_or_else(|_| format!("#{i}"), |n| n.into_owned());
        let metadata = name == "META-INF/MANIFEST.MF"
            || name == "fabric.mod.json"
            || (!name.contains('/') && name.ends_with(".mixins.json"));
        let own = name.starts_with(OWN_PREFIX) && !name.contains("..");
        if !metadata && !own {
            foreign.push(name);
        }
    }
    if foreign.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} holds entries that are not Ruthenium's own: {}",
            jar.display(),
            foreign.join(", ")
        ))
    }
}
