//! Ruthenium's renderer: GPU start-up for a window, the pipeline set, the upload ring and
//! texture table, and a frame that draws the world's terrain or the menu background, and
//! the GUI's draw list over it, recorded and presented through
//! [`ruthenium_gpu::FrameLoop`].

mod backdrop;
mod check_pass;
mod colour_check;
mod gui;
mod overlay;
mod pipelines;
#[cfg(test)]
mod reflect;
mod starfield;
mod stats;
mod table_check;
pub mod terrain;
mod terrain_check;
mod terrain_vertex;

pub use colour_check::ColourCheckReport;
pub use gui::GuiFrame;
pub use overlay::{LINES as OVERLAY_LINES, Overlay, OverlayStats};
pub use pipelines::PipelineId;
pub use stats::{FrameTimes, Summary};
pub use table_check::TableCheckReport;
pub use terrain::{
    CameraPose, LatencySummary, TerrainDraws, WorldAssets, WorldLatencies, WorldStats,
};
pub use terrain_check::TerrainCheckReport;
pub use terrain_vertex::{TERRAIN_VERTEX_CANDIDATES, VertexCandidate};

use std::path::Path;
use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, DepthTarget, Encoder, FrameData, FrameLoop, Gpu, GpuOptions, GpuTime, Instance,
    InstanceDesc, PipelineCache, PipelineLayout, Pipelines, PoolStats, PresentPolicy, Presented,
    Screenshot, StreamCheck, StreamCheckReport, SwapchainInfo, TextureDesc, TextureId,
    TextureTable, TextureTableStats, UploadCheck, UploadCheckReport, UploadRing, UploadStats,
    Validation, vk,
};
use ruthenium_platform::{Platform, Window};
use ruthenium_sim::environment::Environment;
use ruthenium_sim::scene::Scene;

use terrain::world::WorldRenderer;
use terrain::{LightmapInputs, View};

/// Creates the Vulkan instance, the window's surface and the device.
///
/// `window` must outlive the returned Gpu (and everything created from it).
pub fn create_gpu(
    platform: &Platform,
    window: &Window,
    validation: Validation,
    options: &GpuOptions,
) -> Result<Arc<Gpu>, Box<dyn std::error::Error>> {
    let surface_extensions = platform.vulkan_instance_extensions();
    let instance = Instance::new(&InstanceDesc {
        get_instance_proc_addr: platform.vk_get_instance_proc_addr()?,
        surface_extensions: &surface_extensions,
        validation,
    })?;
    // SAFETY: the instance was created with SDL's surface extensions; Gpu takes ownership
    // of the surface and destroys it before the instance; the caller keeps the window alive.
    let surface = unsafe { window.create_surface(instance.handle())? };
    Ok(Arc::new(Gpu::new(instance, surface, options)?))
}

/// What one rendered frame reports back to the loop.
#[derive(Clone, Copy, Debug)]
pub struct FrameReport {
    /// The frame's number: the one its GPU time is reported under later, and
    /// [`ruthenium_gpu::FrameTimings::frame`].
    pub frame: u64,
    pub presented: Presented,
    /// GPU time of an earlier frame that has just been read back.
    pub gpu_time: Option<GpuTime>,
}

/// The menu background: a vertical gradient between two display-encoded RGBA colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Background {
    pub top: [f32; 4],
    pub bottom: [f32; 4],
}

impl Background {
    /// `background.frag`'s push constants: the two colours, then the starfield texture.
    fn push_constants(&self, stars: TextureId) -> [u8; 36] {
        let mut b = [0u8; 36];
        for (i, v) in self.top.iter().chain(&self.bottom).enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
        b[32..].copy_from_slice(&stars.index().to_ne_bytes());
        b
    }
}

/// The render thread's time for the terrain in one frame, in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldTimes {
    /// Taking in what changed in the level and the finished meshes, staging their
    /// uploads, handing out jobs and putting arrived meshes in place.
    pub stage_ns: u64,
    /// Choosing the sections in view, writing their records and recording their draws.
    pub draw_ns: u64,
}

/// The world as one frame draws it.
#[derive(Debug, Clone, Copy)]
pub struct WorldFrame<'a> {
    /// What the game thread publishes: the level, to mesh from.
    pub scene: &'a Scene,
    pub camera: CameraPose,
    /// How many chunks far the world is drawn: the render distance the server was
    /// told.
    pub view_distance: i32,
    /// The dimension's own look: how faces are shaded, its light and its sky.
    pub environment: &'a Environment,
}

/// The colour the world's pass is cleared to: the dimension's sky colour as its type
/// gives it (black where it gives none). Until the sky is drawn, that is all of it.
fn sky_colour(environment: &Environment) -> [f32; 4] {
    let [r, g, b] =
        terrain::colour_attribute(environment, "minecraft:visual/sky_color").unwrap_or([0.0; 3]);
    [r, g, b, 1.0]
}

