//! What a run measures about itself: where each frame's main-thread time went, an optional
//! line per frame, and the summary `cargo xtask bench` reads.
//!
//! A frame is known by its number in the frame loop. The time input was sampled, the
//! phases, the acquire and the GPU time read back a frame or two later all carry it.

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use ruthenium_gpu::{
    Acquired, DeviceCaps, FrameCounters, FrameTimings, GpuTime, MemoryReport, PresentPolicy,
    SwapchainInfo,
};
use ruthenium_render::{FrameTimes, Summary};
use serde_json::{Value, json};

/// The parts of a frame on the main thread, in the order they happen.
pub const LIMITER: usize = 0;
pub const GOVERNOR: usize = 1;
pub const EVENTS: usize = 2;
pub const GAME: usize = 3;
pub const GUI: usize = 4;
pub const RECORD: usize = 5;
pub const BEGIN_WAIT: usize = 6;
pub const RECREATE: usize = 7;
pub const ACQUIRE: usize = 8;
pub const SUBMIT: usize = 9;
pub const PRESENT: usize = 10;
pub const WORK: usize = 11;
pub const PHASE_COUNT: usize = 12;

/// Their names in the summary and the frame log:
/// - `limiter`: sleeping to the frame limiter's deadline;
/// - `governor`: waiting for the GPU to finish earlier frames, before input is sampled;
/// - `events`: draining events and handing input on;
/// - `game`: the game's frame (packets, ticks, settings);
/// - `gui`: building the GUI's draw list;
/// - `record`: the render call, less the five calls that follow;
/// - `begin_wait`, `recreate`, `acquire`, `submit`, `present`: waiting for the frame
///   slot, rebuilding the swapchain, and the three Vulkan calls;
/// - `work`: events, game, GUI, record and submit: the thread's own work, without waits.
pub const PHASE_NAMES: [&str; PHASE_COUNT] = [
    "limiter",
    "governor",
    "events",
    "game",
    "gui",
    "record",
    "begin_wait",
    "recreate",
    "acquire",
    "submit",
    "present",
    "work",
];

/// Frames the log holds: allocated at once, so logging never allocates in a frame.
const LOG_FRAMES: usize = 1 << 20;

/// When each part of one frame started on the main thread.
#[derive(Clone, Copy, Debug)]
pub struct Marks {
    /// The process's start: the log's time zero.
    pub started: Instant,
    pub limiter: Instant,
    pub governor: Instant,
    /// Input was sampled here: the frame shows nothing newer.
    pub input: Instant,
    pub game: Instant,
    pub gui: Instant,
    pub render: Instant,
    /// The present call has returned.
    pub end: Instant,
}

/// One frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    pub frame: u64,
    /// When input was sampled, since the process started.
    pub input_ns: u64,
    /// From sampling input to the present call returning.
    pub cpu_ns: u64,
    /// Since the frame presented before this one (0: none, or this one was not presented).
    pub interval_ns: u64,
    /// GPU time, once it has been read back (0 until then).
    pub gpu_ns: u64,
    pub phases: [u64; PHASE_COUNT],
    pub acquired: Acquired,
    pub presented: bool,
    pub swapchains: u32,
}

fn ns(d: Duration) -> u64 {
    d.as_nanos() as u64
}

impl FrameSample {
    pub fn new(m: &Marks, t: &FrameTimings, presented: bool, interval_ns: u64) -> FrameSample {
        let mut phases = [0u64; PHASE_COUNT];
        phases[LIMITER] = ns(m.governor - m.limiter);
        phases[GOVERNOR] = ns(m.input - m.governor);
        phases[EVENTS] = ns(m.game - m.input);
        phases[GAME] = ns(m.gui - m.game);
        phases[GUI] = ns(m.render - m.gui);
        phases[BEGIN_WAIT] = t.begin_wait_ns;
        phases[RECREATE] = t.recreate_ns;
        phases[ACQUIRE] = t.acquire_ns;
        phases[SUBMIT] = t.submit_ns;
        phases[PRESENT] = t.present_ns;
        let calls = t.begin_wait_ns + t.recreate_ns + t.acquire_ns + t.submit_ns + t.present_ns;
        phases[RECORD] = ns(m.end - m.render).saturating_sub(calls);
        phases[WORK] =
            phases[EVENTS] + phases[GAME] + phases[GUI] + phases[RECORD] + phases[SUBMIT];
        FrameSample {
            frame: t.frame,
            input_ns: ns(m.input - m.started),
            cpu_ns: ns(m.end - m.input),
            interval_ns,
            gpu_ns: 0,
            phases,
            acquired: t.acquired,
            presented,
            swapchains: t.swapchains,
        }
    }
}

