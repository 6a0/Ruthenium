//! Finding a Java runtime to run the data generator and exporters with.
//!
//! Order: an explicit path, Java runtimes the launcher downloaded (Mojang's
//! own runtime first), `JAVA_HOME`, then `java` on `PATH`. Each candidate is
//! started once to read its version.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::discover::Install;
use crate::version::JavaRequirement;

/// How far below a launcher's Java folder to look for `bin/java`. The deepest
/// known layout is Mojang's macOS runtime:
/// `runtime/<component>/mac-os-arm64/<component>/jre.bundle/Contents/Home/bin/java`.
const SEARCH_DEPTH: u32 = 6;

const JAVA_EXE: &str = if cfg!(windows) { "java.exe" } else { "java" };

/// Where a Java candidate came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaSource {
    Explicit,
    Launcher,
    JavaHome,
    Path,
}

/// A runtime that satisfies the version's requirement.
#[derive(Debug, Clone)]
pub struct Java {
    pub path: PathBuf,
    pub source: JavaSource,
    /// Feature release, from `java.specification.version`.
    pub major: u32,
    /// e.g. `OpenJDK 64-Bit Server VM 25.0.1+8-LTS (Microsoft)`.
    pub description: String,
}

/// No usable runtime: every candidate tried, with the reason it was rejected.
#[derive(Debug)]
pub struct NoJava {
    pub required: u32,
    pub tried: Vec<(PathBuf, String)>,
}

impl fmt::Display for NoJava {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no Java {}+ runtime found", self.required)?;
        for (path, why) in &self.tried {
            write!(f, "\n  {}: {why}", path.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for NoJava {}

/// Finds a runtime. With `explicit`, only that path is tried.
pub fn find_java(
    install: Option<&Install>,
    req: &JavaRequirement,
    explicit: Option<&Path>,
) -> Result<Java, NoJava> {
    let mut candidates: Vec<(PathBuf, JavaSource)> = Vec::new();
    if let Some(p) = explicit {
        candidates.push((p.to_owned(), JavaSource::Explicit));
    } else {
        if let Some(install) = install {
            let mut found = Vec::new();
            for root in &install.java_roots {
                find_java_exes(root, SEARCH_DEPTH, &mut found);
            }
            // Mojang's runtime for this version first, then anything newer.
            found.sort_by_key(|p| !p.components().any(|c| c.as_os_str() == req.component));
            candidates.extend(found.into_iter().map(|p| (p, JavaSource::Launcher)));
        }
        if let Some(home) = std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
            candidates.push((
                PathBuf::from(home).join("bin").join(JAVA_EXE),
                JavaSource::JavaHome,
            ));
        }
        candidates.push((PathBuf::from(JAVA_EXE), JavaSource::Path));
    }

    let mut tried = Vec::new();
    for (path, source) in candidates {
        match probe(&path) {
            Ok((major, description)) if major >= req.major => {
                return Ok(Java {
                    path,
                    source,
                    major,
                    description,
                });
            }
            Ok((major, _)) => tried.push((path, format!("Java {major}"))),
            Err(e) => tried.push((path, e)),
        }
    }
    Err(NoJava {
        required: req.major,
        tried,
    })
}

/// Collects `<dir>/bin/java` up to `depth` levels below `root`.
fn find_java_exes(root: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let exe = root.join("bin").join(JAVA_EXE);
    if exe.is_file() {
        out.push(exe);
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        find_java_exes(&dir, depth - 1, out);
    }
}

/// Starts `java` once and reads its version.
fn probe(java: &Path) -> Result<(u32, String), String> {
    let out = Command::new(java)
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("exited with {}", out.status));
    }
    // The settings go to stderr.
    parse_properties(&String::from_utf8_lossy(&out.stderr))
        .ok_or_else(|| "could not read java.specification.version".into())
}

fn parse_properties(text: &str) -> Option<(u32, String)> {
    let prop = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.trim().split_once(" = ")?;
            (k == key).then(|| v.trim().to_owned())
        })
    };
    let spec = prop("java.specification.version")?;
    // Java 8 reports "1.8".
    let major = spec.strip_prefix("1.").unwrap_or(&spec).parse().ok()?;
    let description = format!(
        "{} {} ({})",
        prop("java.vm.name").unwrap_or_default(),
        prop("java.runtime.version").unwrap_or_default(),
        prop("java.vendor").unwrap_or_default()
    );
    Some((major, description))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mojang_macos_runtime_layout() {
        let root =
            std::env::temp_dir().join(format!("ruthenium-java-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join(
            "runtime/java-runtime-epsilon/mac-os-arm64/java-runtime-epsilon/jre.bundle/Contents/Home",
        );
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("bin").join(JAVA_EXE), b"").unwrap();
        let mut found = Vec::new();
        find_java_exes(&root.join("runtime"), SEARCH_DEPTH, &mut found);
        assert_eq!(found, [home.join("bin").join(JAVA_EXE)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_version_properties() {
        let text = "Property settings:\n    java.runtime.version = 25.0.1+8-LTS\n    java.specification.version = 25\n    java.vendor = Microsoft\n    java.vm.name = OpenJDK 64-Bit Server VM\n";
        let (major, desc) = parse_properties(text).unwrap();
        assert_eq!(major, 25);
        assert_eq!(desc, "OpenJDK 64-Bit Server VM 25.0.1+8-LTS (Microsoft)");
        assert_eq!(
            parse_properties("    java.specification.version = 1.8\n")
                .unwrap()
                .0,
            8
        );
        assert!(parse_properties("garbage").is_none());
    }
}
