//! `cargo xtask bench`: baseline measurements of the vanilla 26.3 client.
//!
//! The client runs from the player's own install (see [`crate::game`]) with
//! Fabric Loader and one tiny mod, `tools/java/bench`: a single hook at the
//! end of each frame that drives the scene and records frame intervals. No
//! Fabric API is loaded. From outside, this task samples the process's
//! resident memory (`/proc`) and the GPU's memory, clock and throttling (see
//! [`crate::gpu`]; `--gpu` picks the GPU on a hybrid laptop).
//!
//! - `bench setup` creates the benchmark world from a fixed seed and
//!   pre-generates the fly path; it is kept as a template in `cache/bench`.
//! - `bench run` runs each scene on each backend, every world run on a
//!   fresh copy of the template, and writes one JSON file per run.
//! - `bench report` turns a results folder into Markdown tables.
//! - `bench dedicated` runs the world scenes against a dedicated server instead
//!   (see [`crate::bench_dedicated`]).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::gpu::{self, Gpu};
use crate::{game, mods};

const WORLD: &str = "ruthenium-bench";
const DEFAULT_SEED: i64 = 20_261_121;
const SCENES: &[&str] = &["menu", "idle", "spin", "fly"];
const BACKENDS: &[&str] = &["opengl", "vulkan"];

