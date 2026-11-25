//! Ruthenium's renderer: GPU start-up for a window, the pipeline set, the upload ring and
//! texture table, and a frame that draws the world's terrain or the menu background, and
//! the GUI's draw list over it, recorded and presented through
//! [`ruthenium_gpu::FrameLoop`].

mod atmosphere;
mod backdrop;
pub mod block_entity;
mod check_pass;
mod colour_check;
mod gui;
mod overlay;
mod pipelines;
#[cfg(test)]
mod reflect;
mod sky;
mod starfield;
mod stats;
mod table_check;
pub mod terrain;
mod terrain_check;
mod terrain_vertex;

pub use atmosphere::{AtmosphereOptions, Fog, FogInputs, Medium, fog, rain_fog_target};
pub use colour_check::ColourCheckReport;
pub use gui::GuiFrame;
pub use overlay::{LINES as OVERLAY_LINES, Overlay, OverlayStats};
pub use pipelines::PipelineId;
pub use sky::{MOON_PHASES, SkyImage, SkyImages, SkyOrder};
pub use stats::{FrameTimes, Summary};
pub use table_check::TableCheckReport;
pub use terrain::{
    CameraPose, LatencySummary, TerrainDraws, TerrainShader, WorldAssets, WorldLatencies,
    WorldStats,
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
use ruthenium_sim::scene::{Frame, Scene};

use atmosphere::{Atmosphere, Look, WaterOverlay};
use block_entity::renderer::BlockEntityRenderer;
use ruthenium_gpu::SamplerId;
use ruthenium_sim::environment::CardinalLight;
use ruthenium_ui::Quad;
use sky::{Sky, SkyRenderer};
use terrain::View;
use terrain::world::WorldRenderer;

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
    /// The game thread's frame the camera is of: its clocks, its weather and the
    /// dimension's own look (how faces are shaded, its light, its fog and its sky),
    /// and how far its tick has come.
    pub frame: &'a Frame,
    pub partial_tick: f32,
}

/// The colour the world's pass is cleared to: the fog's, as in vanilla. Where no sky is
/// drawn over it, that is what is seen behind the world.
fn clear_colour(look: &Look) -> [f32; 4] {
    let [r, g, b, _] = look.fog.colour;
    [r, g, b, 1.0]
}

/// How often the picture of water repeats from one side of its quad to the other.
const WATER_REPEATS: f32 = 4.0;
/// The field of view the hand and the picture of water are seen with, in degrees.
const HAND_FOV: f32 = 70.0;

/// The quad of the picture of water on a target of `size` pixels: a square two blocks
/// wide, half a block before the eyes, seen with the field of view `fov`.
fn water_quad(overlay: &WaterOverlay, texture: TextureId, size: [f32; 2], fov: f32) -> Quad {
    let half_height = 0.5 * (fov.to_radians() / 2.0).tan();
    let [w, h] = size;
    let dy = h / 2.0 / half_height;
    // The target's pixels are square: as many of them to a block sideways.
    let dx = dy;
    let [u, v] = overlay.offset;
    Quad {
        rect: [w / 2.0 - dx, h / 2.0 - dy, w / 2.0 + dx, h / 2.0 + dy],
        // Mirrored sideways, as the game has it.
        uv: [u + WATER_REPEATS, v, u, v + WATER_REPEATS],
        color: overlay.colour,
        texture: texture.index(),
    }
}

/// What is in front of the world in a frame: the quad of the picture of water, and the
/// size of the target it is laid out for.
#[derive(Debug, Clone, Copy)]
struct Front {
    quad: Quad,
    size: [f32; 2],
}

