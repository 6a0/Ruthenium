//! The client's main loop: window, GPU, and a paced frame loop.
//!
//! Each frame does, in this order (research 04 §5.4):
//! 1. **Wait**, the only intentional wait: the frame limiter's deadline (FPS cap), then the
//!    GPU governor, so input is never sampled and then left queued behind the GPU.
//! 2. **Input**: drain SDL's events.
//! 3. **Simulation** (not yet: ticks, packets, camera turn).
//! 4. **Render**: record, acquire the swapchain image late, submit, present.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use ruthenium_gpu::{GpuOptions, PresentPolicy, Presented, Validation};
use ruthenium_platform::{DEFAULT_SIZE, Event, Platform, Sleeper, next_deadline};
use ruthenium_render::{Background, FrameTimes, OverlayStats, Renderer, Summary, create_gpu};

/// SDL scancode of the V key: toggles VSync (development aid until options exist).
const SCANCODE_V: u32 = 25;
/// SDL scancode of F3: shows or hides the frame-stats overlay.
const SCANCODE_F3: u32 = 60;
/// Vanilla throttles a minimised window to 10 FPS (`FramerateLimitTracker.java:30`).
const HIDDEN_FRAME: Duration = Duration::from_millis(100);
/// Frames before this are start-up noise and left out of the run summary.
const WARMUP: Duration = Duration::from_secs(1);
/// Deep space: near-black blue at the top, a hint of violet at the bottom.
const BACKGROUND: Background = Background {
    top: [0.010, 0.010, 0.030, 1.0],
    bottom: [0.060, 0.030, 0.090, 1.0],
};

#[derive(Debug)]
pub struct Options {
    pub policy: PresentPolicy,
    pub fps_cap: Option<f64>,
    pub gpu: Option<String>,
    pub validation: Validation,
    pub quit_after: Option<Duration>,
    pub stats: bool,
    /// Start with the frame-stats overlay shown (F3 toggles it).
    pub overlay: bool,
    /// Development: resize the window and toggle VSync every few frames, to exercise
    /// swapchain recreation (run it under validation).
    pub exercise_swapchain: bool,
    /// Keep the pipeline cache on disk (off: every launch compiles from scratch).
    pub pipeline_cache: bool,
    /// Write a JSON summary of the run (after warm-up) here at exit, for `cargo xtask
    /// bench ab`.
    pub bench_out: Option<std::path::PathBuf>,
    /// Upload buffers on the graphics queue even if the GPU has a transfer queue.
    pub no_transfer_queue: bool,
    /// Development: push checked data through the upload ring every frame (run it under
    /// validation); a mismatch fails the run.
    pub exercise_uploads: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options {
            policy: PresentPolicy::VSync,
            fps_cap: None,
            gpu: std::env::var("RUTHENIUM_GPU").ok(),
            validation: match std::env::var("RUTHENIUM_VALIDATION").as_deref() {
                Ok("1") => Validation::Required,
                Ok("0") => Validation::Off,
                _ if cfg!(debug_assertions) => Validation::IfAvailable,
                _ => Validation::Off,
            },
            quit_after: None,
            stats: false,
            overlay: false,
            exercise_swapchain: false,
            pipeline_cache: true,
            no_transfer_queue: false,
            exercise_uploads: false,
            bench_out: None,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |what: &str| it.next().cloned().ok_or(format!("{a} needs {what}"));
            match a.as_str() {
                "--vsync" => o.policy = PresentPolicy::VSync,
                "--no-vsync" => o.policy = PresentPolicy::NoVSync,
                "--fps-cap" => {
                    let v = value("a frame rate")?;
                    let fps: f64 = v
                        .parse()
                        .map_err(|_| format!("--fps-cap: not a number: {v}"))?;
                    if !(1.0..=10_000.0).contains(&fps) {
                        return Err("--fps-cap must be between 1 and 10000".into());
                    }
                    o.fps_cap = Some(fps);
                }
                "--gpu" => o.gpu = Some(value("a GPU name")?),
                "--validation" => o.validation = Validation::Required,
                "--no-validation" => o.validation = Validation::Off,
                "--quit-after" => {
                    let v = value("a number of seconds")?;
                    let s: f64 = v
                        .parse()
                        .map_err(|_| format!("--quit-after: not a number: {v}"))?;
                    o.quit_after = Some(Duration::from_secs_f64(s.max(0.0)));
                }
                "--stats" => o.stats = true,
                "--overlay" => o.overlay = true,
                "--exercise-swapchain" => o.exercise_swapchain = true,
                "--no-pipeline-cache" => o.pipeline_cache = false,
                "--no-transfer-queue" => o.no_transfer_queue = true,
                "--exercise-uploads" => o.exercise_uploads = true,
                "--bench-out" => o.bench_out = Some(value("a file")?.into()),
                other => return Err(format!("unknown option {other}")),
            }
        }
        Ok(o)
    }
}