/// A run's statistics after warm-up, and its frame log if one was asked for.
#[derive(Debug)]
pub struct Telemetry {
    pub interval: FrameTimes,
    pub cpu: FrameTimes,
    pub gpu: FrameTimes,
    pub phases: [FrameTimes; PHASE_COUNT],
    /// Time from one frame sampling its input to the next doing so (frames of a hidden or
    /// covered window, which are throttled, left out): how promptly events are handled.
    pub event_gap: FrameTimes,
    /// Frames rendered, those of them that presented nothing, and the time those took,
    /// warm-up included.
    pub frames: u64,
    pub skipped: u64,
    pub skipped_ns: u64,
    log: Option<Vec<FrameSample>>,
    /// Frames the log had no room for.
    unlogged: u64,
}

impl Telemetry {
    pub fn new(log: bool) -> Telemetry {
        Telemetry {
            interval: FrameTimes::unbounded(),
            cpu: FrameTimes::unbounded(),
            gpu: FrameTimes::unbounded(),
            phases: std::array::from_fn(|_| FrameTimes::unbounded()),
            event_gap: FrameTimes::unbounded(),
            frames: 0,
            skipped: 0,
            skipped_ns: 0,
            log: log.then(|| Vec::with_capacity(LOG_FRAMES)),
            unlogged: 0,
        }
    }

    /// Adds a frame: to the log always, to the statistics once `measuring`.
    pub fn frame(&mut self, s: &FrameSample, measuring: bool) {
        self.frames += 1;
        if !s.presented {
            self.skipped += 1;
            self.skipped_ns += s.phases[LIMITER] + s.phases[GOVERNOR] + s.cpu_ns;
        }
        if let Some(log) = &mut self.log {
            if log.len() < log.capacity() {
                log.push(*s);
            } else {
                self.unlogged += 1;
            }
        }
        if !measuring {
            return;
        }
        if s.interval_ns > 0 {
            self.interval.push(s.interval_ns);
        }
        self.cpu.push(s.cpu_ns);
        for (series, &v) in self.phases.iter_mut().zip(&s.phases) {
            series.push(v);
        }
    }

    /// Adds the time since the frame before this one sampled its input.
    pub fn event_gap(&mut self, ns: u64, measuring: bool) {
        if measuring {
            self.event_gap.push(ns);
        }
    }

    /// Adds the GPU time of an earlier frame, to that frame's line in the log.
    pub fn gpu_time(&mut self, g: GpuTime, measuring: bool) {
        if measuring {
            self.gpu.push(g.nanos);
        }
        // Frame numbers run without gaps, so the frame's place follows from the first.
        if let Some(log) = &mut self.log
            && let Some(first) = log.first().map(|s| s.frame)
            && let Some(s) = g
                .frame
                .checked_sub(first)
                .and_then(|i| log.get_mut(i as usize))
            && s.frame == g.frame
        {
            s.gpu_ns = g.nanos;
        }
    }

