//! The world's terrain on the GPU: meshing jobs on worker threads, their vertices in
//! pooled buffer pages, and the draws of a frame.
//!
//! A section's way to the screen:
//! 1. The [`Tracker`] hands out a job for a dirty section, pinned as the level is.
//! 2. A worker meshes it into ordinary memory and sends the result back.
//! 3. The render thread, which owns the upload ring, reserves a slice of worker staging
//!    of the result's size; a worker copies the vertices into it (the render thread
//!    copies no mesh bytes itself).
//! 4. Once written, a range of a buffer page is allocated and the copy from the slice
//!    to it is recorded: on the copy queue, or into the next frame for the player's own
//!    changes.
//! 5. When the copy can be read, and the level still is what the mesh was made from,
//!    the mesh replaces the section's old one, which is drawn until then and retired
//!    after the frames that drew it.
//!
//! A mesh's translucent quads are drawn after everything else, the farthest section
//! first and each section's quads farthest first, through indices of the section's own.
//! The worker that meshes a section sorts them for where the camera then is, and those
//! indices travel behind the vertices in the same range. When the camera has moved
//! enough ([`super::sort::needs_sort`]) a worker sorts the kept centres again; the new
//! indices go into a range of their own in the frame they come back in, and the draw
//! switches to them once that frame may read them.
//!
//! Every stage is bounded: jobs out at once, results waiting for staging, slices
//! reserved, and the pool's budget.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ruthenium_assets::AtlasAnimations;
use ruthenium_assets::animation::Rect;
use ruthenium_gpu::{
    BufferPool, DRAW_COMMAND_BYTES, Error, FrameData, Gpu, ImageRegion, MAX_INDIRECT_DRAWS, Pass,
    PoolConfig, PoolError, PoolStats, RangeHandle, Reservation, ReservationState, StagingWriter,
    TextureDesc, TextureId, TextureTable, UploadError, UploadRing, UploadTicket, Urgency, Users,
    vk,
};
use ruthenium_shaders::interface::{SECTION, WORLD, WORLD_MATRIX};
use ruthenium_sim::environment::Environment;
use ruthenium_sim::scene::{Frame, Scene, SectionKey, Stamp};

use super::camera::{CameraPose, View};
use super::facing::{self, Facings};
use super::lightmap::{self, LightmapInputs};
use super::medium;
use super::mesher::{MeshOptions, Mesher, SectionMesh};
use super::sort::{self, PointOfView, Sorter};
use super::table::BlockTable;
use super::tint::{BiomeTints, ColourMaps, MAX_BLEND_RADIUS};
use super::tracker::{Job, Since, Tracker, TrackerStats};
use super::visibility::{Batch, Camera, Connectivity, Visibility};
use crate::atmosphere::{Fog, Medium};
use crate::block_entity::renderer::{
    BlockEntityRenderer, MIN_VISIBILITY, PlayerView, SectionEntities,
};
use crate::pipelines::PipelineId;
use crate::terrain_vertex::VERTEX_BYTES;

/// Quads one indexed draw can address with 16-bit indices (four vertices each).
pub(crate) const QUADS_PER_DRAW: u32 = 1 << 14;
/// Bytes of the shared quad index buffer: six indices for each of those quads.
const INDEX_BYTES: u64 = QUADS_PER_DRAW as u64 * 6 * 2;

/// Buffer pages for vertices: their size.
const PAGE_BYTES: u64 = 4 << 20;
/// What the terrain's buffers are planned to take at a 12-chunk view distance. Without
/// cave culling every loaded section is meshed, also those underground that nothing can
/// be seen of, and in a jungle that takes more: passing the envelope is said once and
/// shown in the statistics, and meshes are only left out at twice it.
pub const POOL_ENVELOPE: u64 = 108 << 20;
const POOL_BUDGET: u64 = 2 * POOL_ENVELOPE;
/// The second staging buffer, which workers write meshes into.
const WORKER_STAGING_BYTES: u64 = 8 << 20;
/// Results that may wait for staging, a place in a page or their copy at once; no new
/// job is handed out beyond it.
const MAX_PENDING: usize = 48;
/// Jobs out per worker.
const JOBS_PER_WORKER: usize = 2;
/// Sections whose translucent quads are being sorted again or whose new order is on its
/// way to the GPU, at once: the nearest go first, the others wait for a later frame.
const MAX_SORTS: usize = 32;
/// `GameType.SPECTATOR`'s id.
const SPECTATOR: i32 = 3;
/// Bytes of animated sprites staged in one piece: a piece the upload ring has no room
/// for waits for the next frame, the ones before it are on their way.
const ANIMATION_PIECE: usize = 256 << 10;
/// Over how long a tick's animated sprites are written, where frames are shorter than
/// that: each frame writes its share. A longer frame writes them all.
const ANIMATION_SPREAD: Duration = Duration::from_millis(8);

/// How many of the `stale` sprites a tick left a frame writes that came `since` after the
/// one before.
fn animation_share(stale: usize, since: Duration) -> usize {
    let share = since.as_secs_f64() / ANIMATION_SPREAD.as_secs_f64();
    ((stale as f64 * share.min(1.0)).ceil() as usize).min(stale)
}

/// How a frame's sections are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerrainDraws {
    /// One indexed draw a section and layer, strictly nearest first: the vertex buffer
    /// is bound again whenever the next section is in another page.
    #[default]
    Direct,
    /// One indexed draw a section and layer, a buffer page at a time: the pages by
    /// their nearest section, each page's sections nearest first.
    Paged,
    /// In that order, with one call a page and layer that reads the draws as commands
    /// from the frame's data (multi-draw indirect built on the CPU).
    Indirect,
}

impl TerrainDraws {
    pub fn name(self) -> &'static str {
        match self {
            TerrainDraws::Direct => "direct",
            TerrainDraws::Paged => "paged",
            TerrainDraws::Indirect => "mdi",
        }
    }

    pub fn parse(name: &str) -> Option<TerrainDraws> {
        [
            TerrainDraws::Direct,
            TerrainDraws::Paged,
            TerrainDraws::Indirect,
        ]
        .into_iter()
        .find(|d| d.name() == name)
    }
}

/// How the terrain's texels are filtered, and with what besides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerrainShader {
    /// Far texels averaged as vanilla's Texture Filtering "RGSS" averages them (the
    /// Fancy preset's), taking only the samples that count for something, with its
    /// square roots and divisions turned into multiplications where they can be.
    #[default]
    Rgss,
    /// The same picture with all nine samples taken for every pixel and vanilla's
    /// arithmetic, as its shader is written: what `Rgss` is measured against.
    RgssReference,
    /// Every texel with hard edges, as with Texture Filtering off (the Fast preset's).
    Nearest,
    /// `Nearest` without fog and without fade: what those two cost is measured
    /// against it.
    Plain,
}

impl TerrainShader {
    pub const ALL: [TerrainShader; 4] = [
        TerrainShader::Rgss,
        TerrainShader::RgssReference,
        TerrainShader::Nearest,
        TerrainShader::Plain,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TerrainShader::Rgss => "rgss",
            TerrainShader::RgssReference => "rgss-reference",
            TerrainShader::Nearest => "nearest",
            TerrainShader::Plain => "plain",
        }
    }

    pub fn parse(name: &str) -> Option<TerrainShader> {
        TerrainShader::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The pipelines of the solid, the cutout and the translucent layer.
    pub(crate) fn pipelines(self) -> [PipelineId; 3] {
        match self {
            TerrainShader::Rgss => [
                PipelineId::TerrainRgss,
                PipelineId::TerrainRgssCutout,
                PipelineId::TerrainRgssTranslucent,
            ],
            TerrainShader::RgssReference => [
                PipelineId::TerrainRgssReference,
                PipelineId::TerrainRgssReferenceCutout,
                PipelineId::TerrainRgssReferenceTranslucent,
            ],
            TerrainShader::Nearest => [
                PipelineId::TerrainNearest,
                PipelineId::TerrainNearestCutout,
                PipelineId::TerrainNearestTranslucent,
            ],
            TerrainShader::Plain => [
                PipelineId::TerrainPlain,
                PipelineId::TerrainPlainCutout,
                PipelineId::TerrainPlainTranslucent,
            ],
        }
    }
}

/// How long a section takes to fade in, unless said otherwise: the default of
/// vanilla's Chunk Fade option, in milliseconds.
pub const DEFAULT_FADE_MS: u32 = 750;

/// How far a section has faded in at `now`: none of the way when it first had a mesh
/// (`since`), all the way `fade_ms` later (`RenderSection.getVisibility`).
pub(crate) fn visibility(now: Instant, since: Instant, fade_ms: u32) -> f32 {
    let elapsed = now.saturating_duration_since(since).as_millis();
    if elapsed >= u128::from(fade_ms) {
        1.0
    } else {
        elapsed as f32 / fade_ms as f32
    }
}

/// Latencies kept for a summary at most; later ones are only counted.
const LATENCY_SAMPLES: usize = 1 << 16;

/// From the render thread learning that a section changed to the first frame that draws
/// the mesh made of the change: as time and as frames (0: the same frame).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LatencySummary {
    pub samples: u64,
    pub p50_ms: f32,
    pub p99_ms: f32,
    pub max_ms: f32,
    pub p50_frames: u32,
    pub p99_frames: u32,
    pub max_frames: u32,
}

/// The latencies of the meshes installed since the last [`WorldRenderer::reset_latencies`],
/// by what the mesh was for.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldLatencies {
    /// A section's first mesh, from the moment the chunks around it were there.
    pub first: LatencySummary,
    /// A change that came from the server.
    pub changed: LatencySummary,
    /// The player's own change.
    pub own: LatencySummary,
}

#[derive(Debug)]
struct LatencySeries {
    /// Microseconds and frames.
    samples: Vec<(u32, u32)>,
    count: u64,
}

impl LatencySeries {
    fn new() -> LatencySeries {
        LatencySeries {
            samples: Vec::with_capacity(LATENCY_SAMPLES),
            count: 0,
        }
    }

