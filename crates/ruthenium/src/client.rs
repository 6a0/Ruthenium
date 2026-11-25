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

use ruthenium_gpu::{
    ACQUIRE_TIMEOUT, GpuOptions, PresentPolicy, Presented, PretendSrgb, SwapchainMaintenance,
    TextureTablePath, Validation,
};
use ruthenium_platform::{DEFAULT_SIZE, Event, Platform, Sleeper, next_deadline};
use ruthenium_render::{
    Background, FrameTimes, OverlayStats, Renderer, Summary, TerrainDraws, WorldFrame, create_gpu,
};
use ruthenium_ui::modern::Action;
use serde_json::{Value, json};

use crate::menu::Menu;
use crate::route::{Expected, Route, RoutePlayer, Terrain};
use crate::settings::{Settings, SettingsFile};
use crate::telemetry::{FrameSample, Marks, RunInfo, Startup, Telemetry};

/// SDL scancode of F3: shows or hides the frame-stats overlay.
const SCANCODE_F3: u32 = 60;
/// SDL scancode of F11: fullscreen on or off, as in vanilla.
const SCANCODE_F11: u32 = 68;
/// How long a fullscreen request waits for the window manager to confirm it; events
/// meanwhile are the window catching up with earlier requests.
const FULLSCREEN_PENDING: Duration = Duration::from_secs(2);
/// Vanilla throttles a minimised window to 10 FPS (`FramerateLimitTracker.java:30`).
const HIDDEN_FRAME: Duration = Duration::from_millis(100);
/// `--exercise-minimize` minimises the window, and brings it back, this often.
const MINIMIZE_EVERY: Duration = Duration::from_millis(1500);
/// An acquire may block until this many pacing intervals (the longer of the frame limiter's
/// and the display's) after the frame sampled its input, and at least this long. Then the
/// frame goes on without an image, and the loop is back at its events.
const PACE_INTERVALS: u32 = 2;
const PACE_FLOOR: Duration = Duration::from_millis(8);
/// The display's refresh rate when the window system doesn't say.
const ASSUMED_REFRESH_HZ: f32 = 60.0;
/// Frames before this are start-up noise and left out of the run summary.
const WARMUP: Duration = Duration::from_secs(1);
/// Deep space: near-black blue at the top, a hint of violet at the bottom.
const BACKGROUND: Background = Background {
    top: [0.010, 0.010, 0.030, 1.0],
    bottom: [0.060, 0.030, 0.090, 1.0],
};

/// How long a frame may wait for a swapchain image before it goes on without one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AcquireWait {
    /// This long in the acquire call.
    Fixed(Duration),
    /// Until [`PACE_INTERVALS`] pacing intervals after the frame's input was sampled.
    Pace,
}