    /// Writes the log as CSV: a line per frame, times in nanoseconds.
    pub fn write_log(&self, path: &Path) -> std::io::Result<()> {
        let Some(log) = &self.log else {
            return Ok(());
        };
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        write!(out, "frame,input_ns,interval_ns,cpu_ns,gpu_ns")?;
        for name in PHASE_NAMES {
            write!(out, ",{name}_ns")?;
        }
        writeln!(out, ",acquired,presented,swapchains")?;
        for s in log {
            write!(
                out,
                "{},{},{},{},{}",
                s.frame, s.input_ns, s.interval_ns, s.cpu_ns, s.gpu_ns
            )?;
            for v in s.phases {
                write!(out, ",{v}")?;
            }
            writeln!(
                out,
                ",{},{},{}",
                s.acquired.name(),
                u8::from(s.presented),
                s.swapchains
            )?;
        }
        if self.unlogged > 0 {
            eprintln!(
                "[ruthenium] the frame log is full: {} later frames are not in it",
                self.unlogged
            );
        }
        out.flush()
    }

    /// One line on where the main thread's time went.
    pub fn main_thread_line(&self, c: &FrameCounters) -> String {
        let p = |i: usize| self.phases[i].summary();
        let (work, governor, acquire, present) = (p(WORK), p(GOVERNOR), p(ACQUIRE), p(PRESENT));
        let events = self.event_gap.summary();
        format!(
            "main thread: work p50 {:.3} ms, p99 {:.3} ms; governor p50 {:.3} ms; acquire p50 {:.3} ms, p99 {:.3} ms, max {:.2} ms; present p50 {:.3} ms, p99 {:.3} ms; events every {:.3} ms, p99 {:.2} ms, max {:.2} ms; {} of {} frames without a present ({:.0} ms); {} images ({} suboptimal), {} timeouts, {} not ready, {} out of date, {} frames without a surface, {} swapchains",
            work.p50_ms,
            work.p99_ms,
            governor.p50_ms,
            acquire.p50_ms,
            acquire.p99_ms,
            acquire.max_ms,
            present.p50_ms,
            present.p99_ms,
            events.p50_ms,
            events.p99_ms,
            events.max_ms,
            self.skipped,
            self.frames,
            self.skipped_ns as f64 / 1e6,
            c.images,
            c.suboptimal,
            c.timeouts,
            c.not_ready,
            c.out_of_date + c.present_out_of_date,
            c.no_surface,
            c.swapchains
        )
    }

    /// The run summary `cargo xtask bench` reads (times in milliseconds, sizes in bytes).
    pub fn bench_json(&self, run: &RunInfo<'_>) -> String {
        let phases: serde_json::Map<String, Value> = PHASE_NAMES
            .iter()
            .zip(&self.phases)
            .map(|(name, s)| ((*name).to_owned(), series(&s.summary())))
            .collect();
        let c = &run.counters;
        let (rss, peak) = process_rss_kib();
        let v = json!({
            "version": 2,
            "scene": run.scene,
            "gpu": run.caps.name,
            "device": {
                "type": format!("{:?}", run.caps.device_type),
                "vendor_id": run.caps.vendor_id,
                "device_id": run.caps.device_id,
                "driver": run.caps.driver,
                "api": ruthenium_gpu::version_string(run.caps.api_version),
                "unified_memory": run.caps.unified_memory,
                "texture_slots": run.caps.texture_slots,
                "transfer_queue": run.transfer_queue,
                "swapchain_maintenance": run.caps.swapchain_maintenance.name(),
                "swapchain_maintenance_khr": run.caps.swapchain_maintenance_khr,
                "swapchain_maintenance_ext": run.caps.swapchain_maintenance_ext,
            },
            "policy": format!("{:?}", run.policy),
            "present_mode": format!("{:?}", run.swapchain.map(|s| s.present_mode).unwrap_or_default()),
            "frames_ahead": run.frames_ahead,
            "acquire_wait": run.acquire_wait,
            "fps_cap": run.fps_cap,
            "swapchain": run.swapchain.map(|s| json!({
                "width": s.extent.width,
                "height": s.extent.height,
                "format": format!("{:?}", s.format),
                "images": s.images,
            })),
            "window": {
                "points": [run.window_points.0, run.window_points.1],
                "pixels": [run.window_pixels.0, run.window_pixels.1],
            },
            "startup": {
                "window_ms": run.startup.window_ms,
                "gpu_ms": run.startup.gpu_ms,
                "renderer_ms": run.startup.renderer_ms,
                "first_frame_ms": run.startup.first_frame_ms,
                "text_ready_ms": run.startup.text_ready_ms,
            },
            "frames": self.frames,
            "skipped": self.skipped,
            "skipped_ms": self.skipped_ns as f64 / 1e6,
            "event_gap": series(&self.event_gap.summary()),
            "frame": series(&self.interval.summary()),
            "cpu": series(&self.cpu.summary()),
            "gpu_time": series(&self.gpu.summary()),
            "phases": phases,
            "acquire": {
                "images": c.images,
                "suboptimal": c.suboptimal,
                "timeouts": c.timeouts,
                "not_ready": c.not_ready,
                "out_of_date": c.out_of_date,
                "no_surface": c.no_surface,
            },
            "present": {
                "suboptimal": c.present_suboptimal,
                "out_of_date": c.present_out_of_date,
            },
            "swapchains": c.swapchains,
            "swapchains_destroyed": c.swapchains_destroyed,
            "retired_peak": c.retired_peak,
            "memory": {
                "rss_kib": rss,
                "rss_peak_kib": peak,
                "allocations": run.memory.allocations,
                "gpu_only": run.memory.gpu_only,
                "upload": run.memory.upload,
                "readback": run.memory.readback,
                "dynamic": run.memory.dynamic,
                // The driver's: four bytes a pixel is the least they take.
                "swapchain_estimate": run.swapchain.map(|s| {
                    u64::from(s.extent.width) * u64::from(s.extent.height) * 4 * u64::from(s.images)
                }),
                "heaps": run.memory.heaps.iter().map(|h| json!({
                    "size": h.size,
                    "device_local": h.device_local,
                    "usage": h.usage,
                    "budget": h.budget,
                })).collect::<Vec<_>>(),
            },
        });
        format!("{v}\n")
    }
}

