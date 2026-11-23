//! `cargo xtask fixtures`: runs the client's development checks, one short run each, and
//! fails if any of them does. What Step 2 of the renderer work has to keep passing on
//! every machine and runner that can open a window:
//!
//! - the colour fixture and the upload check, which also writes the adapter report;
//! - the texture table filled to exhaustion and reloaded at 256 slots, on ordinary
//!   descriptor sets and, where the GPU has them, on update-after-bind sets;
//! - the pipeline cache: a second launch of the same build must find the first one's.
//!
//! Each run's output goes to `target/fixtures/`, with the adapter report beside it.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Lines of a run's output worth showing in the summary.
const SHOWN: &[&str] = &[
    "[ruthenium] GPU:",
    "[render] texture table:",
    "colour check",
    "texture table check",
    "upload check",
    "[colour-check]",
    "[table-check]",
    "[upload-check]",
    "[vulkan ",
    "validation message",
    "ruthenium:",
];

struct Options {
    binary: Option<PathBuf>,
    features: Option<String>,
    gpu: Option<String>,
    seconds: String,
    out: PathBuf,
    extra: Vec<String>,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    match run_inner(root, args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("fixtures: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(root: &Path, args: &[String]) -> Result<bool, String> {
    let mut o = Options {
        binary: None,
        features: None,
        gpu: None,
        seconds: "6".into(),
        out: root.join("target/fixtures"),
        extra: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--bin" => o.binary = Some(value("an executable")?.into()),
            "--features" => o.features = Some(value("cargo features")?),
            "--gpu" => o.gpu = Some(value("a GPU name")?),
            "--seconds" => o.seconds = value("a number of seconds")?,
            "--out" => o.out = value("a folder")?.into(),
            "--" => {
                o.extra = it.cloned().collect();
                break;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let binary = match &o.binary {
        Some(b) => b.clone(),
        None => {
            let mut build = Command::new(env!("CARGO"));
            build.current_dir(root).args(["build", "-p", "ruthenium"]);
            if let Some(f) = &o.features {
                build.args(["--features", f]);
            }
            let status = build.status().map_err(|e| format!("running cargo: {e}"))?;
            if !status.success() {
                return Err(format!("cargo build exited with {status}"));
            }
            root.join("target/debug")
                .join(format!("ruthenium{}", std::env::consts::EXE_SUFFIX))
        }
    };
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let adapter = o.out.join("adapter.json");
    let _ = std::fs::remove_file(&adapter);
    let adapter_arg = adapter.to_string_lossy().into_owned();

    let mut passed = true;
    let mut check = |name: &str, r: Result<Run, String>| -> Option<Run> {
        match r {
            Ok(run) => {
                println!("{} {name}", if run.ok { "ok  " } else { "FAIL" });
                for line in run
                    .output
                    .lines()
                    .filter(|l| SHOWN.iter().any(|s| l.contains(s)))
                {
                    println!("       {line}");
                }
                passed &= run.ok;
                Some(run)
            }
            Err(e) => {
                println!("FAIL {name}: {e}");
                passed = false;
                None
            }
        }
    };

    check(
        "colour, uploads, adapter report",
        launch(
            &binary,
            &o,
            "colour",
            &o.seconds,
            &[
                "--exercise-colour",
                "--exercise-uploads",
                "--adapter-report",
                &adapter_arg,
            ],
        ),
    );
    check(
        "texture table, 256 slots, ordinary sets",
        launch(
            &binary,
            &o,
            "table-ordinary",
            &o.seconds,
            &[
                "--exercise-textures",
                "--exercise-colour",
                "--texture-slots",
                "256",
                "--texture-table",
                "ordinary",
            ],
        ),
    );
    // Only a GPU with the feature can make the table of update-after-bind sets.
    let report: serde_json::Value = std::fs::read_to_string(&adapter)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let limit = report["texture_table"]["update_after_bind_limit"].as_u64();
    if limit.is_some_and(|n| n >= 256) {
        check(
            "texture table, 256 slots, update-after-bind sets",
            launch(
                &binary,
                &o,
                "table-update-after-bind",
                &o.seconds,
                &[
                    "--exercise-textures",
                    "--exercise-colour",
                    "--texture-slots",
                    "256",
                    "--texture-table",
                    "update-after-bind",
                ],
            ),
        );
    } else {
        println!(
            "skip texture table, 256 slots, update-after-bind sets: the adapter report gives {limit:?} such textures"
        );
    }
    // The same build twice: whatever the first launch found, the second finds its cache.
    let first = check(
        "pipeline cache, first launch",
        launch(&binary, &o, "cache-1", "1", &[]),
    );
    let second = check(
        "pipeline cache, second launch",
        launch(&binary, &o, "cache-2", "1", &[]),
    );
    if let (Some(first), Some(second)) = (first, second) {
        let (a, b) = (cache_line(&first.output), cache_line(&second.output));
        let warm = matches!((&a, &b), (Some((file, _)), Some((again, true))) if file == again);
        println!(
            "{} pipeline cache is warm on the second launch",
            if warm { "ok  " } else { "FAIL" }
        );
        if !warm {
            println!("       first: {a:?}, second: {b:?}");
            passed = false;
        }
    }
    println!(
        "fixtures: {}; output in {}",
        if passed { "all passed" } else { "FAILED" },
        o.out.display()
    );
    Ok(passed)
}

struct Run {
    ok: bool,
    output: String,
}

/// One run of the client with `args`, its output saved as `<name>.log`.
fn launch(
    binary: &Path,
    o: &Options,
    name: &str,
    seconds: &str,
    args: &[&str],
) -> Result<Run, String> {
    let mut cmd = Command::new(binary);
    cmd.arg("--data-dir")
        .arg(o.out.join("data"))
        .args(["--quit-after", seconds])
        .args(args)
        .args(&o.extra);
    if let Some(gpu) = &o.gpu {
        cmd.args(["--gpu", gpu]);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("running {}: {e}", binary.display()))?;
    let output = String::from_utf8_lossy(&out.stderr).into_owned();
    let log = o.out.join(format!("{name}.log"));
    std::fs::write(&log, &output).map_err(|e| format!("{}: {e}", log.display()))?;
    Ok(Run {
        ok: out.status.success(),
        output,
    })
}

/// The cache file a run opened and whether it loaded anything from it.
fn cache_line(output: &str) -> Option<(String, bool)> {
    let line = output
        .lines()
        .find_map(|l| l.strip_prefix("[gpu] pipeline cache "))?;
    let (file, state) = line.split_once(": ")?;
    Some((file.to_owned(), state.ends_with("(warm)")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_cache_line() {
        let cold = "[gpu] Vulkan validation layer enabled\n[gpu] pipeline cache pipelines-10de-28e0-ab-01.bin: new (cold)\n";
        assert_eq!(
            cache_line(cold),
            Some(("pipelines-10de-28e0-ab-01.bin".to_owned(), false))
        );
        let warm = "[gpu] pipeline cache pipelines-10de-28e0-ab-01.bin: 127.4 KiB loaded (warm)\n";
        assert_eq!(
            cache_line(warm),
            Some(("pipelines-10de-28e0-ab-01.bin".to_owned(), true))
        );
        assert_eq!(cache_line("[gpu] pipeline cache: off\n"), None);
        assert_eq!(cache_line(""), None);
    }
}
