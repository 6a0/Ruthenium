//! `cargo xtask bench ab`: quick A/B comparisons of two Ruthenium builds for day-to-day
//! work (PLAN Stage 1). Each side is built in release (a git revision is exported with
//! `git archive` into `target/bench-ab`, so no worktree is registered; `work` is the
//! working tree), then the two run in turn, A B B A A B B A…, which cancels a linear
//! thermal drift. Every run writes its own summary (`ruthenium --bench-out`); the report
//! gives each metric's median per side, the change, and the spread between a side's
//! rounds, and calls a change no larger than that spread noise. `--a X --b X` (an A/A
//! run) measures the noise itself.
//!
//! For recorded baselines and stage gates, use the full vanilla protocol (`bench run`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[derive(Debug)]
struct Options {
    a: String,
    b: String,
    rounds: u32,
    duration: f64,
    gpu: Option<String>,
    /// Extra arguments for every run (after `--`).
    extra: Vec<String>,
}

const USAGE: &str = "usage: cargo xtask bench ab [--a <rev>] [--b <rev>|work] [--rounds N] [--duration S] [--gpu <name>] [-- <ruthenium options>]";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        a: "HEAD".into(),
        b: "work".into(),
        rounds: 4,
        duration: 15.0,
        gpu: None,
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
    let b = build(root, &dir, &o.b)?;
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
            runs[side].push(run_once(bin, &out, &o)?);
        }
    }
    println!("{}", report(&o, &a.0, &b.0, &runs));
    eprintln!("bench ab: run summaries in {}", results.display());
    Ok(())
}

/// Builds one side and returns its label and binary.
fn build(root: &Path, dir: &Path, rev: &str) -> Result<(String, PathBuf), String> {
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

fn run_once(bin: &Path, out: &Path, o: &Options) -> Result<Value, String> {
    let mut cmd = Command::new(bin);
    // Uncapped, so the numbers are the build's cost rather than the display's rate; the
    // client measures after its own one-second warm-up.
    cmd.args(["--no-vsync", "--no-validation", "--quit-after"])
        .arg(format!("{}", o.duration + 1.5))
        .arg("--bench-out")
        .arg(out);
    if let Some(g) = &o.gpu {
        cmd.args(["--gpu", g]);
    }
    cmd.args(&o.extra);
    let output = cmd
        .output()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("unknown option --bench-out") {
            return Err(format!(
                "{} has no --bench-out: compare revisions from the commit that added \
                 `bench ab` on",
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
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", out.display()))
}

/// One reported metric: where it is in a run summary, and which way is better.
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
];

fn value(run: &Value, m: &Metric) -> Option<f64> {
    let v = run.get(m.series)?.get(m.field)?.as_f64()?;
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
    let mut s = format!(
        "## bench ab: A = {} ({a}), B = {} ({b})\n\n{} rounds of {:.0} s each, ABBA order, uncapped, on {gpu}.\n\n",
        o.a, o.b, o.rounds, o.duration
    );
    s += "| metric | A | B | change | A spread | B spread | verdict |\n|---|---|---|---|---|---|---|\n";
    for m in METRICS {
        let va: Vec<f64> = runs[0].iter().filter_map(|r| value(r, m)).collect();
        let vb: Vec<f64> = runs[1].iter().filter_map(|r| value(r, m)).collect();
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

/// Warns about what makes numbers incomparable (the user's laptop: power profile, other
/// games or servers, heat). Never blocks.
fn conditions() {
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
    }
}