    fn push(&mut self, since: Since, frame: u64) {
        self.count += 1;
        if self.samples.len() < LATENCY_SAMPLES {
            let micros = since.at.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
            let frames = frame.saturating_sub(since.frame).min(u64::from(u32::MAX)) as u32;
            self.samples.push((micros, frames));
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.count = 0;
    }

    fn summary(&self) -> LatencySummary {
        if self.samples.is_empty() {
            return LatencySummary::default();
        }
        // Nearest rank.
        let at = |sorted: &[u32], p: usize| sorted[(sorted.len() * p).div_ceil(100).max(1) - 1];
        let mut micros: Vec<u32> = self.samples.iter().map(|s| s.0).collect();
        let mut frames: Vec<u32> = self.samples.iter().map(|s| s.1).collect();
        micros.sort_unstable();
        frames.sort_unstable();
        let ms = |us: u32| us as f32 / 1e3;
        LatencySummary {
            samples: self.count,
            p50_ms: ms(at(&micros, 50)),
            p99_ms: ms(at(&micros, 99)),
            max_ms: ms(at(&micros, 100)),
            p50_frames: at(&frames, 50),
            p99_frames: at(&frames, 99),
            max_frames: at(&frames, 100),
        }
    }
}

/// What the client prepares when a world is entered: every block state's model and
/// rules, and the block atlas's levels as RGBA8 (level 0 first).
#[derive(Debug)]
pub struct WorldAssets {
    pub table: Arc<BlockTable>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub atlas_levels: Vec<Vec<u8>>,
    /// The atlas's animated sprites.
    pub animations: AtlasAnimations,
    /// The picture of water before a player whose eyes are in it.
    pub underwater: Option<crate::sky::SkyImage>,
    /// The colour maps biomes take their grass and foliage colours from.
    pub colour_maps: ColourMaps,
    /// The sun, the moon's phases and the End's sky.
    pub sky: crate::sky::SkyImages,
    /// The clouds picture (`textures/environment/clouds.png`): one cell a texel.
    pub clouds: Option<crate::sky::SkyImage>,
    /// Rain's and snow's pictures (`textures/environment/rain.png`, `snow.png`).
    pub rain: Option<crate::sky::SkyImage>,
    pub snow: Option<crate::sky::SkyImage>,
    /// What block entities are drawn from; without it they are not drawn.
    pub block_entities: Option<crate::block_entity::renderer::BlockEntityAssets>,
    /// The cracks in a block being broken: its ten stages' pictures in order
    /// (`textures/block/destroy_stage_0.png` to `_9`); a stage without one is not drawn.
    pub cracks: Vec<crate::sky::SkyImage>,
    /// The GUI's atlas, which the HUD's sprites are in; without it there is no HUD.
    pub gui: Option<ruthenium_assets::atlas::Atlas>,
    /// The picture that darkens the frame's corners in the dark
    /// (`textures/misc/vignette.png`).
    pub vignette: Option<crate::sky::SkyImage>,
    /// The container screens' pictures by name (`textures/gui/container/<name>.png`).
    pub containers: Vec<(String, crate::sky::SkyImage)>,
}

/// The indices of quads as two triangles each, sharing the diagonal from a quad's first
/// corner to its third, as the game draws them.
pub(crate) fn quad_indices() -> Vec<u8> {
    let mut out = Vec::with_capacity(INDEX_BYTES as usize);
    for quad in 0..QUADS_PER_DRAW {
        for corner in [0, 1, 2, 2, 3, 0] {
            out.extend_from_slice(&((quad * 4 + corner) as u16).to_le_bytes());
        }
    }
    out
}

/// A section's mesh in a buffer page.
#[derive(Debug, Clone)]
struct MeshRange {
    handle: RangeHandle,
    buffer: vk::Buffer,
    page: u32,
    /// The index of its first vertex in the page.
    first_vertex: u32,
    solid_quads: u32,
    cutout_quads: u32,
    /// How the solid and the cutout quads are grouped by the way they face.
    facings: Facings,
    /// In blocks from the section's corner.
    bounds: [[f32; 3]; 2],
    translucent: Option<Translucent>,
}

/// Where a section's sorted indices are: the buffer, the index of the first of them in
/// it, and whether they are 32-bit ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexRange {
    buffer: vk::Buffer,
    first: u32,
    wide: bool,
}

/// The translucent quads of a mesh: they follow the solid and the cutout ones among
/// its vertices.
#[derive(Debug, Clone)]
struct Translucent {
    quads: u32,
    indices: IndexRange,
    /// The range the indices have to themselves, once sorted again; until then they
    /// are behind the vertices in the mesh's own range.
    own: Option<RangeHandle>,
    /// The points the quads are sorted by.
    centres: Arc<[[f32; 3]]>,
    /// The block the camera was in and its point of view when the order drawn now (or
    /// the one on its way) was made.
    sorted_at: [i32; 3],
    sorted_from: PointOfView,
    /// A new order is being made or is on its way to the GPU.
    sorting: bool,
}

impl MeshRange {
    /// Bytes of its vertices and of the indices that came with them.
    fn bytes(&self) -> u64 {
        let translucent = self.translucent.as_ref().map_or(0, |t| t.quads);
        u64::from(self.solid_quads + self.cutout_quads + translucent) * 4 * VERTEX_BYTES as u64
            + sort::index_bytes(translucent as usize) as u64
    }
}

/// Lets go of a mesh's ranges once `users` are done with them.
fn retire(pool: &mut BufferPool, mesh: &MeshRange, users: Users) {
    pool.retire(mesh.handle, users);
    if let Some(own) = mesh.translucent.as_ref().and_then(|t| t.own) {
        pool.retire(own, users);
    }
}

enum Work {
    Mesh {
        job: Box<Job>,
        options: MeshOptions,
        biomes: Arc<BiomeTints>,
        /// Where the camera is: what the translucent quads are first sorted for.
        camera: [f64; 3],
        /// Which run of sessions the job is of ([`WorldRenderer::clear`]).
        session: u64,
    },
    /// Copy a result into its slice of staging.
    Fill {
        writer: StagingWriter,
        bytes: Vec<u8>,
    },
    /// Sort a mesh's translucent quads again.
    Sort {
        session: u64,
        key: SectionKey,
        /// The mesh the centres are of.
        mesh: RangeHandle,
        centres: Arc<[[f32; 3]]>,
        camera: [f64; 3],
    },
}

struct Meshed {
    session: u64,
    stamp: Stamp,
    mesh: SectionMesh,
    /// The translucent quads' indices, sorted for the job's camera.
    indices: Vec<u8>,
    sorted_at: [i32; 3],
    sorted_from: PointOfView,
    urgent: bool,
    first: bool,
    since: Since,
    micros: u64,
}

struct Sorted {
    session: u64,
    key: SectionKey,
    mesh: RangeHandle,
    indices: Vec<u8>,
    sorted_at: [i32; 3],
    sorted_from: PointOfView,
}

enum Done {
    Meshed(Box<Meshed>),
    Sorted(Sorted),
}

#[derive(Default)]
struct WorkQueue {
    work: VecDeque<Work>,
    stop: bool,
}