#[derive(Debug)]
pub(crate) struct Options {
    pub backends: Vec<String>,
    pub scenes: Vec<String>,
    pub reps: u32,
    pub warmup: f64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub seed: i64,
    pub java: Option<PathBuf>,
    pub results: Option<PathBuf>,
    pub screenshot: bool,
    pub render_distance: u32,
    pub gpu: Gpu,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        backends: BACKENDS.iter().map(|s| s.to_string()).collect(),
        scenes: SCENES.iter().map(|s| s.to_string()).collect(),
        reps: 3,
        warmup: 10.0,
        duration: 60.0,
        width: 1920,
        height: 1080,
        seed: DEFAULT_SEED,
        java: None,
        results: None,
        screenshot: false,
        render_distance: 12,
        gpu: Gpu::Auto,
    };
    let list = |v: &str| v.split(',').map(str::to_owned).collect::<Vec<_>>();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--screenshot" {
            o.screenshot = true;
            continue;
        }
        let value = it.next().ok_or_else(|| format!("{arg} needs a value"))?;
        let num = |v: &str| v.parse::<f64>().map_err(|e| format!("{arg}: {e}"));
        match arg.as_str() {
            "--backend" => o.backends = list(value),
            "--scene" => o.scenes = list(value),
            "--reps" => o.reps = num(value)? as u32,
            "--warmup" => o.warmup = num(value)?,
            "--duration" => o.duration = num(value)?,
            "--width" => o.width = num(value)? as u32,
            "--height" => o.height = num(value)? as u32,
            "--seed" => o.seed = num(value)? as i64,
            "--render-distance" => o.render_distance = num(value)? as u32,
            "--java" => o.java = Some(PathBuf::from(value)),
            "--results" => o.results = Some(PathBuf::from(value)),
            "--gpu" => o.gpu = Gpu::parse(value)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if let Some(b) = o.backends.iter().find(|b| !BACKENDS.contains(&b.as_str())) {
        return Err(format!("unknown backend {b:?} (opengl, vulkan)"));
    }
    if let Some(s) = o.scenes.iter().find(|s| !SCENES.contains(&s.as_str())) {
        return Err(format!("unknown scene {s:?} ({})", SCENES.join(", ")));
    }
    Ok(o)
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("setup") => parse(&args[1..]).and_then(|o| setup(root, &o)),
        Some("run") => parse(&args[1..]).and_then(|o| run_all(root, &o)),
        Some("report") => parse_report(&args[1..])
            .and_then(|(dir, with, baseline)| report(root, dir, with, baseline)),
        Some("dedicated") => crate::bench_dedicated::run(root, &args[1..]),
        // Internal: keeps a dedicated server from outliving the task that started it.
        Some("supervise") => crate::server::supervise(&args[1..]),
        Some("ab") => crate::bench_ab::run(root, &args[1..]),
        Some("menu") => crate::bench_menu::run(root, &args[1..]),
        _ => Err("usage: cargo xtask bench <setup|run|report|dedicated|ab|menu> [options]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bench: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What `bench report` was given: the folder, and those of `--compare` and `--baseline`.
type ReportFolders = (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>);

/// `bench report [folder] [--compare <folder>] [--baseline <folder>]`.
fn parse_report(args: &[String]) -> Result<ReportFolders, String> {
    let (mut dir, mut compare, mut baseline) = (None, None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--compare" {
            compare = Some(PathBuf::from(it.next().ok_or("--compare needs a folder")?));
        } else if arg == "--baseline" {
            baseline = Some(PathBuf::from(it.next().ok_or("--baseline needs a folder")?));
        } else if dir.is_none() && !arg.starts_with("--") {
            dir = Some(PathBuf::from(arg));
        } else {
            return Err(format!("unknown argument {arg:?}"));
        }
    }
    Ok((dir, compare, baseline))
}

pub(crate) fn bench_dir(root: &Path) -> PathBuf {
    root.join("cache/bench")
}

/// Vanilla's default graphics preset (Fancy, which the game applies at startup
/// unless the preset is "custom"), written out so render distance can be 12
/// (the plan's gates) instead of Fancy's 16. Otherwise only what a repeatable
/// measurement needs: no VSync or frame cap, no pausing or throttling when the
/// window loses focus, sound muted, no first-launch screens.
pub(crate) fn options(backend: &str, render_distance: u32) -> Vec<(&'static str, String)> {
    let fancy = [
        ("ao", "true"),
        ("biomeBlendRadius", "2"),
        ("cutoutLeaves", "true"),
        ("entityDistanceScaling", "1.0"),
        ("entityShadows", "true"),
        ("prioritizeChunkUpdates", "1"),
        ("maxAnisotropyBit", "1"),
        ("textureFiltering", "1"),
        ("improvedTransparency", "false"),
        ("mipmapLevels", "4"),
        ("particles", "0"),
        ("renderClouds", "\"true\""),
        ("cloudRange", "64"),
        ("simulationDistance", "12"),
        ("weatherRadius", "10"),
        ("menuBackgroundBlurriness", "5"),
    ];
    let mut o: Vec<(&'static str, String)> =
        fancy.iter().map(|(k, v)| (*k, v.to_string())).collect();
    o.extend([
        ("version", "5023".into()),
        ("graphicsPreset", "\"custom\"".into()),
        ("preferredGraphicsBackend", format!("\"{backend}\"")),
        ("renderDistance", render_distance.to_string()),
        ("enableVsync", "false".into()),
        ("maxFps", "260".into()),
        ("fullscreen", "false".into()),
        ("pauseOnLostFocus", "false".into()),
        ("inactivityFpsLimit", "\"minimized\"".into()),
        ("onboardAccessibility", "false".into()),
        ("tutorialStep", "none".into()),
        ("joinedFirstServer", "true".into()),
        ("skipMultiplayerWarning", "true".into()),
        ("soundCategory_master", "0.0".into()),
    ]);
    o
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

pub(crate) struct Run<'a> {
    pub scene: &'a str,
    pub backend: &'a str,
    pub out: PathBuf,
    pub quick_play: bool,
    /// A dedicated server's address and the route file to play there: the client joins
    /// it instead of loading a world of its own.
    pub dedicated: Option<(String, PathBuf)>,
    /// Called with every resource sample, twice a second, while the client runs; an
    /// error stops the client.
    pub watch: Option<&'a mut dyn FnMut() -> Result<(), String>>,
}

/// One client launch. Returns the mod's JSON plus resource samples.
pub(crate) fn launch(
    root: &Path,
    game: &game::Game,
    jar: &Path,
    o: &Options,
    run: &mut Run<'_>,
) -> Result<Value, String> {
    let game_dir = bench_dir(root).join("game");
    game::write_options(&game_dir, &options(run.backend, o.render_distance))?;
    let _ = std::fs::remove_file(&run.out);
    let mut game_args = Vec::new();
    if run.quick_play {
        let saves = game_dir.join("saves");
        let _ = std::fs::remove_dir_all(saves.join(WORLD));
        copy_dir(&bench_dir(root).join("world"), &saves.join(WORLD))?;
        game_args = vec!["--quickPlaySingleplayer".into(), WORLD.into()];
    }
    let out = std::path::absolute(&run.out).map_err(|e| e.to_string())?;
    let mut jvm_args = vec![
        format!("-Druthenium.bench.scene={}", run.scene),
        format!("-Druthenium.bench.out={}", out.display()),
        format!("-Druthenium.bench.warmup={}", o.warmup),
        format!("-Druthenium.bench.duration={}", o.duration),
        format!("-Druthenium.bench.world={WORLD}"),
        format!("-Druthenium.bench.seed={}", o.seed),
        format!("-Druthenium.bench.screenshot={}", o.screenshot),
    ];
    if let Some((address, route)) = &run.dedicated {
        let route = std::path::absolute(route).map_err(|e| e.to_string())?;
        jvm_args.push(format!("-Druthenium.bench.route={}", route.display()));
        game_args = vec!["--quickPlayMultiplayer".into(), address.clone()];
    }
    let launch = game::Launch {
        game_dir: game_dir.clone(),
        mods: vec![jar.to_owned()],
        jvm_args,
        game_args,
        env: o.gpu.env()?,
        width: o.width,
        height: o.height,
    };
    let mut child = game.spawn(root, &launch)?;
    let pid = child.id();
    let limit = Duration::from_secs_f64(o.warmup + o.duration)
        + Duration::from_secs(if run.scene == "setup" { 1800 } else { 300 });
    let start = Instant::now();
    let mut samples = Vec::new();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            return Err(format!(
                "{} timed out; see {}",
                run.scene,
                game_dir.join("launch.log").display()
            ));
        }
        if let Some(watch) = run.watch.as_mut()
            && let Err(e) = watch()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let (rss, g) = (rss_kib(pid), o.gpu.sample(pid));
        samples.push(json!([
            epoch_ms(),
            rss,
            g.mem_mib,
            cpu_temp_c(),
            cpu_mhz(),
            g.temp_c,
            g.reasons,
            g.clock_mhz
        ]));
        std::thread::sleep(Duration::from_millis(500));
    };
    let text = std::fs::read_to_string(&out).map_err(|_| {
        format!(
            "{} wrote no results ({status}); see {}",
            run.scene,
            game_dir.join("launch.log").display()
        )
    })?;
    let mut v: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", out.display()))?;
    let (vendor, name) = (
        v["device"]["vendor"].as_str().unwrap_or(""),
        v["device"]["name"].as_str().unwrap_or(""),
    );
    if !o.gpu.matches(vendor, name) {
        return Err(format!(
            "{} {}: asked for the {} GPU, but the game used {name:?} ({vendor})",
            run.backend,
            run.scene,
            o.gpu.name()
        ));
    }
    v["backendRequested"] = json!(run.backend);
    v["gpuRequested"] = json!(o.gpu.name());
    v["samples"] = Value::Array(samples);
    std::fs::write(&out, serde_json::to_string(&v).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(v)
}

pub(crate) fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub(crate) fn rss_kib(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// The CPU package temperature (`x86_pkg_temp`, else `TCPU`).
pub(crate) fn cpu_temp_c() -> Option<f64> {
    let mut fallback = None;
    for entry in std::fs::read_dir("/sys/class/thermal").ok()?.flatten() {
        let kind = read_trim(&entry.path().join("type").to_string_lossy());
        let temp = read_trim(&entry.path().join("temp").to_string_lossy())
            .and_then(|t| t.parse::<f64>().ok())
            .map(|t| t / 1000.0);
        match kind.as_deref() {
            Some("x86_pkg_temp") => return temp,
            Some("TCPU") => fallback = temp,
            _ => {}
        }
    }
    fallback
}

/// The mean current clock over all CPUs.
pub(crate) fn cpu_mhz() -> Option<f64> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mhz: Vec<f64> = info
        .lines()
        .filter(|l| l.starts_with("cpu MHz"))
        .filter_map(|l| l.split(':').nth(1)?.trim().parse().ok())
        .collect();
    mean(&mhz).map(|m| m.round())
}

pub(crate) fn prepare(root: &Path, o: &Options) -> Result<(game::Game, PathBuf), String> {
    o.gpu.env()?; // fail before any results are written if the GPU isn't there
    let game = game::locate(o.java.as_deref())?;
    println!("bench: {} from {}", game.java_description, game.launcher);
    let jar = mods::build(root, "bench", &[])?;
    Ok((game, jar))
}

fn setup(root: &Path, o: &Options) -> Result<(), String> {
    let (game, jar) = prepare(root, o)?;
    let saves = bench_dir(root).join("game/saves");
    let _ = std::fs::remove_dir_all(saves.join(WORLD));
    println!(
        "bench: creating the world (seed {}) and pre-generating the fly path",
        o.seed
    );
    let out = bench_dir(root).join("setup.json");
    let v = launch(
        root,
        &game,
        &jar,
        o,
        &mut Run {
            scene: "setup",
            backend: "opengl",
            out,
            quick_play: false,
            dedicated: None,
            watch: None,
        },
    )?;
    let template = bench_dir(root).join("world");
    let _ = std::fs::remove_dir_all(&template);
    copy_dir(&saves.join(WORLD), &template)?;
    println!(
        "bench: world template in {} (spawn {})",
        template.display(),
        v["spawn"]
    );
    Ok(())
}

fn run_all(root: &Path, o: &Options) -> Result<(), String> {
    if o.scenes.iter().any(|s| s != "menu") && !bench_dir(root).join("world").is_dir() {
        return Err("no benchmark world: run `cargo xtask bench setup` first".into());
    }
    let (game, jar) = prepare(root, o)?;
    let dir = o.results.clone().unwrap_or_else(|| {
        bench_dir(root)
            .join("results")
            .join(format!("{}", epoch_ms() / 1000))
    });
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("machine.json"), machine(&game, o).to_string())
        .map_err(|e| e.to_string())?;
    for rep in 1..=o.reps {
        for backend in &o.backends {
            for scene in &o.scenes {
                let out = dir.join(format!("{backend}-{scene}-{rep}.json"));
                let mut run = Run {
                    scene,
                    backend,
                    out,
                    quick_play: scene != "menu",
                    dedicated: None,
                    watch: None,
                };
                let v = launch(root, &game, &jar, o, &mut run)?;
                let s = summarize(&v);
                println!(
                    "bench: {backend:<6} {scene:<4} #{rep}: {:.0} fps avg, 1% low {:.0}, 0.1% low {:.0}, RSS {} MiB, VRAM {} MiB ({})",
                    s["avgFps"].as_f64().unwrap_or(0.0),
                    s["low1Fps"].as_f64().unwrap_or(0.0),
                    s["low01Fps"].as_f64().unwrap_or(0.0),
                    s["rssMeanMiB"],
                    s["vramMeanMiB"],
                    v["device"]["backend"].as_str().unwrap_or("?"),
                );
            }
        }
    }
    println!("bench: results in {}", dir.display());
    report(root, Some(dir), None, None)
}

