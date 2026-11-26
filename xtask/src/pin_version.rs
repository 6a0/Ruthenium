//! `cargo xtask pin-version`: turn Mojang's version JSON into the pinned
//! hashes ruthenium-prepare checks installs against.
//!
//! Only facts go into the generated file: Maven names, relative paths, sizes and
//! SHA-1s, and the digest of the asset index's entries other than translations
//! (`ruthenium_prepare::asset_index`). No download URLs, because Ruthenium
//! downloads nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ruthenium_prepare::asset_index;
use serde_json::Value;
use sha1::{Digest, Sha1};

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let mut json = root.join("reference/26.3/jars/26.3.json");
    let mut out = root.join("crates/ruthenium-prepare/src/pins.rs");
    let mut launch_out = root.join("xtask/src/launch_pins.rs");
    let mut index = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(value) = it.next() else {
            eprintln!("pin-version: {arg} needs a value");
            return ExitCode::FAILURE;
        };
        match arg.as_str() {
            "--json" => json = PathBuf::from(value),
            "--asset-index" => index = Some(PathBuf::from(value)),
            "--out" => out = PathBuf::from(value),
            "--launch-out" => launch_out = PathBuf::from(value),
            other => {
                eprintln!("pin-version: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let text = match std::fs::read_to_string(&json) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("pin-version: {}: {e}", json.display());
            return ExitCode::FAILURE;
        }
    };
    let generated = index_digest(&text, &json, index.as_deref())
        .and_then(|digest| Ok([generate(&text, &digest)?, generate_launch(&text)?]));
    let generated = match generated {
        Ok(g) => g,
        Err(e) => {
            eprintln!("pin-version: {}: {e}", json.display());
            return ExitCode::FAILURE;
        }
    };
    for (code, out) in generated.into_iter().zip([&out, &launch_out]) {
        if let Err(e) = std::fs::write(out, code) {
            eprintln!("pin-version: {}: {e}", out.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", out.display());
    }
    ExitCode::SUCCESS
}

/// The digest to pin for the asset index the version JSON names. The index is
/// read from `index`, or from `<asset index id>.json` beside the version JSON,
/// and must be the very file the JSON names: its SHA-1 is checked. Mojang
/// republishes the index with new translations, so the JSON and the index must
/// come from the same download; the digest leaves the translations out and
/// holds for every republished index.
fn index_digest(text: &str, json: &Path, index: Option<&Path>) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let id = v["assetIndex"]["id"]
        .as_str()
        .ok_or("assetIndex.id is not a string")?;
    let sha1 = v["assetIndex"]["sha1"]
        .as_str()
        .ok_or("assetIndex.sha1 is not a string")?;
    let path = match index {
        Some(path) => path.to_owned(),
        None => json.with_file_name(format!("{id}.json")),
    };
    let bytes = std::fs::read(&path).map_err(|e| {
        format!(
            "asset index {}: {e} (pass --asset-index <file>: the launcher's assets/indexes/{id}.json)",
            path.display()
        )
    })?;
    let actual: String = Sha1::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != sha1 {
        return Err(format!(
            "{} is not the asset index this version JSON names (SHA-1 {actual}, expected {sha1})",
            path.display()
        ));
    }
    let entries =
        asset_index::parse(&bytes).map_err(|e| format!("asset index {}: {e}", path.display()))?;
    Ok(asset_index::pinned_digest(&entries))
}

/// The OSes Mojang's rules name.
const OSES: &[&str] = &["linux", "osx", "windows"];

/// Which OSes a rules list allows, for a 64-bit machine on a current OS
/// version: rules on 32-bit x86, old Windows versions or launcher features
/// allow nothing.
fn allowed_oses(rules: Option<&Value>) -> Vec<&'static str> {
    let Some(rules) = rules.and_then(Value::as_array) else {
        return OSES.to_vec();
    };
    let mut out = Vec::new();
    for rule in rules {
        if rule["action"] != "allow" || rule.get("features").is_some() {
            continue;
        }
        let os = &rule["os"];
        if os.get("arch").is_some() || os["versionRange"].get("max").is_some() {
            continue;
        }
        match os["name"].as_str() {
            Some(name) => out.extend(OSES.iter().filter(|o| **o == name)),
            None => out.extend(OSES),
        }
    }
    out
}