struct Workers {
    shared: Arc<(Mutex<WorkQueue>, Condvar)>,
    results: mpsc::Receiver<Done>,
    threads: Vec<JoinHandle<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Workers {
    fn start(table: &Arc<BlockTable>, count: usize) -> Workers {
        let shared = Arc::new((Mutex::new(WorkQueue::default()), Condvar::new()));
        let (sender, results) = mpsc::channel();
        let threads = (0..count.max(1))
            .filter_map(|i| {
                let (shared, sender, table) = (shared.clone(), sender.clone(), table.clone());
                std::thread::Builder::new()
                    .name(format!("mesh-{i}"))
                    .spawn(move || work(&shared, &sender, table))
                    .ok()
            })
            .collect();
        Workers {
            shared,
            results,
            threads,
        }
    }

    fn push(&self, work: Work) {
        let mut q = lock(&self.shared.0);
        match work {
            // Copies first: they free the memory of finished results. Sorts before
            // meshes too: they are short, and what they are for is on screen.
            Work::Fill { .. } | Work::Sort { .. } => q.work.push_front(work),
            Work::Mesh { .. } => q.work.push_back(work),
        }
        drop(q);
        self.shared.1.notify_one();
    }

    /// Ends the threads. Work not begun is dropped: a writer among it gives up its
    /// slice.
    fn stop(&mut self) {
        {
            let mut q = lock(&self.shared.0);
            q.stop = true;
            q.work.clear();
        }
        self.shared.1.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn work(
    shared: &(Mutex<WorkQueue>, Condvar),
    results: &mpsc::Sender<Done>,
    table: Arc<BlockTable>,
) {
    let mut mesher = Mesher::new(table);
    let mut sorter = Sorter::default();
    loop {
        let next = {
            let mut q = lock(&shared.0);
            loop {
                if q.stop {
                    return;
                }
                if let Some(w) = q.work.pop_front() {
                    break w;
                }
                q = shared.1.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        match next {
            Work::Mesh {
                job,
                options,
                biomes,
                camera,
                session,
            } => {
                profiling::scope!("mesh_section");
                let started = Instant::now();
                let mesh = mesher.mesh(&job.neighbourhood, &options, &biomes);
                let section = job.neighbourhood.section();
                let mut indices = Vec::new();
                if !mesh.centres.is_empty() {
                    sorter.indices(&mesh.centres, sort::eye(camera, section), &mut indices);
                }
                let done = Meshed {
                    session,
                    stamp: job.neighbourhood.stamp().clone(),
                    mesh,
                    indices,
                    sorted_at: camera.map(ruthenium_javamath::mth::floor),
                    sorted_from: sort::point_of_view(camera, section),
                    urgent: job.urgent,
                    first: job.first,
                    since: job.since,
                    micros: started.elapsed().as_micros() as u64,
                };
                if results.send(Done::Meshed(Box::new(done))).is_err() {
                    return;
                }
            }
            Work::Sort {
                session,
                key,
                mesh,
                centres,
                camera,
            } => {
                profiling::scope!("sort_section");
                let mut indices = Vec::new();
                sorter.indices(&centres, sort::eye(camera, key), &mut indices);
                let done = Sorted {
                    session,
                    key,
                    mesh,
                    indices,
                    sorted_at: camera.map(ruthenium_javamath::mth::floor),
                    sorted_from: sort::point_of_view(camera, key),
                };
                if results.send(Done::Sorted(done)).is_err() {
                    return;
                }
            }
            Work::Fill { mut writer, bytes } => {
                profiling::scope!("fill_staging");
                // A writer dropped without finishing gives its slice up.
                if writer.push(&bytes) {
                    writer.finish();
                }
            }
        }
    }
}

/// Where a result is between its worker and the screen.
enum Stage {
    /// It has no slice of staging yet.
    Waiting(Vec<u8>),
    /// A worker copies it into its slice.
    Writing(Reservation),
    /// Written; it has no place in a page yet.
    Placing(Reservation),
    /// Its copy into the page is recorded.
    Copying(RangeHandle, UploadTicket),
}

struct Pending {
    stamp: Stamp,
    urgent: bool,
    first: bool,
    /// When the change it shows was known.
    known: Since,
    /// Bytes of the vertices and of the indices behind them, and of the vertices.
    bytes: u64,
    vertex_bytes: u64,
    solid_quads: u32,
    cutout_quads: u32,
    facings: Facings,
    translucent_quads: u32,
    centres: Arc<[[f32; 3]]>,
    sorted_at: [i32; 3],
    sorted_from: PointOfView,
    bounds: [[f32; 3]; 2],
    connectivity: Connectivity,
    since: Instant,
    stage: Stage,
}

/// A new order of a mesh's translucent quads between its worker and the screen.
struct PendingSort {
    sorted: Sorted,
    /// Its range and its copy, once it has them.
    placed: Option<(RangeHandle, UploadTicket)>,
}

/// A section the frame draws.
#[derive(Clone, Copy)]
struct Visible {
    distance: i64,
    origin: [i32; 3],
    /// How far it has faded in.
    visibility: f32,
    buffer: vk::Buffer,
    page: u32,
    first_vertex: u32,
    solid_quads: u32,
    cutout_quads: u32,
    /// Its solid and cutout groups ([`facing::SLOTS`]), and the directions whose
    /// groups can be seen from the camera.
    solid: [u32; 7],
    cutout: [u32; 7],
    shown: u8,
    /// Its translucent quads and where their indices are.
    translucent: Option<(u32, IndexRange)>,
}

/// What the terrain is doing, for the debug text and the logs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldStats {
    /// Sections of loaded chunks, sections with something to draw, sections waiting
    /// for a mesh.
    pub sections: usize,
    pub meshed: usize,
    pub dirty: usize,
    pub tracker: TrackerStats,
    /// Jobs with a worker, and results on their way into a page.
    pub jobs_out: usize,
    pub pending: usize,
    /// Meshes installed, and the quads and bytes they had.
    pub installed: u64,
    pub installed_quads: u64,
    pub installed_bytes: u64,
    /// Time workers spent meshing, in all and for the slowest section, in microseconds.
    pub mesh_micros: u64,
    pub slowest_mesh_micros: u64,
    /// The longest a mesh took from its result to being drawn, in milliseconds.
    pub slowest_install_ms: f32,
    /// Translucent quads so far (in meshes that arrived), and the times a section's
    /// were sorted again and drawn in their new order.
    pub translucent_quads: u64,
    pub sorts: u64,
    /// The last frame's translucent quads and the sections they are of.
    pub translucent_drawn: u64,
    pub translucent_sections: u32,
    /// Orders being made or on their way.
    pub sorting: usize,
    /// Animated sprites, the ticks they have played, the times one was written into the
    /// atlas again, the bytes that took, the times the upload ring had no room for some,
    /// and the longest a frame spent on them, in microseconds.
    pub animated_sprites: usize,
    pub animation_ticks: u64,
    pub animation_writes: u64,
    pub animation_bytes: u64,
    pub animation_waits: u64,
    pub slowest_animation_micros: u32,
    /// Corners and texture coordinates moved into a vertex's range.
    pub out_of_range: u64,
    /// Meshes that found no room in the pool's budget, or were larger than a page.
    pub dropped: u64,
    /// The last frame: sections tested against the view, sections drawn, their quads,
    /// the draws issued, the calls that issued them (as many, unless one call issues
    /// several) and the vertex buffers bound.
    pub tested: u32,
    pub visible: u32,
    pub quads: u64,
    pub draws: u32,
    pub calls: u32,
    pub binds: u32,
    /// Sections a frame could not draw because its data had no room for their records.
    pub overflowed: u64,
    /// The bytes of the meshes there are: what the pool's payload comes to but for the
    /// quad indices and the meshes on their way in or out.
    pub mesh_bytes: u64,
    pub pool: PoolStats,
    /// The last frame's block entities.
    pub block_entities: crate::block_entity::renderer::BlockEntityStats,
    /// Cave culling is on: only what the visibility graph reaches from the camera is
    /// meshed and drawn.
    pub culling: bool,
    /// Sections the graph reaches (every section without culling), and meshes of
    /// sections it reached once and does not now.
    pub reached: usize,
    pub hidden: usize,
    /// The graph has yet to answer something it was told.
    pub graph_behind: bool,
    /// The graph's answers so far, of them walks from the camera, and the time its
    /// walks took: in all, and the longest one, in microseconds.
    pub graph_updates: u64,
    pub graph_walks: u64,
    pub graph_micros: u64,
    pub slowest_graph_micros: u32,
}

/// See the module docs.
pub(crate) struct WorldRenderer {
    atlas: TextureId,
    atlas_size: [f32; 2],
    animations: AtlasAnimations,
    /// Whether they play, and the level and the tick they last played in.
    animate: bool,
    animated_at: Option<(u64, u64)>,
    /// How many sprites the last tick left to write, and when the last frame was.
    animated_stale: usize,
    animated_frame: Option<Instant>,
    /// Scratch of [`Self::animate`]: a piece's sprites, rectangles and texels.
    animated: Vec<usize>,
    animated_rects: Vec<Rect>,
    animated_texels: Vec<u8>,
    lightmap: Option<TextureId>,
    lightmap_inputs: LightmapInputs,
    pool: BufferPool,
    /// The quad index buffer's range, and its upload once it is staged.
    indices: RangeHandle,
    indices_upload: Option<UploadTicket>,
    indices_ready: bool,
    tracker: Tracker<MeshRange>,
    table: Arc<BlockTable>,
    /// Cave culling: the graph on its thread, the batches sent to it and the last one
    /// it answered, and the camera's cell and options when one was last sent.
    visibility: Option<Visibility>,
    batches: u64,
    settled: u64,
    cell: Option<([i64; 3], bool, i32)>,
    /// Counts the times everything was forgotten: a result of an earlier count is of
    /// a session that is over, whose stamps mean nothing in this one.
    session: u64,
    workers: Workers,
    /// How many meshing threads there are.
    worker_count: usize,
    jobs_out: usize,
    pending: Vec<Pending>,
    /// Sorts with a worker, and their results on the way to the GPU.
    sorts_out: usize,
    sorted: Vec<PendingSort>,
    /// Where the camera is (the last [`Self::update`]'s).
    camera: [f64; 3],
    removed: Vec<MeshRange>,
    /// The block entities of the sections that have some, as last meshed.
    block_entities: HashMap<SectionKey, Box<SectionEntities>>,
    visible: Vec<Visible>,
    /// Scratch of [`Self::draw`]: the visible sections whose translucent quads are to
    /// be sorted again, with their distance.
    unsorted: Vec<(i64, SectionKey)>,
    /// What [`Self::draw`] leaves for [`Self::draw_translucent`]: the frame's push
    /// constants and how many of the visible sections have a record.
    drawn: Option<([u8; TERRAIN_PUSH_BYTES], usize)>,
    /// Scratch of [`Self::draw_translucent`]: the sections to draw, farthest first, as
    /// their places among the visible ones; and the indirect calls (vertex buffer,
    /// indices, first command and how many).
    translucent_order: Vec<u32>,
    translucent_calls: Vec<(vk::Buffer, IndexRange, u32, u32)>,
    /// How the sections are drawn, and whether the GPU can draw them indirectly.
    draws: TerrainDraws,
    indirect: bool,
    /// How their texels are filtered, and how long a section takes to fade in (0: it
    /// is there at once).
    shader: TerrainShader,
    fade_ms: u32,
    /// Whether the translucent layer is drawn (and sorted again as the camera moves).
    translucent: bool,
    /// Whether a section's groups of quads the camera is behind are left out
    /// ([`facing`]).
    face_culling: bool,
    /// Scratch of [`Self::draw`]: each page's nearest section and then its place in the
    /// order of pages; a layer's draw commands; and the calls that read them (a vertex
    /// buffer, its first command and how many).
    page_order: Vec<i64>,
    commands: Vec<u8>,
    calls: Vec<(vk::Buffer, u32, u32)>,
    latencies: [LatencySeries; 3],
    options: MeshOptions,
    colour_maps: ColourMaps,
    /// The dimension's environment the biomes' colours were last worked out from.
    environment: Option<Arc<Environment>>,
    biomes: Arc<BiomeTints>,
    stats: WorldStats,
    warned_dropped: bool,
    warned_envelope: bool,
}

/// The series of [`WorldLatencies`], in its fields' order.
const FIRST: usize = 0;
const CHANGED: usize = 1;
const OWN: usize = 2;

/// One indexed draw command of a single instance, as the GPU reads it. The solid and
/// the cutout layer start at the shared quad indices' beginning (`first_index` 0).
pub(crate) fn draw_command(
    indices: u32,
    first_index: u32,
    vertex_offset: i32,
    instance: u32,
) -> [u8; DRAW_COMMAND_BYTES as usize] {
    let mut out = [0u8; DRAW_COMMAND_BYTES as usize];
    out[0..4].copy_from_slice(&indices.to_ne_bytes());
    out[4..8].copy_from_slice(&1u32.to_ne_bytes());
    out[8..12].copy_from_slice(&first_index.to_ne_bytes());
    out[12..16].copy_from_slice(&vertex_offset.to_ne_bytes());
    out[16..20].copy_from_slice(&instance.to_ne_bytes());
    out
}

impl std::fmt::Debug for WorldRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldRenderer")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

const WORLD_CAMERA_OFFSET: usize = WORLD.offset("camera_offset") as usize * 4;
const WORLD_ATLAS_SIZE: usize = WORLD.offset("atlas_size") as usize * 4;
const WORLD_LIGHTMAP: usize = WORLD.offset("lightmap") as usize * 4;
const SECTION_ORIGIN: usize = SECTION.offset("origin") as usize * 4;
const SECTION_VISIBILITY: usize = SECTION.offset("visibility") as usize * 4;
/// Bytes of one section record.
pub(crate) const SECTION_BYTES: usize = SECTION.bytes() as usize;

fn put(bytes: &mut [u8], at: usize, word: [u8; 4]) {
    bytes[at..at + 4].copy_from_slice(&word);
}

/// Where the terrain's push constants are, in bytes: four words the vertex stage reads
/// (the word offsets of the frame's world record, its matrix and its first section
/// record, and that the draw is a real one), then what every fragment reads: the fog's
/// colour and its two lines ([`Fog::lines`]), one texel of the atlas in its coordinates
/// and the atlas's size in texels, the atlas's index in the texture table, and one over
/// the shorter side of a texel, squared and as its base-2 logarithm.
/// `terrain_frag.glsl` declares the same block.
pub(crate) const PUSH_ENABLED: usize = 12;
pub(crate) const PUSH_FOG_COLOUR: usize = 16;
pub(crate) const PUSH_FOG_ENVIRONMENTAL: usize = 32;
pub(crate) const PUSH_FOG_RENDER_DISTANCE: usize = 40;
pub(crate) const PUSH_ATLAS_TEXEL: usize = 48;
pub(crate) const PUSH_ATLAS_SIZE: usize = 56;
pub(crate) const PUSH_ATLAS: usize = 64;
pub(crate) const PUSH_TEXEL_SCALE_SQUARED: usize = 68;
pub(crate) const PUSH_TEXEL_SCALE_LOG2: usize = 72;
/// Bytes of the terrain's push constants.
pub(crate) const TERRAIN_PUSH_BYTES: usize = 76;

/// The terrain's push constants for a frame's draws.
pub(crate) fn terrain_push(
    world: u32,
    matrix: u32,
    sections: u32,
    fog: &Fog,
    atlas: TextureId,
    atlas_size: [f32; 2],
) -> [u8; TERRAIN_PUSH_BYTES] {
    let mut push = [0u8; TERRAIN_PUSH_BYTES];
    put(&mut push, 0, world.to_ne_bytes());
    put(&mut push, 4, matrix.to_ne_bytes());
    put(&mut push, 8, sections.to_ne_bytes());
    put(&mut push, PUSH_ENABLED, 1u32.to_ne_bytes());
    for (i, c) in fog.colour.iter().enumerate() {
        put(&mut push, PUSH_FOG_COLOUR + i * 4, c.to_ne_bytes());
    }
    let [environmental, render_distance] = fog.lines();
    for i in 0..2 {
        put(
            &mut push,
            PUSH_FOG_ENVIRONMENTAL + i * 4,
            environmental[i].to_ne_bytes(),
        );
        put(
            &mut push,
            PUSH_FOG_RENDER_DISTANCE + i * 4,
            render_distance[i].to_ne_bytes(),
        );
        put(
            &mut push,
            PUSH_ATLAS_TEXEL + i * 4,
            (1.0 / atlas_size[i]).to_ne_bytes(),
        );
        put(
            &mut push,
            PUSH_ATLAS_SIZE + i * 4,
            atlas_size[i].to_ne_bytes(),
        );
    }
    put(&mut push, PUSH_ATLAS, atlas.index().to_ne_bytes());
    // One over the shorter side of a texel: the atlas's longer side.
    let scale = atlas_size[0].max(atlas_size[1]);
    put(
        &mut push,
        PUSH_TEXEL_SCALE_SQUARED,
        (scale * scale).to_ne_bytes(),
    );
    put(&mut push, PUSH_TEXEL_SCALE_LOG2, scale.log2().to_ne_bytes());
    push
}

/// Writes a world record: what every terrain vertex of a frame reads but the camera's
/// matrix.
pub(crate) fn write_world_record(
    bytes: &mut [u8],
    camera_offset: [f32; 3],
    atlas_size: [f32; 2],
    lightmap: TextureId,
) {
    for (i, c) in camera_offset.iter().enumerate() {
        put(bytes, WORLD_CAMERA_OFFSET + i * 4, c.to_ne_bytes());
    }
    put(bytes, WORLD_ATLAS_SIZE, atlas_size[0].to_ne_bytes());
    put(bytes, WORLD_ATLAS_SIZE + 4, atlas_size[1].to_ne_bytes());
    put(bytes, WORLD_LIGHTMAP, lightmap.index().to_ne_bytes());
}

/// Writes a frame's world record and its matrix into the frame's data: their word
/// offsets, or `None` if it has no room.
pub(crate) fn write_world(
    frame_data: &mut FrameData,
    view: &View,
    atlas_size: [f32; 2],
    lightmap: TextureId,
) -> Option<(u32, u32)> {
    let (world, bytes) = frame_data.alloc(WORLD.words(), 4)?;
    write_world_record(bytes, view.offset, atlas_size, lightmap);
    let (matrix, bytes) = frame_data.alloc(WORLD_MATRIX.words(), 4)?;
    for (i, c) in view.view_projection.iter().enumerate() {
        put(bytes, i * 4, c.to_ne_bytes());
    }
    Some((world, matrix))
}

/// Writes one section record: where the section's corner is from the camera's block,
/// and how far it has faded in.
pub(crate) fn write_section(bytes: &mut [u8], origin: [i32; 3], visibility: f32) {
    for (i, c) in origin.iter().enumerate() {
        put(bytes, SECTION_ORIGIN + i * 4, c.to_ne_bytes());
    }
    put(bytes, SECTION_VISIBILITY, visibility.to_ne_bytes());
}

impl WorldRenderer {
    /// Creates the atlas texture and the vertex pool's first page, and starts `workers`
    /// meshing threads, and with `culling` the visibility graph's. Not on a frame's
    /// path: behind the screen that says the world is loading.
    pub fn new(
        gpu: &Arc<Gpu>,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        assets: WorldAssets,
        workers: usize,
        culling: bool,
    ) -> Result<WorldRenderer, Error> {
        uploads.enable_worker_staging(WORKER_STAGING_BYTES)?;
        let mut pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: PAGE_BYTES,
                budget_bytes: POOL_BUDGET,
                low_watermark: 1 << 20,
                high_watermark: 6 << 20,
                release_after: 600,
                granule: VERTEX_BYTES as u64,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER,
        );
        pool.grow_now()?;
        let indices = pool
            .alloc(INDEX_BYTES, VERTEX_BYTES as u64)
            .map_err(|e| Error::Unsupported(format!("no room for the quad indices: {e:?}")))?;
        let desc = TextureDesc {
            width: assets.atlas_width,
            height: assets.atlas_height,
            mip_levels: assets.atlas_levels.len() as u32,
            format: vk::Format::R8G8B8A8_UNORM,
        };
        let atlas = textures.create(desc)?;
        if let Err(e) = textures.stream(uploads, atlas, assets.atlas_levels) {
            textures.destroy(atlas);
            return Err(Error::Upload(e));
        }
        Ok(WorldRenderer {
            atlas,
            atlas_size: [assets.atlas_width as f32, assets.atlas_height as f32],
            animations: assets.animations,
            animate: true,
            animated_at: None,
            animated_stale: 0,
            animated_frame: None,
            animated: Vec::new(),
            animated_rects: Vec::new(),
            animated_texels: Vec::new(),
            lightmap: None,
            lightmap_inputs: LightmapInputs::default(),
            pool,
            indices,
            indices_upload: None,
            indices_ready: false,
            tracker: if culling {
                Tracker::culled(assets.table.clone())
            } else {
                Tracker::new()
            },
            visibility: culling.then(Visibility::start),
            batches: 0,
            settled: 0,
            cell: None,
            session: 0,
            workers: Workers::start(&assets.table, workers),
            worker_count: workers,
            table: assets.table,
            jobs_out: 0,
            pending: Vec::new(),
            sorts_out: 0,
            sorted: Vec::new(),
            camera: [0.0; 3],
            removed: Vec::new(),
            block_entities: HashMap::new(),
            visible: Vec::new(),
            unsorted: Vec::new(),
            drawn: None,
            translucent_order: Vec::new(),
            translucent_calls: Vec::new(),
            draws: TerrainDraws::default(),
            indirect: {
                let o = gpu.caps().optional;
                o.multi_draw_indirect && o.draw_indirect_first_instance
            },
            shader: TerrainShader::default(),
            fade_ms: DEFAULT_FADE_MS,
            translucent: true,
            face_culling: true,
            page_order: Vec::new(),
            commands: Vec::new(),
            calls: Vec::new(),
            latencies: [
                LatencySeries::new(),
                LatencySeries::new(),
                LatencySeries::new(),
            ],
            options: MeshOptions {
                shades: [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
                smooth_lighting: true,
                biome_blend: 2,
                cutout_leaves: true,
            },
            colour_maps: assets.colour_maps,
            environment: None,
            biomes: Arc::new(BiomeTints::default()),
            stats: WorldStats::default(),
            warned_dropped: false,
            warned_envelope: false,
        })
    }

    /// Takes a resource reload's assets (other packs): the block table and models, the
    /// atlas (a new texture streamed, the old one destroyed), its animations and the
    /// colour maps. Every section's mesh is forgotten, as on a new level, and every
    /// loaded section is meshed again from the new table (vanilla's `allChanged`); the
    /// meshing threads start again with it.
    pub fn replace_assets(
        &mut self,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        assets: WorldAssets,
        frame: u64,
    ) -> Result<(), Error> {
        let desc = TextureDesc {
            width: assets.atlas_width,
            height: assets.atlas_height,
            mip_levels: assets.atlas_levels.len() as u32,
            format: vk::Format::R8G8B8A8_UNORM,
        };
        let atlas = textures.create(desc)?;
        if let Err(e) = textures.stream(uploads, atlas, assets.atlas_levels) {
            textures.destroy(atlas);
            return Err(Error::Upload(e));
        }
        // The old threads mesh with the old table: their results are of a session
        // that is over once `clear` has counted on.
        self.workers.stop();
        textures.destroy(std::mem::replace(&mut self.atlas, atlas));
        self.atlas_size = [assets.atlas_width as f32, assets.atlas_height as f32];
        self.animations = assets.animations;
        self.animated_at = None;
        self.animated_stale = 0;
        self.animated_frame = None;
        self.table = assets.table;
        self.colour_maps = assets.colour_maps;
        // The biomes' colours are worked out again from the new maps.
        self.environment = None;
        self.clear(uploads, frame);
        self.tracker.rescan_next();
        self.workers = Workers::start(&self.table, self.worker_count);
        Ok(())
    }

    /// Ends the worker threads. They write into the upload ring's staging memory, so
    /// this comes before the ring goes.
    pub fn stop_workers(&mut self) {
        self.workers.stop();
        if let Some(v) = &mut self.visibility {
            v.stop();
        }
    }

    /// Asks for the sections to be drawn this way; a GPU without multi-draw indirect
    /// draws them a page at a time instead. Returns the way they will be drawn.
    pub fn set_draws(&mut self, draws: TerrainDraws) -> TerrainDraws {
        self.draws = match draws {
            TerrainDraws::Indirect if !self.indirect => TerrainDraws::Paged,
            other => other,
        };
        self.draws
    }

    pub fn draws(&self) -> TerrainDraws {
        self.draws
    }

    /// The latencies of the meshes installed since the last reset.
    pub fn latencies(&self) -> WorldLatencies {
        WorldLatencies {
            first: self.latencies[FIRST].summary(),
            changed: self.latencies[CHANGED].summary(),
            own: self.latencies[OWN].summary(),
        }
    }

    /// Forgets the latencies so far (a measurement begins).
    pub fn reset_latencies(&mut self) {
        for l in &mut self.latencies {
            l.clear();
        }
    }

    fn note_latency(&mut self, urgent: bool, first: bool, since: Since, frame: u64) {
        let series = match (urgent, first) {
            (true, _) => OWN,
            (false, true) => FIRST,
            (false, false) => CHANGED,
        };
        self.latencies[series].push(since, frame);
    }

    pub fn stats(&self) -> WorldStats {
        let (sections, meshed, dirty) = self.tracker.counts();
        WorldStats {
            sections,
            meshed,
            dirty,
            tracker: self.tracker.stats(),
            jobs_out: self.jobs_out,
            pending: self.pending.len(),
            mesh_bytes: self.tracker.all_meshes().map(MeshRange::bytes).sum(),
            sorting: self.sorts_out + self.sorted.len(),
            animated_sprites: self.animations.len(),
            pool: self.pool.stats(),
            culling: self.visibility.is_some(),
            reached: self.tracker.reached(),
            hidden: meshed - self.tracker.meshes().count(),
            graph_behind: self.visibility.is_some()
                && (self.settled != self.batches || self.tracker.has_events()),
            ..self.stats
        }
    }

    /// Forgets every section (the session ended, or another began): their meshes are
    /// retired after `frame`, and results still on their way are thrown away.
    pub fn clear(&mut self, uploads: &mut UploadRing, frame: u64) {
        self.tracker.clear(&mut self.removed);
        for m in self.removed.drain(..) {
            retire(&mut self.pool, &m, Users::until_frame(frame));
        }
        for s in self.sorted.drain(..) {
            if let Some((handle, ticket)) = s.placed {
                self.pool.retire(handle, uploads.users_of(ticket));
            }
        }
        self.drawn = None;
        self.block_entities.clear();
        for p in self.pending.drain(..) {
            match p.stage {
                Stage::Waiting(_) => {}
                Stage::Writing(r) | Stage::Placing(r) => uploads.release(r),
                Stage::Copying(handle, ticket) => {
                    self.pool.retire(handle, uploads.users_of(ticket));
                }
            }
        }
        self.tracker = if self.visibility.is_some() {
            Tracker::culled(self.table.clone())
        } else {
            Tracker::new()
        };
        // The next frame tells the graph where the camera is, whatever it knew.
        self.cell = None;
        self.session += 1;
    }

    /// Sets what meshes are shaded and tinted by: the dimension's cardinal light and
    /// its biomes' colours. A change makes every mesh out of date; a dimension change,
    /// which is when it happens, empties them anyway.
    pub fn set_environment(&mut self, environment: &Arc<Environment>) {
        if self
            .environment
            .as_ref()
            .is_some_and(|e| Arc::ptr_eq(e, environment))
        {
            return;
        }
        self.options.shades = environment.cardinal_light.shades();
        self.biomes = Arc::new(BiomeTints::new(&self.colour_maps, &environment.biomes));
        self.environment = Some(environment.clone());
    }

    /// Whether the translucent layer is drawn. It applies from the next frame.
    pub fn set_translucent(&mut self, on: bool) {
        self.translucent = on;
    }

    /// Whether the groups of a section's solid and cutout quads that face away from the
    /// camera are left out of the frame (on unless said otherwise). The picture is the
    /// same either way. It applies from the next frame.
    pub fn set_face_culling(&mut self, on: bool) {
        self.face_culling = on;
    }

    /// How the terrain's texels are filtered. It applies from the next frame.
    pub fn set_shader(&mut self, shader: TerrainShader) {
        self.shader = shader;
    }

    /// How long a section takes to fade in once it has a mesh, in milliseconds (750
    /// unless said otherwise; 0: it is there at once).
    pub fn set_fade_ms(&mut self, ms: u32) {
        self.fade_ms = ms;
    }

    /// Biome Blend: how many columns each way a biome colour is the mean over (2 unless
    /// said otherwise, 7 at most). To be set before anything is meshed.
    pub fn set_biome_blend(&mut self, radius: u8) {
        self.options.biome_blend = radius.min(MAX_BLEND_RADIUS);
    }

    pub fn biome_blend(&self) -> u8 {
        self.options.biome_blend
    }

    /// Smooth Lighting (on unless said otherwise). To be set before anything is meshed:
    /// meshes made before a change keep the lighting they were made with.
    pub fn set_smooth_lighting(&mut self, smooth: bool) {
        self.options.smooth_lighting = smooth;
    }

    pub fn smooth_lighting(&self) -> bool {
        self.options.smooth_lighting
    }

    /// Creates the light map, or rewrites it when its inputs changed.
    fn update_lightmap(
        &mut self,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        inputs: LightmapInputs,
    ) {
        let texels = || inputs.texels();
        match self.lightmap {
            None => {
                let desc = TextureDesc::rgba8(lightmap::SIZE, lightmap::SIZE);
                if let Ok(id) = textures.create_with(
                    uploads,
                    desc,
                    &[&texels()],
                    ruthenium_gpu::Arrival::NextFrame,
                ) {
                    self.lightmap = Some(id);
                    self.lightmap_inputs = inputs;
                }
            }
            Some(id) if inputs != self.lightmap_inputs => {
                let whole = ruthenium_gpu::ImageRegion::whole_mip(
                    vk::Extent2D {
                        width: lightmap::SIZE,
                        height: lightmap::SIZE,
                    },
                    0,
                );
                // Full or not ready: the old light stays for another frame.
                if textures.update(uploads, id, &[(whole, &texels())]).is_ok() {
                    self.lightmap_inputs = inputs;
                }
            }
            Some(_) => {}
        }
    }

    /// The part of a frame's work that stages uploads: before the frame's
    /// `UploadRing::record`. Takes in what changed in the level, takes finished meshes
    /// from the workers, moves results towards their pages and hands out new jobs.
    /// `frame` is the frame being recorded, `camera` where the world is seen from and
    /// `view_distance` how many chunks far.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        scene: &Scene,
        camera: &CameraPose,
        view_distance: i32,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        lightmap: LightmapInputs,
        frame: u64,
    ) {
        profiling::scope!("world_update");
        let block = camera.position.map(|c| c.floor() as i32);
        let section: SectionKey = block.map(|b| b >> 4);
        self.camera = camera.position;
        self.tracker.set_clock(Since {
            at: Instant::now(),
            frame,
        });
        // What the graph reaches now, before work is handed out by it.
        if let Some(v) = &self.visibility {
            while let Some(delta) = v.poll() {
                self.tracker.reach(&delta);
                self.settled = delta.seq;
                self.stats.graph_updates += 1;
                self.stats.graph_walks += u64::from(delta.full);
                self.stats.graph_micros += u64::from(delta.micros);
                self.stats.slowest_graph_micros = self.stats.slowest_graph_micros.max(delta.micros);
            }
        }
        // What this frame learns goes to the graph in one batch, at its end.
        self.tracker.set_batch(self.batches + 1);
        self.update_lightmap(textures, uploads, lightmap);
        self.pool.maintain(&uploads.completed());
        let reserved = self.pool.stats().reserved_bytes;
        if reserved > POOL_ENVELOPE && !std::mem::replace(&mut self.warned_envelope, true) {
            eprintln!(
                "[render] the terrain's buffers hold {} MiB, more than the {} MiB planned for them",
                reserved >> 20,
                POOL_ENVELOPE >> 20
            );
        }
        if self.indices_upload.is_none()
            && let Some(range) = self.pool.resolve(self.indices)
        {
            // Full for now: the next frame tries again.
            self.indices_upload = uploads
                .upload_buffer(range.buffer, range.offset, &quad_indices())
                .ok();
        }
        self.tracker.sync(scene, &mut self.removed);
        for m in self.removed.drain(..) {
            retire(&mut self.pool, &m, Users::until_frame(frame));
        }

        while let Ok(done) = self.workers.results.try_recv() {
            let done = match done {
                Done::Meshed(done) => *done,
                Done::Sorted(sorted) => {
                    self.sorts_out -= 1;
                    if sorted.session == self.session {
                        self.sorted.push(PendingSort {
                            sorted,
                            placed: None,
                        });
                    }
                    continue;
                }
            };
            self.jobs_out -= 1;
            if done.session != self.session {
                continue;
            }
            self.stats.mesh_micros += done.micros;
            self.stats.slowest_mesh_micros = self.stats.slowest_mesh_micros.max(done.micros);
            if !self.tracker.accept(scene, &done.stamp) {
                continue;
            }
            let key = done.stamp.section;
            if done.mesh.block_entities.is_empty() {
                self.block_entities.remove(&key);
            } else {
                let list = done.mesh.block_entities.clone().into_boxed_slice();
                self.block_entities.insert(key, list);
            }
            let Some(bounds) = done.mesh.bounds else {
                // Nothing to draw there: that is its mesh.
                self.note_installed(&done.mesh, 0, Instant::now());
                self.note_latency(done.urgent, done.first, done.since, frame);
                if let Some(old) = self.tracker.install(done.stamp, None) {
                    retire(&mut self.pool, &old, Users::until_frame(frame));
                }
                self.tracker.connect(key, done.mesh.connectivity, true);
                continue;
            };
            // What the mesh opens is reached while it is on its way to the GPU; what
            // it closes stays reached until it is drawn.
            self.tracker.connect(key, done.mesh.connectivity, false);
            self.stats.translucent_quads += u64::from(done.mesh.translucent_quads);
            self.stats.out_of_range += u64::from(done.mesh.out_of_range);
            // The indices travel behind the vertices, in one range.
            let mut bytes = done.mesh.vertices;
            let vertex_bytes = bytes.len() as u64;
            bytes.extend_from_slice(&done.indices);
            self.pending.push(Pending {
                stamp: done.stamp,
                urgent: done.urgent,
                first: done.first,
                known: done.since,
                bytes: bytes.len() as u64,
                vertex_bytes,
                solid_quads: done.mesh.solid_quads,
                cutout_quads: done.mesh.cutout_quads,
                facings: done.mesh.facings,
                translucent_quads: done.mesh.translucent_quads,
                centres: done.mesh.centres.into(),
                sorted_at: done.sorted_at,
                sorted_from: done.sorted_from,
                bounds,
                connectivity: done.mesh.connectivity,
                since: Instant::now(),
                stage: Stage::Waiting(bytes),
            });
        }

        let mut i = 0;
        while i < self.pending.len() {
            if self.advance(i, uploads) {
                i += 1;
            } else {
                self.pending.swap_remove(i);
            }
        }
        self.place_sorts(uploads);

        let limit = self.workers.threads.len() * JOBS_PER_WORKER;
        while self.jobs_out < limit && self.jobs_out + self.pending.len() < MAX_PENDING {
            let Some(job) = self.tracker.next_job(scene, section) else {
                break;
            };
            self.jobs_out += 1;
            self.workers.push(Work::Mesh {
                job: Box::new(job),
                options: self.options,
                biomes: self.biomes.clone(),
                camera: camera.position,
                session: self.session,
            });
        }

        if let Some(v) = &self.visibility {
            // A spectator inside a solid block sees through everything.
            let inside = || {
                scene
                    .with_terrain(|t| t.probe(block))
                    .is_some_and(|p| self.table.is_solid(p.block))
            };
            let spectator = scene
                .frame()
                .is_some_and(|f| f.player.game_type == SPECTATOR);
            let camera = Camera {
                position: camera.position,
                smart: !(spectator && inside()),
                view_distance,
            };
            let cell = camera.cell();
            if self.tracker.has_events() || self.cell != Some(cell) {
                self.cell = Some(cell);
                self.batches += 1;
                v.send(Batch {
                    seq: self.batches,
                    camera,
                    events: self.tracker.take_events(),
                });
            }
        }
    }

    /// The translucent quads a new order is for, if their mesh is still the one drawn.
    fn sorting_mesh<'t>(
        tracker: &'t mut Tracker<MeshRange>,
        sorted: &Sorted,
    ) -> Option<&'t mut Translucent> {
        tracker
            .mesh_mut(sorted.key)
            .filter(|m| m.handle == sorted.mesh)
            .and_then(|m| m.translucent.as_mut())
    }

    /// Gives every new order that has none a range of its own and records the copy into
    /// it: into this frame, so that the frame may draw with it.
    fn place_sorts(&mut self, uploads: &mut UploadRing) {
        let mut i = 0;
        while i < self.sorted.len() {
            if self.sorted[i].placed.is_some() {
                i += 1;
                continue;
            }
            // The mesh was replaced meanwhile: the new one came with its own order.
            if Self::sorting_mesh(&mut self.tracker, &self.sorted[i].sorted).is_none() {
                self.sorted.swap_remove(i);
                continue;
            }
            let bytes = self.sorted[i].sorted.indices.len() as u64;
            let placed = match self.pool.alloc(bytes, VERTEX_BYTES as u64) {
                Ok(handle) => {
                    let range = self.pool.resolve(handle).expect("just allocated");
                    match uploads.upload_buffer_with(
                        range.buffer,
                        range.offset,
                        &self.sorted[i].sorted.indices,
                        Urgency::Edit,
                    ) {
                        Ok(ticket) => Ok((handle, ticket)),
                        Err(e) => {
                            self.pool.retire(handle, Users::NONE);
                            Err(matches!(e, UploadError::Full))
                        }
                    }
                }
                Err(e) => Err(matches!(e, PoolError::Growing)),
            };
            match placed {
                Ok(placed) => {
                    let s = &mut self.sorted[i];
                    s.placed = Some(placed);
                    s.sorted.indices = Vec::new();
                    i += 1;
                }
                // No room this frame: the next one asks again.
                Err(true) => i += 1,
                // No room at all: the section keeps the order it has, and is asked
                // again only when the camera has moved on.
                Err(false) => {
                    let s = self.sorted.swap_remove(i).sorted;
                    if let Some(t) = Self::sorting_mesh(&mut self.tracker, &s) {
                        t.sorting = false;
                        t.sorted_at = s.sorted_at;
                        t.sorted_from = s.sorted_from;
                    }
                }
            }
        }
    }

    fn note_installed(&mut self, mesh: &SectionMesh, bytes: u64, since: Instant) {
        self.stats.installed += 1;
        self.stats.installed_quads += u64::from(mesh.quads());
        self.stats.installed_bytes += bytes;
        self.stats.slowest_install_ms = self
            .stats
            .slowest_install_ms
            .max(since.elapsed().as_secs_f32() * 1e3);
    }

    fn drop_for_memory(&mut self, p: usize, why: &str) {
        self.stats.dropped += 1;
        if !std::mem::replace(&mut self.warned_dropped, true) {
            eprintln!(
                "[render] a section's mesh of {} KiB is not drawn: {why}",
                self.pending[p].bytes >> 10
            );
        }
        // Its section keeps what it draws now and is not meshed again for this state
        // of the level: trying again would only fail again.
        let stamp = self.pending[p].stamp.clone();
        self.tracker.install(stamp, None);
    }

    /// Moves a waiting result one stage on if it can go. `false`: it is done with (its
    /// section was given up or dropped) and leaves the list.
    fn advance(&mut self, p: usize, uploads: &mut UploadRing) -> bool {
        let urgency = if self.pending[p].urgent {
            Urgency::Edit
        } else {
            Urgency::Normal
        };
        if let Stage::Waiting(bytes) = &mut self.pending[p].stage {
            if bytes.len() as u64 > PAGE_BYTES {
                self.drop_for_memory(p, "it is larger than a buffer page");
                return false;
            }
            match uploads.reserve(bytes.len() as u64, urgency) {
                Ok((reservation, writer)) => {
                    let bytes = std::mem::take(bytes);
                    self.workers.push(Work::Fill { writer, bytes });
                    self.pending[p].stage = Stage::Writing(reservation);
                }
                // No slice for now: the next frame asks again.
                Err(UploadError::Full) => {}
                Err(_) => {
                    self.drop_for_memory(p, "it is larger than the staging buffer");
                    return false;
                }
            }
            return true;
        }
        if let Stage::Writing(r) = self.pending[p].stage {
            match uploads.reservation(r) {
                ReservationState::Writing => return true,
                ReservationState::Complete { .. } => self.pending[p].stage = Stage::Placing(r),
                ReservationState::Abandoned | ReservationState::Gone => {
                    uploads.release(r);
                    self.tracker.give_up(self.pending[p].stamp.section);
                    return false;
                }
            }
        }
        if let Stage::Placing(r) = self.pending[p].stage {
            let handle = match self.pool.alloc(self.pending[p].bytes, VERTEX_BYTES as u64) {
                Ok(h) => h,
                // A page is on its way.
                Err(PoolError::Growing) => return true,
                Err(PoolError::Full | PoolError::TooLarge) => {
                    uploads.release(r);
                    self.drop_for_memory(p, "the terrain's memory budget is used up");
                    return false;
                }
            };
            let range = self.pool.resolve(handle).expect("just allocated");
            match uploads.copy_reserved(r, range.buffer, range.offset, urgency) {
                Ok(ticket) => {
                    uploads.release(r);
                    self.pending[p].stage = Stage::Copying(handle, ticket);
                }
                // The batch's copy list is full: the next frame tries again.
                Err(UploadError::Full | UploadError::NotReady) => {
                    self.pool.retire(handle, Users::NONE);
                }
                Err(_) => {
                    self.pool.retire(handle, Users::NONE);
                    uploads.release(r);
                    self.tracker.give_up(self.pending[p].stamp.section);
                    return false;
                }
            }
        }
        true
    }

    /// The part of a frame's work that needs to know what the frame may read: after its
    /// `UploadRing::record`. Puts the meshes whose copies have arrived in place of their
    /// sections' old ones.
    pub fn install_ready(&mut self, scene: &Scene, uploads: &UploadRing, frame: u64) {
        if !self.indices_ready {
            self.indices_ready = self.indices_upload.is_some_and(|t| uploads.is_ready(t));
        }
        let mut i = 0;
        while i < self.pending.len() {
            let Stage::Copying(handle, ticket) = self.pending[i].stage else {
                i += 1;
                continue;
            };
            if !uploads.is_ready(ticket) {
                i += 1;
                continue;
            }
            let p = self.pending.swap_remove(i);
            let range = self.pool.resolve(handle).expect("a live range");
            // The level may have changed since the result came back.
            if !self.tracker.accept(scene, &p.stamp) {
                self.pool.retire(handle, uploads.users_of(ticket));
                continue;
            }
            let wide = p.translucent_quads as usize > sort::NARROW_QUADS;
            let mesh = MeshRange {
                handle,
                buffer: range.buffer,
                page: range.page,
                first_vertex: (range.offset / VERTEX_BYTES as u64) as u32,
                solid_quads: p.solid_quads,
                cutout_quads: p.cutout_quads,
                facings: p.facings,
                bounds: p.bounds,
                translucent: (p.translucent_quads > 0).then(|| Translucent {
                    quads: p.translucent_quads,
                    indices: IndexRange {
                        buffer: range.buffer,
                        first: ((range.offset + p.vertex_bytes) / if wide { 4 } else { 2 }) as u32,
                        wide,
                    },
                    own: None,
                    centres: p.centres,
                    sorted_at: p.sorted_at,
                    sorted_from: p.sorted_from,
                    sorting: false,
                }),
            };
            self.stats.installed += 1;
            self.stats.installed_quads +=
                u64::from(p.solid_quads + p.cutout_quads + p.translucent_quads);
            self.stats.installed_bytes += p.bytes;
            self.stats.slowest_install_ms = self
                .stats
                .slowest_install_ms
                .max(p.since.elapsed().as_secs_f32() * 1e3);
            self.note_latency(p.urgent, p.first, p.known, frame);
            let key = p.stamp.section;
            if let Some(old) = self.tracker.install(p.stamp, Some(mesh)) {
                // Frames up to the one before this may have drawn it.
                retire(&mut self.pool, &old, Users::until_frame(frame));
            }
            self.tracker.connect(key, p.connectivity, true);
        }
        // New orders whose copies this frame may read.
        let mut i = 0;
        while i < self.sorted.len() {
            let Some((handle, ticket)) = self.sorted[i].placed.filter(|p| uploads.is_ready(p.1))
            else {
                i += 1;
                continue;
            };
            let s = self.sorted.swap_remove(i).sorted;
            let range = self.pool.resolve(handle).expect("a live range");
            let Some(t) = Self::sorting_mesh(&mut self.tracker, &s) else {
                self.pool.retire(handle, uploads.users_of(ticket));
                continue;
            };
            let wide = t.indices.wide;
            t.indices = IndexRange {
                buffer: range.buffer,
                first: (range.offset / if wide { 4 } else { 2 }) as u32,
                wide,
            };
            t.sorted_at = s.sorted_at;
            t.sorted_from = s.sorted_from;
            t.sorting = false;
            let old = t.own.replace(handle);
            if let Some(old) = old {
                self.pool.retire(old, Users::until_frame(frame));
            }
            self.stats.sorts += 1;
        }
    }

    /// Whether the animated textures play (they do unless said otherwise); stopped, they
    /// show what they last showed.
    pub fn set_animate(&mut self, on: bool) {
        self.animate = on;
    }

    /// Plays the animated textures on and stages what changed of the atlas, before the
    /// frame's `UploadRing::record`. They move on once in a frame in which the game has
    /// ticked, however many ticks it caught up on, and not while it is frozen
    /// (`Minecraft.runTick`).
    pub fn animate(
        &mut self,
        frame: &Frame,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
    ) {
        if !textures.is_ready(self.atlas) || self.animations.is_empty() {
            return;
        }
        profiling::scope!("animate_textures");
        let started = Instant::now();
        let now = Some((frame.epoch, frame.ticks));
        if now != self.animated_at {
            self.animated_at = now;
            if self.animate && !frame.frozen {
                self.animations.tick();
                self.animated_stale = self.animations.stale_count();
                self.stats.animation_ticks += 1;
            }
        }
        let since = self
            .animated_frame
            .replace(started)
            .map_or(ANIMATION_SPREAD, |last| {
                started.saturating_duration_since(last)
            });
        let mut left = animation_share(self.animated_stale, since);
        let mut from = 0;
        let mut worked = false;
        while left > 0
            && let Some(first) = self.animations.next_stale(from)
        {
            worked = true;
            // A piece: some sprites, with all their levels.
            self.animated.clear();
            self.animated_rects.clear();
            self.animated_texels.clear();
            let mut next = Some(first);
            // Keep each update batch comfortably below `MAX_IMAGE_COPIES` (1024) so many
            // tiny animated sprites don't permanently fail with `UploadError::TooLarge`.
            const MAX_BATCH_RECTS: usize = 512;
            while let Some(sprite) = next {
                self.animated.push(sprite);
                self.animations.compose(
                    sprite,
                    &mut self.animated_rects,
                    &mut self.animated_texels,
                );
                from = sprite + 1;
                left -= 1;
                next = (left > 0
                    && self.animated_texels.len() < ANIMATION_PIECE
                    && self.animated_rects.len() < MAX_BATCH_RECTS)
                    .then(|| self.animations.next_stale(from))
                    .flatten();
            }
            let mut rest = &self.animated_texels[..];
            let regions: Vec<(ImageRegion, &[u8])> = self
                .animated_rects
                .iter()
                .map(|r| {
                    let (texels, after) = rest.split_at((r.width * r.height * 4) as usize);
                    rest = after;
                    let region = ImageRegion {
                        mip: r.level,
                        x: r.x,
                        y: r.y,
                        width: r.width,
                        height: r.height,
                    };
                    (region, texels)
                })
                .collect();
            if let Err(e) = textures.update(uploads, self.atlas, &regions) {
                if !e.try_again() {
                    // Permanent failure (e.g. sprite too large for the upload ring):
                    // disable it so it doesn't stall every subsequent frame.
                    for &sprite in &self.animated {
                        self.animations.disable(sprite);
                    }
                    continue;
                }
                // No room: the next frame writes what they should show by then.
                self.stats.animation_waits += 1;
                break;
            }
            for &sprite in &self.animated {
                self.animations.written(sprite);
            }
            self.stats.animation_writes += self.animated.len() as u64;
            self.stats.animation_bytes += self.animated_texels.len() as u64;
        }
        if worked {
            let micros = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
            self.stats.slowest_animation_micros = self.stats.slowest_animation_micros.max(micros);
        }
    }

    /// What a camera is in, whose field of view is the option's; `aspect` is the
    /// target's width over its height.
    pub fn medium_at(&self, scene: &Scene, camera: &CameraPose, aspect: f32) -> Medium {
        scene.with_terrain(|t| medium::medium_at(t, &self.table, camera, aspect))
    }

    /// Gathers the block entities of the sections `view` can see into, for this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn gather_block_entities(
        &mut self,
        renderer: &mut BlockEntityRenderer,
        mut marks: Option<&mut crate::block_marks::BlockMarks>,
        scene: &Scene,
        frame: &Frame,
        partial: f32,
        camera: [f64; 3],
        view: &View,
        textures: &TextureTable,
    ) {
        profiling::scope!("block_entities_gather");
        let tracker = &self.tracker;
        self.block_entities.retain(|key, _| tracker.tracks(*key));
        if self.block_entities.is_empty() {
            return;
        }
        let now = Instant::now();
        scene.with_terrain(|terrain| {
            for (&key, list) in &self.block_entities {
                let low: [f32; 3] = std::array::from_fn(|i| ((key[i] << 4) - view.block[i]) as f32);
                if !view.sees(low, low.map(|c| c + 16.0)) {
                    continue;
                }
                let Some(since) = self.tracker.shown_since(key) else {
                    continue;
                };
                if visibility(now, since, self.fade_ms) < MIN_VISIBILITY {
                    continue;
                }
                renderer.gather(
                    key,
                    list,
                    terrain,
                    &self.table,
                    frame,
                    partial,
                    camera,
                    view,
                    textures,
                    marks.as_deref_mut(),
                );
            }
        });
    }

