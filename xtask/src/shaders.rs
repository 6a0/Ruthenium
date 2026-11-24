//! `cargo xtask shaders`: compile `crates/ruthenium-shaders/glsl` to SPIR-V with a pinned
//! glslc, into the committed `crates/ruthenium-shaders/spirv` and the generated list
//! `src/generated.rs`. Building Ruthenium never compiles shaders; with `--check`, nothing
//! is written and the task fails if any committed file differs from what glslc produces.
//!
//! The compiler is the glslc of one Vulkan SDK release, pinned by the SDK archive's
//! SHA-256. On Linux x86-64 it is downloaded once into `cache/glslc/` (git-ignored; only
//! the glslc binary is kept). Elsewhere, pass `--glslc` from the same SDK release: its
//! `--version` must match exactly, because another version produces different bytes.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha2::{Digest, Sha256};

/// The tables the world shaders' accessors are written from. Included as source: the
/// xtask must build without the compiled shaders it is about to write.
#[path = "../../crates/ruthenium-shaders/src/interface.rs"]
#[allow(dead_code)]
mod interface;

const SDK_VERSION: &str = "1.4.363.1";
const SDK_URL: &str =
    "https://sdk.lunarg.com/sdk/download/1.4.363.1/linux/vulkansdk-linux-x86_64-1.4.363.1.tar.xz";
const SDK_SHA256: &str = "b7867d8ca658563813af666f34b93db87f1b1c7d9d69d97e924c210c7ba33eb8";
/// `glslc --version` of that SDK.
const GLSLC_VERSION: &str = "shaderc v2026.4 v2026.4\nspirv-tools v2026.4 v2022.4-1347-gef96ed76\nglslang 11.1.0-1566-ge1b562a8\n\nTarget: SPIR-V 1.0";

