//! `cargo xtask trace`: record golden traces from the vanilla 26.3 client.
//!
//! The client runs from the player's own install (see [`crate::game`]) with
//! Fabric Loader, the Fabric API client gametest module (and the two modules
//! it needs), and Ruthenium's recorder, `tools/java/trace`. Client gametests
//! run each scenario with one client tick per frame and one server tick per
//! client tick, so inputs land on exact ticks. Each scenario writes
//! `<name>.jsonl` (format: docs/golden-traces.md).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::{fabric, game, mods};

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    match run_inner(root, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trace: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(root: &Path, args: &[String]) -> Result<(), String> {
    let mut only = String::new();
    let mut out = root.join("cache/traces/26.3");
    let mut java: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--only" => only = value.clone(),
            "--out" => out = PathBuf::from(value),
            "--java" => java = Some(PathBuf::from(value)),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let game = game::locate(java.as_deref())?;
    let fabric_api = fabric::fetch_all(root, fabric::GAMETEST_MODS)?;
    let jar = mods::build(root, "trace", &fabric_api)?;
    let out = std::path::absolute(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    let game_dir = root.join("cache/traces/game");
    // The gametest API restores its own default options before each test.
    let _ = std::fs::remove_file(game_dir.join("options.txt"));
    let _ = std::fs::remove_dir_all(game_dir.join("saves"));
    let mut mods = fabric_api;
    mods.push(jar);
    let launch = game::Launch {
        game_dir: game_dir.clone(),
        mods,
        jvm_args: vec![
            "-Dfabric.client.gametest".into(),
            format!("-Druthenium.trace.out={}", out.display()),
            format!("-Druthenium.trace.only={only}"),
        ],
        game_args: Vec::new(),
        env: Vec::new(),
        width: 854,
        height: 480,
    };
    println!("trace: {} from {}", game.java_description, game.launcher);
    let mut child = game.spawn(root, &launch)?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(900) {
            let _ = child.kill();
            return Err("timed out after 15 minutes".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let log = game_dir.join("launch.log");
    let mut traces: Vec<PathBuf> = std::fs::read_dir(&out)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    traces.sort();
    for t in &traces {
        let text = std::fs::read_to_string(t).map_err(|e| e.to_string())?;
        let count = |k: &str| {
            text.lines()
                .filter(|l| l.contains(&format!("\"k\":\"{k}\"")))
                .count()
        };
        println!(
            "trace: {:<16} {:>4} ticks, {:>4} packets out, {:>4} handled",
            t.file_stem().unwrap_or_default().to_string_lossy(),
            count("tick"),
            count("out"),
            count("handle")
        );
    }
    if !status.success() {
        return Err(format!(
            "the client exited with {status}; see {}",
            log.display()
        ));
    }
    if traces.is_empty() {
        return Err(format!("no traces written; see {}", log.display()));
    }
    println!(
        "trace: {} trace(s) in {} ({:.0} s)",
        traces.len(),
        out.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