    /// Gathers the players in view, for this frame.
    pub fn gather_players(
        &self,
        renderer: &mut BlockEntityRenderer,
        scene: &Scene,
        frame: &Frame,
        seen: &PlayerView<'_>,
        textures: &TextureTable,
    ) {
        profiling::scope!("players_gather");
        if frame.entities.iter().all(|e| e.avatar.is_none()) {
            return;
        }
        let now = Instant::now();
        scene.with_terrain(|terrain| {
            let dims = terrain.dims();
            renderer.gather_players(
                frame,
                terrain,
                &self.table,
                seen,
                textures,
                self.atlas,
                |block| {
                    let key = block.map(|c| c >> 4);
                    // Above and below the level there are no sections.
                    if !(0..dims.sections).contains(&(key[1] - dims.min_section)) {
                        return true;
                    }
                    self.tracker
                        .meshed_since(key)
                        .is_some_and(|s| visibility(now, s, self.fade_ms) >= MIN_VISIBILITY)
                },
            );
        });
    }

    /// Gathers the player's own hands, for this frame.
    /// The block atlas, which items that are blocks are drawn from.
    pub fn block_atlas(&self) -> TextureId {
        self.atlas
    }

    pub fn gather_hand(
        &self,
        renderer: &mut BlockEntityRenderer,
        scene: &Scene,
        frame: &Frame,
        seen: &PlayerView<'_>,
        textures: &TextureTable,
        options: crate::block_entity::renderer::ViewOptions,
    ) {
        profiling::scope!("hand_gather");
        scene.with_terrain(|terrain| {
            renderer.gather_hand(frame, terrain, seen, textures, self.atlas, options);
        });
    }