/// Vulkan 1.2 (SPIR-V 1.5) is the baseline; warnings are errors; optimised, with no
/// debug information (no source text or names).
const FLAGS: &[&str] = &["--target-env=vulkan1.2", "-O", "-Werror", "-Iinclude"];
const STAGES: &[&str] = &["vert", "frag", "comp"];
const CRATE: &str = "crates/ruthenium-shaders";

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    match run_inner(root, args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("shaders: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(root: &Path, args: &[String]) -> Result<ExitCode, String> {
    let mut check = false;
    let mut glslc: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--glslc" => glslc = Some(PathBuf::from(it.next().ok_or("--glslc needs a path")?)),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let glslc = match glslc {
        Some(g) => g,
        None => pinned_glslc(root)?,
    };
    let version = glslc_version(&glslc)?;
    if GLSLC_VERSION.starts_with('@') {
        return Err(format!(
            "glslc is not pinned yet; {} reports:\n{version}",
            glslc.display()
        ));
    }
    if version != GLSLC_VERSION {
        return Err(format!(
            "{} is not the pinned glslc (Vulkan SDK {SDK_VERSION}):\n--- found\n{version}\n--- expected\n{GLSLC_VERSION}",
            glslc.display()
        ));
    }

    let glsl = root.join(CRATE).join("glsl");
    let spirv_dir = root.join(CRATE).join("spirv");
    let mut sources: Vec<String> = std::fs::read_dir(&glsl)
        .map_err(|e| format!("{}: {e}", glsl.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| {
            n.rsplit_once('.')
                .is_some_and(|(_, ext)| STAGES.contains(&ext))
        })
        .collect();
    sources.sort();

    let tmp = root.join("target/shaders");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let mut outputs: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    // The generated include first: the shaders compiled below read it. A check compiles
    // against the committed one and reports it if it is stale.
    let world = glsl.join("include/world.glsl");
    let generated = interface::world_glsl().into_bytes();
    if !check && std::fs::read(&world).ok().as_deref() != Some(generated.as_slice()) {
        std::fs::write(&world, &generated).map_err(|e| format!("{}: {e}", world.display()))?;
        println!(
            "updated {}",
            world.strip_prefix(root).unwrap_or(&world).display()
        );
    }
    outputs.push((world, generated));
    for name in &sources {
        let out = tmp.join(format!("{name}.spv"));
        // Relative paths from inside glsl/, so nothing machine-specific reaches the output.
        let result = Command::new(&glslc)
            .current_dir(&glsl)
            .args(FLAGS)
            .arg(name)
            .arg("-o")
            .arg(&out)
            .output()
            .map_err(|e| format!("running {}: {e}", glslc.display()))?;
        if !result.status.success() {
            return Err(format!(
                "{name}:\n{}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        let bytes = std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        outputs.push((spirv_dir.join(format!("{name}.spv")), bytes));
    }
    outputs.push((
        root.join(CRATE).join("src/generated.rs"),
        generated_rs(&sources).into_bytes(),
    ));

    let mut changed = Vec::new();
    for (path, bytes) in &outputs {
        if std::fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
            continue;
        }
        if !check {
            std::fs::create_dir_all(path.parent().expect("has a parent"))
                .map_err(|e| e.to_string())?;
            std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        changed.push(path.clone());
    }
    // SPIR-V whose source is gone.
    if let Ok(entries) = std::fs::read_dir(&spirv_dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "spv")
                && !outputs.iter().any(|(p, _)| *p == path)
            {
                if !check {
                    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                }
                changed.push(path);
            }
        }
    }

    let shown = |p: &PathBuf| p.strip_prefix(root).unwrap_or(p).display().to_string();
    if check {
        if changed.is_empty() {
            println!("shaders: {} shader(s) up to date", sources.len());
            return Ok(ExitCode::SUCCESS);
        }
        eprintln!("shaders: out of date (run `cargo xtask shaders`):");
        for p in &changed {
            eprintln!("  {}", shown(p));
        }
        return Ok(ExitCode::FAILURE);
    }
    for p in &changed {
        println!("updated {}", shown(p));
    }
    println!(
        "shaders: {} shader(s), {} file(s) changed",
        sources.len(),
        changed.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// `background.frag` -> `BACKGROUND_FRAG`.
fn const_name(source: &str) -> String {
    source
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn generated_rs(sources: &[String]) -> String {
    let mut s = String::from(
        "//! The compiled shaders, one constant per GLSL source.\n//!\n//! @generated by `cargo xtask shaders`. Do not edit by hand.\n\nuse crate::Spirv;\n",
    );
    for name in sources {
        s += &format!(
            "\n/// `glsl/{name}`\npub static {}: Spirv = crate::spirv!(\"{name}\");\n",
            const_name(name)
        );
    }
    s += "\n/// Every shader, in file-name order.\npub static ALL: &[&Spirv] = &[";
    let items: Vec<String> = sources
        .iter()
        .map(|n| format!("&{}", const_name(n)))
        .collect();
    // Laid out as rustfmt does (`array_width` is 60 at the default width of 100), so
    // `cargo fmt --check` and `--check` here agree on the committed file.
    let one_line = items.join(", ");
    if one_line.len() <= 60 {
        s += &one_line;
    } else {
        s += "\n";
        for item in &items {
            s += &format!("    {item},\n");
        }
    }
    s += "];\n";
    s
}

fn glslc_version(glslc: &Path) -> Result<String, String> {
    let out = Command::new(glslc)
        .arg("--version")
        .output()
        .map_err(|e| format!("running {}: {e}", glslc.display()))?;
    if !out.status.success() {
        return Err(format!("{} --version failed", glslc.display()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `cache/glslc/<sdk>/glslc`, downloading the SDK and extracting glslc if needed.
fn pinned_glslc(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("cache/glslc").join(SDK_VERSION);
    let glslc = dir.join("glslc");
    if glslc.is_file() {
        return Ok(glslc);
    }
    if !(cfg!(target_os = "linux") && cfg!(target_arch = "x86_64")) {
        return Err(format!(
            "the pinned glslc is only downloaded on Linux x86-64; install Vulkan SDK {SDK_VERSION} and pass --glslc <its glslc>"
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let archive = dir.join("sdk.tar.xz");
    println!("shaders: downloading Vulkan SDK {SDK_VERSION} (about 370 MB, once) for its glslc");
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
        .arg(&archive)
        .arg(SDK_URL)
        .status()
        .map_err(|e| format!("running curl: {e}"))?;
    if !status.success() {
        return Err(format!("downloading {SDK_URL}: curl exited with {status}"));
    }
    let sha = sha256_of(&archive)?;
    if sha != SDK_SHA256 {
        let _ = std::fs::remove_file(&archive);
        return Err(format!("{SDK_URL}: SHA-256 {sha}, expected {SDK_SHA256}"));
    }
    let member = format!("{SDK_VERSION}/x86_64/bin/glslc");
    let status = Command::new("tar")
        .arg("-xJf")
        .arg(&archive)
        .arg("-C")
        .arg(&dir)
        .arg("--strip-components=3")
        .arg(&member)
        .status()
        .map_err(|e| format!("running tar: {e}"))?;
    let _ = std::fs::remove_file(&archive);
    if !status.success() || !glslc.is_file() {
        return Err(format!("could not extract {member} from the SDK archive"));
    }
    Ok(glslc)
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