/// Draws the world and its sky into `pass`, which has a depth attachment, in the order
/// asked for, and `front` over them.
///
/// # Safety
/// As [`WorldRenderer::draw`] and [`SkyRenderer::draw`].
#[allow(clippy::too_many_arguments)]
unsafe fn draw_world(
    world: &mut WorldRenderer,
    sky: Option<&mut SkyRenderer>,
    order: SkyOrder,
    pass: &mut ruthenium_gpu::Pass<'_>,
    frame_data: &mut FrameData,
    textures: &TextureTable,
    view: &View,
    look: &Look,
    front: Option<Front>,
    models: Option<(&mut BlockEntityRenderer, CardinalLight)>,
) {
    let mut sky = sky.filter(|_| order != SkyOrder::Off && look.sky != Sky::None);
    let mut draw_sky = |pass: &mut ruthenium_gpu::Pass<'_>, frame_data: &mut FrameData, behind| {
        if let Some(s) = &mut sky {
            // SAFETY: the caller's.
            unsafe {
                s.draw(
                    pass,
                    frame_data,
                    textures,
                    view,
                    &look.sky,
                    look.fog.colour,
                    look.fog.sky_end,
                    behind,
                );
            }
        }
    };
    if order == SkyOrder::First {
        draw_sky(pass, frame_data, false);
    }
    // SAFETY: the caller's.
    unsafe { world.draw(pass, frame_data, view, &look.fog) };
    if let (Some((models, cardinal)), Some((lightmap, indices))) = (models, world.shared()) {
        // SAFETY: the caller's; the world is ready, and with it what it shares.
        unsafe {
            models.draw(
                pass, frame_data, view, &look.fog, cardinal, lightmap, indices,
            );
        }
    }
    if order == SkyOrder::Late {
        draw_sky(pass, frame_data, true);
    }
    // What is seen through comes last: over the opaque world and the sky.
    // SAFETY: the caller's.
    unsafe { world.draw_translucent(pass, frame_data) };
    if let Some(f) = front
        && let Some((first, bytes)) = frame_data.alloc(1, gui::QUAD_BYTES)
    {
        gui::write_quad_with(f.quad, SamplerId::NearestRepeat, bytes);
        let mut push = [0u8; 16];
        push[0..4].copy_from_slice(&f.size[0].to_ne_bytes());
        push[4..8].copy_from_slice(&f.size[1].to_ne_bytes());
        push[8..12].copy_from_slice(&1.0f32.to_ne_bytes());
        push[12..16].copy_from_slice(&first.to_ne_bytes());
        // SAFETY: the pipeline is the one of passes with a depth attachment, its push
        // constants are laid out as gui.vert declares them, and its one quad was just
        // written; the texture is the renderer's own.
        unsafe { pass.draw(PipelineId::Gui.over_world().index(), &push, 6, 1) };
    }
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
    smooth_lighting: bool,
    biome_blend: u8,
    terrain_shader: TerrainShader,
    chunk_fade_ms: u32,
    /// Whether the terrain's translucent layer is drawn.
    translucent: bool,
    /// Whether animated textures play.
    animate: bool,
    /// The FOV Effects option, 0 to 1.
    fov_effects: f32,
    /// The picture of water before a player whose eyes are in it.
    underwater: Option<TextureId>,
    sky_order: SkyOrder,
    atmosphere_options: AtmosphereOptions,
    /// The air of the level being drawn, and the sky's meshes and pictures once the
    /// world's assets are loaded.
    atmosphere: Atmosphere,
    sky: Option<SkyRenderer>,
    /// The block entities' models and pictures, with them; and whether they are drawn.
    block_entities: Option<BlockEntityRenderer>,
    show_block_entities: bool,
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
            smooth_lighting: true,
            biome_blend: 2,
            terrain_shader: TerrainShader::default(),
            chunk_fade_ms: terrain::DEFAULT_FADE_MS,
            translucent: true,
            animate: true,
            fov_effects: 1.0,
            underwater: None,
            sky_order: SkyOrder::default(),
            atmosphere_options: AtmosphereOptions::default(),
            atmosphere: Atmosphere::new(),
            sky: None,
            block_entities: None,
            show_block_entities: true,
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
        mut assets: WorldAssets,
        workers: usize,
    ) -> Result<(), ruthenium_gpu::Error> {
        if self.world.is_none() {
            let gpu = self.frames.gpu().clone();
            // A sky that cannot be made is left out: the world is drawn without it.
            self.sky =
                match SkyRenderer::new(&gpu, &mut self.textures, &mut self.uploads, &assets.sky) {
                    Ok(sky) => Some(sky),
                    Err(e) => {
                        eprintln!("[render] no sky: {e}");
                        None
                    }
                };
            self.block_entities = assets.block_entities.take().and_then(|models| {
                BlockEntityRenderer::new(&gpu, &mut self.textures, &mut self.uploads, models)
                    .inspect_err(|e| eprintln!("[render] no block entities: {e}"))
                    .ok()
            });
            // Without its picture, water is in front of nothing.
            self.underwater = assets.underwater.as_ref().and_then(|i| {
                self.textures
                    .create_with(
                        &mut self.uploads,
                        TextureDesc::rgba8(i.width, i.height),
                        &[&i.rgba],
                        Arrival::NextFrame,
                    )
                    .ok()
            });
            self.world = Some(WorldRenderer::new(
                &gpu,
                &mut self.textures,
                &mut self.uploads,
                assets,
                workers,
                self.terrain_culling,
            )?);
            self.set_terrain_draws(self.terrain_draws);
            if let Some(w) = &mut self.world {
                w.set_smooth_lighting(self.smooth_lighting);
                w.set_biome_blend(self.biome_blend);
                w.set_shader(self.terrain_shader);
                w.set_fade_ms(self.chunk_fade_ms);
                w.set_translucent(self.translucent);
                w.set_animate(self.animate);
            }
        }
        Ok(())
    }

    /// Whether the block atlas's animated textures play (they do unless said otherwise).
    /// Stopped, two frames of one place and time are the same picture.
    pub fn set_animate(&mut self, on: bool) {
        self.animate = on;
        if let Some(w) = &mut self.world {
            w.set_animate(on);
        }
    }

    pub fn animate(&self) -> bool {
        self.animate
    }

    /// The FOV Effects option, 0 to 1 (1 unless said otherwise): how much of the field
    /// of view's narrowing in water and lava shows.
    pub fn set_fov_effects(&mut self, scale: f32) {
        self.fov_effects = scale.clamp(0.0, 1.0);
    }

    pub fn fov_effects(&self) -> f32 {
        self.fov_effects
    }

    /// Whether the terrain's translucent layer (water, stained glass, ice) is drawn: it
    /// is unless said otherwise. Off, it is meshed and kept all the same, and only
    /// left out of the frame: what drawing and sorting it cost is measured against that.
    pub fn set_translucent(&mut self, on: bool) {
        self.translucent = on;
        if let Some(w) = &mut self.world {
            w.set_translucent(on);
        }
    }

    pub fn translucent(&self) -> bool {
        self.translucent
    }

    /// Whether block entities are drawn (they are unless said otherwise).
    pub fn set_block_entities(&mut self, on: bool) {
        self.show_block_entities = on;
    }

    pub fn block_entities(&self) -> bool {
        self.show_block_entities
    }

    /// How the terrain's texels are filtered (as vanilla's RGSS unless said otherwise).
    pub fn set_terrain_shader(&mut self, shader: TerrainShader) {
        self.terrain_shader = shader;
        if let Some(w) = &mut self.world {
            w.set_shader(shader);
        }
    }

    pub fn terrain_shader(&self) -> TerrainShader {
        self.terrain_shader
    }

    /// Chunk Fade: how long a section takes to come out of the fog's colour once it
    /// has a mesh, in milliseconds (750 unless said otherwise; 0: no fade).
    pub fn set_chunk_fade_ms(&mut self, ms: u32) {
        self.chunk_fade_ms = ms;
        if let Some(w) = &mut self.world {
            w.set_fade_ms(ms);
        }
    }

    pub fn chunk_fade_ms(&self) -> u32 {
        self.chunk_fade_ms
    }

    /// When the sky is drawn: after the world's opaque layers unless said otherwise.
    pub fn set_sky_order(&mut self, order: SkyOrder) {
        self.sky_order = order;
    }

    pub fn sky_order(&self) -> SkyOrder {
        self.sky_order
    }

    /// Whether the terrain is fogged and block light flickers, and how bright the
    /// Brightness option makes things.
    pub fn set_atmosphere_options(&mut self, options: AtmosphereOptions) {
        self.atmosphere_options = options;
    }

    pub fn atmosphere_options(&self) -> AtmosphereOptions {
        self.atmosphere_options
    }

    /// Whether blocks are lit corner by corner from the blocks around them (Smooth
    /// Lighting, on unless said otherwise) or a face at a time. Takes effect when the
    /// world's assets are loaded ([`Self::load_world`]).
    pub fn set_smooth_lighting(&mut self, smooth: bool) {
        self.smooth_lighting = smooth;
    }

    /// Biome Blend: over how many columns each way grass, leaves and water take the mean
    /// of the biomes' colours (0 to 7; 2 unless said otherwise). Takes effect when the
    /// world's assets are loaded ([`Self::load_world`]).
    pub fn set_biome_blend(&mut self, radius: u8) {
        self.biome_blend = radius.min(terrain::MAX_BLEND_RADIUS);
    }

    pub fn biome_blend(&self) -> u8 {
        self.world
            .as_ref()
            .map_or(self.biome_blend, |w| w.biome_blend())
    }

    /// Whether the terrain is meshed with smooth lighting (what was asked for until a
    /// world is loaded).
    pub fn smooth_lighting(&self) -> bool {
        self.world
            .as_ref()
            .map_or(self.smooth_lighting, |w| w.smooth_lighting())
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
        let mut stats = self.world.as_ref().map(WorldRenderer::stats)?;
        stats.block_entities = self
            .block_entities
            .as_ref()
            .filter(|_| self.show_block_entities)
            .map(BlockEntityRenderer::stats)
            .unwrap_or_default();
        Some(stats)
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
                let size = [s.extent.width as f32, s.extent.height.max(1) as f32];
                let aspect = size[0] / size[1];
                let medium = renderer.medium_at(w.scene, &w.camera, aspect);
                let fov_scale = medium.fov_scale(self.fov_effects);
                let mut camera = w.camera;
                camera.fov *= fov_scale;
                let look = self.atmosphere.update(
                    w.frame,
                    w.partial_tick,
                    &camera,
                    medium,
                    w.scene,
                    w.view_distance,
                    started,
                    &self.atmosphere_options,
                );
                renderer.set_environment(&w.frame.environment);
                renderer.animate(w.frame, &mut self.textures, &mut self.uploads);
                renderer.update(
                    w.scene,
                    &camera,
                    w.view_distance,
                    &mut self.textures,
                    &mut self.uploads,
                    look.lightmap,
                    ctx.number,
                );
                if let Some(sky) = &mut self.sky {
                    sky.stage(&mut self.uploads);
                }
                if let Some(models) = &mut self.block_entities {
                    models.stage(&mut self.uploads);
                }
                self.world_times.stage_ns = started.elapsed().as_nanos() as u64;
                let front = match (&look.water, self.underwater) {
                    (Some(water), Some(texture)) if self.textures.is_ready(texture) => {
                        let quad = water_quad(water, texture, size, HAND_FOV * fov_scale);
                        Some(Front { quad, size })
                    }
                    _ => None,
                };
                Some((View::new(&camera, aspect), look, front))
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
            if let Some(sky) = &mut self.sky {
                sky.note_ready(&self.uploads);
            }
            if let Some(models) = &mut self.block_entities {
                models.note_ready(&self.uploads);
            }
            self.world_times.stage_ns += started.elapsed().as_nanos() as u64;
        }
        // The world is drawn once everything its draws read has arrived.
        let view = view.filter(|_| {
            self.depth.is_some() && self.world.as_ref().is_some_and(|w| w.ready(&self.textures))
        });
        let cardinal = world.as_ref().map_or(CardinalLight::Default, |w| {
            w.frame.environment.cardinal_light
        });
        if let (Some(w), Some((view, ..)), Some(renderer), Some(models)) = (
            &world,
            &view,
            &mut self.world,
            self.block_entities
                .as_mut()
                .filter(|_| self.show_block_entities),
        ) {
            let started = std::time::Instant::now();
            renderer.gather_block_entities(
                models,
                w.scene,
                w.frame,
                w.partial_tick,
                w.camera.position,
                view,
                &self.textures,
            );
            self.world_times.stage_ns += started.elapsed().as_nanos() as u64;
        }
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
                    (Some((view, look, front)), Some(renderer), Some(depth)) => {
                        let mut pass = encoder.texture_pass_with_depth(
                            &self.pipelines,
                            &self.textures,
                            b.scene(),
                            depth,
                            clear_colour(look),
                        );
                        // SAFETY: `view` is only kept once the world is ready, and this
                        // frame's uploads are recorded and its meshes installed.
                        unsafe {
                            draw_world(
                                renderer,
                                self.sky.as_mut(),
                                self.sky_order,
                                &mut pass,
                                &mut self.frame_data,
                                &self.textures,
                                view,
                                look,
                                *front,
                                self.block_entities
                                    .as_mut()
                                    .filter(|_| self.show_block_entities)
                                    .map(|m| (m, cardinal)),
                            );
                        }
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
                (Some((view, look, front)), Some(depth)) if !world_in_backdrop => {
                    Some((view, look, *front, depth))
                }
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
                Some((_, look, _, depth)) => {
                    encoder.window_pass_with_depth(&self.pipelines, &t, depth, clear_colour(look))
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
                    (Some((view, look, front, _)), Some(renderer), _) => {
                        let started = std::time::Instant::now();
                        draw_world(
                            renderer,
                            self.sky.as_mut(),
                            self.sky_order,
                            &mut pass,
                            &mut self.frame_data,
                            &self.textures,
                            view,
                            look,
                            front,
                            self.block_entities
                                .as_mut()
                                .filter(|_| self.show_block_entities)
                                .map(|m| (m, cardinal)),
                        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_of_water_is_half_a_block_away() {
        let overlay = WaterOverlay {
            colour: 0x19FF_FFFF,
            offset: [0.25, -0.5],
        };
        // With 90 degrees the target's height spans one block half a block away: the
        // quad, two blocks wide, is twice as high, whatever the target's width.
        let quad = water_quad(&overlay, TextureId::MISSING, [1600.0, 900.0], 90.0);
        for (a, b) in quad.rect.iter().zip([-100.0, -450.0, 1700.0, 1350.0]) {
            assert!((a - b).abs() < 1e-2, "{a} {b}");
        }
        assert_eq!(quad.uv, [4.25, -0.5, 0.25, 3.5]);
        assert_eq!(quad.color, 0x19FF_FFFF);
        // A narrower view takes it nearer.
        let near = water_quad(&overlay, TextureId::MISSING, [1600.0, 900.0], 60.0);
        assert!(near.rect[1] < quad.rect[1] && near.rect[3] > quad.rect[3]);
    }
}