/// Staging ring size (research 04 §4.3: 8 MiB on the lowest tier, 16 MiB by default).
const UPLOAD_RING_BYTES: u64 = 16 << 20;
/// Bytes one frame may upload: less on integrated GPUs, which share memory bandwidth
/// with the CPU. One larger upload still goes, alone in its frame.
const UPLOAD_BUDGET_DISCRETE: u64 = 4 << 20;
const UPLOAD_BUDGET_INTEGRATED: u64 = 3 << 19;
/// Per-frame data each frame may write (GUI and overlay rectangles for now).
const FRAME_DATA_BYTES: u64 = 1 << 20;

/// Draws frames into the window.
#[derive(Debug)]
pub struct Renderer {
    // Fields drop in order: the frame loop first (its Drop waits for the GPU), then what
    // its frames used, then the cache, which saves itself as it goes.
    frames: FrameLoop,
    /// `--exercise-streaming`. Before the upload ring: its worker thread writes into
    /// the ring's staging memory and has to end first.
    stream_check: Option<StreamCheck>,
    pipelines: Pipelines,
    layout: PipelineLayout,
    textures: TextureTable,
    uploads: UploadRing,
    /// The world's terrain, once its assets are loaded. After the upload ring: its
    /// buffer pages go once the ring's copies into them have finished. (Its worker
    /// threads, which write into the ring's staging, are stopped before anything is
    /// dropped: see `Drop`.)
    world: Option<WorldRenderer>,
    /// The depth image of the passes the world is drawn in, the size of the window.
    depth: Option<DepthTarget>,
    /// How the terrain's sections are to be drawn, for the world once it is loaded.
    terrain_draws: TerrainDraws,
    terrain_culling: bool,
    /// The terrain's part of the last frame.
    world_times: WorldTimes,
    /// The number of the last frame recorded.
    last_frame: u64,
    frame_data: FrameData,
    overlay: Overlay,
    cache: PipelineCache,
    stars: TextureId,
    /// `--exercise-uploads`.
    check: Option<UploadCheck>,
    /// `--exercise-textures`.
    table_check: Option<table_check::TableCheck>,
    /// `--exercise-colour`.
    colour_check: Option<colour_check::ColourCheck>,
    /// `--exercise-terrain`.
    terrain_check: Option<terrain_check::TerrainCheck>,
    /// The backdrop could not be made in the last frame that wanted it.
    backdrop_failed: bool,
    /// The next presented frame is copied for a screenshot.
    screenshot_requested: bool,
    screenshot: Option<Screenshot>,
    /// The frame, swapchain and "first image of that swapchain" of the last screenshot
    /// recorded.
    screenshot_of: Option<(u64, u32, bool)>,
    /// The menus' backdrop, while frames draw shapes.
    backdrop: Option<backdrop::Backdrop>,
    /// Frames that drew glass shapes into the window without a backdrop to sample.
    glass_fallbacks: u64,
    /// `--exercise-swapchain`: every so often a frame replaces its swapchain twice.
    exercise_recreation: bool,
}

impl Renderer {
    /// Creates the frame loop and builds every pipeline through the on-disk pipeline cache
    /// in `cache_dir` (`None`: no cache file).
    pub fn new(
        gpu: Arc<Gpu>,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
        cache_dir: Option<&Path>,
    ) -> Result<Renderer, ruthenium_gpu::Error> {
        let frames = FrameLoop::new(gpu.clone(), pixel_size, policy)?;
        let budget = if gpu.caps().unified_memory {
            UPLOAD_BUDGET_INTEGRATED
        } else {
            UPLOAD_BUDGET_DISCRETE
        };
        let mut uploads = UploadRing::new(gpu.clone(), UPLOAD_RING_BYTES, budget)?;
        let table_started = std::time::Instant::now();
        let mut textures = TextureTable::new(gpu.clone(), &mut uploads)?;
        let table_ms = table_started.elapsed().as_secs_f64() * 1e3;
        let stars = textures.create_with(
            &mut uploads,
            TextureDesc::rgba8(starfield::SIZE, starfield::SIZE),
            &[&starfield::bake(starfield::SEED)],
            // The first frame draws it.
            Arrival::NextFrame,
        )?;
        let limits = gpu.caps().texture_table_limits;
        eprintln!(
            "[render] texture table: {} slots in {} descriptor sets, made in {table_ms:.2} ms (this GPU: up to {} ordinary, {} update-after-bind)",
            textures.capacity(),
            textures.path().name(),
            limits.ordinary,
            match limits.update_after_bind {
                Some(n) => n.to_string(),
                None => "no".to_owned(),
            }
        );
        eprintln!(
            "[render] upload ring {} MiB, {:.1} MiB per frame, buffer copies and background textures on the {}",
            uploads.capacity() >> 20,
            uploads.budget() as f64 / f64::from(1 << 20),
            match gpu.transfer_family() {
                Some(f) => format!("transfer queue (family {f})"),
                None => "graphics queue".to_owned(),
            }
        );
        let frame_data = FrameData::new(gpu.clone(), FRAME_DATA_BYTES)?;
        let layout = PipelineLayout::new(gpu.clone(), &[textures.layout(), frame_data.layout()])?;
        let descs = pipelines::descs();
        let interface = ruthenium_gpu::interface_hash(&descs, &textures);
        let mut cache = PipelineCache::open(gpu.clone(), cache_dir, interface)?;
        let pipelines = Pipelines::build(gpu, &mut cache, &layout, &descs, frames.color_format()?)?;
        Ok(Renderer {
            frames,
            stream_check: None,
            pipelines,
            layout,
            textures,
            uploads,
            world: None,
            depth: None,
            terrain_draws: TerrainDraws::default(),
            terrain_culling: true,
            world_times: WorldTimes::default(),
            last_frame: 0,
            frame_data,
            overlay: Overlay::new(),
            cache,
            stars,
            check: None,
            table_check: None,
            colour_check: None,
            terrain_check: None,
            backdrop_failed: false,
            screenshot_requested: false,
            screenshot: None,
            screenshot_of: None,
            backdrop: None,
            glass_fallbacks: 0,
            exercise_recreation: false,
        })
    }

