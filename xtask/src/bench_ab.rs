//! `cargo xtask bench ab`: quick A/B comparisons of two Ruthenium builds for day-to-day
//! work (PLAN Stage 1). Each side is built in release (a git revision is exported with
//! `git archive` into `target/bench-ab`, so no worktree is registered; `work` is the
//! working tree), then the two run in turn, A B B A A B B A…, which cancels a linear
//! thermal drift. Every run writes its own summary (`ruthenium --bench-out`); the report
//! gives each metric's median per side, the change, and the spread between a side's
//! rounds, and calls a change no larger than that spread noise. `--a X --b X` (an A/A
//! run) measures the noise itself; `--a-args` and `--b-args` give one side its own
//! options, to compare two settings of one build (`--a work --b-args "--frames-ahead 1"`).
//! While a run measures, its resident memory and GPU memory are sampled from outside.
//!
//! For recorded baselines and stage gates, use the full vanilla protocol (`bench run`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

#[derive(Debug)]
struct Options {
    a: String,
    b: String,
    rounds: u32,
    duration: f64,
    gpu: Option<String>,
    /// Extra arguments for one side's runs.
    a_args: Vec<String>,
    b_args: Vec<String>,
    /// Extra arguments for every run (after `--`).
    extra: Vec<String>,
}