    /// Gathers the outline of the block the crosshair is on and the cracks in the blocks
    /// being broken, for this frame.
    pub fn gather_marks(
        &self,
        marks: &mut crate::block_marks::BlockMarks,
        scene: &Scene,
        frame: &Frame,
        camera: [f64; 3],
        view: &View,
        target_width: f32,
    ) {
        profiling::scope!("marks_gather");
        if frame.player.target.is_none() && frame.breaking.is_empty() {
            marks.gather_nothing();
            return;
        }
        scene.with_terrain(|terrain| {
            marks.gather(frame, terrain, &self.table, camera, view, target_width);
        });
    }

    /// The light map and the shared quad indices (their buffer and where in it), which can
    /// be read once [`Self::ready`].
    pub fn shared(&self) -> Option<(TextureId, (vk::Buffer, u64))> {
        let range = self.pool.resolve(self.indices)?;
        Some((self.lightmap?, (range.buffer, range.offset)))
    }

    /// Whether the textures and the index buffer the draws read have arrived.
    pub fn ready(&self, textures: &TextureTable) -> bool {
        self.indices_ready
            && textures.is_ready(self.atlas)
            && self.lightmap.is_some_and(|l| textures.is_ready(l))
    }

    /// How far the atlas has got on its way to the GPU, 0 to 1.
    pub fn atlas_progress(&self, textures: &TextureTable) -> f32 {
        if textures.is_ready(self.atlas) {
            return 1.0;
        }
        textures.stream_progress(self.atlas).map_or(0.0, |p| {
            if p.total == 0 {
                0.0
            } else {
                p.completed as f32 / p.total as f32
            }
        })
    }