    /// Takes the world's assets: creates the block atlas's texture (which then streams
    /// to the GPU over the next frames) and the terrain's first buffer page, and starts
    /// `workers` meshing threads. Call it behind a loading screen. A world already
    /// loaded is kept as it is.
    pub fn load_world(
        &mut self,
        assets: WorldAssets,
        workers: usize,
    ) -> Result<(), ruthenium_gpu::Error> {
        if self.world.is_none() {
            let gpu = self.frames.gpu().clone();
            self.world = Some(WorldRenderer::new(
                &gpu,
                &mut self.textures,
                &mut self.uploads,
                assets,
                workers,
                self.terrain_culling,
            )?);
            self.set_terrain_draws(self.terrain_draws);
        }
        Ok(())
    }

    /// Whether only what can be seen from the camera through the sections' open faces
    /// is meshed and drawn (cave culling, on unless said otherwise), or everything
    /// loaded that is inside the view. Takes effect when the world's assets are loaded
    /// ([`Self::load_world`]).
    pub fn set_terrain_culling(&mut self, culling: bool) {
        self.terrain_culling = culling;
    }

    /// Whether the terrain is drawn with cave culling (what was asked for until a
    /// world is loaded).
    pub fn terrain_culling(&self) -> bool {
        self.world
            .as_ref()
            .map_or(self.terrain_culling, |w| w.stats().culling)
    }

    /// Asks for the terrain's sections to be drawn this way. A GPU that cannot draw
    /// several commands in one call draws a page at a time instead, and says so once.
    pub fn set_terrain_draws(&mut self, draws: TerrainDraws) {
        self.terrain_draws = draws;
        if let Some(w) = &mut self.world {
            let actual = w.set_draws(draws);
            if actual != draws {
                eprintln!(
                    "[render] this GPU has no multi-draw indirect with a first instance: the terrain is drawn {} instead",
                    actual.name()
                );
            }
        }
    }

    /// How the terrain's sections are drawn (what was asked for until a world is
    /// loaded).
    pub fn terrain_draws(&self) -> TerrainDraws {
        self.world
            .as_ref()
            .map_or(self.terrain_draws, WorldRenderer::draws)
    }

    /// The terrain's part of the render thread's time in the last frame.
    pub fn world_times(&self) -> WorldTimes {
        self.world_times
    }

    /// From a change to the frame that shows it, for the meshes installed since the
    /// last [`Self::reset_world_latencies`].
    pub fn world_latencies(&self) -> Option<WorldLatencies> {
        self.world.as_ref().map(WorldRenderer::latencies)
    }

    /// Forgets the latencies so far (a measurement begins).
    pub fn reset_world_latencies(&mut self) {
        if let Some(w) = &mut self.world {
            w.reset_latencies();
        }
    }

    /// The upload ring's counters, and how full it is now.
    pub fn upload_stats(&self) -> ruthenium_gpu::UploadStats {
        self.uploads.stats()
    }

    /// Whether the world's assets are loaded ([`Self::load_world`]).
    pub fn has_world(&self) -> bool {
        self.world.is_some()
    }

    /// Whether frames given a world draw it: its atlas, light map and indices have
    /// arrived on the GPU.
    pub fn world_ready(&self) -> bool {
        self.world.as_ref().is_some_and(|w| w.ready(&self.textures))
    }

    /// How far the block atlas has got on its way to the GPU, 0 to 1.
    pub fn world_progress(&self) -> f32 {
        self.world
            .as_ref()
            .map_or(0.0, |w| w.atlas_progress(&self.textures))
    }

    /// Forgets every section's mesh: the session they were of has ended. The assets
    /// stay for the next one.
    pub fn clear_world(&mut self) {
        if let Some(w) = &mut self.world {
            w.clear(&mut self.uploads, self.last_frame);
        }
    }