#[derive(Debug)]
pub struct Options {
    /// VSync on or off for this run, over the saved setting.
    pub vsync: Option<bool>,
    /// A frame limiter for this run, over the saved setting (until it changes).
    pub fps_cap: Option<f64>,
    pub gpu: Option<String>,
    pub validation: Validation,
    pub quit_after: Option<Duration>,
    pub stats: bool,
    /// Start with the frame-stats overlay shown, over the saved setting (F3 toggles it).
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
    /// Development: stream a texture larger than the upload ring, worker-written blocks
    /// and pooled ranges every frame (run it under validation); a wrong byte or a broken
    /// rule fails the run.
    pub exercise_streaming: bool,
    /// Development: write the window's contents to this PNG once the menus show, then quit.
    pub screenshot: Option<std::path::PathBuf>,
    /// Development: wait this long after the menus show before the screenshot.
    pub screenshot_after: Duration,
    /// Ruthenium's data folder (default: the per-user one): accounts, servers.
    pub data_dir: Option<std::path::PathBuf>,
    /// The server list file (default: `servers.dat` in Ruthenium's data folder).
    pub servers_file: Option<std::path::PathBuf>,
    /// Development: start on this menu screen (`servers`, `add`, `direct`, `delete`,
    /// `accounts`, `sign-in`, `code`, `sign-in-failed`, `sign-out`, `settings`).
    pub screen: Option<String>,
    /// Development: select this server (or account) in the list, or focus this setting.
    pub select: Option<usize>,
    /// The window's starting size in points.
    pub window_size: (u32, u32),
    /// Development: join this server (an address as typed) from the server list at once.
    pub join: Option<String>,
    /// Benchmarks: how many frames the GPU may still be working on when the next one
    /// samples input (0 or 1), over what the present policy would choose.
    pub frames_ahead: Option<u64>,
    /// Benchmarks: draw only the background, no menus (an empty window's cost).
    pub empty_window: bool,
    /// Benchmarks: write a CSV line per frame here at exit.
    pub frame_log: Option<std::path::PathBuf>,
    /// Development: minimise the window and bring it back every 1.5 s.
    pub exercise_minimize: bool,
    /// Use only this swapchain maintenance extension, or none (default: the best the GPU
    /// has), to test each way of retiring swapchains.
    pub swapchain_maintenance: Option<SwapchainMaintenance>,
    /// How long a frame waits for a swapchain image.
    pub acquire_wait: AcquireWait,
    /// Development: pretend the window's surface offers only sRGB formats.
    pub pretend_srgb: PretendSrgb,
    /// Development: with `--screenshot`, resize the window to this size in points once
    /// the menus show, and take the screenshot of the first frame at the new size.
    pub screenshot_resize: Option<(u32, u32)>,
    /// Texture-table slots to ask the GPU for (default: the GPU crate's).
    pub texture_slots: Option<u32>,
    /// Make the texture table of this kind of descriptor set only (default: ordinary
    /// sets, update-after-bind when that gives a larger table).
    pub texture_table: Option<TextureTablePath>,
    /// Development: fill, reload and read back the texture table every frame (run it
    /// under validation); a wrong slot fails the run.
    pub exercise_textures: bool,
    /// Development: draw a known grey and a translucent swatch over it every frame, off
    /// screen and into the window, and read the bytes back; a wrong byte fails the run.
    pub exercise_colour: bool,
    /// Development: draw a made-up scene with the terrain pipelines every frame, off
    /// screen, and read it back; a wrong pixel fails the run.
    pub exercise_terrain: bool,
    /// Write what the GPU reported and what was chosen from it to this JSON file, once the
    /// renderer exists.
    pub adapter_report: Option<std::path::PathBuf>,
    /// Threads that mesh the world's sections (default: by the machine's size).
    pub mesh_workers: Option<usize>,
    /// Development: with `--join`, leave the server this long after the start and join
    /// it again at once, in the same run.
    pub rejoin_after: Option<Duration>,
    /// Benchmarks: with `--join`, play this route file once in the world
    /// ([`crate::route`]), measure the frames of its capture and quit when it ends.
    pub route: Option<std::path::PathBuf>,
    /// Write the route's record here.
    pub route_out: Option<std::path::PathBuf>,
    /// The checksums the route's record must end with.
    pub expected: Expected,
    /// How the terrain's sections are drawn (default: one direct draw each, nearest
    /// first).
    pub terrain_draws: Option<TerrainDraws>,
    /// Whether only what can be seen from the camera is meshed and drawn (cave
    /// culling: the default) or everything loaded inside the view.
    pub terrain_culling: Option<bool>,
    /// Smooth Lighting: corners lit by the blocks around them (the default), or off.
    pub smooth_lighting: Option<bool>,
    /// Biome Blend: over how many columns each way biome colours are the mean (0 to 7;
    /// default 2).
    pub biome_blend: Option<u8>,
    /// How the terrain's texels are filtered (default: as vanilla's RGSS).
    pub terrain_shader: Option<ruthenium_render::TerrainShader>,
    /// Whether the terrain is fogged (the default) or not.
    pub fog: Option<bool>,
    /// Chunk Fade: how long a section takes to fade in, in milliseconds (default 750).
    pub chunk_fade_ms: Option<u32>,
    /// When the sky is drawn (default: after the world's opaque layers).
    pub sky: Option<ruthenium_render::SkyOrder>,
    /// Whether block light flickers (the default) or holds still.
    pub light_flicker: Option<bool>,
    /// Whether the terrain's translucent layer is drawn (the default) or left out.
    pub translucent: Option<bool>,
    /// Whether block entities (chests, skulls, banners, the text of signs) are drawn (the default).
    pub block_entities: Option<bool>,
    pub players: Option<bool>,
    /// Whether entities cast shadows (the default).
    pub entity_shadows: Option<bool>,
    /// Whether the player's own hands are drawn, the block the crosshair is on is
    /// outlined, and the view bobs with the player's steps (the defaults).
    pub hand: Option<bool>,
    pub block_outline: Option<bool>,
    pub view_bobbing: Option<bool>,
    /// Whether the crosshair is drawn (the default), and the HUD at all (the default).
    pub crosshair: Option<bool>,
    pub hud: Option<bool>,
    /// Whether the frame's corners darken in the dark (the default).
    pub vignette: Option<bool>,
    /// Development: show the Game Menu as soon as the world is there.
    pub game_menu: Option<crate::menu::ShownMenu>,
    /// Show the player list without its key (default: no).
    pub player_list: Option<bool>,
    /// Whether animated textures play (the default) or hold still.
    pub animations: Option<bool>,
    /// Whether the field of view narrows in water and lava (the default).
    pub fov_effects: Option<bool>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut o = Options {
            vsync: None,
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
            exercise_streaming: false,
            bench_out: None,
            screenshot: None,
            screenshot_after: Duration::ZERO,
            data_dir: None,
            servers_file: None,
            screen: None,
            select: None,
            window_size: DEFAULT_SIZE,
            join: None,
            frames_ahead: None,
            empty_window: false,
            frame_log: None,
            exercise_minimize: false,
            swapchain_maintenance: None,
            acquire_wait: AcquireWait::Pace,
            pretend_srgb: PretendSrgb::Never,
            screenshot_resize: None,
            texture_slots: None,
            texture_table: None,
            exercise_textures: false,
            exercise_colour: false,
            exercise_terrain: false,
            adapter_report: None,
            mesh_workers: None,
            rejoin_after: None,
            route: None,
            route_out: None,
            expected: Expected::default(),
            terrain_draws: None,
            terrain_culling: None,
            smooth_lighting: None,
            biome_blend: None,
            terrain_shader: None,
            fog: None,
            chunk_fade_ms: None,
            sky: None,
            light_flicker: None,
            translucent: None,
            block_entities: None,
            players: None,
            entity_shadows: None,
            hand: None,
            block_outline: None,
            view_bobbing: None,
            crosshair: None,
            hud: None,
            vignette: None,
            game_menu: None,
            player_list: None,
            animations: None,
            fov_effects: None,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |what: &str| it.next().cloned().ok_or(format!("{a} needs {what}"));
            match a.as_str() {
                "--vsync" => o.vsync = Some(true),
                "--no-vsync" => o.vsync = Some(false),
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
                "--exercise-minimize" => o.exercise_minimize = true,
                "--frames-ahead" => {
                    let v = value("0 or 1")?;
                    o.frames_ahead = match v.as_str() {
                        "0" => Some(0),
                        "1" => Some(1),
                        _ => return Err(format!("--frames-ahead: not 0 or 1: {v}")),
                    };
                }
                "--empty-window" => o.empty_window = true,
                "--frame-log" => o.frame_log = Some(value("a file")?.into()),
                "--no-pipeline-cache" => o.pipeline_cache = false,
                "--no-transfer-queue" => o.no_transfer_queue = true,
                "--exercise-uploads" => o.exercise_uploads = true,
                "--exercise-streaming" => o.exercise_streaming = true,
                "--bench-out" => o.bench_out = Some(value("a file")?.into()),
                "--screenshot" => o.screenshot = Some(value("a file")?.into()),
                "--screenshot-after" => {
                    let v = value("a number of seconds")?;
                    let s: f64 = v
                        .parse()
                        .map_err(|_| format!("--screenshot-after: not a number: {v}"))?;
                    o.screenshot_after = Duration::from_secs_f64(s.clamp(0.0, 600.0));
                }
                "--window-size" => {
                    let v = value("a size like 1280x720")?;
                    o.window_size =
                        parse_size(&v).ok_or(format!("--window-size: not a size: {v}"))?;
                }
                "--screenshot-resize" => {
                    let v = value("a size like 1280x720")?;
                    o.screenshot_resize = Some(
                        parse_size(&v).ok_or(format!("--screenshot-resize: not a size: {v}"))?,
                    );
                }
                "--swapchain-maintenance" => {
                    let v = value("auto, khr, ext or none")?;
                    o.swapchain_maintenance = match v.as_str() {
                        "auto" => None,
                        "khr" => Some(SwapchainMaintenance::Khr),
                        "ext" => Some(SwapchainMaintenance::Ext),
                        "none" => Some(SwapchainMaintenance::None),
                        _ => {
                            return Err(format!(
                                "--swapchain-maintenance: not auto, khr, ext or none: {v}"
                            ));
                        }
                    };
                }
                "--acquire-wait" => {
                    let v = value("milliseconds, or pace")?;
                    o.acquire_wait = if v == "pace" {
                        AcquireWait::Pace
                    } else {
                        let ms: f64 = v
                            .parse()
                            .ok()
                            .filter(|ms| (0.0..=100.0).contains(ms))
                            .ok_or(format!("--acquire-wait: not pace or 0 to 100 ms: {v}"))?;
                        AcquireWait::Fixed(Duration::from_secs_f64(ms / 1e3))
                    };
                }
                "--pretend-srgb-surface" => {
                    let v = value("start or recreate")?;
                    o.pretend_srgb = match v.as_str() {
                        "start" => PretendSrgb::Always,
                        "recreate" => PretendSrgb::OnRecreation,
                        _ => {
                            return Err(format!(
                                "--pretend-srgb-surface: not start or recreate: {v}"
                            ));
                        }
                    };
                }
                "--texture-slots" => {
                    let v = value("a number of slots")?;
                    let range = ruthenium_gpu::MIN_TEXTURE_SLOTS..=ruthenium_gpu::MAX_TEXTURE_SLOTS;
                    o.texture_slots =
                        Some(v.parse().ok().filter(|n| range.contains(n)).ok_or(format!(
                            "--texture-slots: not {} to {}: {v}",
                            range.start(),
                            range.end()
                        ))?);
                }
                "--texture-table" => {
                    let v = value("auto, ordinary or update-after-bind")?;
                    o.texture_table = match v.as_str() {
                        "auto" => None,
                        "ordinary" => Some(TextureTablePath::Ordinary),
                        "update-after-bind" => Some(TextureTablePath::UpdateAfterBind),
                        _ => {
                            return Err(format!(
                                "--texture-table: not auto, ordinary or update-after-bind: {v}"
                            ));
                        }
                    };
                }
                "--exercise-textures" => o.exercise_textures = true,
                "--exercise-colour" => o.exercise_colour = true,
                "--exercise-terrain" => o.exercise_terrain = true,
                "--adapter-report" => o.adapter_report = Some(value("a file")?.into()),
                "--data-dir" => o.data_dir = Some(value("a folder")?.into()),
                "--join" => o.join = Some(value("a server address")?),
                "--rejoin-after" => {
                    let v = value("a number of seconds")?;
                    let s: f64 = v
                        .parse()
                        .map_err(|_| format!("--rejoin-after: not a number: {v}"))?;
                    o.rejoin_after = Some(Duration::from_secs_f64(s.max(0.0)));
                }
                "--mesh-workers" => {
                    let v = value("a number of threads")?;
                    o.mesh_workers = Some(
                        v.parse()
                            .ok()
                            .filter(|n| (1..=64).contains(n))
                            .ok_or(format!("--mesh-workers: not 1 to 64: {v}"))?,
                    );
                }
                "--route" => o.route = Some(value("a route file")?.into()),
                "--route-out" => o.route_out = Some(value("a file")?.into()),
                "--expect-positions" => o.expected.positions = Some(value("a checksum")?),
                "--expect-footprint" => o.expected.footprint = Some(value("a checksum")?),
                "--terrain-draws" => {
                    let v = value("direct, paged or mdi")?;
                    o.terrain_draws = Some(
                        TerrainDraws::parse(&v)
                            .ok_or(format!("--terrain-draws: not direct, paged or mdi: {v}"))?,
                    );
                }
                "--terrain-culling" => {
                    o.terrain_culling = Some(match value("graph or frustum")?.as_str() {
                        "graph" => true,
                        "frustum" => false,
                        v => return Err(format!("--terrain-culling: not graph or frustum: {v}")),
                    });
                }
                "--smooth-lighting" => {
                    o.smooth_lighting = Some(match value("on or off")?.as_str() {
                        "on" => true,
                        "off" => false,
                        v => return Err(format!("--smooth-lighting: not on or off: {v}")),
                    });
                }
                "--biome-blend" => {
                    let v = value("a radius from 0 to 7")?;
                    o.biome_blend = Some(
                        v.parse()
                            .ok()
                            .filter(|r| *r <= ruthenium_render::terrain::MAX_BLEND_RADIUS)
                            .ok_or(format!("--biome-blend: not a radius from 0 to 7: {v}"))?,
                    );
                }
                "--terrain-shader" => {
                    let v = value("rgss, rgss-reference, nearest or plain")?;
                    o.terrain_shader =
                        Some(ruthenium_render::TerrainShader::parse(&v).ok_or(format!(
                            "--terrain-shader: not rgss, rgss-reference, nearest or plain: {v}"
                        ))?);
                }
                "--fog" | "--light-flicker" | "--translucent" | "--animations"
                | "--fov-effects" | "--block-entities" | "--players" | "--entity-shadows"
                | "--hand" | "--block-outline" | "--view-bobbing" | "--crosshair" | "--hud"
                | "--vignette" | "--player-list" => {
                    let on = match value("on or off")?.as_str() {
                        "on" => true,
                        "off" => false,
                        v => return Err(format!("{a}: not on or off: {v}")),
                    };
                    match a.as_str() {
                        "--fog" => o.fog = Some(on),
                        "--light-flicker" => o.light_flicker = Some(on),
                        "--animations" => o.animations = Some(on),
                        "--fov-effects" => o.fov_effects = Some(on),
                        "--block-entities" => o.block_entities = Some(on),
                        "--players" => o.players = Some(on),
                        "--entity-shadows" => o.entity_shadows = Some(on),
                        "--hand" => o.hand = Some(on),
                        "--block-outline" => o.block_outline = Some(on),
                        "--view-bobbing" => o.view_bobbing = Some(on),
                        "--crosshair" => o.crosshair = Some(on),
                        "--hud" => o.hud = Some(on),
                        "--vignette" => o.vignette = Some(on),
                        "--player-list" => o.player_list = Some(on),
                        _ => o.translucent = Some(on),
                    }
                }
                "--chunk-fade" => {
                    let v = value("seconds from 0 to 1.5")?;
                    let seconds: f64 = v
                        .parse()
                        .ok()
                        .filter(|s| (0.0..=1.5).contains(s))
                        .ok_or(format!("--chunk-fade: not seconds from 0 to 1.5: {v}"))?;
                    // Whole milliseconds, as the game counts the fade.
                    o.chunk_fade_ms = Some((seconds * 1000.0) as u32);
                }
                "--sky" => {
                    let v = value("late, first or off")?;
                    o.sky = Some(
                        ruthenium_render::SkyOrder::parse(&v)
                            .ok_or(format!("--sky: not late, first or off: {v}"))?,
                    );
                }
                "--servers-file" => o.servers_file = Some(value("a file")?.into()),
                "--game-menu" => {
                    let v = value("0 to 1, ruthenium, mc or mc:<page>")?;
                    o.game_menu = Some(crate::menu::ShownMenu::parse(&v).ok_or(format!(
                        "--game-menu: not 0 to 1, ruthenium, mc or mc:<page>: {v}"
                    ))?);
                }
                "--screen" => o.screen = Some(value("a screen name")?),
                "--select" => {
                    let v = value("a server number")?;
                    o.select = Some(
                        v.parse()
                            .map_err(|_| format!("--select: not a number: {v}"))?,
                    );
                }
                other => return Err(format!("unknown option {other}")),
            }
        }
        if o.route.is_some() && (o.join.is_none() || o.rejoin_after.is_some()) {
            return Err("--route plays in the one session --join starts: give --join, and no --rejoin-after".into());
        }
        if o.route.is_none()
            && (o.route_out.is_some()
                || o.expected.positions.is_some()
                || o.expected.footprint.is_some())
        {
            return Err("--route-out and --expect-... are for a --route".into());
        }
        if o.exercise_streaming && (o.exercise_uploads || o.exercise_textures) {
            return Err(
                "--exercise-streaming needs the upload ring to itself: run it without --exercise-uploads and --exercise-textures"
                    .into(),
            );
        }
        Ok(o)
    }
}