    /// Draws the sections `view` can see into `pass`, which has a depth attachment:
    /// the solid layer nearest first, then the cutout layer, in `fog`. Their translucent
    /// layer is [`Self::draw_translucent`]'s, after whatever else is opaque.
    ///
    /// # Safety
    /// [`Self::ready`] holds for the frame being recorded, whose `UploadRing::record`
    /// and [`Self::install_ready`] have run; `pass` is of that frame.
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
    ) {
        profiling::scope!("world_draw");
        self.visible.clear();
        self.unsorted.clear();
        self.drawn = None;
        let mut tested = 0;
        let now = Instant::now();
        let fade_ms = self.fade_ms;
        let camera = self.camera;
        for (key, mesh, since) in self.tracker.meshes_since() {
            tested += 1;
            let origin = [
                (key[0] << 4) - view.block[0],
                (key[1] << 4) - view.block[1],
                (key[2] << 4) - view.block[2],
            ];
            let corner = |c: [f32; 3]| std::array::from_fn(|i| origin[i] as f32 + c[i]);
            if !view.sees(corner(mesh.bounds[0]), corner(mesh.bounds[1])) {
                continue;
            }
            // From the camera to the section's middle, in blocks.
            let distance = origin
                .iter()
                .map(|c| i64::from(c + 8) * i64::from(c + 8))
                .sum();
            // The camera from the section's corner, in blocks.
            let eye = std::array::from_fn(|i| view.eye[i] - origin[i] as f32);
            let shown = if self.face_culling {
                facing::shown(&mesh.facings.planes, eye)
            } else {
                0b11_1111
            };
            if let Some(t) = &mesh.translucent
                && self.translucent
                && !t.sorting
                && sort::needs_sort(camera, key, t.sorted_at, t.sorted_from)
            {
                self.unsorted.push((distance, key));
            }
            self.visible.push(Visible {
                distance,
                origin,
                visibility: visibility(now, since, fade_ms),
                buffer: mesh.buffer,
                page: mesh.page,
                first_vertex: mesh.first_vertex,
                solid_quads: mesh.solid_quads,
                cutout_quads: mesh.cutout_quads,
                solid: mesh.facings.solid,
                cutout: mesh.facings.cutout,
                shown,
                translucent: mesh.translucent.as_ref().map(|t| (t.quads, t.indices)),
            });
        }
        // The sections in view whose translucent quads are in an order made for
        // another place: the nearest are sorted again first.
        if !self.unsorted.is_empty() {
            self.unsorted.sort_unstable();
            for i in 0..self.unsorted.len() {
                if self.sorts_out + self.sorted.len() >= MAX_SORTS {
                    break;
                }
                let key = self.unsorted[i].1;
                let Some(mesh) = self.tracker.mesh_mut(key) else {
                    continue;
                };
                let handle = mesh.handle;
                let Some(t) = &mut mesh.translucent else {
                    continue;
                };
                t.sorting = true;
                self.sorts_out += 1;
                self.workers.push(Work::Sort {
                    session: self.session,
                    key,
                    mesh: handle,
                    centres: t.centres.clone(),
                    camera,
                });
            }
        }
        self.stats.tested = tested;
        self.stats.translucent_drawn = 0;
        self.stats.translucent_sections = 0;
        self.stats.visible = 0;
        self.stats.quads = 0;
        self.stats.draws = 0;
        self.stats.calls = 0;
        self.stats.binds = 0;
        match self.draws {
            // Nearest first: what is behind fails the depth test before it is shaded.
            TerrainDraws::Direct => self
                .visible
                .sort_unstable_by_key(|v| (v.distance, v.page, v.first_vertex)),
            // A page at a time, the page with the nearest section first, and nearest
            // first within a page.
            TerrainDraws::Paged | TerrainDraws::Indirect => {
                let nearest = &mut self.page_order;
                nearest.clear();
                for v in &self.visible {
                    let page = v.page as usize;
                    if nearest.len() <= page {
                        nearest.resize(page + 1, i64::MAX);
                    }
                    nearest[page] = nearest[page].min(v.distance);
                }
                self.visible.sort_unstable_by_key(|v| {
                    (nearest[v.page as usize], v.page, v.distance, v.first_vertex)
                });
            }
        }
        let (Some(lightmap), Some(indices)) = (self.lightmap, self.pool.resolve(self.indices))
        else {
            return;
        };
        let Some((world, matrix)) = write_world(frame_data, view, self.atlas_size, lightmap) else {
            self.stats.overflowed += self.visible.len() as u64;
            return;
        };
        // As many section records as the frame's data has room for.
        let room = (frame_data.capacity() - frame_data.used()) / u64::from(SECTION.bytes());
        let count = self.visible.len().min(room.saturating_sub(1) as usize);
        self.stats.overflowed += (self.visible.len() - count) as u64;
        if count == 0 {
            return;
        }
        let Some((first, bytes)) = frame_data.alloc(count as u32 * SECTION.words(), 4) else {
            self.stats.overflowed += count as u64;
            return;
        };
        for (v, record) in self.visible[..count]
            .iter()
            .zip(bytes.as_chunks_mut::<SECTION_BYTES>().0)
        {
            write_section(record, v.origin, v.visibility);
        }
        let push = terrain_push(world, matrix, first, fog, self.atlas, self.atlas_size);
        let [solid, with_holes, _] = self.shader.pipelines();
        self.stats.visible = count as u32;
        self.drawn = Some((push, count));
        let indirect = self.draws == TerrainDraws::Indirect;
        // SAFETY: the push constants are terrain.vert's and the records they point at
        // were written just above; the atlas and the light map are textures of the
        // table that have arrived (the caller's guarantee), as have the indices; every
        // mesh drawn was installed only once its copy could be read, and draws no more
        // vertices than were copied; the draw commands are written into this frame's
        // data before the calls that read them.
        unsafe {
            pass.bind_index_buffer(indices.buffer, indices.offset);
            for (pipeline, cutout) in [(solid, false), (with_holes, true)] {
                let mut bound = None;
                let mut pushed = false;
                self.commands.clear();
                self.calls.clear();
                for (instance, v) in self.visible[..count].iter().enumerate() {
                    let (groups, layer_first) = if cutout {
                        (&v.cutout, v.first_vertex + v.solid_quads * 4)
                    } else {
                        (&v.solid, v.first_vertex)
                    };
                    for (start, quads) in facing::runs(groups, v.shown) {
                        let mut quads = quads;
                        let mut first_vertex = layer_first + start * 4;
                        if bound != Some(v.page) {
                            bound = Some(v.page);
                            self.stats.binds += 1;
                            if indirect {
                                let next = self.commands.len() as u32 / DRAW_COMMAND_BYTES;
                                self.calls.push((v.buffer, next, 0));
                            } else {
                                pass.bind_vertex_buffer(v.buffer, 0);
                            }
                        }
                        self.stats.quads += u64::from(quads);
                        // More quads than 16-bit indices reach: in several draws.
                        while quads > 0 {
                            let n = quads.min(QUADS_PER_DRAW);
                            if indirect {
                                self.commands.extend_from_slice(&draw_command(
                                    n * 6,
                                    0,
                                    first_vertex as i32,
                                    instance as u32,
                                ));
                                if let Some(call) = self.calls.last_mut() {
                                    call.2 += 1;
                                }
                            } else {
                                pass.draw_indexed(
                                    pipeline.index(),
                                    if pushed { &[] } else { &push },
                                    n * 6,
                                    0,
                                    first_vertex as i32,
                                    instance as u32,
                                );
                                pushed = true;
                                self.stats.calls += 1;
                            }
                            self.stats.draws += 1;
                            quads -= n;
                            first_vertex += n * 4;
                        }
                    }
                }
                if !indirect || self.commands.is_empty() {
                    continue;
                }
                let commands = self.commands.len() as u32 / DRAW_COMMAND_BYTES;
                let Some((offset, bytes)) = frame_data.alloc_draw_commands(commands) else {
                    // No room for the layer's commands: it is not drawn this frame.
                    self.stats.overflowed += u64::from(commands);
                    self.stats.draws -= commands;
                    continue;
                };
                bytes.copy_from_slice(&self.commands);
                let buffer = frame_data.buffer();
                for &(vertices, first_command, draws) in &self.calls {
                    pass.bind_vertex_buffer(vertices, 0);
                    let mut done = 0;
                    while done < draws {
                        let n = (draws - done).min(MAX_INDIRECT_DRAWS);
                        pass.draw_indexed_indirect(
                            pipeline.index(),
                            if pushed { &[] } else { &push },
                            buffer,
                            offset
                                + u64::from(first_command + done) * u64::from(DRAW_COMMAND_BYTES),
                            n,
                        );
                        pushed = true;
                        self.stats.calls += 1;
                        done += n;
                    }
                }
            }
        }
    }
}