fn command_line(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// What the numbers depend on, recorded next to them.
pub(crate) fn machine(game: &game::Game, o: &Options) -> Value {
    json!({
        "cpu": command_line("sh", &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"]),
        "cpus": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "memory": command_line("sh", &["-c", "grep MemTotal /proc/meminfo"]),
        "kernel": command_line("uname", &["-r"]),
        "os": command_line("sh", &["-c", ". /etc/os-release && echo $PRETTY_NAME"]),
        "governor": read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        "platformProfile": read_trim("/sys/firmware/acpi/platform_profile"),
        "powerProfile": command_line("powerprofilesctl", &["get"]),
        "gpu": command_line("nvidia-smi", &["--query-gpu=name,driver_version,memory.total,power.limit", "--format=csv,noheader"]),
        "gpuRequested": o.gpu.name(),
        "drmCards": gpu::drm_cards()
            .iter()
            .map(|c| format!("{} {}:{} {}", c.pdev, c.vendor, c.device, c.driver))
            .collect::<Vec<_>>(),
        "session": std::env::var("XDG_SESSION_TYPE").ok(),
        "desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
        "java": game.java_description,
        "launcher": game.launcher,
    })
}

/// Nearest-rank percentile of sorted values, `p` in percent with at most one
/// decimal (computed in integers: `0.999 * 1000` is not 999 in floating point).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let per_mille = (p * 10.0).round() as usize;
    let rank = (per_mille * sorted.len()).div_ceil(1000);
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Frame-time statistics. 1% and 0.1% lows are 1000 / P99 and 1000 / P99.9 of
/// the frame time in milliseconds (the PresentMon / CapFrameX convention).
pub(crate) fn summarize(v: &Value) -> Value {
    let ms: Vec<f64> = v["frameIntervalsNs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_f64)
                .map(|n| n / 1e6)
                .collect()
        })
        .unwrap_or_default();
    let mut sorted = ms.clone();
    sorted.sort_by(f64::total_cmp);
    let total_s = ms.iter().sum::<f64>() / 1000.0;
    let (start, end) = (
        v["measureStartEpochMs"].as_f64().unwrap_or(0.0),
        v["measureEndEpochMs"].as_f64().unwrap_or(0.0),
    );
    let window: Vec<&Value> = v["samples"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|s| s[0].as_f64().is_some_and(|t| t >= start && t <= end))
                .collect()
        })
        .unwrap_or_default();
    let col = |i: usize, scale: f64| -> Vec<f64> {
        window
            .iter()
            .filter_map(|s| s[i].as_f64())
            .map(|x| x * scale)
            .collect()
    };
    let rss = col(1, 1.0 / 1024.0);
    let vram = col(2, 1.0);
    let cpu_temp = col(3, 1.0);
    let cpu_mhz = col(4, 1.0);
    let gpu_temp = col(5, 1.0);
    let gpu_clock = col(7, 1.0);
    let slowdown: Vec<f64> = window
        .iter()
        .filter_map(|s| s[6].as_str())
        .filter_map(gpu::is_slowdown)
        .map(|slow| f64::from(u8::from(slow)))
        .collect();
    let max = |v: &[f64]| {
        v.iter()
            .copied()
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
    };
    let secs = |a: &str| {
        let (t, j) = (
            v[a].as_f64().unwrap_or(-1.0),
            v["jvmStartEpochMs"].as_f64().unwrap_or(0.0),
        );
        (t > 0.0).then(|| (t - j) / 1000.0)
    };
    json!({
        "frames": ms.len(),
        "seconds": total_s,
        "avgFps": ms.len() as f64 / total_s,
        "medianMs": percentile(&sorted, 50.0),
        "p99Ms": percentile(&sorted, 99.0),
        "p999Ms": percentile(&sorted, 99.9),
        "maxMs": sorted.last(),
        "low1Fps": 1000.0 / percentile(&sorted, 99.0),
        "low01Fps": 1000.0 / percentile(&sorted, 99.9),
        "rssMeanMiB": mean(&rss).map(|x| x.round()),
        "rssMaxMiB": max(&rss).map(|x| x.round()),
        "vramMeanMiB": mean(&vram).map(|x| x.round()),
        "vramMaxMiB": max(&vram).map(|x| x.round()),
        "firstFrameS": secs("firstFrameEpochMs"),
        "menuReadyS": secs("menuReadyEpochMs"),
        "worldReadyS": secs("worldReadyEpochMs"),
        "cpuTempMaxC": max(&cpu_temp),
        "cpuMhzMean": mean(&cpu_mhz).map(|x| x.round()),
        "gpuTempMaxC": max(&gpu_temp),
        "gpuClockMeanMhz": mean(&gpu_clock).map(|x| x.round()),
        "gpuSlowdownPct": mean(&slowdown).map(|x| (x * 100.0).round()),
        "gcCount": v["gcCount"],
        "gcTimeMs": v["gcTimeMs"],
    })
}