fn parse_size(v: &str) -> Option<(u32, u32)> {
    v.split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .filter(|&(w, h)| (64..=16384).contains(&w) && (64..=16384).contains(&h))
}

/// The latest moment an acquire may still be waiting, for a frame whose input was sampled
/// at `input`. An acquire can take a whole refresh of the display with VSync, and the
/// frame limiter may be slower still, so the longer of the two is the pacing interval.
fn pace_deadline(input: Instant, limiter: Option<Duration>, refresh_hz: Option<f32>) -> Instant {
    let refresh = Duration::from_secs_f32(1.0 / refresh_hz.unwrap_or(ASSUMED_REFRESH_HZ));
    let interval = limiter.map_or(refresh, |p| p.max(refresh));
    input
        + (interval * PACE_INTERVALS)
            .max(PACE_FLOOR)
            .min(ACQUIRE_TIMEOUT)
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

/// The game's options the renderer keeps (the menu hands over the others every frame).
fn apply_game_options(renderer: &mut Renderer, game: &crate::settings::GameOptions) {
    renderer.set_entity_shadows(game.entity_shadows);
    renderer.set_fov_effects(game.fov_effects as f32 / 100.0);
    renderer.set_view_options(ruthenium_render::ViewOptions {
        bobbing: game.view_bobbing,
        damage_tilt: game.damage_tilt as f32 / 100.0,
    });
}

fn policy(vsync: bool) -> PresentPolicy {
    if vsync {
        PresentPolicy::VSync
    } else {
        PresentPolicy::NoVSync
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

    let env = ruthenium_prepare::discover::Env::from_process();
    let data_dir = opts
        .data_dir
        .clone()
        .or_else(|| ruthenium_prepare::default_data_dir(&env));
    let servers_file = opts
        .servers_file
        .clone()
        .or_else(|| data_dir.as_ref().map(|d| d.join("servers.dat")));
    // The account in use signs in again on the accounts thread meanwhile.
    let accounts = crate::account::Accounts::start(
        data_dir.clone(),
        ruthenium_prepare::default_cache_dir(&env).as_deref(),
    );
    // Benchmarks run on the defaults, and so do screenshots unless given a data folder.
    let mut settings = match &data_dir {
        Some(d)
            if opts.bench_out.is_none()
                && (opts.screenshot.is_none() || opts.data_dir.is_some()) =>
        {
            let mut f = SettingsFile::load(SettingsFile::path_in(d));
            f.import_mouse(crate::controls::vanilla_mouse);
            f
        }
        _ => SettingsFile::transient(),
    };
    // Command-line options override the saved settings for this run (and are saved
    // with them if a setting changes).
    if let Some(v) = opts.vsync {
        settings.settings.vsync = v;
    }
    if opts.overlay {
        settings.settings.frame_stats = true;
    }
    let game = &mut settings.settings.game;
    game.vignette = opts.vignette.unwrap_or(game.vignette);
    game.entity_shadows = opts.entity_shadows.unwrap_or(game.entity_shadows);
    game.view_bobbing = opts.view_bobbing.unwrap_or(game.view_bobbing);
    if let Some(on) = opts.fov_effects {
        game.fov_effects = if on { 100 } else { 0 };
    }
    let mut menu = Menu::new(servers_file, accounts, settings);
    if let Some(n) = opts.mesh_workers {
        menu.set_mesh_workers(n);
    }
    if let Some(on) = opts.crosshair {
        menu.set_crosshair(on);
    }
    if let Some(on) = opts.hud {
        menu.set_hud(on);
    }
    if let Some(shown) = opts.game_menu {
        menu.show_menu(shown);
    }
    if let Some(on) = opts.player_list {
        menu.set_player_list(on);
    }
    if let Some(path) = &opts.route {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let route = Route::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        menu.set_route(RoutePlayer::new(route, opts.expected.clone()));
    }
    let started_epoch_ms =
        crate::route::epoch_ms().saturating_sub(started.elapsed().as_millis() as u64);
    menu.set_joiner(crate::join::Joiner::new(
        data_dir.as_ref().map(|d| d.join("game")),
        ruthenium_prepare::default_cache_dir(&env),
    ));
    let mut applied: Settings = menu.settings().clone();
    let mut platform = Platform::init(env!("CARGO_PKG_VERSION"))?;
    let window = platform.create_window("Ruthenium", opts.window_size.0, opts.window_size.1)?;
    // A fullscreen request waiting for the window manager: what was asked, and when.
    let mut fullscreen_pending: Option<(bool, Instant)> = None;
    if applied.fullscreen {
        window.set_fullscreen(true);
        fullscreen_pending = Some((true, Instant::now()));
    }
    let mut startup = Startup {
        window_ms: started.elapsed().as_secs_f64() * 1e3,
        ..Startup::default()
    };
    let gpu = create_gpu(
        &platform,
        &window,
        opts.validation,
        &GpuOptions {
            prefer: opts.gpu.clone(),
            no_transfer_queue: opts.no_transfer_queue,
            swapchain_maintenance: opts.swapchain_maintenance,
            pretend_srgb: opts.pretend_srgb,
            texture_slots: opts.texture_slots,
            texture_table: opts.texture_table,
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
        eprintln!(
            "[ruthenium] swapchain maintenance: {} (KHR {}, EXT {}); finished presents are known from {}",
            c.swapchain_maintenance.name(),
            if c.swapchain_maintenance_khr {
                "usable"
            } else {
                "missing"
            },
            if c.swapchain_maintenance_ext {
                "usable"
            } else {
                "missing"
            },
            match c.swapchain_maintenance {
                SwapchainMaintenance::None => "the fences of later acquires",
                _ => "present fences",
            }
        );
        let optional = c
            .optional
            .named()
            .map(|(name, on)| format!("{name} {}", if on { "on" } else { "missing" }));
        eprintln!("[ruthenium] optional features: {}", optional.join(", "));
    }
    startup.gpu_ms = started.elapsed().as_secs_f64() * 1e3;
    // Declared after `window`, so it is dropped first: the surface must go before the window.
    let cache_dir = opts
        .pipeline_cache
        .then(|| {
            ruthenium_prepare::default_cache_dir(&ruthenium_prepare::discover::Env::from_process())
        })
        .flatten();
    let gpu_display = gpu.caps().name.clone();
    let gpu_name = gpu_display.to_uppercase();
    let mut renderer = Renderer::new(
        gpu,
        window.pixel_size(),
        policy(applied.vsync),
        cache_dir.as_deref(),
    )?;
    startup.renderer_ms = started.elapsed().as_secs_f64() * 1e3;
    {
        let layouts = renderer.vertex_layout_checks();
        for (name, c) in &layouts {
            // Worth a line only where the device changes or refuses a layout.
            if c.stride != c.bytes || !c.usable() {
                eprintln!(
                    "[ruthenium] vertex layout {name}: stride {} for {} bytes{}{}",
                    c.stride,
                    c.bytes,
                    if c.unsupported.is_empty() {
                        String::new()
                    } else {
                        format!(", cannot read {:?}", c.unsupported)
                    },
                    if c.beyond_stride {
                        ", an attribute reaches past the vertex"
                    } else {
                        ""
                    }
                );
            }
        }
        if let Some(path) = &opts.adapter_report {
            let report = crate::telemetry::adapter_report(
                renderer.gpu().caps(),
                renderer.gpu().transfer_family().is_some(),
                renderer.frames().swapchain_info(),
                &layouts,
            );
            if let Err(e) = std::fs::write(path, report) {
                eprintln!("[ruthenium] could not write {}: {e}", path.display());
                return Ok(ExitCode::FAILURE);
            }
        }
    }
    renderer.overlay_mut().set_visible(applied.frame_stats);
    if let Some(draws) = opts.terrain_draws {
        renderer.set_terrain_draws(draws);
    }
    if let Some(culling) = opts.terrain_culling {
        renderer.set_terrain_culling(culling);
    }
    if let Some(smooth) = opts.smooth_lighting {
        renderer.set_smooth_lighting(smooth);
    }
    if let Some(radius) = opts.biome_blend {
        renderer.set_biome_blend(radius);
    }
    if let Some(shader) = opts.terrain_shader {
        renderer.set_terrain_shader(shader);
    }
    if let Some(ms) = opts.chunk_fade_ms {
        renderer.set_chunk_fade_ms(ms);
    }
    if let Some(order) = opts.sky {
        renderer.set_sky_order(order);
    }
    if let Some(on) = opts.translucent {
        renderer.set_translucent(on);
    }
    if let Some(on) = opts.block_entities {
        renderer.set_block_entities(on);
    }
    if let Some(on) = opts.players {
        renderer.set_players(on);
    }
    apply_game_options(&mut renderer, &applied.game);
    if let Some(on) = opts.hand {
        renderer.set_hand(on);
    }
    if let Some(on) = opts.block_outline {
        renderer.set_block_outline(on);
    }
    if let Some(on) = opts.animations {
        renderer.set_animate(on);
    }
    {
        let mut air = renderer.atmosphere_options();
        air.fog = opts.fog.unwrap_or(air.fog);
        air.flicker = opts.light_flicker.unwrap_or(air.flicker);
        renderer.set_atmosphere_options(air);
    }
    let mut mode_name = String::with_capacity(32);
    if opts.exercise_uploads {
        renderer.enable_upload_check()?;
    }
    if opts.exercise_streaming {
        renderer.enable_stream_check()?;
    }
    if opts.exercise_colour {
        renderer.enable_colour_check()?;
    }
    if opts.exercise_terrain {
        renderer.enable_terrain_check()?;
    }
    if opts.exercise_textures {
        renderer.enable_table_check()?;
    }
    if opts.exercise_swapchain {
        renderer.exercise_recreation();
    }
    let mut refresh_hz = window.refresh_rate();
    let acquire_wait = match opts.acquire_wait {
        AcquireWait::Fixed(d) => format!("{} ms", d.as_secs_f64() * 1e3),
        AcquireWait::Pace => "pace".to_owned(),
    };
    if let AcquireWait::Fixed(d) = opts.acquire_wait {
        renderer.frames_mut().set_acquire_limit(d, None);
    }

    let mut sleeper = Sleeper::new();
    let mut deadline = Instant::now();
    // The command line's cap holds until the Max Frame Rate setting changes.
    let cap = |s: &Settings| s.max_fps.map(f64::from);
    let mut period = opts
        .fps_cap
        .or(cap(&applied))
        .map(|f| Duration::from_secs_f64(1.0 / f));

    let mut interval = FrameTimes::new(1 << 13);
    let mut cpu = FrameTimes::new(1 << 13);
    let mut gpu_times = FrameTimes::new(1 << 13);
    // The run after warm-up, for the summary at exit.
    let mut telemetry = Telemetry::new(opts.frame_log.is_some(), opts.route.is_some());
    // A route's capture: whether its ticks are being played, what the terrain and the
    // uploads had done when it began and when it ended, and its frames that were held
    // back because the window was hidden or covered.
    let mut capturing = false;
    let mut frames_after_route = 0u32;
    let mut capture: [Option<Value>; 2] = [None, None];
    let mut throttled: u64 = 0;
    let mut last_present: Option<Instant> = None;
    let mut last_report = Instant::now();
    let mut first_frame: Option<Duration> = None;

    let mut quit = false;
    let mut visible = true;
    let mut occluded = false;
    let mut resized = None;
    // Development: --exercise-swapchain flips VSync (without touching the setting).
    let mut toggle_vsync = false;
    let mut toggle_overlay = false;
    let mut toggle_fullscreen = false;
    let mut fullscreen_event: Option<bool> = None;
    // Left mouse button changes (down, x, y, shift) and keys (scancode, down, repeat,
    // shift, shortcut), handed to the menu after the events are drained, and typed text.
    let mut clicks: Vec<(bool, f32, f32, bool)> = Vec::with_capacity(4);
    let mut keys: Vec<(u32, bool, bool, bool, bool)> = Vec::with_capacity(4);
    let mut typed = String::new();
    let mut shift_held = false;
    let mut text_input = false;
    let mut mouse_grabbed = false;
    let mut screenshot_requested = false;
    let mut text_shown: Option<Instant> = None;
    let mut start_screen = opts.screen.clone();
    let mut start_join = opts.join.clone();
    let mut rejoin_after = opts.rejoin_after;
    let mut frames: u64 = 0;
    // --exercise-minimize: whether the window was last asked to minimise.
    let mut minimize_asked = false;
    // When input was last sampled by a frame that was not throttled afterwards.
    let mut last_input: Option<Instant> = None;
    // --screenshot-resize: the resize has been asked for.
    let mut resize_asked = false;
    // --exercise-swapchain: screenshots read back, and those of a swapchain's first frame.
    let mut captures = (0u32, 0u32);

    loop {
        profiling::scope!("frame");
        // 1. Wait.
        let t_limiter = Instant::now();
        if let Some(p) = period {
            deadline = next_deadline(deadline, p, t_limiter);
            sleeper.sleep_until(deadline);
        }
        let t_governor = Instant::now();
        let ahead = opts
            .frames_ahead
            .unwrap_or_else(|| frames_ahead(renderer.frames().policy()));
        renderer.frames().wait_for_gpu(ahead)?;

        // 2. Input.
        let t_input = Instant::now();
        let event_gap = last_input.map(|prev| (t_input - prev).as_nanos() as u64);
        last_input = Some(t_input);
        platform.poll_events(|e| match e {
            Event::Quit | Event::CloseRequested => quit = true,
            Event::PixelSizeChanged { width, height } => resized = Some((width, height)),
            Event::Minimized | Event::Hidden => visible = false,
            Event::Restored | Event::Shown => {
                visible = true;
                occluded = false;
            }
            Event::Occluded => occluded = true,
            Event::Exposed => occluded = false,
            Event::FocusGained => {
                occluded = false;
                menu.focus(true);
            }
            Event::FocusLost => menu.focus(false),
            Event::Fullscreen(on) => fullscreen_event = Some(on),
            Event::Key {
                scancode: SCANCODE_F11,
                down: true,
                repeat: false,
                ..
            } => toggle_fullscreen = true,
            Event::Key {
                scancode: SCANCODE_F3,
                down: true,
                repeat: false,
                ..
            } => toggle_overlay = true,
            Event::Key {
                scancode,
                down,
                repeat,
                shift,
                shortcut,
                ..
            } => {
                shift_held = shift;
                keys.push((scancode, down, repeat, shift, shortcut));
            }
            Event::TextInput { .. } => typed.push_str(e.text()),
            Event::MouseMotion { x, y, dx, dy } => menu.mouse_moved(x, y, dx, dy),
            Event::MouseLeft => menu.mouse_left(),
            Event::MouseWheel { y, raw_x, raw_y } => menu.wheel(y, (raw_x, raw_y)),
            Event::MouseButton {
                button: 1,
                down,
                x,
                y,
                ..
            } => clicks.push((down, x, y, shift_held)),
            Event::MouseButton { button, down, .. } => menu.other_mouse_button(button, down),
        });
        let mut actions: Vec<Action> = clicks
            .drain(..)
            .filter_map(|(down, x, y, shift)| {
                menu.mouse_button(
                    down,
                    x,
                    y,
                    shift,
                    window.pixel_size(),
                    window.size(),
                    &mut renderer,
                    &window,
                )
            })
            .collect();
        for (code, down, repeat, shift, shortcut) in keys.drain(..) {
            actions.extend(menu.key(code, down, repeat, shift, shortcut, &window));
        }
        if !typed.is_empty() {
            menu.text_input(&typed);
            typed.clear();
        }
        if menu.wants_mouse_grab() != mouse_grabbed {
            mouse_grabbed = !mouse_grabbed;
            if !window.set_mouse_grabbed(mouse_grabbed) {
                eprintln!("[ruthenium] the mouse couldn't be grabbed or freed");
            }
        }
        if menu.wants_text_input() != text_input {
            text_input = !text_input;
            if text_input {
                window.start_text_input();
            } else {
                window.stop_text_input();
            }
        }
        for action in actions {
            match action {
                Action::Quit => quit = true,
                other => eprintln!("[ruthenium] {other:?}: not there yet"),
            }
        }
        // Checked here, not after the frame: a minimised window never gets that far.
        if quit || opts.quit_after.is_some_and(|q| started.elapsed() >= q) {
            break;
        }
        // The game's frame runs every time round, drawn or not, as vanilla's `runTick`.
        let t_game = Instant::now();
        menu.game_frame(&|| Some(terrain_state(&renderer)));
        if let Some(route) = menu.route() {
            if route.outcome().is_some() {
                // With --screenshot, the frame after the route's end is captured (the
                // player at rest, everything loaded with its mesh): the loop ends when
                // the picture is written, or when it cannot be had.
                frames_after_route += 1;
                if opts.screenshot.is_none() || frames_after_route > 200 {
                    break;
                }
                if !screenshot_requested {
                    renderer.request_screenshot();
                    screenshot_requested = true;
                }
            }
            if route.capturing() != capturing {
                capturing = !capturing;
                capture[usize::from(!capturing)] = Some(capture_state(&renderer));
                if capturing {
                    renderer.reset_world_latencies();
                }
            }
        }
        if let Some(size) = resized.take() {
            renderer.frames_mut().resize(size);
            if resize_asked && !screenshot_requested {
                // The frame about to be drawn is the first at the new size.
                renderer.request_screenshot();
                screenshot_requested = true;
            }
        }
        if std::mem::take(&mut toggle_overlay) {
            menu.toggle_frame_stats();
        }
        if std::mem::take(&mut toggle_fullscreen) {
            menu.toggle_fullscreen();
        }
        if let Some(on) = fullscreen_event.take() {
            // The window confirming a request (or catching up with an earlier one) changes
            // nothing; a change of its own (the window manager's) is the new setting.
            match fullscreen_pending {
                Some((asked, at)) if at.elapsed() < FULLSCREEN_PENDING => {
                    if asked == on {
                        fullscreen_pending = None;
                    }
                }
                _ => {
                    fullscreen_pending = None;
                    applied.fullscreen = on;
                    menu.set_fullscreen(on);
                }
            }
        }
        let wanted = menu.settings();
        if *wanted != applied {
            if wanted.vsync != applied.vsync {
                renderer.frames_mut().set_policy(policy(wanted.vsync));
            }
            if wanted.max_fps != applied.max_fps {
                period = cap(wanted).map(|f| Duration::from_secs_f64(1.0 / f));
            }
            if wanted.fullscreen != applied.fullscreen {
                window.set_fullscreen(wanted.fullscreen);
                fullscreen_pending = Some((wanted.fullscreen, Instant::now()));
            }
            if wanted.frame_stats != applied.frame_stats {
                renderer.overlay_mut().set_visible(wanted.frame_stats);
            }
            if wanted.game != applied.game {
                apply_game_options(&mut renderer, &wanted.game);
            }
            applied = wanted.clone();
        }
        if std::mem::take(&mut toggle_vsync) {
            let next = match renderer.frames().policy() {
                PresentPolicy::VSync => PresentPolicy::NoVSync,
                PresentPolicy::NoVSync => PresentPolicy::VSync,
            };
            renderer.frames_mut().set_policy(next);
            eprintln!("[ruthenium] {next:?}");
        }
        if opts.exercise_minimize {
            let minimize = (started.elapsed().as_millis() / MINIMIZE_EVERY.as_millis()) % 2 == 1;
            if minimize != minimize_asked {
                minimize_asked = minimize;
                if minimize {
                    window.minimize();
                } else {
                    window.restore();
                }
            }
        }
        if !visible {
            // Nothing is shown: tick at 10 FPS without presenting.
            throttled += u64::from(capturing);
            std::thread::sleep(HIDDEN_FRAME);
            last_present = None;
            last_input = None;
            continue;
        }

        frames += 1;
        if opts.exercise_swapchain {
            // A new swapchain in the first frame, with its warm-up draws, and then three
            // frames running now and then: each is replaced after one present.
            if frames == 1 || frames % 60 < 3 {
                renderer.frames_mut().invalidate();
            }
            // A capture of a swapchain's first frame.
            if frames % 60 == 1 && opts.screenshot.is_none() {
                renderer.request_screenshot();
            }
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
        if let Some(name) = start_screen.take() {
            if name == "settings" {
                menu.open_settings_named(window.pixel_size(), opts.select);
            } else if matches!(
                name.as_str(),
                "accounts" | "sign-in" | "code" | "sign-in-failed" | "sign-out"
            ) {
                menu.open_accounts_named(&name, window.pixel_size());
                if let Some(i) = opts.select {
                    menu.select_account(i);
                }
            } else {
                menu.open_servers(window.pixel_size());
                if let Some(i) = opts.select {
                    menu.select_server(i);
                }
                menu.open_form_named(&name, window.pixel_size());
            }
        }
        if let Some(address) = start_join.take() {
            menu.open_servers(window.pixel_size());
            menu.join_now(&address, window.pixel_size());
        }
        if let (Some(after), Some(address)) = (rejoin_after, &opts.join)
            && started.elapsed() >= after
        {
            // Joining again drops the session there is, which disconnects it.
            rejoin_after = None;
            eprintln!("[ruthenium] leaving and joining {address} again");
            menu.join_now(address, window.pixel_size());
        }
        let t_gui = Instant::now();
        // The world as the game thread last published it, seen from where the mouse
        // has turned to: taken before the menus borrow themselves for the frame.
        let world = menu.world_view();
        let hud = menu.hud_params();
        let gui = if opts.empty_window {
            None
        } else {
            let text_ready = menu.text_ready();
            if text_ready && text_shown.is_none() {
                text_shown = Some(Instant::now());
            }
            let gui = menu.frame(&mut renderer, window.pixel_size(), window.size(), |rgba| {
                window.set_icon(rgba, crate::menu::ICON_SIZE)
            });
            if opts.screenshot.is_some()
                && opts.route.is_none()
                && !screenshot_requested
                && !resize_asked
                && text_ready
                // (A drawn world has no text over it unless the statistics are on.)
                && (!gui.glyphs.is_empty() || renderer.world_ready())
                && text_shown.is_some_and(|t| t.elapsed() >= opts.screenshot_after)
            {
                match opts.screenshot_resize {
                    // The screenshot follows when the new size arrives.
                    Some((w, h)) => {
                        window.request_size(w, h);
                        resize_asked = true;
                    }
                    None => {
                        renderer.request_screenshot();
                        screenshot_requested = true;
                    }
                }
            }
            Some(gui)
        };
        let t_render = Instant::now();
        if opts.acquire_wait == AcquireWait::Pace {
            let deadline = pace_deadline(t_input, period, refresh_hz);
            renderer
                .frames_mut()
                .set_acquire_limit(ACQUIRE_TIMEOUT, Some(deadline));
        }
        let world = world.as_ref().map(|w| WorldFrame {
            scene: &w.scene,
            camera: w.camera,
            view_distance: w.view_distance,
            frame: &w.frame,
            partial_tick: w.partial_tick,
            hud,
        });
        let report = renderer.render(&BACKGROUND, world, gui)?;
        if opts.exercise_swapchain
            && opts.screenshot.is_none()
            && let Some((w, h, rgba)) = renderer.take_screenshot()?
        {
            if rgba.len() != w as usize * h as usize * 4 {
                return Err(format!("a {w}x{h} capture has {} bytes", rgba.len()).into());
            }
            captures.0 += 1;
            captures.1 += u32::from(renderer.screenshot_of().is_some_and(|s| s.2));
        }
        if let Some(path) = &opts.screenshot
            && let Some((w, h, rgba)) = renderer.take_screenshot()?
        {
            let png = ruthenium_assets::Image {
                width: w,
                height: h,
                pixels: rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    // Opaque, as the window shows the frame (and as the game saves its
                    // screenshots): the frame's own alpha is what was last drawn there,
                    // a cut-out texel's among others.
                    .map(|p| {
                        ruthenium_assets::image::argb(255, p[0].into(), p[1].into(), p[2].into())
                    })
                    .collect(),
            }
            .encode_png();
            std::fs::write(path, png)?;
            eprintln!("[ruthenium] screenshot written to {}", path.display());
            if let Some((frame, generation, first)) = renderer.screenshot_of() {
                eprintln!(
                    "[ruthenium] it is {w}x{h} pixels, of frame {frame}, {} drawn to swapchain {generation}",
                    if first { "the first" } else { "not the first" }
                );
            }
            break;
        }
        let now = Instant::now();
        profiling::finish_frame!();

        let presented = report.presented == Presented::Yes;
        let mut interval_ns = 0;
        if presented {
            if first_frame.is_none() {
                first_frame = Some(started.elapsed());
                eprintln!(
                    "[ruthenium] window after {:.0} ms, first frame after {:.0} ms",
                    startup.window_ms,
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
            if let Some(prev) = last_present {
                interval_ns = (now - prev).as_nanos() as u64;
                interval.push(interval_ns);
                renderer.overlay_mut().push_frame(interval_ns as f32 / 1e6);
            }
            last_present = Some(now);
        }
        // A route says which frames count; without one, those after the start-up.
        let measuring = match menu.route() {
            Some(_) => capturing,
            None => started.elapsed() > WARMUP + first_frame.unwrap_or(Duration::MAX / 4),
        };
        let marks = Marks {
            started,
            limiter: t_limiter,
            governor: t_governor,
            input: t_input,
            game: t_game,
            gui: t_gui,
            render: t_render,
            end: now,
        };
        let sample = FrameSample::new(
            &marks,
            renderer.frames().timings(),
            renderer.world_times(),
            presented,
            interval_ns,
        );
        debug_assert_eq!(sample.frame, report.frame);
        cpu.push(sample.cpu_ns);
        telemetry.frame(&sample, measuring);
        if let Some(gap) = event_gap {
            telemetry.event_gap(gap, measuring);
        }
        if let Some(g) = report.gpu_time {
            gpu_times.push(g.nanos);
            telemetry.gpu_time(g, measuring);
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
            // The window may have moved to another display.
            refresh_hz = window.refresh_rate();
        }
        if occluded {
            // Covered by other windows: keep presenting (so the compositor can tell us when
            // it is visible again) but only at 10 FPS.
            throttled += u64::from(capturing);
            std::thread::sleep(HIDDEN_FRAME);
            last_present = None;
            last_input = None;
        }
    }

    menu.save_settings();
    // A route's record, and how it ended.
    let mut failed = false;
    let route_run = menu.route().map(RoutePlayer::record);
    if let Some(route) = menu.route() {
        match route.outcome() {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                eprintln!("[ruthenium] the {} route failed: {e}", route.route().name);
                failed = true;
            }
            None => {
                eprintln!(
                    "[ruthenium] the {} route was not played to its end",
                    route.route().name
                );
                failed = true;
            }
        }
        if throttled > 0 {
            eprintln!(
                "[ruthenium] {throttled} frames of the route's capture were held back: the window was hidden or covered"
            );
            failed = true;
        }
    }
    if let (Some(path), Some(record)) = (&opts.route_out, &route_run)
        && let Err(e) = std::fs::write(path, record.to_string())
    {
        eprintln!("[ruthenium] could not write {}: {e}", path.display());
        return Ok(ExitCode::FAILURE);
    }
    let run = telemetry.interval.summary();
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
    let counters = *renderer.frames().counters();
    if telemetry.frames > 0 {
        eprintln!("[ruthenium] {}", telemetry.main_thread_line(&counters));
    }
    if let Some(path) = &opts.frame_log
        && let Err(e) = telemetry.write_log(path)
    {
        eprintln!("[ruthenium] could not write {}: {e}", path.display());
        return Ok(ExitCode::FAILURE);
    }
    if let Some(path) = &opts.bench_out {
        startup.first_frame_ms = first_frame.map(|d| d.as_secs_f64() * 1e3);
        startup.text_ready_ms = text_shown.map(|t| (t - started).as_secs_f64() * 1e3);
        let policy = renderer.frames().policy();
        let world = renderer.world_stats().map(|_| {
            let [at_start, at_end] = &capture;
            let session = menu.session_stats().map(|(handoff, dirty)| {
                json!({
                    "input_frames": handoff.frames,
                    "input_queue_high_water": handoff.high_water,
                    "oldest_input_ms": handoff.oldest_age.as_secs_f64() * 1e3,
                    "main_thread_waits": handoff.stalls,
                    "main_thread_wait_ms": handoff.stall_time.as_secs_f64() * 1e3,
                    "longest_wait_ms": handoff.longest_stall.as_secs_f64() * 1e3,
                    "dirty_queue_high_water": dirty.high_water,
                    "dirty_queue_overflows": dirty.overflows,
                    "dirty_keys": dirty.queued,
                    "dirty_keys_coalesced": dirty.coalesced,
                })
            });
            json!({
                "draws": renderer.terrain_draws().name(),
                "culling": if renderer.terrain_culling() { "graph" } else { "frustum" },
                "smooth_lighting": renderer.smooth_lighting(),
                "biome_blend": renderer.biome_blend(),
                "shader": renderer.terrain_shader().name(),
                "fog": renderer.atmosphere_options().fog,
                "chunk_fade_ms": renderer.chunk_fade_ms(),
                "sky": renderer.sky_order().name(),
                "translucent": renderer.translucent(),
                "block_entities": renderer.block_entities(),
                "players": renderer.players(),
                "entity_shadows": renderer.entity_shadows(),
                "hand": renderer.hand(),
                "block_outline": renderer.block_outline(),
                "view_bobbing": renderer.view_options().bobbing,
                "animations": renderer.animate(),
                "fov_effects": renderer.fov_effects(),
                "light_flicker": renderer.atmosphere_options().flicker,
                "mesh_workers": menu.mesh_workers(),
                "envelope_bytes": ruthenium_render::terrain::POOL_ENVELOPE,
                "at_capture_start": at_start,
                "at_capture_end": at_end,
                "at_exit": capture_state(&renderer),
                "latency": renderer.world_latencies().map(|l| json!({
                    "first_mesh": latency_json(&l.first),
                    "server_change": latency_json(&l.changed),
                    "own_change": latency_json(&l.own),
                })),
                "session": session,
            })
        });
        let json = telemetry.bench_json(&RunInfo {
            started_epoch_ms,
            world,
            route_run,
            throttled_frames: throttled,
            scene: match menu.route() {
                Some(r) => r.route().name.as_str(),
                None if opts.empty_window => "empty",
                None => "menu",
            },
            caps: renderer.gpu().caps(),
            transfer_queue: renderer.gpu().transfer_family().is_some(),
            policy,
            frames_ahead: opts.frames_ahead.unwrap_or_else(|| frames_ahead(policy)),
            acquire_wait: &acquire_wait,
            fps_cap: period.map(|p| 1.0 / p.as_secs_f64()),
            swapchain: renderer.frames().swapchain_info(),
            window_points: window.size(),
            window_pixels: window.pixel_size(),
            startup,
            counters,
            memory: renderer.gpu().memory_report(),
            textures: renderer.texture_stats(),
            pipeline_cache: renderer.pipeline_cache(),
        });
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("[ruthenium] could not write {}: {e}", path.display());
            return Ok(ExitCode::FAILURE);
        }
    }
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
    if let Some((c, ring, pool, limit_hits)) = renderer.stream_check() {
        eprintln!(
            "{}",
            crate::telemetry::stream_check_lines(&c, &ring, &pool, limit_hits)
        );
        match c.missing() {
            Some(what) => eprintln!("[ruthenium] streaming check: the run did not get to {what}"),
            None if limit_hits == 0 => {
                eprintln!("[ruthenium] streaming check: no reservation was ever refused")
            }
            None => {}
        }
        failed |= !c.passed() || limit_hits == 0;
    }
    let table = renderer.texture_stats();
    eprintln!(
        "[ruthenium] texture table: at most {} of {} slots taken ({} live, {} being freed){}",
        table.peak_taken,
        table.capacity,
        table.peak_live,
        table.peak_retiring,
        match table.refused {
            0 => String::new(),
            n => format!("; {n} creations refused when full"),
        }
    );
    if let Some(c) = renderer.table_check() {
        eprintln!(
            "[ruthenium] texture table check ({} slots, {} sets): {} grids, {} slots compared, {} wrong; {} textures created, table full {} times{}, {} reloads, {} creations deferred while slots were freed, {} stuck",
            table.capacity,
            renderer.gpu().caps().texture_table.name(),
            c.grids,
            c.slots,
            c.mismatches,
            c.created,
            c.exhausted,
            if c.capped {
                " (larger than the check fills)"
            } else {
                ""
            },
            c.reloads,
            c.deferred,
            c.stuck
        );
        failed |= !c.passed();
    }
    if let Some(c) = renderer.colour_check() {
        eprintln!(
            "[ruthenium] colour check ({:?}): grey and a swatch blended over it read back {} times off screen and {} times from the window, {} wrong{}",
            renderer
                .frames()
                .swapchain_info()
                .map(|s| s.format)
                .unwrap_or_default(),
            c.offscreen,
            c.window,
            c.mismatches,
            match c.window_unreadable {
                0 => String::new(),
                n => format!("; the window could not be read in {n} frames"),
            }
        );
        failed |= !c.passed();
    }
    if let Some(s) = renderer.world_stats() {
        for line in crate::ingame::terrain_lines(&s).lines() {
            eprintln!("[ruthenium] {line}");
        }
    }
    if let Some(c) = renderer.terrain_check() {
        eprintln!(
            "[ruthenium] terrain check: {} frames of {} rectangles compared, {} wrong",
            c.compared, c.probes, c.mismatches
        );
        failed |= !c.passed();
    }
    if opts.exercise_swapchain {
        let fallbacks = renderer.glass_fallbacks();
        eprintln!(
            "[ruthenium] swapchain exercise: {} created, {} destroyed while running, at most {} waiting at once; {} captures read back, {} of a swapchain's first frame; {} frames drew glass without its backdrop",
            counters.swapchains,
            counters.swapchains_destroyed,
            counters.retired_peak,
            captures.0,
            captures.1,
            fallbacks
        );
        // Each replaced swapchain must go while the client runs, and the backdrop must be
        // there in a swapchain's first frame.
        // (With --exercise-textures the table is full on purpose, and a backdrop remade
        // for a new size waits for a slot like any other texture.)
        failed |= (fallbacks > 0 && !opts.exercise_textures)
            || captures.0 == 0
            || counters.swapchains_destroyed + 8 < counters.swapchains;
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

/// What a route waits for and records of the terrain: whether everything loaded has
/// its mesh on the GPU, and the numbers of the moment.
fn terrain_state(renderer: &Renderer) -> Terrain {
    let Some(s) = renderer.world_stats() else {
        return Terrain {
            pending: Some("the world's models and textures are still being prepared".into()),
            mark: Value::Null,
        };
    };
    let pending = if !renderer.world_ready() {
        Some(
            "the block atlas, the light map or the quad indices are on their way to the GPU".into(),
        )
    } else if s.dirty + s.jobs_out + s.pending > 0 {
        Some(format!(
            "sections are still being built: {} wait for a mesh, {} are with a worker, {} on their way to the GPU",
            s.dirty, s.jobs_out, s.pending
        ))
    } else if s.graph_behind {
        Some("what the new meshes let the camera see is still being worked out".into())
    } else {
        None
    };
    Terrain {
        pending,
        mark: terrain_json(&s),
    }
}

/// The terrain's numbers of the moment.
fn terrain_json(s: &ruthenium_render::WorldStats) -> Value {
    json!({
        "sections": s.sections,
        "meshed": s.meshed,
        "dirty": s.dirty,
        "jobs_out": s.jobs_out,
        "pending": s.pending,
        "jobs": s.tracker.jobs,
        "own_change_jobs": s.tracker.urgent_jobs,
        "stale_results": s.tracker.stale,
        "jobs_by_cause": ruthenium_render::terrain::CAUSES
            .iter()
            .zip(s.tracker.causes)
            .map(|(name, n)| ((*name).to_owned(), json!(n)))
            .collect::<serde_json::Map<String, Value>>(),
        "rescans": s.tracker.rescans,
        "installed": s.installed,
        "installed_quads": s.installed_quads,
        "installed_bytes": s.installed_bytes,
        "mesh_micros": s.mesh_micros,
        "slowest_mesh_micros": s.slowest_mesh_micros,
        "slowest_install_ms": s.slowest_install_ms,
        "translucent_quads": s.translucent_quads,
        "translucent_sorts": s.sorts,
        "translucent_quads_drawn": s.translucent_drawn,
        "translucent_sections_drawn": s.translucent_sections,
        "block_entities_drawn": s.block_entities.drawn,
        "players_drawn": s.block_entities.players,
        "names_drawn": s.block_entities.names,
        "shadow_pieces": s.block_entities.shadows,
        "block_entity_parts": s.block_entities.parts,
        "hand_parts": s.block_entities.hand_parts,
        "sign_texts": s.block_entities.texts,
        "sign_glyphs": s.block_entities.glyphs,
        "block_entity_draws": s.block_entities.draws,
        "block_entity_parts_overflowed": s.block_entities.overflowed,
        "animated_sprites": s.animated_sprites,
        "animation_ticks": s.animation_ticks,
        "animation_writes": s.animation_writes,
        "animation_bytes": s.animation_bytes,
        "animation_waits": s.animation_waits,
        "slowest_animation_micros": s.slowest_animation_micros,
        "out_of_range": s.out_of_range,
        "meshes_dropped": s.dropped,
        "sections_overflowed": s.overflowed,
        "mesh_bytes": s.mesh_bytes,
        "culling": s.culling,
        "reached": s.reached,
        "hidden_meshes": s.hidden,
        "graph_updates": s.graph_updates,
        "graph_walks": s.graph_walks,
        "graph_micros": s.graph_micros,
        "slowest_graph_micros": s.slowest_graph_micros,
        "tested": s.tested,
        "visible": s.visible,
        "quads": s.quads,
        "draws": s.draws,
        "calls": s.calls,
        "binds": s.binds,
        "pool": {
            "pages": s.pool.pages,
            "reserved_bytes": s.pool.reserved_bytes,
            "payload_bytes": s.pool.payload_bytes,
            "retiring_bytes": s.pool.retiring_bytes,
            "alignment_waste": s.pool.alignment_waste,
            "free_bytes": s.pool.free_bytes,
            "largest_free": s.pool.largest_free,
            "live": s.pool.live,
            "retiring": s.pool.retiring,
            "failures": s.pool.failures,
            "failures_with_space": s.pool.failures_with_space,
            "peak_reserved_bytes": s.pool.peak_reserved_bytes,
            "peak_payload_bytes": s.pool.peak_payload_bytes,
            "pages_added": s.pool.pages_added,
            "pages_released": s.pool.pages_released,
        },
    })
}

/// The terrain and the upload ring as they stand: taken when a route's capture begins
/// and ends, so that what happened in between is the difference.
fn capture_state(renderer: &Renderer) -> Value {
    let u = renderer.upload_stats();
    json!({
        "terrain": renderer.world_stats().map(|s| terrain_json(&s)),
        "block_marks": renderer.mark_stats().map(|s| json!({
            "outline_edges": s.lines, "cracked_blocks": s.cracked, "crack_quads": s.cracks
        })),
        "uploads": {
            "render_memcpy_bytes": u.render_memcpy_bytes,
            "worker_write_bytes": u.worker_write_bytes,
            "gpu_copies": u.gpu_copies,
            "gpu_copy_bytes": u.gpu_copy_bytes,
            "ring_occupied": u.ring_occupied,
            "ring_peak": u.ring_peak,
            "oldest_wait": u.oldest_wait,
            "oldest_reservation_wait": u.oldest_reservation_wait,
            "copy_submissions": u.copy_submissions,
            "deferred": u.deferred,
            "budget_refusals": u.budget_refusals,
            "reservations": u.reservations,
            "reservations_refused": u.reservations_refused,
            "reservations_peak": u.reservations_peak,
            "worker_bytes_peak": u.worker_bytes_peak,
        },
        "memory": {
            "gpu_only": renderer.gpu().memory_report().gpu_only,
            "upload": renderer.gpu().memory_report().upload,
            "dynamic": renderer.gpu().memory_report().dynamic,
        },
    })
}

fn latency_json(l: &ruthenium_render::LatencySummary) -> Value {
    json!({
        "samples": l.samples,
        "p50_ms": l.p50_ms,
        "p99_ms": l.p99_ms,
        "max_ms": l.max_ms,
        "p50_frames": l.p50_frames,
        "p99_frames": l.p99_frames,
        "max_frames": l.max_frames,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acquire_may_wait_two_pacing_intervals() {
        let t = Instant::now();
        let after = |limiter: Option<f64>, hz: Option<f32>| {
            let d = pace_deadline(t, limiter.map(Duration::from_secs_f64), hz) - t;
            (d.as_secs_f64() * 1e3).round() as u32
        };
        // The display's interval, twice; 60 Hz when it is unknown.
        assert_eq!(after(None, Some(60.0)), 33);
        assert_eq!(after(None, None), 33);
        // A limiter slower than the display counts instead; a faster one doesn't, as with
        // VSync an acquire still takes up to a refresh.
        assert_eq!(after(Some(1.0 / 30.0), Some(60.0)), 67);
        assert_eq!(after(Some(1.0 / 240.0), Some(60.0)), 33);
        // Never less than the floor, nor more than the longest an acquire may block.
        assert_eq!(after(None, Some(240.0)), 8);
        assert_eq!(after(None, Some(500.0)), 8);
        assert_eq!(after(Some(1.0), Some(60.0)), 100);
    }
}
