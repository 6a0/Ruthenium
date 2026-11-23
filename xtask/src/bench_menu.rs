//! `cargo xtask bench menu`: what Ruthenium's own window and menus cost, before a world
//! is drawn: launch times with and without the pipeline cache, memory with an empty
//! window and with the menus at a fixed pixel size, and frame pacing with VSync. The
//! working tree is built in release and run as `bench ab` runs it; every run's summary
//! and the report go to `target/bench-menu/<time>`.
//!
//! Throughput comparisons (uncapped, A/A, one setting against another) are `bench ab`'s.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::bench_ab::{build, conditions, run_client};

#[derive(Debug)]
struct Options {
    gpu: Option<String>,
    /// Seconds each memory and pacing run measures.
    duration: f64,
    /// Launches per row of the launch table.
    launches: u32,
    /// The window's size in pixels for the memory and pacing runs.
    pixels: (u32, u32),
    /// Extra arguments for every run (after `--`).
    extra: Vec<String>,
}

const USAGE: &str = "usage: cargo xtask bench menu [--gpu <name>] [--duration S] [--launches N] [--pixels WxH] [-- <ruthenium options>]";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        gpu: None,
        duration: 30.0,
        launches: 5,
        pixels: (1920, 1080),
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
            "--gpu" => o.gpu = Some(value.clone()),
            "--duration" => o.duration = num(value)?.max(2.0),
            "--launches" => o.launches = (num(value)? as u32).max(1),
            "--pixels" => {
                o.pixels = value
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or_else(|| format!("--pixels: not a size like 1920x1080: {value}"))?;
            }
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(o)
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    conditions();
    let (label, bin) = build(root, &root.join("target/bench-ab"), "work")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let results = root.join("target/bench-menu").join(stamp.to_string());
    std::fs::create_dir_all(&results).map_err(|e| format!("{}: {e}", results.display()))?;
    let with = |args: &[&str]| -> Vec<String> {
        args.iter()
            .map(|s| (*s).to_owned())
            .chain(o.extra.iter().cloned())
            .collect()
    };

    // Launches: long enough for the menus' text, no longer.
    let mut launches: [Vec<Value>; 2] = [Vec::new(), Vec::new()];
    for (kind, (name, args)) in [
        ("compiled", &["--no-pipeline-cache"][..]),
        ("cached", &[][..]),
    ]
    .into_iter()
    .enumerate()
    {
        for i in 0..o.launches {
            eprintln!(
                "bench menu: launch {}/{}, pipelines {name}",
                i + 1,
                o.launches
            );
            let out = results.join(format!("launch-{name}-{i}.json"));
            launches[kind].push(run_client(&bin, &out, 2.0, o.gpu.as_deref(), &with(args))?);
        }
    }

    // The window size in points that gives the wanted pixels on this display.
    let scale = launches[0]
        .first()
        .and_then(|r| Some(num(r, "window/pixels/0")? / num(r, "window/points/0")?))
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(1.0);
    let size = format!(
        "{}x{}",
        (f64::from(o.pixels.0) / scale).round(),
        (f64::from(o.pixels.1) / scale).round()
    );
    let mut scenes = Vec::new();
    for (name, args) in [
        (
            "empty",
            &["--vsync", "--window-size", size.as_str(), "--empty-window"][..],
        ),
        ("menu", &["--vsync", "--window-size", size.as_str()][..]),
    ] {
        eprintln!("bench menu: {name} window, {:.0} s with VSync", o.duration);
        let out = results.join(format!("vsync-{name}.json"));
        scenes.push(run_client(
            &bin,
            &out,
            o.duration,
            o.gpu.as_deref(),
            &with(args),
        )?);
    }

    let text = report(&o, &label, &launches, &scenes);
    println!("{text}");
    std::fs::write(results.join("report.md"), &text).map_err(|e| e.to_string())?;
    eprintln!("bench menu: run summaries in {}", results.display());
    Ok(())
}

/// A number in a run summary, by its path (`phases/acquire/p99`, `window/pixels/0`).
fn num(run: &Value, path: &str) -> Option<f64> {
    path.split('/')
        .try_fold(run, |v, key| match key.parse::<usize>() {
            Ok(i) => v.get(i),
            Err(_) => v.get(key),
        })?
        .as_f64()
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
}