/// What a run was, for its summary.
#[derive(Debug)]
pub struct RunInfo<'a> {
    /// `menu`, or `empty` for a window with only the background.
    pub scene: &'a str,
    pub caps: &'a DeviceCaps,
    pub transfer_queue: bool,
    pub policy: PresentPolicy,
    pub frames_ahead: u64,
    /// `pace`, or the longest an acquire may block.
    pub acquire_wait: &'a str,
    pub fps_cap: Option<f64>,
    pub swapchain: Option<SwapchainInfo>,
    pub window_points: (u32, u32),
    pub window_pixels: (u32, u32),
    pub startup: Startup,
    pub counters: FrameCounters,
    pub memory: MemoryReport,
}

/// Milliseconds from the process's start to each start-up milestone.
#[derive(Clone, Copy, Debug, Default)]
pub struct Startup {
    pub window_ms: f64,
    /// The Vulkan device exists.
    pub gpu_ms: f64,
    /// The renderer exists: swapchain made, pipelines built.
    pub renderer_ms: f64,
    pub first_frame_ms: Option<f64>,
    /// The menus have faded in.
    pub text_ready_ms: Option<f64>,
}

fn series(s: &Summary) -> Value {
    json!({
        "samples": s.samples,
        "mean": s.mean_ms,
        "p50": s.p50_ms,
        "p99": s.p99_ms,
        "p999": s.p999_ms,
        "max": s.max_ms,
    })
}

/// This process's resident memory now and at its peak, in KiB (Linux; elsewhere none).
fn process_rss_kib() -> (Option<u64>, Option<u64>) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    (status_kib(&status, "VmRSS:"), status_kib(&status, "VmHWM:"))
}

