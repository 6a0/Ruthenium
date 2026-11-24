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
//! Every stage is bounded: jobs out at once, results waiting for staging, slices
//! reserved, and the pool's budget.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_gpu::{
    BufferPool, Error, FrameData, Gpu, Pass, PoolConfig, PoolError, PoolStats, RangeHandle,
    Reservation, ReservationState, StagingWriter, TextureDesc, TextureId, TextureTable,
    UploadError, UploadRing, UploadTicket, Urgency, Users, vk,
};
use ruthenium_shaders::interface::{SECTION, WORLD, WORLD_MATRIX};
use ruthenium_sim::scene::{Scene, SectionKey, Stamp};

use super::camera::View;
use super::lightmap::{self, LightmapInputs};
use super::mesher::{MeshOptions, Mesher, SectionMesh};
use super::table::BlockTable;
use super::tracker::{Job, Tracker, TrackerStats};
use crate::pipelines::PipelineId;
use crate::terrain_vertex::VERTEX_BYTES;

/// Quads one indexed draw can address with 16-bit indices (four vertices each).
pub(crate) const QUADS_PER_DRAW: u32 = 1 << 14;
/// Bytes of the shared quad index buffer: six indices for each of those quads.
const INDEX_BYTES: u64 = QUADS_PER_DRAW as u64 * 6 * 2;

/// Buffer pages for vertices: their size.
const PAGE_BYTES: u64 = 4 << 20;
/// What the terrain's buffers are planned to take at a 12-chunk view distance. Every
/// loaded section is meshed for now, also those underground that nothing can be seen
/// of, and in a jungle that takes more: passing the envelope is said once and shown in
/// the statistics, and meshes are only left out at twice it.
pub const POOL_ENVELOPE: u64 = 108 << 20;
const POOL_BUDGET: u64 = 2 * POOL_ENVELOPE;
/// The second staging buffer, which workers write meshes into.
const WORKER_STAGING_BYTES: u64 = 8 << 20;
/// Results that may wait for staging, a place in a page or their copy at once; no new
/// job is handed out beyond it.
const MAX_PENDING: usize = 48;
/// Jobs out per worker.
const JOBS_PER_WORKER: usize = 2;