/// A whole number, or a dash for a value that wasn't measured (an
/// integrated GPU has no temperature sensor of its own).
pub(crate) fn dash(x: f64) -> String {
    if x.is_nan() {
        "–".into()
    } else {
        format!("{x:.0}")
    }
}

pub(crate) fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        f64::NAN
    } else {
        v[v.len() / 2]
    }
}

/// A run in a results folder: backend, scene, repetition, summary and the run's file.
pub(crate) type Row = (String, String, u32, Value, Value);

/// The client runs in a results folder (`<backend>-<scene>-<rep>.json` with frame
/// intervals), sorted by backend, scene and repetition.
pub(crate) fn load_rows(dir: &Path) -> Result<Vec<Row>, String> {
    let mut rows: Vec<Row> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let parts: Vec<&str> = stem.split('-').collect();
        let [backend, scene, rep] = parts[..] else {
            continue;
        };
        let Ok(rep) = rep.parse::<u32>() else {
            continue;
        };
        let text = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
        // A headless route check has no frames to summarise.
        if !v["frameIntervalsNs"].is_array() {
            continue;
        }
        rows.push((backend.into(), scene.into(), rep, summarize(&v), v));
    }
    rows.sort_by(|a, b| {
        let key = |r: &Row| {
            (
                r.0.clone(),
                SCENES.iter().position(|s| *s == r.1).unwrap_or(9),
                r.2,
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(rows)
}

/// The results folders, oldest name first.
pub(crate) fn result_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let results = bench_dir(root).join("results");
    let mut runs: Vec<PathBuf> = std::fs::read_dir(&results)
        .map_err(|e| format!("{}: {e}", results.display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    runs.sort();
    Ok(runs)
}

/// Prints Markdown: one row per backend and scene (median over repetitions),
/// then every run. A folder of dedicated-server runs is printed by
/// [`crate::bench_dedicated::report`], next to the integrated-server runs of `compare`
/// and, for Ruthenium's runs, the recorded vanilla runs of `baseline`.
pub(crate) fn report(
    root: &Path,
    dir: Option<PathBuf>,
    compare: Option<PathBuf>,
    baseline: Option<PathBuf>,
) -> Result<(), String> {
    let dir = match dir {
        Some(d) => d,
        None => result_dirs(root)?.pop().ok_or("no results yet")?,
    };
    let rows = load_rows(&dir)?;
    if crate::bench_dedicated::is_dedicated(&dir) {
        return crate::bench_dedicated::report(root, &dir, &rows, compare, baseline);
    }
    if compare.is_some() || baseline.is_some() {
        return Err("--compare and --baseline are for a folder of dedicated-server runs".into());
    }
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(f64::NAN);
    let mut devices: Vec<String> = rows
        .iter()
        .map(|r| {
            let d = &r.4["device"];
            format!(
                "{} → {} ({})",
                r.0,
                d["name"].as_str().unwrap_or("?"),
                d["driver"].as_str().unwrap_or("?")
            )
        })
        .collect();
    devices.sort();
    devices.dedup();
    println!("\nDevices: {}", devices.join("; "));
    println!(
        "\n| Backend | Scene | Avg FPS | Median ms | 1% low FPS | 0.1% low FPS | Max ms | RSS MiB (mean / max) | VRAM MiB (mean / max) | GC (n / ms) | CPU / GPU °C max | GPU slowdown % | Runs |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut groups: Vec<(String, String)> =
        rows.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
    groups.dedup();
    for (backend, scene) in &groups {
        let g: Vec<&Value> = rows
            .iter()
            .filter(|r| &r.0 == backend && &r.1 == scene)
            .map(|r| &r.3)
            .collect();
        let m = |k: &str| median(g.iter().map(|s| f(s, k)).collect());
        let actual = rows
            .iter()
            .find(|r| &r.0 == backend && &r.1 == scene)
            .and_then(|r| r.4["device"]["backend"].as_str())
            .unwrap_or("?");
        println!(
            "| {backend} ({actual}) | {scene} | {:.0} | {:.2} | {:.0} | {:.0} | {:.1} | {:.0} / {:.0} | {:.0} / {:.0} | {:.0} / {:.0} | {:.0} / {} | {:.0} | {} |",
            m("avgFps"),
            m("medianMs"),
            m("low1Fps"),
            m("low01Fps"),
            m("maxMs"),
            m("rssMeanMiB"),
            m("rssMaxMiB"),
            m("vramMeanMiB"),
            m("vramMaxMiB"),
            m("gcCount"),
            m("gcTimeMs"),
            m("cpuTempMaxC"),
            dash(m("gpuTempMaxC")),
            m("gpuSlowdownPct"),
            g.len()
        );
    }
    let launches: Vec<&Value> = rows
        .iter()
        .filter(|r| r.1 == "menu")
        .map(|r| &r.3)
        .collect();
    if !launches.is_empty() {
        println!(
            "\nLaunch (median of {} menu runs): first frame {:.2} s, main menu ready {:.2} s after JVM start.",
            launches.len(),
            median(launches.iter().map(|s| f(s, "firstFrameS")).collect()),
            median(launches.iter().map(|s| f(s, "menuReadyS")).collect())
        );
    }
    let worlds: Vec<&Value> = rows
        .iter()
        .filter(|r| r.1 != "menu")
        .map(|r| &r.3)
        .collect();
    if !worlds.is_empty() {
        println!(
            "World load (median of {} quick-play runs): world ready {:.2} s after JVM start.",
            worlds.len(),
            median(worlds.iter().map(|s| f(s, "worldReadyS")).collect())
        );
    }
    println!("\n| Run | Avg FPS | 1% low | 0.1% low | Max ms | RSS mean MiB | VRAM mean MiB |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for (backend, scene, rep, s, _) in &rows {
        println!(
            "| {backend} {scene} #{rep} | {:.0} | {:.0} | {:.0} | {:.1} | {:.0} | {:.0} |",
            f(s, "avgFps"),
            f(s, "low1Fps"),
            f(s, "low01Fps"),
            f(s, "maxMs"),
            f(s, "rssMeanMiB"),
            f(s, "vramMeanMiB")
        );
    }
    println!("\n(results: {})", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lows_use_nearest_rank_percentiles() {
        let mut ns: Vec<u64> = vec![4_000_000; 998];
        ns.push(20_000_000);
        ns.push(40_000_000);
        let v = json!({ "frameIntervalsNs": ns, "measureStartEpochMs": 0, "measureEndEpochMs": 1 });
        let s = summarize(&v);
        assert_eq!(s["frames"], 1000);
        assert_eq!(s["p99Ms"].as_f64(), Some(4.0));
        assert_eq!(s["p999Ms"].as_f64(), Some(20.0));
        assert_eq!(s["low01Fps"].as_f64(), Some(50.0));
    }
}
