//! Guard against Mojang content entering the repository or build artifacts.
//!
//! Two layers:
//! 1. Rules on every file git would commit (tracked, plus untracked files that
//!    are not ignored): forbidden file types and paths, and text markers of
//!    decompiled Mojang source.
//! 2. Optional hash comparison: no file may be byte-identical to an entry of
//!    the vanilla client jar or to an object of an asset index.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha1::{Digest, Sha1};

/// Never allowed anywhere: game archives, compiled classes, sound, world and
/// pack-metadata files, vanilla core-shader sources.
const DENIED_EXTENSIONS: &[&str] = &["jar", "class", "ogg", "nbt", "mca", "mcmeta", "fsh", "vsh"];

/// Path fragments that only occur when copying out of a jar or resource pack.
const DENIED_PATH_PARTS: &[&str] = &["assets/minecraft/", "data/minecraft/", "reference/"];

/// Ruthenium's own Java (exporters, trace agent, test dumpers) lives here and
/// only here. Everything in it must be original code.
const JAVA_DIR: &str = "tools/java/";

/// Ruthenium's own artwork. PNGs anywhere else are rejected.
const PNG_DIR: &str = "branding/";

/// Bundled third-party fonts, each with its licence. Font files anywhere else are
/// rejected (vanilla's fonts must never be committed).
const FONT_DIR: &str = "crates/ruthenium-ui/fonts/";
const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2"];

/// Text that appears in decompiled Mojang classes but never in code that merely
/// calls them. Split so this file does not match itself.
const DENIED_TEXT: &[&str] = &[
    concat!("package ", "net.minecraft"),
    concat!("package ", "com.mojang"),
];

/// Files shorter than this are skipped in hash mode; tiny files (an empty
/// JSON object, a newline) collide by accident.
const MIN_HASHED_LEN: usize = 16;

pub fn run(args: &[String]) -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
        .to_path_buf();

    let mut jars = Vec::new();
    let mut indexes = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let target = match arg.as_str() {
            "--jar" => &mut jars,
            "--asset-index" => &mut indexes,
            other => {
                eprintln!("check-content: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        };
        match it.next() {
            Some(path) => target.push(PathBuf::from(path)),
            None => {
                eprintln!("check-content: {arg} needs a path");
                return ExitCode::FAILURE;
            }
        }
    }
    if jars.is_empty() {
        let local = root.join("reference/26.3/jars/client.jar");
        if local.is_file() {
            jars.push(local);
        }
    }

    match check(&root, &jars, &indexes) {
        Ok(problems) if problems.is_empty() => ExitCode::SUCCESS,
        Ok(problems) => {
            eprintln!("check-content: {} problem(s):", problems.len());
            for p in &problems {
                eprintln!("  {p}");
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("check-content: {e}");
            ExitCode::FAILURE
        }
    }
}

fn check(root: &Path, jars: &[PathBuf], indexes: &[PathBuf]) -> Result<Vec<String>, String> {
    let files = candidate_files(root)?;

    let mut known: HashMap<String, String> = HashMap::new();
    for jar in jars {
        hash_jar(jar, &mut known)?;
    }
    for index in indexes {
        hash_asset_index(index, &mut known)?;
    }

    let mut problems = Vec::new();
    for rel in &files {
        let path = root.join(rel);
        // Deleted-but-tracked files show up in ls-files; nothing to check.
        let Ok(bytes) = fs::read(&path) else { continue };
        problems.extend(rule_problems(rel, &bytes));
        if bytes.len() >= MIN_HASHED_LEN
            && let Some(origin) = known.get(&sha1_hex(&bytes))
        {
            problems.push(format!("{rel}: byte-identical to {origin}"));
        }
    }

    let mode = if known.is_empty() {
        "rules only".to_string()
    } else {
        format!("rules + {} known Mojang hashes", known.len())
    };
    println!("check-content: {} file(s) checked ({mode})", files.len());
    Ok(problems)
}

fn rule_problems(rel: &str, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let lower = rel.to_ascii_lowercase();
    let ext = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    if DENIED_EXTENSIONS.contains(&ext) {
        out.push(format!("{rel}: .{ext} files are never allowed"));
    }
    for part in DENIED_PATH_PARTS {
        if lower.starts_with(part) || lower.contains(&format!("/{part}")) {
            out.push(format!("{rel}: path contains {part:?}"));
        }
    }
    if ext == "java" && !lower.starts_with(JAVA_DIR) {
        out.push(format!(
            "{rel}: Java sources are only allowed under {JAVA_DIR}"
        ));
    }
    if ext == "png" && !lower.starts_with(PNG_DIR) {
        out.push(format!("{rel}: images are only allowed under {PNG_DIR}"));
    }
    if FONT_EXTENSIONS.contains(&ext) && !lower.starts_with(FONT_DIR) {
        out.push(format!("{rel}: fonts are only allowed under {FONT_DIR}"));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        for marker in DENIED_TEXT {
            if text.contains(marker) {
                out.push(format!(
                    "{rel}: contains {marker:?} (decompiled Mojang source?)"
                ));
            }
        }
    }
    out
}

/// Every file git would commit: tracked files plus untracked, non-ignored ones.
fn candidate_files(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut seen = HashSet::new();
    Ok(out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .filter(|s| seen.insert(s.clone()))
        .collect())
}

fn hash_jar(jar: &Path, known: &mut HashMap<String, String>) -> Result<(), String> {
    let file = File::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", jar.display()))?;
    let jar_name = jar
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut buf = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| format!("{}: {e}", jar.display()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_or_else(|_| format!("entry #{i}"), |n| n.into_owned());
        buf.clear();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("{}!{name}: {e}", jar.display()))?;
        if buf.len() >= MIN_HASHED_LEN {
            known.insert(sha1_hex(&buf), format!("{jar_name}!/{name}"));
        }
    }
    Ok(())
}

fn hash_asset_index(index: &Path, known: &mut HashMap<String, String>) -> Result<(), String> {
    let text = fs::read_to_string(index).map_err(|e| format!("{}: {e}", index.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", index.display()))?;
    let objects = json["objects"]
        .as_object()
        .ok_or_else(|| format!("{}: no \"objects\" map", index.display()))?;
    for (name, obj) in objects {
        if let Some(hash) = obj["hash"].as_str() {
            known.insert(hash.to_ascii_lowercase(), format!("asset object {name}"));
        }
    }
    Ok(())
}

fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::rule_problems;

    #[test]
    fn rejects_mojang_shaped_files() {
        assert!(!rule_problems("x/assets/minecraft/textures/a.png", b"").is_empty());
        assert!(!rule_problems("lib/client.jar", b"").is_empty());
        assert!(!rule_problems("src/Foo.java", b"").is_empty());
        assert!(!rule_problems("assets/font.ttf", b"").is_empty());
        let decompiled = concat!("package ", "net.minecraft.client;\n");
        assert!(!rule_problems("tools/java/Foo.java", decompiled.as_bytes()).is_empty());
    }

    #[test]
    fn accepts_own_files() {
        assert!(rule_problems("crates/ruthenium/src/main.rs", b"fn main() {}").is_empty());
        assert!(rule_problems("branding/logo.png", b"").is_empty());
        assert!(rule_problems("crates/ruthenium-ui/fonts/InterVariable.ttf", b"").is_empty());
        let exporter =
            "package dev.ruthenium.exporter;\nimport net.minecraft.world.level.block.Block;\n";
        assert!(rule_problems("tools/java/Exporter.java", exporter.as_bytes()).is_empty());
    }
}