impl WorldRenderer {
    /// Draws the translucent layer of the sections [`Self::draw`] drew, over what is in
    /// `pass` by now: the farthest section first, each section's quads in the order
    /// they were last sorted in. A fragment less than a tenth opaque is left out; the
    /// others are blended over what is behind them, and write their depth.
    ///
    /// # Safety
    /// As [`Self::draw`], which has run for this frame and this `pass`.
    pub unsafe fn draw_translucent(&mut self, pass: &mut Pass<'_>, frame_data: &mut FrameData) {
        profiling::scope!("world_draw_translucent");
        let Some((push, count)) = self.drawn.take() else {
            return;
        };
        if !self.translucent {
            return;
        }
        let visible = &self.visible[..count];
        self.translucent_order.clear();
        self.translucent_order
            .extend((0..count as u32).filter(|&i| visible[i as usize].translucent.is_some()));
        if self.translucent_order.is_empty() {
            return;
        }
        // Farthest first. Two sections as far are told apart by where they are, not by
        // where their meshes happen to be in memory: the same picture every run.
        self.translucent_order.sort_unstable_by_key(|&i| {
            let v = &visible[i as usize];
            (std::cmp::Reverse(v.distance), v.origin)
        });
        let pipeline = self.shader.pipelines()[2];
        let indirect = self.draws == TerrainDraws::Indirect;
        self.commands.clear();
        self.translucent_calls.clear();
        self.stats.translucent_sections = self.translucent_order.len() as u32;
        let mut bound = None;
        let mut pushed = false;
        // SAFETY: as in `draw`: the records and push constants are this frame's; every
        // mesh's indices were copied with its vertices, or into a range of their own
        // whose copy this frame may read (`install_ready`), and name only the mesh's
        // translucent vertices.
        unsafe {
            for &i in &self.translucent_order {
                let v = &visible[i as usize];
                let Some((quads, indices)) = v.translucent else {
                    continue;
                };
                let first_vertex = v.first_vertex + (v.solid_quads + v.cutout_quads) * 4;
                let buffers = (v.buffer, indices.buffer, indices.wide);
                if bound != Some(buffers) {
                    bound = Some(buffers);
                    self.stats.binds += 1;
                    if indirect {
                        let next = self.commands.len() as u32 / DRAW_COMMAND_BYTES;
                        self.translucent_calls.push((v.buffer, indices, next, 0));
                    } else {
                        pass.bind_vertex_buffer(v.buffer, 0);
                        pass.bind_index_buffer_of(indices.buffer, 0, indices.wide);
                    }
                }
                self.stats.translucent_drawn += u64::from(quads);
                self.stats.quads += u64::from(quads);
                self.stats.draws += 1;
                if indirect {
                    self.commands.extend_from_slice(&draw_command(
                        quads * 6,
                        indices.first,
                        first_vertex as i32,
                        i,
                    ));
                    if let Some(call) = self.translucent_calls.last_mut() {
                        call.3 += 1;
                    }
                } else {
                    pass.draw_indexed(
                        pipeline.index(),
                        if pushed { &[] } else { &push },
                        quads * 6,
                        indices.first,
                        first_vertex as i32,
                        i,
                    );
                    pushed = true;
                    self.stats.calls += 1;
                }
            }
            if !indirect {
                return;
            }
            let commands = self.commands.len() as u32 / DRAW_COMMAND_BYTES;
            let Some((offset, bytes)) = frame_data.alloc_draw_commands(commands) else {
                // No room for the layer's commands: it is not drawn this frame.
                self.stats.overflowed += u64::from(commands);
                self.stats.draws -= commands;
                return;
            };
            bytes.copy_from_slice(&self.commands);
            let buffer = frame_data.buffer();
            for &(vertices, indices, first_command, draws) in &self.translucent_calls {
                pass.bind_vertex_buffer(vertices, 0);
                pass.bind_index_buffer_of(indices.buffer, 0, indices.wide);
                let mut done = 0;
                while done < draws {
                    let n = (draws - done).min(MAX_INDIRECT_DRAWS);
                    pass.draw_indexed_indirect(
                        pipeline.index(),
                        if pushed { &[] } else { &push },
                        buffer,
                        offset + u64::from(first_command + done) * u64::from(DRAW_COMMAND_BYTES),
                        n,
                    );
                    pushed = true;
                    self.stats.calls += 1;
                    done += n;
                }
            }
        }
    }
}