/// A cell: the number with `digits` decimals, or a dash.
fn cell(v: Option<f64>, digits: usize) -> String {
    v.map_or_else(|| "–".to_owned(), |v| format!("{v:.digits$}"))
}

fn mib(bytes: Option<f64>) -> String {
    cell(bytes.map(|b| b / f64::from(1 << 20)), 1)
}

fn report(o: &Options, label: &str, launches: &[Vec<Value>; 2], scenes: &[Value]) -> String {
    let first = scenes.first().or(launches[0].first());
    let text = |path: &str| {
        first
            .and_then(|r| path.split('/').try_fold(r, |v, k| v.get(k))?.as_str())
            .unwrap_or("?")
            .to_owned()
    };
    let mut s = format!(
        "## bench menu: {label} on {} ({}, driver {})\n\n",
        text("gpu"),
        text("device/type"),
        text("device/driver")
    );

    s += &format!(
        "### Launch\n\nMilliseconds after the process starts; medians of {} launches, files in the page cache.\n\n",
        o.launches
    );
    s += "| Pipelines | Window | Device | Renderer | First frame | Menus faded in |\n|---|---:|---:|---:|---:|---:|\n";
    for (name, runs) in [
        ("compiled (no cache)", &launches[0]),
        ("from the cache", &launches[1]),
    ] {
        let col = |key: &str| {
            cell(
                median(
                    runs.iter()
                        .filter_map(|r| num(r, &format!("startup/{key}")))
                        .collect(),
                ),
                0,
            )
        };
        s += &format!(
            "| {name} | {} | {} | {} | {} | {} |\n",
            col("window_ms"),
            col("gpu_ms"),
            col("renderer_ms"),
            col("first_frame_ms"),
            col("text_ready_ms")
        );
    }

    s += "\n### Memory\n\nWith VSync. RSS and the GPU counter are sampled from outside the process (`nvidia-smi`'s process table, or the kernel's DRM client statistics); the allocated columns are what Ruthenium itself asked the driver for.\n\n";
    s += "| Scene | Pixels | Images | RSS MiB (mean / max) | GPU MiB (mean / max) | Textures and targets MiB | Staging MiB | Per-frame data MiB | Swapchain MiB (estimate) | Driver's heap usage MiB (device / host) |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for r in scenes {
        let heap = |device: bool| {
            let sum: f64 = r["memory"]["heaps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|h| h["device_local"].as_bool() == Some(device))
                .filter_map(|h| h["usage"].as_f64())
                .sum();
            let known = r["memory"]["heaps"]
                .as_array()
                .is_some_and(|hs| hs.iter().any(|h| h["usage"].is_number()));
            mib(known.then_some(sum))
        };
        s += &format!(
            "| {} | {} × {} | {} | {} / {} | {} / {} | {} | {} | {} | {} | {} / {} |\n",
            r["scene"].as_str().unwrap_or("?"),
            cell(num(r, "swapchain/width"), 0),
            cell(num(r, "swapchain/height"), 0),
            cell(num(r, "swapchain/images"), 0),
            cell(num(r, "sampled/rss_mib/mean"), 0),
            cell(num(r, "sampled/rss_mib/max"), 0),
            cell(num(r, "sampled/gpu_mib/mean"), 0),
            cell(num(r, "sampled/gpu_mib/max"), 0),
            mib(num(r, "memory/gpu_only")),
            mib(num(r, "memory/upload")),
            mib(num(r, "memory/dynamic")),
            mib(num(r, "memory/swapchain_estimate")),
            heap(true),
            heap(false)
        );
    }

    s += &format!(
        "\n### Pacing with VSync\n\n{:.0} s each after warm-up. P99.9 over the median is the menu's pacing gate (at most 1.5).\n\n",
        o.duration
    );
    s += "| Scene | Present mode | Frames | Avg FPS | Median ms | P99 ms | P99.9 ms | P99.9 / median | Max ms | Work p50 / p99 ms | Acquire p99 / max ms | Acquire timeouts (whole run) | GPU p50 ms |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for r in scenes {
        let (p50, p999) = (num(r, "frame/p50"), num(r, "frame/p999"));
        s += &format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} / {} | {} / {} | {} | {} |\n",
            r["scene"].as_str().unwrap_or("?"),
            r["present_mode"].as_str().unwrap_or("?"),
            cell(num(r, "frame/samples"), 0),
            cell(num(r, "frame/mean").map(|m| 1000.0 / m), 1),
            cell(p50, 3),
            cell(num(r, "frame/p99"), 3),
            cell(p999, 3),
            cell(p50.zip(p999).map(|(m, t)| t / m), 2),
            cell(num(r, "frame/max"), 2),
            cell(num(r, "phases/work/p50"), 3),
            cell(num(r, "phases/work/p99"), 3),
            cell(num(r, "phases/acquire/p99"), 3),
            cell(num(r, "phases/acquire/max"), 2),
            cell(num(r, "acquire/timeouts"), 0),
            cell(num(r, "gpu_time/p50"), 3)
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options() {
        let args: Vec<String> = [
            "--pixels",
            "1280x720",
            "--launches",
            "3",
            "--",
            "--gpu",
            "x",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = parse(&args).unwrap();
        assert_eq!((o.pixels, o.launches), ((1280, 720), 3));
        assert_eq!(o.extra, vec!["--gpu", "x"]);
        assert!(parse(&["--pixels".to_owned(), "big".to_owned()]).is_err());
    }

    #[test]
    fn reports_what_the_runs_say() {
        let launch: Value = serde_json::from_str(
            r#"{"gpu": "G", "device": {"type": "DISCRETE_GPU", "driver": "D 1"},
                "window": {"points": [854, 480], "pixels": [1708, 960]},
                "startup": {"window_ms": 100.0, "gpu_ms": 150.0, "renderer_ms": 180.0,
                            "first_frame_ms": 200.4, "text_ready_ms": null}}"#,
        )
        .unwrap();
        assert_eq!(num(&launch, "window/pixels/0"), Some(1708.0));
        let scene: Value = serde_json::from_str(
            r#"{"scene": "empty", "gpu": "G", "device": {"type": "DISCRETE_GPU", "driver": "D 1"},
                "present_mode": "FIFO",
                "swapchain": {"width": 1920, "height": 1080, "images": 3},
                "frame": {"samples": 7200, "mean": 4.0, "p50": 4.0, "p99": 4.4, "p999": 5.0, "max": 9.0},
                "phases": {"work": {"p50": 0.2, "p99": 0.4}, "acquire": {"p99": 3.5, "max": 8.0}},
                "acquire": {"timeouts": 0}, "gpu_time": {"p50": 0.1},
                "memory": {"gpu_only": 2097152, "upload": 16777216, "dynamic": 2097152,
                           "swapchain_estimate": 24883200,
                           "heaps": [{"device_local": true, "usage": 52428800},
                                     {"device_local": false, "usage": 20971520}]},
                "sampled": {"rss_mib": {"mean": 120.2, "max": 121.0}, "gpu_mib": null}}"#,
        )
        .unwrap();
        let o = parse(&[]).unwrap();
        let text = report(
            &o,
            "work",
            &[vec![launch.clone(), launch], Vec::new()],
            &[scene],
        );
        assert!(text.contains("on G (DISCRETE_GPU, driver D 1)"), "{text}");
        assert!(
            text.contains("| compiled (no cache) | 100 | 150 | 180 | 200 | – |"),
            "{text}"
        );
        assert!(
            text.contains("| from the cache | – | – | – | – | – |"),
            "{text}"
        );
        assert!(
            text.contains("| empty | 1920 × 1080 | 3 | 120 / 121 | – / – | 2.0 | 16.0 | 2.0 | 23.7 | 50.0 / 20.0 |"),
            "{text}"
        );
        assert!(
            text.contains("| empty | FIFO | 7200 | 250.0 | 4.000 | 4.400 | 5.000 | 1.25 | 9.00 | 0.200 / 0.400 | 3.500 / 8.00 | 0 | 0.100 |"),
            "{text}"
        );
    }
}