/// What the client prepares when a world is entered: every block state's model and
/// rules, and the block atlas's levels as RGBA8 (level 0 first).
#[derive(Debug)]
pub struct WorldAssets {
    pub table: Arc<BlockTable>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub atlas_levels: Vec<Vec<u8>>,
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
#[derive(Debug, Clone, Copy)]
struct MeshRange {
    handle: RangeHandle,
    buffer: vk::Buffer,
    page: u32,
    /// The index of its first vertex in the page.
    first_vertex: u32,
    solid_quads: u32,
    cutout_quads: u32,
    /// In blocks from the section's corner.
    bounds: [[f32; 3]; 2],
}

enum Work {
    Mesh {
        job: Box<Job>,
        options: MeshOptions,
        /// Which run of sessions the job is of ([`WorldRenderer::clear`]).
        session: u64,
    },
    /// Copy a result into its slice of staging.
    Fill {
        writer: StagingWriter,
        bytes: Vec<u8>,
    },
}

struct Meshed {
    session: u64,
    stamp: Stamp,
    mesh: SectionMesh,
    urgent: bool,
    micros: u64,
}

#[derive(Default)]
struct WorkQueue {
    work: VecDeque<Work>,
    stop: bool,
}

struct Workers {
    shared: Arc<(Mutex<WorkQueue>, Condvar)>,
    results: mpsc::Receiver<Meshed>,
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
            // Copies first: they free the memory of finished results.
            Work::Fill { .. } => q.work.push_front(work),
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
    results: &mpsc::Sender<Meshed>,
    table: Arc<BlockTable>,
) {
    let mut mesher = Mesher::new(table);
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
                session,
            } => {
                profiling::scope!("mesh_section");
                let started = Instant::now();
                let mesh = mesher.mesh(&job.neighbourhood, &options);
                let done = Meshed {
                    session,
                    stamp: job.neighbourhood.stamp().clone(),
                    mesh,
                    urgent: job.urgent,
                    micros: started.elapsed().as_micros() as u64,
                };
                if results.send(done).is_err() {
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
    bytes: u64,
    solid_quads: u32,
    cutout_quads: u32,
    bounds: [[f32; 3]; 2],
    since: Instant,
    stage: Stage,
}

/// A section the frame draws.
#[derive(Clone, Copy)]
struct Visible {
    distance: i64,
    mesh: MeshRange,
    origin: [i32; 3],
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
    /// Translucent quads left out so far (in meshes installed).
    pub translucent_quads: u64,
    /// Corners and texture coordinates moved into a vertex's range.
    pub out_of_range: u64,
    /// Meshes that found no room in the pool's budget, or were larger than a page.
    pub dropped: u64,
    /// The last frame: sections tested against the view, sections drawn, their quads,
    /// the draws issued and the vertex buffers bound.
    pub tested: u32,
    pub visible: u32,
    pub quads: u64,
    pub draws: u32,
    pub binds: u32,
    /// Sections a frame could not draw because its data had no room for their records.
    pub overflowed: u64,
    pub pool: PoolStats,
}

/// See the module docs.
pub(crate) struct WorldRenderer {
    atlas: TextureId,
    atlas_size: [f32; 2],
    lightmap: Option<TextureId>,
    lightmap_inputs: LightmapInputs,
    pool: BufferPool,
    /// The quad index buffer's range, and its upload once it is staged.
    indices: RangeHandle,
    indices_upload: Option<UploadTicket>,
    indices_ready: bool,
    tracker: Tracker<MeshRange>,
    /// Counts the times everything was forgotten: a result of an earlier count is of
    /// a session that is over, whose stamps mean nothing in this one.
    session: u64,
    workers: Workers,
    jobs_out: usize,
    pending: Vec<Pending>,
    removed: Vec<MeshRange>,
    visible: Vec<Visible>,
    options: MeshOptions,
    stats: WorldStats,
    warned_dropped: bool,
    warned_envelope: bool,
}

impl std::fmt::Debug for WorldRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldRenderer")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

const WORLD_CAMERA_OFFSET: usize = WORLD.offset("camera_offset") as usize * 4;
const WORLD_ATLAS: usize = WORLD.offset("atlas") as usize * 4;
const WORLD_ATLAS_SIZE: usize = WORLD.offset("atlas_size") as usize * 4;
const WORLD_LIGHTMAP: usize = WORLD.offset("lightmap") as usize * 4;
const SECTION_ORIGIN: usize = SECTION.offset("origin") as usize * 4;
const SECTION_VISIBILITY: usize = SECTION.offset("visibility") as usize * 4;
/// Bytes of one section record.
pub(crate) const SECTION_BYTES: usize = SECTION.bytes() as usize;

fn put(bytes: &mut [u8], at: usize, word: [u8; 4]) {
    bytes[at..at + 4].copy_from_slice(&word);
}

/// `terrain.vert`'s push constants: the word offsets of the frame's world record, its
/// matrix and its first section record, and that the draw is a real one.
pub(crate) fn terrain_push(world: u32, matrix: u32, sections: u32) -> [u8; 16] {
    let mut push = [0u8; 16];
    put(&mut push, 0, world.to_ne_bytes());
    put(&mut push, 4, matrix.to_ne_bytes());
    put(&mut push, 8, sections.to_ne_bytes());
    put(&mut push, 12, 1u32.to_ne_bytes());
    push
}

/// Writes a frame's world record and its matrix into the frame's data: their word
/// offsets, or `None` if it has no room.
pub(crate) fn write_world(
    frame_data: &mut FrameData,
    view: &View,
    atlas: TextureId,
    atlas_size: [f32; 2],
    lightmap: TextureId,
) -> Option<(u32, u32)> {
    let (world, bytes) = frame_data.alloc(WORLD.words(), 4)?;
    for (i, c) in view.offset.iter().enumerate() {
        put(bytes, WORLD_CAMERA_OFFSET + i * 4, c.to_ne_bytes());
    }
    put(bytes, WORLD_ATLAS, atlas.index().to_ne_bytes());
    put(bytes, WORLD_ATLAS_SIZE, atlas_size[0].to_ne_bytes());
    put(bytes, WORLD_ATLAS_SIZE + 4, atlas_size[1].to_ne_bytes());
    put(bytes, WORLD_LIGHTMAP, lightmap.index().to_ne_bytes());
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
    /// meshing threads. Not on a frame's path: behind the screen that says the world is
    /// loading.
    pub fn new(
        gpu: &Arc<Gpu>,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        assets: WorldAssets,
        workers: usize,
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
            lightmap: None,
            lightmap_inputs: LightmapInputs::default(),
            pool,
            indices,
            indices_upload: None,
            indices_ready: false,
            tracker: Tracker::new(),
            session: 0,
            workers: Workers::start(&assets.table, workers),
            jobs_out: 0,
            pending: Vec::new(),
            removed: Vec::new(),
            visible: Vec::new(),
            options: MeshOptions {
                shades: [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
                cutout_leaves: true,
            },
            stats: WorldStats::default(),
            warned_dropped: false,
            warned_envelope: false,
        })
    }

    /// Ends the worker threads. They write into the upload ring's staging memory, so
    /// this comes before the ring goes.
    pub fn stop_workers(&mut self) {
        self.workers.stop();
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
            pool: self.pool.stats(),
            ..self.stats
        }
    }