fn status_kib(status: &str, key: &str) -> Option<u64> {
    let line = status.lines().find(|l| l.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(started: Instant) -> Marks {
        let at = |us: u64| started + Duration::from_micros(us);
        Marks {
            started,
            limiter: at(1000),
            governor: at(1100),
            input: at(1400),
            game: at(1450),
            gui: at(1470),
            render: at(1500),
            end: at(2000),
        }
    }

    fn timings() -> FrameTimings {
        FrameTimings {
            frame: 7,
            begin_wait_ns: 10_000,
            recreate_ns: 0,
            acquire_ns: 100_000,
            acquired: Acquired::Image,
            submit_ns: 40_000,
            present_ns: 50_000,
            swapchains: 1,
        }
    }

    #[test]
    fn a_frame_splits_into_its_phases() {
        let s = FrameSample::new(&marks(Instant::now()), &timings(), true, 600_000);
        assert_eq!(s.frame, 7);
        assert_eq!(s.input_ns, 1_400_000);
        assert_eq!(s.cpu_ns, 600_000);
        assert_eq!(s.phases[LIMITER], 100_000);
        assert_eq!(s.phases[GOVERNOR], 300_000);
        assert_eq!(s.phases[EVENTS], 50_000);
        assert_eq!(s.phases[GAME], 20_000);
        assert_eq!(s.phases[GUI], 30_000);
        // The render call took 500 µs, 200 µs of it in the five calls.
        assert_eq!(s.phases[RECORD], 300_000);
        assert_eq!(s.phases[WORK], 50_000 + 20_000 + 30_000 + 300_000 + 40_000);
        // What follows input adds up to the frame's CPU time.
        let after_input: u64 = s.phases[EVENTS..=PRESENT].iter().sum();
        assert_eq!(after_input, s.cpu_ns);
    }

    #[test]
    fn statistics_start_after_warm_up_and_the_log_takes_gpu_times() {
        let started = Instant::now();
        let mut t = Telemetry::new(true);
        for frame in 5..9 {
            let mut timing = timings();
            timing.frame = frame;
            let s = FrameSample::new(&marks(started), &timing, frame != 6, 600_000);
            t.frame(&s, frame >= 7);
        }
        assert_eq!((t.frames, t.skipped), (4, 1));
        // The skipped frame's whole turn of the loop: limiter, governor and the rest.
        assert_eq!(t.skipped_ns, 100_000 + 300_000 + 600_000);
        assert_eq!(t.cpu.len(), 2);
        assert_eq!(t.phases[ACQUIRE].len(), 2);
        t.gpu_time(
            GpuTime {
                frame: 6,
                nanos: 123,
            },
            false,
        );
        t.gpu_time(
            GpuTime {
                frame: 8,
                nanos: 456,
            },
            true,
        );
        // Frames the log never saw change nothing.
        t.gpu_time(GpuTime { frame: 4, nanos: 1 }, true);
        t.gpu_time(
            GpuTime {
                frame: 99,
                nanos: 1,
            },
            true,
        );
        let log = t.log.as_ref().unwrap();
        let gpu: Vec<u64> = log.iter().map(|s| s.gpu_ns).collect();
        assert_eq!(gpu, [0, 123, 0, 456]);
        assert_eq!(t.gpu.len(), 3);
    }

    #[test]
    fn the_summary_is_json_with_every_phase() {
        let mut t = Telemetry::new(false);
        let s = FrameSample::new(&marks(Instant::now()), &timings(), true, 600_000);
        t.frame(&s, true);
        let caps = DeviceCaps {
            name: "A \"quoted\" GPU".into(),
            ..DeviceCaps::default()
        };
        let text = t.bench_json(&RunInfo {
            scene: "menu",
            caps: &caps,
            transfer_queue: false,
            policy: PresentPolicy::NoVSync,
            frames_ahead: 1,
            acquire_wait: "100 ms",
            fps_cap: None,
            swapchain: None,
            window_points: (854, 480),
            window_pixels: (1708, 960),
            startup: Startup::default(),
            counters: FrameCounters::default(),
            memory: MemoryReport::default(),
        });
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["gpu"], "A \"quoted\" GPU");
        assert_eq!(v["frames_ahead"], 1);
        assert_eq!(v["frame"]["samples"], 1);
        for name in PHASE_NAMES {
            assert_eq!(v["phases"][name]["samples"], 1, "{name}");
        }
        assert!((v["phases"]["acquire"]["p50"].as_f64().unwrap() - 0.1).abs() < 0.001);
        assert_eq!(
            status_kib("VmHWM:\t  2048 kB\nVmRSS:\t  1024 kB\n", "VmRSS:"),
            Some(1024)
        );
    }
}