const USAGE: &str = "usage: cargo xtask bench ab [--a <rev>|work] [--b <rev>|work] [--a-args '<options>'] [--b-args '<options>'] [--rounds N] [--duration S] [--gpu <name>] [-- <ruthenium options>]";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        a: "HEAD".into(),
        b: "work".into(),
        rounds: 4,
        duration: 15.0,
        gpu: None,
        a_args: Vec::new(),
        b_args: Vec::new(),
        extra: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            o.extra = it.by_ref().cloned().collect();
            break;
        }
        let value = it
            .next()
            .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?;
        let num = |v: &str| v.parse::<f64>().map_err(|e| format!("{arg}: {e}"));
        match arg.as_str() {
            "--a" => o.a = value.clone(),
            "--b" => o.b = value.clone(),
            "--rounds" => o.rounds = (num(value)? as u32).max(1),
            "--duration" => o.duration = num(value)?.max(2.0),
            "--gpu" => o.gpu = Some(value.clone()),
            "--a-args" => o.a_args = value.split_whitespace().map(str::to_owned).collect(),
            "--b-args" => o.b_args = value.split_whitespace().map(str::to_owned).collect(),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(o)
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let dir = root.join("target/bench-ab");
    conditions();
    let a = build(root, &dir, &o.a)?;
    // The same build twice is one binary: what differs is its options.
    let b = if o.b == o.a {
        a.clone()
    } else {
        build(root, &dir, &o.b)?
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let results = dir.join("results").join(stamp.to_string());
    std::fs::create_dir_all(&results).map_err(|e| format!("{}: {e}", results.display()))?;
    let mut runs: [Vec<Value>; 2] = [Vec::new(), Vec::new()];
    for round in 0..o.rounds {
        // A B, B A, A B, ...
        let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for side in order {
            let (label, bin) = if side == 0 {
                (&a.0, &a.1)
            } else {
                (&b.0, &b.1)
            };
            let out = results.join(format!("{}-{}-{round}.json", ["a", "b"][side], label));
            eprintln!(
                "bench ab: round {}/{}, {} ({label})",
                round + 1,
                o.rounds,
                ["A", "B"][side]
            );
            runs[side].push(run_once(bin, &out, &o, side)?);
        }
    }
    let text = report(&o, &a.0, &b.0, &runs);
    println!("{text}");
    let _ = std::fs::write(results.join("report.md"), &text);
    eprintln!("bench ab: run summaries in {}", results.display());
    Ok(())
}

/// Builds one side and returns its label and binary.
pub(crate) fn build(root: &Path, dir: &Path, rev: &str) -> Result<(String, PathBuf), String> {
    let bins = dir.join("bin");
    std::fs::create_dir_all(&bins).map_err(|e| format!("{}: {e}", bins.display()))?;
    let (label, src, target) = if rev == "work" {
        ("work".to_owned(), root.to_path_buf(), root.join("target"))
    } else {
        let out = Command::new("git")
            .args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
            .current_dir(root)
            .output()
            .map_err(|e| format!("running git: {e}"))?;
        if !out.status.success() {
            return Err(format!("{rev:?} is not a commit"));
        }
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let label = sha[..12].to_owned();
        let src = dir.join("src").join(&label);
        if !src.join("Cargo.toml").exists() {
            std::fs::create_dir_all(&src).map_err(|e| format!("{}: {e}", src.display()))?;
            eprintln!("bench ab: exporting {rev} ({label})");
            let status = Command::new("sh")
                .arg("-c")
                .arg("git archive --format=tar \"$1\" | tar -x -C \"$2\"")
                .args(["sh", &sha])
                .arg(&src)
                .current_dir(root)
                .status()
                .map_err(|e| format!("running git archive: {e}"))?;
            if !status.success() {
                let _ = std::fs::remove_dir_all(&src);
                return Err(format!("could not export {rev}"));
            }
        }
        // One target folder for every exported revision, so rebuilds are incremental.
        (label, src, dir.join("target"))
    };
    eprintln!("bench ab: building {label} (release)");
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "-p", "ruthenium", "--target-dir"])
        .arg(&target)
        .current_dir(&src)
        .status()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building {label} failed"));
    }
    let bin = bins.join(format!("ruthenium-{label}{}", std::env::consts::EXE_SUFFIX));
    let built = target
        .join("release")
        .join(format!("ruthenium{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&built, &bin).map_err(|e| format!("{}: {e}", built.display()))?;
    Ok((label, bin))
}

fn run_once(bin: &Path, out: &Path, o: &Options, side: usize) -> Result<Value, String> {
    // Uncapped, so the numbers are the build's cost rather than the display's rate; the
    // client measures after its own one-second warm-up.
    let mut args: Vec<String> = vec!["--no-vsync".into()];
    args.extend(o.extra.iter().cloned());
    args.extend([&o.a_args, &o.b_args][side].iter().cloned());
    run_client(bin, out, o.duration, o.gpu.as_deref(), &args)
}

/// Which GPU's counters to sample for a client asked to run on `gpu`.
fn sampled_gpu(gpu: Option<&str>) -> crate::gpu::Gpu {
    match gpu.map(str::to_lowercase) {
        Some(g) if g.contains("intel") => crate::gpu::Gpu::Intel,
        Some(g) if g.contains("nvidia") || g.contains("rtx") || g.contains("gtx") => {
            crate::gpu::Gpu::Nvidia
        }
        _ => crate::gpu::Gpu::Auto,
    }
}

/// How long a run goes unsampled: its start-up and the client's own warm-up.
const SAMPLE_AFTER: Duration = Duration::from_millis(2500);

/// Runs the client for `duration` measured seconds with `args`, sampling its memory from
/// outside meanwhile, and returns its summary with the samples' statistics under
/// `"sampled"` (also written back to `out`).
pub(crate) fn run_client(
    bin: &Path,
    out: &Path,
    duration: f64,
    gpu: Option<&str>,
    args: &[String],
) -> Result<Value, String> {
    let _ = std::fs::remove_file(out);
    let mut cmd = Command::new(bin);
    cmd.args(["--no-validation", "--quit-after"])
        .arg(format!("{}", duration + 1.5))
        .arg("--bench-out")
        .arg(out);
    if let Some(g) = gpu {
        cmd.args(["--gpu", g]);
    }
    let log = out.with_extension("log");
    let stderr = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut child = cmd
        .args(args)
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    let (pid, started, sampler) = (child.id(), Instant::now(), sampled_gpu(gpu));
    let (mut rss, mut vram, mut temps, mut clocks) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut reasons: Vec<String> = Vec::new();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > Duration::from_secs_f64(duration + 60.0) {
            let _ = child.kill();
            return Err(format!(
                "{} did not quit; see {}",
                bin.display(),
                log.display()
            ));
        }
        if started.elapsed() >= SAMPLE_AFTER {
            let g = sampler.sample(pid);
            rss.extend(crate::bench::rss_kib(pid).map(|k| k as f64 / 1024.0));
            vram.extend(g.mem_mib);
            temps.extend(crate::bench::cpu_temp_c());
            clocks.extend(g.clock_mhz);
            if let Some(r) = g.reasons
                && !reasons.contains(&r)
            {
                reasons.push(r);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        let stderr = std::fs::read_to_string(&log).unwrap_or_default();
        if stderr.contains("unknown option") {
            return Err(format!(
                "{} does not know one of the options (is the revision too old?):\n{stderr}",
                bin.display()
            ));
        }
        return Err(format!("{} failed:\n{stderr}", bin.display()));
    }
    let text = std::fs::read_to_string(out).map_err(|_| {
        format!(
            "{} wrote no summary: is the revision older than --bench-out?",
            bin.display()
        )
    })?;
    let mut v: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", out.display()))?;
    v["sampled"] = json!({
        "rss_mib": stats(&rss),
        "gpu_mib": stats(&vram),
        "cpu_temp_c": stats(&temps),
        "gpu_clock_mhz": stats(&clocks),
        "gpu_reasons": reasons,
        "cpu_mhz": crate::bench::cpu_mhz(),
    });
    std::fs::write(out, format!("{v}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(v)
}

/// The mean and maximum of some samples; null without any.
fn stats(v: &[f64]) -> Value {
    if v.is_empty() {
        return Value::Null;
    }
    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    json!({"mean": v.iter().sum::<f64>() / v.len() as f64, "max": max, "samples": v.len()})
}

/// One reported metric: where it is in a run summary (a series, or a series inside
/// `phases` written `phases/acquire`), and which way is better.
struct Metric {
    name: &'static str,
    series: &'static str,
    field: &'static str,
    /// Report 1000 / value (frames per second) instead of milliseconds.
    rate: bool,
}

const METRICS: &[Metric] = &[
    Metric {
        name: "average FPS",
        series: "frame",
        field: "mean",
        rate: true,
    },
    Metric {
        name: "1% low FPS",
        series: "frame",
        field: "p99",
        rate: true,
    },
    Metric {
        name: "0.1% low FPS",
        series: "frame",
        field: "p999",
        rate: true,
    },
    Metric {
        name: "frame p50 (ms)",
        series: "frame",
        field: "p50",
        rate: false,
    },
    Metric {
        name: "CPU p50 (ms)",
        series: "cpu",
        field: "p50",
        rate: false,
    },
    Metric {
        name: "CPU p99 (ms)",
        series: "cpu",
        field: "p99",
        rate: false,
    },
    Metric {
        name: "GPU p50 (ms)",
        series: "gpu_time",
        field: "p50",
        rate: false,
    },
    Metric {
        name: "main-thread work p50 (ms)",
        series: "phases/work",
        field: "p50",
        rate: false,
    },
    Metric {
        name: "main-thread work p99 (ms)",
        series: "phases/work",
        field: "p99",
        rate: false,
    },
    Metric {
        name: "governor wait p50 (ms)",
        series: "phases/governor",
        field: "p50",
        rate: false,
    },
    Metric {
        name: "acquire p99 (ms)",
        series: "phases/acquire",
        field: "p99",
        rate: false,
    },
    Metric {
        name: "present p99 (ms)",
        series: "phases/present",
        field: "p99",
        rate: false,
    },
    Metric {
        name: "RSS mean (MiB)",
        series: "sampled/rss_mib",
        field: "mean",
        rate: false,
    },
    Metric {
        name: "GPU memory mean (MiB)",
        series: "sampled/gpu_mib",
        field: "mean",
        rate: false,
    },
];

fn value(run: &Value, m: &Metric) -> Option<f64> {
    let series = m.series.split('/').try_fold(run, |v, key| v.get(key))?;
    let v = series.get(m.field)?.as_f64()?;
    if m.rate {
        (v > 0.0).then(|| 1000.0 / v)
    } else {
        Some(v)
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// (max - min) / median, as a fraction.
fn spread(v: &[f64]) -> f64 {
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let m = median(v.to_vec());
    if m.abs() > 0.0 {
        (hi - lo) / m.abs()
    } else {
        0.0
    }
}

fn report(o: &Options, a: &str, b: &str, runs: &[Vec<Value>; 2]) -> String {
    let gpu = runs[0]
        .first()
        .and_then(|r| r.get("gpu")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    let side = |rev: &str, label: &str, args: &[String]| {
        if args.is_empty() {
            format!("{rev} ({label})")
        } else {
            format!("{rev} ({label}) with `{}`", args.join(" "))
        }
    };
    let mut s = format!(
        "## bench ab: A = {}, B = {}\n\n{} rounds of {:.0} s each, ABBA order, on {gpu}{}.\n\n",
        side(&o.a, a, &o.a_args),
        side(&o.b, b, &o.b_args),
        o.rounds,
        o.duration,
        run_line(&runs[0])
    );
    s += "| metric | A | B | change | A spread | B spread | verdict |\n|---|---|---|---|---|---|---|\n";
    for m in METRICS {
        let va: Vec<f64> = runs[0].iter().filter_map(|r| value(r, m)).collect();
        let vb: Vec<f64> = runs[1].iter().filter_map(|r| value(r, m)).collect();
        if va.is_empty() && vb.is_empty() {
            continue;
        }
        let (ma, mb) = (median(va.clone()), median(vb.clone()));
        let change = (mb - ma) / ma;
        let noise = spread(&va).max(spread(&vb));
        let better = if m.rate { change > 0.0 } else { change < 0.0 };
        let verdict = if !change.is_finite() {
            "n/a"
        } else if change.abs() <= noise {
            "noise"
        } else if better {
            "B better"
        } else {
            "B worse"
        };
        s += &format!(
            "| {} | {ma:.3} | {mb:.3} | {:+.1}% | {:.1}% | {:.1}% | {verdict} |\n",
            m.name,
            change * 100.0,
            spread(&va) * 100.0,
            spread(&vb) * 100.0
        );
    }
    s
}

/// What side A's first run says it ran as: present mode, frames ahead, window.
fn run_line(runs: &[Value]) -> String {
    let Some(r) = runs.first() else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(m) = r["present_mode"].as_str() {
        s += &format!(", A presenting {m}");
    }
    if let Some(n) = r["frames_ahead"].as_u64() {
        s += &format!(" with {n} frame(s) ahead");
    }
    if let (Some(w), Some(h)) = (
        r["swapchain"]["width"].as_u64(),
        r["swapchain"]["height"].as_u64(),
    ) {
        s += &format!(", {w} × {h} pixels");
    }
    s
}

/// Warns about what makes numbers incomparable (the user's laptop: power profile, other
/// games or servers, heat). Never blocks.
pub(crate) fn conditions() {
    if let Ok(p) = std::fs::read_to_string("/sys/firmware/acpi/platform_profile") {
        let p = p.trim();
        if p != "performance" {
            eprintln!("bench ab: warning: platform profile is {p:?}, not \"performance\"");
        } else {
            eprintln!("bench ab: platform profile {p}");
        }
    }
    if let Ok(out) = Command::new("pgrep").args(["-c", "java"]).output() {
        let n = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if n != "0" && !n.is_empty() {
            eprintln!("bench ab: warning: {n} Java process(es) running (a game or server?)");
        }
    }
    if let Some(t) = cpu_package_temp() {
        let note = if t >= 85.0 { " (hot: warning)" } else { "" };
        eprintln!("bench ab: CPU package {t:.0} °C{note}");
    }
}

/// The CPU package temperature from coretemp or k10temp, in °C.
fn cpu_package_temp() -> Option<f64> {
    for hw in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let name = std::fs::read_to_string(hw.path().join("name")).unwrap_or_default();
        if !matches!(name.trim(), "coretemp" | "k10temp") {
            continue;
        }
        // temp1 is the package (coretemp) or Tctl (k10temp).
        let milli: f64 = std::fs::read_to_string(hw.path().join("temp1_input"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        return Some(milli / 1000.0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medians_and_spreads() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(vec![4.0, 1.0, 2.0, 3.0]), 2.5);
        assert!((spread(&[9.0, 10.0, 11.0]) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn rates_are_inverted_times() {
        let run: Value = serde_json::from_str(r#"{"frame": {"mean": 4.0, "p99": 8.0}}"#).unwrap();
        assert_eq!(value(&run, &METRICS[0]), Some(250.0));
        assert_eq!(value(&run, &METRICS[1]), Some(125.0));
        assert_eq!(value(&run, &METRICS[4]), None);
        let run: Value = serde_json::from_str(
            r#"{"phases": {"work": {"p50": 0.25}}, "sampled": {"rss_mib": null}}"#,
        )
        .unwrap();
        let named = |name: &str| METRICS.iter().find(|m| m.name == name).unwrap();
        assert_eq!(value(&run, named("main-thread work p50 (ms)")), Some(0.25));
        assert_eq!(value(&run, named("RSS mean (MiB)")), None);
    }

    #[test]
    fn options() {
        let args: Vec<String> = ["--a", "HEAD~1", "--rounds", "2", "--", "--gpu", "intel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = parse(&args).unwrap();
        assert_eq!(
            (o.a.as_str(), o.b.as_str(), o.rounds),
            ("HEAD~1", "work", 2)
        );
        assert_eq!(o.extra, vec!["--gpu", "intel"]);
        let args: Vec<String> = ["--a", "work", "--b-args", "--frames-ahead 1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let o = parse(&args).unwrap();
        assert!(o.a_args.is_empty());
        assert_eq!(o.b_args, vec!["--frames-ahead", "1"]);
        assert_eq!(sampled_gpu(Some("Intel")), crate::gpu::Gpu::Intel);
        assert_eq!(sampled_gpu(Some("4060")), crate::gpu::Gpu::Auto);
    }
}