pub fn run(opts: &Options, started: Instant) -> ExitCode {
    match run_inner(opts, started) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ruthenium: {e}");
            ExitCode::FAILURE
        }
    }
}

/// How many frames the GPU may still be working on when the next frame samples input.
fn frames_ahead(policy: PresentPolicy) -> u64 {
    match policy {
        // FIFO paces at the display; one frame of overlap keeps the GPU busy.
        PresentPolicy::VSync => 1,
        // Uncapped or capped: hold the next frame until the GPU finishes the previous one.
        PresentPolicy::NoVSync => 0,
    }
}

fn run_inner(opts: &Options, started: Instant) -> Result<ExitCode, Box<dyn std::error::Error>> {
    #[cfg(feature = "tracy")]
    profiling::tracy_client::Client::start();

    let mut platform = Platform::init(env!("CARGO_PKG_VERSION"))?;
    let window = platform.create_window("Ruthenium", DEFAULT_SIZE.0, DEFAULT_SIZE.1)?;
    let t_window = started.elapsed();
    let gpu = create_gpu(
        &platform,
        &window,
        opts.validation,
        &GpuOptions {
            prefer: opts.gpu.clone(),
            no_transfer_queue: opts.no_transfer_queue,
        },
    )?;
    {
        let c = gpu.caps();
        eprintln!(
            "[ruthenium] GPU: {} ({:?}), Vulkan {}, driver {}{}",
            c.name,
            c.device_type,
            ruthenium_gpu::version_string(c.api_version),
            c.driver,
            if c.portability_subset {
                ", portability subset"
            } else {
                ""
            }
        );
    }
    // Declared after `window`, so it is dropped first: the surface must go before the window.
    let cache_dir = opts
        .pipeline_cache
        .then(|| {
            ruthenium_prepare::default_cache_dir(&ruthenium_prepare::discover::Env::from_process())
        })
        .flatten();
    let gpu_display = gpu.caps().name.clone();
    let gpu_name = gpu_display.to_uppercase();
    let mut renderer = Renderer::new(gpu, window.pixel_size(), opts.policy, cache_dir.as_deref())?;
    renderer.overlay_mut().set_visible(opts.overlay);
    let mut mode_name = String::with_capacity(32);
    if opts.exercise_uploads {
        renderer.enable_upload_check()?;
    }

    let mut sleeper = Sleeper::new();
    let mut deadline = Instant::now();
    let period = opts.fps_cap.map(|f| Duration::from_secs_f64(1.0 / f));

    let mut interval = FrameTimes::new(1 << 13);
    let mut cpu = FrameTimes::new(1 << 13);
    let mut gpu_times = FrameTimes::new(1 << 13);
    let mut run_interval = FrameTimes::new(1 << 18);
    let mut run_cpu = FrameTimes::new(1 << 18);
    let mut run_gpu = FrameTimes::new(1 << 18);
    let mut last_present: Option<Instant> = None;
    let mut last_report = Instant::now();
    let mut first_frame: Option<Duration> = None;

    let mut quit = false;
    let mut visible = true;
    let mut occluded = false;
    let mut resized = None;
    let mut toggle_vsync = false;
    let mut toggle_overlay = false;
    let mut frames: u64 = 0;

    loop {
        profiling::scope!("frame");
        // 1. Wait.
        if let Some(p) = period {
            deadline = next_deadline(deadline, p, Instant::now());
            sleeper.sleep_until(deadline);
        }
        renderer
            .frames()
            .wait_for_gpu(frames_ahead(renderer.frames().policy()))?;

        // 2. Input.
        let t_input = Instant::now();
        platform.poll_events(|e| match e {
            Event::Quit | Event::CloseRequested => quit = true,
            Event::PixelSizeChanged { width, height } => resized = Some((width, height)),
            Event::Minimized | Event::Hidden => visible = false,
            Event::Restored | Event::Shown => {
                visible = true;
                occluded = false;
            }
            Event::Occluded => occluded = true,
            Event::Exposed | Event::FocusGained => occluded = false,
            Event::Key {
                scancode: SCANCODE_V,
                down: true,
                repeat: false,
                ..
            } => toggle_vsync = true,
            Event::Key {
                scancode: SCANCODE_F3,
                down: true,
                repeat: false,
                ..
            } => toggle_overlay = true,
            _ => {}
        });
        if quit {
            break;
        }
        if let Some(size) = resized.take() {
            renderer.frames_mut().resize(size);
        }
        if std::mem::take(&mut toggle_overlay) {
            let o = renderer.overlay_mut();
            o.set_visible(!o.visible());
        }
        if std::mem::take(&mut toggle_vsync) {
            let next = match renderer.frames().policy() {
                PresentPolicy::VSync => PresentPolicy::NoVSync,
                PresentPolicy::NoVSync => PresentPolicy::VSync,
            };
            renderer.frames_mut().set_policy(next);
            eprintln!("[ruthenium] {next:?}");
        }
        if !visible {
            // Nothing is shown: tick at 10 FPS without presenting.
            std::thread::sleep(HIDDEN_FRAME);
            last_present = None;
            continue;
        }

        frames += 1;
        if opts.exercise_swapchain {
            if frames.is_multiple_of(40) {
                toggle_vsync = true;
            }
            if frames.is_multiple_of(25) {
                let big = (frames / 25) % 2 == 1;
                let (w, h) = if big { (1100, 620) } else { DEFAULT_SIZE };
                window.request_size(w, h);
            }
        }

        // 3. Simulation: nothing yet.

        // 4. Render.
        let report = renderer.render(&BACKGROUND)?;
        let now = Instant::now();
        profiling::finish_frame!();

        if report.presented == Presented::Yes {
            if first_frame.is_none() {
                first_frame = Some(started.elapsed());
                eprintln!(
                    "[ruthenium] window after {:.0} ms, first frame after {:.0} ms",
                    t_window.as_secs_f64() * 1e3,
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
            if let Some(prev) = last_present {
                let ns = (now - prev).as_nanos() as u64;
                interval.push(ns);
                renderer.overlay_mut().push_frame(ns as f32 / 1e6);
                if started.elapsed() > WARMUP + first_frame.unwrap_or_default() {
                    run_interval.push(ns);
                }
            }
            last_present = Some(now);
        }
        let measuring = started.elapsed() > WARMUP + first_frame.unwrap_or(Duration::MAX / 4);
        cpu.push((now - t_input).as_nanos() as u64);
        if measuring {
            run_cpu.push((now - t_input).as_nanos() as u64);
        }
        if let Some(g) = report.gpu_time {
            gpu_times.push(g.nanos);
            if measuring {
                run_gpu.push(g.nanos);
            }
        }

        if now - last_report >= Duration::from_secs(1) {
            let (f, c, g) = (interval.summary(), cpu.summary(), gpu_times.summary());
            let mode = renderer.frames().present_mode();
            if opts.stats {
                print_line(&f, &c, &g, mode);
            }
            if renderer.overlay_mut().visible() && f.samples > 0 {
                use std::fmt::Write as _;
                mode_name.clear();
                let _ = write!(mode_name, "{:?}", mode.unwrap_or_default());
                renderer.overlay_mut().set_stats(&OverlayStats {
                    fps: f.fps(),
                    low_1: f.low_1_percent_fps(),
                    low_01: f.low_01_percent_fps(),
                    frame_ms: f.p50_ms,
                    cpu_ms: c.p50_ms,
                    gpu_ms: g.p50_ms,
                    p999_over_median: f.p999_ms / f.p50_ms,
                    max_ms: f.max_ms,
                    present: &mode_name,
                    gpu: &gpu_name,
                });
            }
            interval.clear();
            cpu.clear();
            gpu_times.clear();
            last_report = now;
        }
        if occluded {
            // Covered by other windows: keep presenting (so the compositor can tell us when
            // it is visible again) but only at 10 FPS.
            std::thread::sleep(HIDDEN_FRAME);
            last_present = None;
        }
        if opts.quit_after.is_some_and(|q| started.elapsed() >= q) {
            break;
        }
    }

    let run = run_interval.summary();
    if run.samples > 0 {
        eprintln!(
            "[ruthenium] run: {} frames, {:.1} fps, median {:.3} ms, 1% low {:.1} fps, 0.1% low {:.1} fps, max {:.2} ms; 0.1%/median = {:.2}",
            run.samples,
            run.fps(),
            run.p50_ms,
            run.low_1_percent_fps(),
            run.low_01_percent_fps(),
            run.max_ms,
            run.p999_ms / run.p50_ms
        );
    }
    if let Some(path) = &opts.bench_out {
        let json = bench_json(
            &run,
            &run_cpu.summary(),
            &run_gpu.summary(),
            &gpu_display,
            renderer.frames().present_mode(),
        );
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("[ruthenium] could not write {}: {e}", path.display());
            return Ok(ExitCode::FAILURE);
        }
    }
    let mut failed = false;
    if let Some((c, deferred, transferred)) = renderer.upload_check() {
        eprintln!(
            "[ruthenium] upload check: {} chunks ({:.1} MiB) compared, {} mismatched; ready after {:.2} frames on average; {} texture read-backs ({} rectangles written, {} destroyed before arriving), {} mismatched, {} late; {} retries when full, {} deferred transfer submits, {} images on the transfer queue",
            c.chunks,
            c.bytes as f64 / f64::from(1 << 20),
            c.mismatches,
            c.mean_latency_frames(),
            c.textures,
            c.texture_rects,
            c.orphans,
            c.texture_mismatches,
            c.late_textures,
            c.retries,
            deferred,
            transferred
        );
        failed |= c.mismatches > 0
            || c.chunks == 0
            || c.texture_mismatches > 0
            || c.late_textures > 0
            || c.textures == 0;
    }
    let validation = ruthenium_gpu::validation_messages();
    if validation > 0 {
        eprintln!("[ruthenium] {validation} Vulkan validation message(s)");
        failed = true;
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// The run summary `cargo xtask bench ab` reads (times in milliseconds).
fn bench_json(
    frame: &Summary,
    cpu: &Summary,
    gpu: &Summary,
    gpu_name: &str,
    mode: Option<ruthenium_gpu::vk::PresentModeKHR>,
) -> String {
    let series = |s: &Summary| {
        format!(
            "{{\"samples\": {}, \"mean\": {:.6}, \"p50\": {:.6}, \"p99\": {:.6}, \"p999\": {:.6}, \"max\": {:.6}}}",
            s.samples, s.mean_ms, s.p50_ms, s.p99_ms, s.p999_ms, s.max_ms
        )
    };
    let escaped: String = gpu_name
        .chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c if c.is_control() => vec![],
            c => vec![c],
        })
        .collect();
    format!(
        "{{\"version\": 1, \"gpu\": \"{escaped}\", \"present_mode\": \"{:?}\", \"frame\": {}, \"cpu\": {}, \"gpu_time\": {}}}\n",
        mode.unwrap_or_default(),
        series(frame),
        series(cpu),
        series(gpu)
    )
}

fn print_line(
    frame: &Summary,
    cpu: &Summary,
    gpu: &Summary,
    mode: Option<ruthenium_gpu::vk::PresentModeKHR>,
) {
    eprintln!(
        "[stats] {:7.1} fps | frame p50 {:6.3} ms, 1% low {:7.1}, 0.1% low {:7.1} | cpu p50 {:.3} ms | gpu p50 {:.3} ms | {:?}",
        frame.fps(),
        frame.p50_ms,
        frame.low_1_percent_fps(),
        frame.low_01_percent_fps(),
        cpu.p50_ms,
        gpu.p50_ms,
        mode.unwrap_or_default()
    );
}