    /// What the terrain is doing, if a world is loaded.
    pub fn world_stats(&self) -> Option<WorldStats> {
        self.world.as_ref().map(WorldRenderer::stats)
    }

    /// Makes the depth image the size of the target the world is drawn into.
    fn ensure_depth(&mut self, extent: vk::Extent2D) -> Result<(), ruthenium_gpu::Error> {
        if self.depth.as_ref().is_some_and(|d| d.extent() == extent) {
            return Ok(());
        }
        // The old one first: it waits for the frames that used it.
        self.depth = None;
        self.depth = Some(DepthTarget::new(self.frames.gpu().clone(), extent)?);
        Ok(())
    }

    /// Starts checking every frame that uploads arrive intact (see [`UploadCheck`]).
    pub fn enable_upload_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.check.is_none() {
            self.check = Some(UploadCheck::new(self.frames.gpu().clone())?);
            // Also run the path where a frame's transfer copies wait for the next frame.
            self.uploads.force_deferral_every(3);
        }
        Ok(())
    }

    /// What the upload check has seen, if it is on; how many frames' transfer copies had
    /// to wait for a free command buffer; and how many images went on the transfer queue.
    pub fn upload_check(&self) -> Option<(UploadCheckReport, u64, u64)> {
        self.check.as_ref().map(|c| {
            (
                c.report(),
                self.uploads.deferred_transfers(),
                self.uploads.transferred_images(),
            )
        })
    }

    /// Starts streaming a texture larger than the upload ring, worker-written blocks and
    /// pooled ranges every frame (see [`StreamCheck`]). It needs the upload ring to
    /// itself: not together with the upload or the texture table check.
    pub fn enable_stream_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.stream_check.is_none() {
            self.stream_check = Some(StreamCheck::new(
                self.frames.gpu().clone(),
                &mut self.uploads,
            )?);
        }
        Ok(())
    }

    /// What the streaming check has seen, if it is on: its report, the upload ring's
    /// counters, its pool's, and how often a reservation was refused for want of room
    /// while an urgent one still got some.
    pub fn stream_check(&self) -> Option<(StreamCheckReport, UploadStats, PoolStats, u64)> {
        self.stream_check.as_ref().map(|c| {
            (
                c.report(),
                self.uploads.stats(),
                c.pool_stats(),
                c.limit_hits(),
            )
        })
    }

    /// Starts filling, reloading and reading back the texture table every frame (see
    /// [`TableCheckReport`]).
    pub fn enable_table_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.table_check.is_none() {
            self.table_check = Some(table_check::TableCheck::new(
                self.frames.gpu().clone(),
                self.pipelines.format(),
                self.textures.capacity(),
            )?);
        }
        Ok(())
    }

    /// Starts drawing and reading back the colour fixture every frame (see
    /// [`ColourCheckReport`]).
    pub fn enable_colour_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.colour_check.is_none() {
            self.colour_check = Some(colour_check::ColourCheck::new(
                self.frames.gpu().clone(),
                self.pipelines.format(),
                &mut self.textures,
                &mut self.uploads,
            )?);
        }
        Ok(())
    }

    /// Starts drawing and reading back the terrain fixture every frame (see
    /// [`TerrainCheckReport`]).
    pub fn enable_terrain_check(&mut self) -> Result<(), ruthenium_gpu::Error> {
        if self.terrain_check.is_none() {
            self.terrain_check = Some(terrain_check::TerrainCheck::new(
                self.frames.gpu().clone(),
                self.pipelines.format(),
                &mut self.textures,
                &mut self.uploads,
            )?);
        }
        Ok(())
    }

    /// What the terrain check has seen, if it is on.
    pub fn terrain_check(&self) -> Option<TerrainCheckReport> {
        self.terrain_check
            .as_ref()
            .map(terrain_check::TerrainCheck::report)
    }

    /// What the colour check has seen, if it is on.
    pub fn colour_check(&self) -> Option<ColourCheckReport> {
        self.colour_check
            .as_ref()
            .map(colour_check::ColourCheck::report)
    }

    /// What the texture table check has seen, if it is on.
    pub fn table_check(&self) -> Option<TableCheckReport> {
        self.table_check
            .as_ref()
            .map(table_check::TableCheck::report)
    }

    /// The pipeline cache this run opened: its interface hash, the bytes it loaded (none
    /// on a cold start) and whether it is kept in a file.
    pub fn pipeline_cache(&self) -> (u64, usize, bool) {
        (
            self.cache.interface(),
            self.cache.loaded_bytes(),
            self.cache.on_disk(),
        )
    }

    /// The terrain vertex candidates as this GPU can draw them: its name and the check.
    pub fn vertex_layout_checks(&self) -> Vec<(&'static str, ruthenium_gpu::VertexLayoutCheck)> {
        TERRAIN_VERTEX_CANDIDATES
            .iter()
            .map(|c| {
                (
                    c.name,
                    self.gpu().check_vertex_layout(c.bytes, c.attributes),
                )
            })
            .collect()
    }

    /// How full the texture table is and has been.
    pub fn texture_stats(&self) -> TextureTableStats {
        self.textures.stats()
    }

    /// Creates a texture and stages its levels (see [`TextureTable::create_with`]). A
    /// full upload ring or texture table ([`ruthenium_gpu::Error::try_again`]) is an error
    /// the caller retries in a later frame.
    pub fn create_texture(
        &mut self,
        desc: TextureDesc,
        mips: &[&[u8]],
        arrival: Arrival,
    ) -> Result<TextureId, ruthenium_gpu::Error> {
        self.textures
            .create_with(&mut self.uploads, desc, mips, arrival)
    }

    /// Destroys a texture once the frames that may use it have finished (see
    /// [`TextureTable::destroy`]).
    pub fn destroy_texture(&mut self, id: TextureId) {
        self.textures.destroy(id);
    }

    /// Whether frames recorded from now on may sample `id`.
    pub fn texture_ready(&self, id: TextureId) -> bool {
        self.textures.is_ready(id)
    }

    /// Copies the next frame drawn into the window (see [`Self::take_screenshot`]).
    pub fn request_screenshot(&mut self) {
        self.screenshot_requested = true;
    }

    /// A requested screenshot once its frame has finished: `(width, height, RGBA)`.
    /// Swapchains that cannot be read give none, and the request stays pending.
    pub fn take_screenshot(&mut self) -> Result<Option<(u32, u32, Vec<u8>)>, ruthenium_gpu::Error> {
        match &self.screenshot {
            Some(s) if s.done(&self.frames)? => {
                Ok(self.screenshot.take().map(Screenshot::into_rgba))
            }
            _ => Ok(None),
        }
    }

    /// Which frame the last screenshot recorded was of, which swapchain it drew to
    /// ([`ruthenium_gpu::SwapchainInfo::generation`]) and whether it was the first frame
    /// drawn to that swapchain.
    pub fn screenshot_of(&self) -> Option<(u64, u32, bool)> {
        self.screenshot_of
    }

    /// Frames that drew glass shapes into the window with no backdrop behind them (a flat
    /// tint instead): none, unless the backdrop's images could not be made.
    pub fn glass_fallbacks(&self) -> u64 {
        self.glass_fallbacks
    }

    /// Development: now and then, replace the swapchain a second time in one frame, so
    /// one is retired that no frame drew to (see also [`FrameLoop::invalidate`]).
    pub fn exercise_recreation(&mut self) {
        self.exercise_recreation = true;
    }

    /// The development overlay (hidden until `set_visible`).
    pub fn overlay_mut(&mut self) -> &mut Overlay {
        &mut self.overlay
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        self.frames.gpu()
    }

    pub fn frames(&self) -> &FrameLoop {
        &self.frames
    }

    pub fn frames_mut(&mut self) -> &mut FrameLoop {
        &mut self.frames
    }

    /// Creates the backdrop's images when shapes are drawn (before the texture table's
    /// `begin_frame`, so this frame's set holds them), remakes them for the swapchain this
    /// frame draws to when its size or format differs, and frees them when no shapes are
    /// drawn. Without a swapchain (a minimised window) they stay as they are.
    fn update_backdrop(&mut self, wanted: bool, surface: Option<SwapchainInfo>) {
        if !wanted {
            if let Some(old) = self.backdrop.take() {
                old.destroy(&mut self.textures);
            }
            return;
        }
        let Some(s) = surface else {
            return;
        };
        if self
            .backdrop
            .as_ref()
            .is_some_and(|b| b.matches(s.format, s.extent))
        {
            return;
        }
        if let Some(old) = self.backdrop.take() {
            old.destroy(&mut self.textures);
        }
        match backdrop::Backdrop::new(&mut self.textures, s.format, s.extent) {
            Ok(b) => {
                self.backdrop = Some(b);
                self.backdrop_failed = false;
            }
            // Glass falls back to a flat tint, and the next frame tries again: said once.
            Err(e) => {
                if !std::mem::replace(&mut self.backdrop_failed, true) {
                    eprintln!("[render] no glass backdrop: {e}");
                }
            }
        }
    }

    /// Rebuilds the pipelines if the swapchain's format changed (rare: a different monitor
    /// or surface), before the frame records anything with them. Waits for earlier frames,
    /// which may still use the old pipelines. Returns whether the pipelines were rebuilt
    /// (they are then not warmed up).
    fn ensure_format(&mut self, format: vk::Format) -> Result<bool, ruthenium_gpu::Error> {
        if format == self.pipelines.format() {
            return Ok(false);
        }
        self.frames.wait_for_gpu(1)?;
        let gpu = self.frames.gpu().clone();
        self.pipelines = Pipelines::build(
            gpu,
            &mut self.cache,
            &self.layout,
            &pipelines::descs(),
            format,
        )?;
        Ok(true)
    }

    /// Records, submits and presents one frame: the world if one is given and its
    /// assets have arrived, else `background`; then the GUI over it. Under menus with
    /// glass shapes the world is drawn into the backdrop they sample instead of the
    /// window.
    pub fn render(
        &mut self,
        background: &Background,
        world: Option<WorldFrame<'_>>,
        gui: Option<GuiFrame<'_>>,
    ) -> Result<FrameReport, ruthenium_gpu::Error> {
        profiling::scope!("render");
        let (mut ctx, gpu_time) = self.frames.begin()?;
        self.last_frame = ctx.number;
        let completed = self.frames.completed()?;
        // The swapchain this frame draws to, replaced first if the window changed, so that
        // everything below is chosen and recorded for its format and size.
        let mut surface = self.frames.prepare()?;
        if self.exercise_recreation && ctx.number.is_multiple_of(97) {
            self.frames.invalidate();
            surface = self.frames.prepare()?;
        }
        if let Some(s) = surface {
            self.ensure_format(s.format)?;
        }
        // Only glass samples what is behind the menus.
        let wants_backdrop = gui.is_some_and(|g| g.shapes.iter().any(|s| s.glass));
        self.update_backdrop(wants_backdrop, surface);
        // The world's work that stages uploads, before the frame's copies are recorded:
        // what changed in the level, finished meshes, new jobs.
        self.world_times = WorldTimes::default();
        let view = match (&world, surface, &mut self.world) {
            (Some(w), Some(s), Some(renderer)) => {
                let started = std::time::Instant::now();
                renderer.set_shades(w.environment.cardinal_light.shades());
                renderer.update(
                    w.scene,
                    &w.camera,
                    w.view_distance,
                    &mut self.textures,
                    &mut self.uploads,
                    LightmapInputs::of(w.environment),
                    ctx.number,
                );
                self.world_times.stage_ns = started.elapsed().as_nanos() as u64;
                let aspect = s.extent.width as f32 / s.extent.height.max(1) as f32;
                Some((View::new(&w.camera, aspect), sky_colour(w.environment)))
            }
            _ => None,
        };
        if let (Some(_), Some(s)) = (&view, surface) {
            self.ensure_depth(s.extent)?;
        }
        if let Some(c) = &mut self.terrain_check {
            c.stage(&mut self.uploads);
        }
        // SAFETY: ctx.cmd is recording frame ctx.number on the graphics queue; completed
        // was just read from the timeline; every upload destination is owned by self and
        // outlives the frame.
        unsafe {
            let zone = self.frames.zone_begin(&ctx, "uploads");
            self.uploads.record(&mut ctx, completed)?;
            self.textures
                .begin_frame(ctx.slot(), ctx.number, completed, &self.uploads);
            self.frames.zone_end(&ctx, zone);
            if let Some(c) = &mut self.check {
                let zone = self.frames.zone_begin(&ctx, "upload check");
                c.step(&mut self.uploads, &mut self.textures, &ctx, completed)?;
                self.frames.zone_end(&ctx, zone);
            }
            if let Some(c) = &mut self.stream_check {
                let zone = self.frames.zone_begin(&ctx, "stream check");
                c.step(&mut self.uploads, &mut self.textures, &ctx, completed)?;
                self.frames.zone_end(&ctx, zone);
            }
        }
        // The meshes whose copies this frame may read take their sections' places.
        if let (Some(w), Some(renderer)) = (&world, &mut self.world) {
            let started = std::time::Instant::now();
            renderer.install_ready(w.scene, &self.uploads, ctx.number);
            self.world_times.stage_ns += started.elapsed().as_nanos() as u64;
        }
        // The world is drawn once everything its draws read has arrived.
        let view = view.filter(|_| {
            self.depth.is_some() && self.world.as_ref().is_some_and(|w| w.ready(&self.textures))
        });
        self.frame_data.begin_frame(ctx.slot());
        let gpu = self.frames.gpu().clone();
        // SAFETY: the layout was made of the texture table's and the frame data's set
        // layouts (Renderer::new); both have begun this frame; self.pipelines are only
        // ever built with it.
        let mut encoder =
            unsafe { Encoder::begin(&gpu, &ctx, &self.layout, &self.textures, &self.frame_data) };
        // First frame (and after a rebuild): draw once with every pipeline.
        if !self.pipelines.warmed_up() {
            let zone = self.frames.zone_begin(&ctx, "warm-up");
            encoder.warm_up(&mut self.pipelines)?;
            self.frames.zone_end(&ctx, zone);
        }
        // Off-screen passes, before the late acquire: the backdrop for this swapchain's
        // size and format. It stays drawn if the acquire then gives no image.
        let mut world_in_backdrop = false;
        let backdrop = match (&mut self.backdrop, surface) {
            (Some(b), Some(s)) if wants_backdrop && b.matches(s.format, s.extent) => {
                let zone = self.frames.zone_begin(&ctx, "backdrop");
                // Its images were created before this frame's begin_frame, and the
                // pipelines are for this format.
                match (&view, &mut self.world, &self.depth) {
                    // The world is what is behind the menu: drawn into the backdrop's
                    // full-size image, then blurred like any background.
                    (Some((view, sky)), Some(renderer), Some(depth)) => {
                        let mut pass = encoder.texture_pass_with_depth(
                            &self.pipelines,
                            &self.textures,
                            b.scene(),
                            depth,
                            *sky,
                        );
                        // SAFETY: `view` is only kept once the world is ready, and this
                        // frame's uploads are recorded and its meshes installed.
                        unsafe { renderer.draw(&mut pass, &mut self.frame_data, view) };
                        pass.finish();
                        b.blur(&mut encoder, &self.pipelines, &self.textures);
                        world_in_backdrop = true;
                    }
                    _ => b.record(
                        &mut encoder,
                        &self.pipelines,
                        &self.textures,
                        background,
                        self.stars,
                    ),
                }
                self.frames.zone_end(&ctx, zone);
                Some(b.textures())
            }
            _ => None,
        };
        if let Some(c) = &mut self.colour_check {
            let zone = self.frames.zone_begin(&ctx, "colour check");
            c.step(
                &mut encoder,
                completed,
                &self.textures,
                &mut self.frame_data,
                &self.pipelines,
            )?;
            self.frames.zone_end(&ctx, zone);
        }
        // The terrain check draws as the terrain is asked to be drawn, where the GPU can.
        let indirect_check = self.terrain_draws == TerrainDraws::Indirect && {
            let o = gpu.caps().optional;
            o.multi_draw_indirect && o.draw_indirect_first_instance
        };
        if let Some(c) = &mut self.terrain_check {
            let zone = self.frames.zone_begin(&ctx, "terrain check");
            c.step(
                &mut encoder,
                completed,
                ctx.number,
                &self.textures,
                &self.uploads,
                &mut self.frame_data,
                &self.pipelines,
                indirect_check,
            )?;
            self.frames.zone_end(&ctx, zone);
        }
        if let Some(c) = &mut self.table_check {
            let zone = self.frames.zone_begin(&ctx, "table check");
            c.step(
                &mut encoder,
                completed,
                &mut self.textures,
                &mut self.uploads,
                &mut self.frame_data,
                &self.pipelines,
            )?;
            self.frames.zone_end(&ctx, zone);
        }
        let target = self.frames.acquire(&ctx)?;
        if let Some(t) = target {
            // The image is from the swapchain prepared above.
            debug_assert_eq!(t.format, self.pipelines.format());
            debug_assert_eq!(Some(t.extent), surface.map(|s| s.extent));
            if wants_backdrop && backdrop.is_none() {
                self.glass_fallbacks += 1;
            }
            let zone = self.frames.zone_begin(&ctx, "window pass");
            // What every GUI pipeline's push constants start with: the target's size.
            let mut screen = [0u8; 8];
            screen[0..4].copy_from_slice(&(t.extent.width as f32).to_ne_bytes());
            screen[4..8].copy_from_slice(&(t.extent.height as f32).to_ne_bytes());
            // A screenshot someone asked for is of the frame as it is, without the colour
            // check's patches.
            let wanted_shot = self.screenshot_requested && self.screenshot.is_none();
            let mut checked = false;
            // The world goes straight into the window unless it went into the backdrop:
            // then the window's pass has a depth attachment, and what is drawn over the
            // world uses the pipelines built for one.
            let in_window = match (&view, &self.depth) {
                (Some((view, sky)), Some(depth)) if !world_in_backdrop => Some((view, *sky, depth)),
                _ => None,
            };
            let over = |id: PipelineId| {
                if in_window.is_some() {
                    id.over_world().index()
                } else {
                    id.index()
                }
            };
            let mut pass = match in_window {
                Some((_, sky, depth)) => {
                    encoder.window_pass_with_depth(&self.pipelines, &t, depth, sky)
                }
                None => encoder.window_pass(&self.pipelines, &t),
            };
            // SAFETY (every draw of the pass): each block of push constants is laid out
            // as its pipeline's shaders declare it; the elements it points at were
            // written into the frame's data just before; texture indices are ones this
            // renderer's table handed out (the starfield, the backdrop's images, the
            // GUI's textures, the colour check's), and none is the window. The world is
            // drawn only once it is ready, after this frame's uploads were recorded and
            // its meshes installed.
            unsafe {
                // The world, or a background that covers every pixel (so the old
                // contents are never loaded); with a backdrop, that is already drawn:
                // copy it.
                match (in_window, &mut self.world, backdrop) {
                    (Some((view, ..)), Some(renderer), _) => {
                        let started = std::time::Instant::now();
                        renderer.draw(&mut pass, &mut self.frame_data, view);
                        self.world_times.draw_ns = started.elapsed().as_nanos() as u64;
                    }
                    (_, _, Some((scene, _))) => pass.draw(
                        PipelineId::Blur.index(),
                        &backdrop::blur_push_copy(scene),
                        3,
                        1,
                    ),
                    _ => pass.draw(
                        PipelineId::Background.index(),
                        &background.push_constants(self.stars),
                        3,
                        1,
                    ),
                }
                if let Some(g) = gui.filter(|g| !g.quads.is_empty() && g.scale > 0)
                    && let Some((first, bytes)) =
                        self.frame_data.alloc(g.quads.len() as u32, gui::QUAD_BYTES)
                {
                    gui::write_quads(g.quads, bytes);
                    let mut push = [0u8; 16];
                    push[0..8].copy_from_slice(&screen);
                    push[8..12].copy_from_slice(&(g.scale as f32).to_ne_bytes());
                    push[12..16].copy_from_slice(&first.to_ne_bytes());
                    pass.draw(over(PipelineId::Gui), &push, 6, g.quads.len() as u32);
                }
                if let Some(g) = gui.filter(|g| !g.shapes.is_empty())
                    && let Some((first, bytes)) = self
                        .frame_data
                        .alloc(g.shapes.len() as u32, gui::SHAPE_BYTES)
                {
                    gui::write_shapes(g.shapes, bytes);
                    let (scene, blurred) =
                        backdrop.unwrap_or((TextureId::MISSING, TextureId::MISSING));
                    let mut push = [0u8; 24];
                    push[0..8].copy_from_slice(&screen);
                    push[8..12].copy_from_slice(&scene.index().to_ne_bytes());
                    push[12..16].copy_from_slice(&blurred.index().to_ne_bytes());
                    push[16..20].copy_from_slice(&first.to_ne_bytes());
                    push[20..24].copy_from_slice(&u32::from(backdrop.is_some()).to_ne_bytes());
                    pass.draw(over(PipelineId::Shape), &push, 6, g.shapes.len() as u32);
                }
                if let Some(g) = gui.filter(|g| !g.glyphs.is_empty())
                    && let Some((first, bytes)) = self
                        .frame_data
                        .alloc(g.glyphs.len() as u32, gui::GLYPH_BYTES)
                {
                    gui::write_glyphs(g.glyphs, bytes);
                    let mut push = [0u8; 16];
                    push[0..8].copy_from_slice(&screen);
                    push[8..12].copy_from_slice(&g.font_atlas.to_ne_bytes());
                    push[12..16].copy_from_slice(&first.to_ne_bytes());
                    pass.draw(over(PipelineId::Text), &push, 6, g.glyphs.len() as u32);
                }
                let rects = self.overlay.build(t.extent.width, t.extent.height);
                if let Some((first, bytes)) = self
                    .frame_data
                    .alloc(rects.len() as u32, overlay::RECT_BYTES)
                    && !rects.is_empty()
                {
                    overlay::write_rects(rects, bytes);
                    let mut push = [0u8; 12];
                    push[0..8].copy_from_slice(&screen);
                    push[8..12].copy_from_slice(&first.to_ne_bytes());
                    pass.draw(over(PipelineId::Overlay), &push, 6, rects.len() as u32);
                }
                // The colour check's patches, over everything else in the window's top
                // left corner.
                if let Some(c) = &self.colour_check
                    && !wanted_shot
                    && c.wants_window(&self.textures, ctx.number)
                    && t.extent.width >= colour_check::WIDTH
                    && t.extent.height >= colour_check::HEIGHT
                    && let Some((first, bytes)) = self.frame_data.alloc(2, gui::QUAD_BYTES)
                {
                    gui::write_quads(&c.quads(), bytes);
                    let push = check_pass::pixel_quads_push(t.extent.width, t.extent.height, first);
                    pass.draw(over(PipelineId::Gui), &push, 6, 2);
                    checked = true;
                }
            }
            // Ready to present, after a copy of it if one is wanted.
            let shot = if wanted_shot || checked {
                pass.finish_captured()?
            } else {
                pass.finish();
                None
            };
            match (shot, &mut self.colour_check) {
                (Some(s), Some(c)) if checked => c.window_recorded(s),
                (Some(s), _) => {
                    self.screenshot_of = Some((ctx.number, t.generation, t.first));
                    self.screenshot = Some(s);
                    self.screenshot_requested = false;
                }
                (None, Some(c)) if checked => c.window_unreadable(),
                (None, _) => {}
            }
            self.frames.zone_end(&ctx, zone);
        }
        let frame = ctx.number;
        let presented = self.frames.submit(ctx, target)?;
        Ok(FrameReport {
            frame,
            presented,
            gpu_time,
        })
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // The meshing threads write into the upload ring's staging memory: they end
        // before any field goes.
        if let Some(w) = &mut self.world {
            w.stop_workers();
        }
    }
}