/// Launch facts for xtask's development launcher: platform-specific libraries
/// (natives, macOS-only jars) and the launcher's JVM arguments.
fn generate_launch(text: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut o = String::from(
        "//! Launch facts for the development launcher (`cargo xtask bench` and\n//! `cargo xtask trace`): platform-specific libraries and the launcher's JVM\n//! arguments for the supported version.\n//!\n//! @generated by `cargo xtask pin-version` from Mojang's version JSON. Facts only:\n//! names, relative paths, sizes, SHA-1s and arguments. Do not edit by hand.\n\n/// A library used only on some platforms: natives, or macOS-only jars.\n#[derive(Debug)]\npub struct PlatformLibrary {\n    pub name: &'static str,\n    /// `linux`, `osx` or `windows`.\n    pub os: &'static str,\n    /// `x86_64`, `aarch64`, or empty for any.\n    pub arch: &'static str,\n    pub path: &'static str,\n    pub sha1: &'static str,\n    pub size: u64,\n}\n\npub static PLATFORM_LIBRARIES: &[PlatformLibrary] = &[\n",
    );
    let libs = v["libraries"]
        .as_array()
        .ok_or("libraries is not an array")?;
    for lib in libs.iter().filter(|l| l.get("rules").is_some()) {
        let name = lib["name"].as_str().ok_or("library name")?;
        let art = &lib["downloads"]["artifact"];
        let classifier = name.split(':').nth(3).unwrap_or("");
        let arch = if classifier.contains("arm64") || classifier.contains("aarch_64") {
            "aarch64"
        } else if classifier.is_empty() {
            ""
        } else {
            "x86_64"
        };
        for os in allowed_oses(lib.get("rules")) {
            let _ = writeln!(
                o,
                "    PlatformLibrary {{\n        name: {name:?},\n        os: {os:?},\n        arch: {arch:?},\n        path: {:?},\n        sha1: {:?},\n        size: {},\n    }},",
                art["path"].as_str().ok_or("artifact path")?,
                art["sha1"].as_str().ok_or("artifact sha1")?,
                art["size"].as_u64().ok_or("artifact size")?
            );
        }
    }
    o.push_str("];\n\n/// JVM arguments the official launcher passes, as `(os, argument)`, in order:\n/// `arguments.jvm` then `arguments.default-user-jvm`. The class path is left\n/// out; `${natives_directory}`, `${launcher_name}` and `${launcher_version}`\n/// are placeholders.\npub static JVM_ARGS: &[(&str, &str)] = &[\n");
    for key in ["jvm", "default-user-jvm"] {
        let args = v["arguments"][key]
            .as_array()
            .ok_or_else(|| format!("arguments.{key} is not an array"))?;
        for arg in args {
            let (oses, values) = match arg {
                Value::String(s) => (OSES.to_vec(), vec![s.as_str()]),
                _ => {
                    let values = match &arg["value"] {
                        Value::String(s) => vec![s.as_str()],
                        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                        _ => return Err(format!("arguments.{key}: bad value {arg}")),
                    };
                    (allowed_oses(arg.get("rules")), values)
                }
            };
            for value in values {
                if value == "-cp" || value.contains("${classpath}") {
                    continue;
                }
                for os in &oses {
                    let _ = writeln!(o, "    ({os:?}, {value:?}),");
                }
            }
        }
    }
    o.push_str("];\n");
    Ok(o)
}

fn generate(text: &str, index_digest: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let s = |v: &Value, what: &str| {
        v.as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("{what} is not a string"))
    };
    let n = |v: &Value, what: &str| v.as_u64().ok_or_else(|| format!("{what} is not a number"));

    let id = s(&v["id"], "id")?;
    let client = &v["downloads"]["client"];
    let index = &v["assetIndex"];
    let java = &v["javaVersion"];
    let konst = id.replace('.', "_").to_uppercase();

    let mut o = String::new();
    let _ = writeln!(
        o,
        "//! Pinned hashes of the supported game version.\n//!\n//! @generated by `cargo xtask pin-version` from Mojang's version JSON. Facts only:\n//! names, relative paths, sizes, SHA-1s and the asset index's digest. Do not edit\n//! by hand.\n\nuse crate::version::{{AssetIndex, GameVersion, JavaRequirement, Library, Pinned}};\n"
    );
    let _ = writeln!(o, "/// Minecraft: Java Edition {id}.");
    let _ = writeln!(o, "pub static V{konst}: GameVersion = GameVersion {{");
    let _ = writeln!(o, "    id: {id:?},");
    let _ = writeln!(
        o,
        "    client_jar: Pinned {{\n        sha1: {:?},\n        size: {},\n    }},",
        s(&client["sha1"], "client sha1")?,
        n(&client["size"], "client size")?
    );
    let _ = writeln!(
        o,
        "    asset_index: AssetIndex {{\n        id: {:?},\n        pinned_sha256: {index_digest:?},\n    }},",
        s(&index["id"], "assetIndex.id")?
    );
    let _ = writeln!(
        o,
        "    java: JavaRequirement {{\n        component: {:?},\n        major: {},\n    }},",
        s(&java["component"], "javaVersion.component")?,
        n(&java["majorVersion"], "javaVersion.majorVersion")?
    );
    let _ = writeln!(o, "    libraries: &[");
    let libs = v["libraries"]
        .as_array()
        .ok_or("libraries is not an array")?;
    let mut count = 0;
    for lib in libs {
        // Libraries with OS rules are natives (and macOS-only jars). The JVM
        // tools Ruthenium runs need only the plain class path.
        if lib.get("rules").is_some() {
            continue;
        }
        let name = s(&lib["name"], "library name")?;
        let art = &lib["downloads"]["artifact"];
        let _ = writeln!(
            o,
            "        Library {{\n            name: {name:?},\n            path: {:?},\n            sha1: {:?},\n            size: {},\n        }},",
            s(&art["path"], "artifact path")?,
            s(&art["sha1"], "artifact sha1")?,
            n(&art["size"], "artifact size")?
        );
        count += 1;
    }
    let _ = writeln!(o, "    ],\n}};");
    if count == 0 {
        return Err("no libraries without OS rules".into());
    }
    Ok(o)
}