    /// Forgets every section (the session ended, or another began): their meshes are
    /// retired after `frame`, and results still on their way are thrown away.
    pub fn clear(&mut self, uploads: &mut UploadRing, frame: u64) {
        self.tracker.clear(&mut self.removed);
        for m in self.removed.drain(..) {
            self.pool.retire(m.handle, Users::until_frame(frame));
        }
        for p in self.pending.drain(..) {
            match p.stage {
                Stage::Waiting(_) => {}
                Stage::Writing(r) | Stage::Placing(r) => uploads.release(r),
                Stage::Copying(handle, ticket) => {
                    self.pool.retire(handle, uploads.users_of(ticket));
                }
            }
        }
        self.tracker = Tracker::new();
        self.session += 1;
    }

    /// Sets what meshes are shaded by (the dimension's cardinal light). A change makes
    /// every mesh out of date; a dimension change, which is when it happens, empties
    /// them anyway.
    pub fn set_shades(&mut self, shades: [f32; 6]) {
        self.options.shades = shades;
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
    /// `frame` is the frame being recorded, `camera` the section the camera is in.
    pub fn update(
        &mut self,
        scene: &Scene,
        camera: SectionKey,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        lightmap: LightmapInputs,
        frame: u64,
    ) {
        profiling::scope!("world_update");
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
            self.pool.retire(m.handle, Users::until_frame(frame));
        }

        while let Ok(done) = self.workers.results.try_recv() {
            self.jobs_out -= 1;
            if done.session != self.session {
                continue;
            }
            self.stats.mesh_micros += done.micros;
            self.stats.slowest_mesh_micros = self.stats.slowest_mesh_micros.max(done.micros);
            if !self.tracker.accept(scene, &done.stamp) {
                continue;
            }
            let Some(bounds) = done.mesh.bounds else {
                // Nothing to draw there: that is its mesh.
                self.note_installed(&done.mesh, 0, Instant::now());
                if let Some(old) = self.tracker.install(done.stamp, None) {
                    self.pool.retire(old.handle, Users::until_frame(frame));
                }
                continue;
            };
            self.stats.translucent_quads += u64::from(done.mesh.translucent_quads);
            self.stats.out_of_range += u64::from(done.mesh.out_of_range);
            self.pending.push(Pending {
                stamp: done.stamp,
                urgent: done.urgent,
                bytes: done.mesh.vertices.len() as u64,
                solid_quads: done.mesh.solid_quads,
                cutout_quads: done.mesh.cutout_quads,
                bounds,
                since: Instant::now(),
                stage: Stage::Waiting(done.mesh.vertices),
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

        let limit = self.workers.threads.len() * JOBS_PER_WORKER;
        while self.jobs_out < limit && self.jobs_out + self.pending.len() < MAX_PENDING {
            let Some(job) = self.tracker.next_job(scene, camera) else {
                break;
            };
            self.jobs_out += 1;
            self.workers.push(Work::Mesh {
                job: Box::new(job),
                options: self.options,
                session: self.session,
            });
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
            let mesh = MeshRange {
                handle,
                buffer: range.buffer,
                page: range.page,
                first_vertex: (range.offset / VERTEX_BYTES as u64) as u32,
                solid_quads: p.solid_quads,
                cutout_quads: p.cutout_quads,
                bounds: p.bounds,
            };
            self.stats.installed += 1;
            self.stats.installed_quads += u64::from(p.solid_quads + p.cutout_quads);
            self.stats.installed_bytes += p.bytes;
            self.stats.slowest_install_ms = self
                .stats
                .slowest_install_ms
                .max(p.since.elapsed().as_secs_f32() * 1e3);
            if let Some(old) = self.tracker.install(p.stamp, Some(mesh)) {
                // Frames up to the one before this may have drawn it.
                self.pool.retire(old.handle, Users::until_frame(frame));
            }
        }
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
    /// the solid layer nearest first, then the cutout layer.
    ///
    /// # Safety
    /// [`Self::ready`] holds for the frame being recorded, whose `UploadRing::record`
    /// and [`Self::install_ready`] have run; `pass` is of that frame.
    pub unsafe fn draw(&mut self, pass: &mut Pass<'_>, frame_data: &mut FrameData, view: &View) {
        profiling::scope!("world_draw");
        self.visible.clear();
        let mut tested = 0;
        for (key, mesh) in self.tracker.meshes() {
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
            self.visible.push(Visible {
                distance,
                mesh: *mesh,
                origin,
            });
        }
        self.stats.tested = tested;
        self.stats.visible = 0;
        self.stats.quads = 0;
        self.stats.draws = 0;
        self.stats.binds = 0;
        // Nearest first: what is behind fails the depth test before it is shaded.
        self.visible
            .sort_unstable_by_key(|v| (v.distance, v.mesh.page, v.mesh.first_vertex));
        let (Some(lightmap), Some(indices)) = (self.lightmap, self.pool.resolve(self.indices))
        else {
            return;
        };
        let Some((world, matrix)) =
            write_world(frame_data, view, self.atlas, self.atlas_size, lightmap)
        else {
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
            write_section(record, v.origin, 1.0);
        }
        let push = terrain_push(world, matrix, first);
        self.stats.visible = count as u32;
        // SAFETY: the push constants are terrain.vert's and the records they point at
        // were written just above; the atlas and the light map are textures of the
        // table that have arrived (the caller's guarantee), as have the indices; every
        // mesh drawn was installed only once its copy could be read, and draws no more
        // vertices than were copied.
        unsafe {
            pass.bind_index_buffer(indices.buffer, indices.offset);
            for (pipeline, cutout) in [
                (PipelineId::TerrainSolid, false),
                (PipelineId::TerrainCutout, true),
            ] {
                let mut bound = None;
                let mut pushed = false;
                for (instance, v) in self.visible[..count].iter().enumerate() {
                    let (mut quads, mut first_vertex) = if cutout {
                        (
                            v.mesh.cutout_quads,
                            v.mesh.first_vertex + v.mesh.solid_quads * 4,
                        )
                    } else {
                        (v.mesh.solid_quads, v.mesh.first_vertex)
                    };
                    if quads == 0 {
                        continue;
                    }
                    if bound != Some(v.mesh.page) {
                        pass.bind_vertex_buffer(v.mesh.buffer, 0);
                        bound = Some(v.mesh.page);
                        self.stats.binds += 1;
                    }
                    self.stats.quads += u64::from(quads);
                    // More quads than 16-bit indices reach: in several draws.
                    while quads > 0 {
                        let n = quads.min(QUADS_PER_DRAW);
                        pass.draw_indexed(
                            pipeline.index(),
                            if pushed { &[] } else { &push },
                            n * 6,
                            0,
                            first_vertex as i32,
                            instance as u32,
                        );
                        pushed = true;
                        self.stats.draws += 1;
                        quads -= n;
                        first_vertex += n * 4;
                    }
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
        let push = terrain_push(7, 14, 30);
        assert_eq!(push[0..4], 7u32.to_ne_bytes());
        assert_eq!(push[4..8], 14u32.to_ne_bytes());
        assert_eq!(push[8..12], 30u32.to_ne_bytes());
        assert_eq!(push[12..16], 1u32.to_ne_bytes());
    }
}