impl Drop for WorldRenderer {
    fn drop(&mut self) {
        self.workers.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_frames_share_a_ticks_animations() {
        let share = |ms: f64| animation_share(40, Duration::from_secs_f64(ms / 1000.0));
        assert_eq!(share(1.0), 5);
        assert_eq!(share(0.4), 2);
        assert_eq!(share(0.0001), 1);
        // A frame at 60 FPS, or a stall, writes them all.
        assert_eq!(share(16.7), 40);
        assert_eq!(share(8.0), 40);
        assert_eq!(share(500.0), 40);
        assert_eq!(animation_share(0, Duration::from_millis(1)), 0);
    }

    #[test]
    fn quads_are_two_triangles_over_the_first_diagonal() {
        let bytes = quad_indices();
        assert_eq!(bytes.len() as u64, INDEX_BYTES);
        let index = |i: usize| u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
        assert_eq!(
            (0..12).map(index).collect::<Vec<_>>(),
            [0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]
        );
        // The last quad's last corner is the last vertex 16 bits can name.
        assert_eq!(index(QUADS_PER_DRAW as usize * 6 - 2), u16::MAX);
    }

    #[test]
    fn records_are_written_where_the_shaders_read_them() {
        let mut record = [0u8; 16];
        write_section(&mut record, [-3, 64, 1_000_000], 0.25);
        let word = |i: usize| u32::from_ne_bytes(record[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!(word(0) as i32, -3);
        assert_eq!(word(1), 64);
        assert_eq!(word(2), 1_000_000);
        assert_eq!(f32::from_bits(word(3)), 0.25);
        // The push constants: four words for the vertex stage, then the fog's colour
        // and lines, a texel of the atlas, its size, the atlas and the texel's scale,
        // where terrain_frag.glsl's block has them.
        let fog = Fog {
            colour: [0.1, 0.2, 0.3, 1.0],
            environmental: [4.0, 5.0],
            render_distance: [6.0, 7.0],
            sky_end: 8.0,
        };
        let push = terrain_push(7, 14, 30, &fog, TextureId::MISSING, [1024.0, 512.0]);
        assert_eq!(push.len(), 76);
        assert_eq!(push[0..4], 7u32.to_ne_bytes());
        assert_eq!(push[4..8], 14u32.to_ne_bytes());
        assert_eq!(push[8..12], 30u32.to_ne_bytes());
        assert_eq!(push[12..16], 1u32.to_ne_bytes());
        let pushed = |at: usize| f32::from_ne_bytes(push[at..at + 4].try_into().unwrap());
        assert_eq!([pushed(16), pushed(20), pushed(24), pushed(28)], fog.colour);
        // Fog from 4 to 5 blocks, and from 6 to 7: a block wide each.
        assert_eq!([pushed(32), pushed(36)], [1.0, -4.0]);
        assert_eq!([pushed(40), pushed(44)], [1.0, -6.0]);
        assert_eq!([pushed(48), pushed(52)], [1.0 / 1024.0, 1.0 / 512.0]);
        assert_eq!([pushed(56), pushed(60)], [1024.0, 512.0]);
        assert_eq!(push[64..68], TextureId::MISSING.index().to_ne_bytes());
        assert_eq!([pushed(68), pushed(72)], [1024.0 * 1024.0, 10.0]);
        // The shorter side of a texel is the one of the longer side of the atlas.
        let tall = terrain_push(7, 14, 30, &fog, TextureId::MISSING, [256.0, 2048.0]);
        let pushed = |at: usize| f32::from_ne_bytes(tall[at..at + 4].try_into().unwrap());
        assert_eq!([pushed(68), pushed(72)], [2048.0 * 2048.0, 11.0]);
        // The world record: the camera's offset, the atlas's size and the light map.
        let mut world = [0u8; WORLD.bytes() as usize];
        write_world_record(
            &mut world,
            [0.5, 0.25, 0.125],
            [1024.0, 512.0],
            TextureId::MISSING,
        );
        let float = |i: usize| f32::from_ne_bytes(world[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!([float(0), float(1), float(2)], [0.5, 0.25, 0.125]);
        assert_eq!([float(3), float(4)], [1024.0, 512.0]);
        assert_eq!(world[20..24], TextureId::MISSING.index().to_ne_bytes());
    }

    #[test]
    fn a_section_fades_in_from_its_first_mesh() {
        let since = Instant::now();
        let after = |ms: u64| since + std::time::Duration::from_millis(ms);
        assert_eq!(visibility(since, since, 750), 0.0);
        assert_eq!(visibility(after(375), since, 750), 0.5);
        assert_eq!(visibility(after(749), since, 750), 749.0 / 750.0);
        assert_eq!(visibility(after(750), since, 750), 1.0);
        assert_eq!(visibility(after(60_000), since, 750), 1.0);
        // Whole milliseconds, as the game counts them.
        assert_eq!(
            visibility(since + std::time::Duration::from_micros(1_900), since, 750),
            1.0 / 750.0
        );
        // No fade: there at once.
        assert_eq!(visibility(since, since, 0), 1.0);
        // A clock read before the mesh was noted is no time at all.
        assert_eq!(visibility(since, after(5), 750), 0.0);
    }

    #[test]
    fn every_shader_has_a_name_of_its_own_and_two_pipelines() {
        for s in TerrainShader::ALL {
            assert_eq!(TerrainShader::parse(s.name()), Some(s));
            let [solid, cutout, translucent] = s.pipelines();
            assert_ne!(solid, cutout);
            assert_ne!(cutout, translucent);
            assert!(
                [solid, cutout, translucent]
                    .iter()
                    .all(|p| p.terrain_stages().is_some())
            );
        }
        assert_eq!(TerrainShader::parse("fancy"), None);
        assert_eq!(TerrainShader::default(), TerrainShader::Rgss);
        // No two draw with the same pipelines, and each with those of its name.
        let names: Vec<[&str; 3]> = TerrainShader::ALL
            .iter()
            .map(|s| s.pipelines().map(|p| p.terrain_stages().unwrap().0))
            .collect();
        assert_eq!(
            names,
            [
                [
                    "terrain_rgss",
                    "terrain_rgss_cutout",
                    "terrain_rgss_translucent"
                ],
                [
                    "terrain_rgss_reference",
                    "terrain_rgss_reference_cutout",
                    "terrain_rgss_reference_translucent"
                ],
                [
                    "terrain_nearest",
                    "terrain_nearest_cutout",
                    "terrain_nearest_translucent"
                ],
                [
                    "terrain_plain",
                    "terrain_plain_cutout",
                    "terrain_plain_translucent"
                ],
            ]
        );
    }
}
