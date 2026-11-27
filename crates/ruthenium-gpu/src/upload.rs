//! The upload ring: one persistently mapped staging buffer that CPU data is copied into,
//! and the copies to buffers and images that go to the GPU from it.
//!
//! - Space is handed out in order and comes back once the batch of copies that reads it
//!   has finished on the GPU, so staging never waits for the GPU.
//! - When the ring, the staging budget or the copy lists are used up, staging returns
//!   [`UploadError::Full`] and the caller tries again later: an upload burst spreads over
//!   frames instead of stalling one. The budget depends on the phase
//!   ([`UploadPhase`]): bursts behind a loading screen, a ceiling per frame while playing.
//! - All copies of a batch are recorded together, between one batch of barriers before and
//!   one after (vanilla puts a full pipeline barrier after every single copy).
//!
//! **Two kinds of batch.** Copies that a frame needs at once (the first upload of a
//! texture the next frame draws, a rewrite of a visible one, an urgent edit) are recorded
//! into that frame's command buffer and are ready in it. Everything else goes to the
//! *copy queue*: the dedicated transfer queue of a GPU with a copy
//! engine (NVIDIA, AMD), else copy-only submissions on the graphics queue. Its
//! submissions have a sequence of their own, so any number can go between two frames
//! ([`UploadRing::drain`]), and a batch that has to wait takes no value: its ticket stays
//! pending and its bytes reserved until a later attempt submits the same ranges. Data
//! copied there is usable from the first frame recorded after the copies have finished:
//! check with [`UploadRing::is_ready`]. That frame's submit waits on the copy timeline
//! (already signalled, so the wait costs nothing), which makes the copies visible to it.
//!
//! **Which copies go where.** Buffers are created with concurrent sharing, so either queue
//! writes them: buffer copies go to the copy queue when it is a transfer queue, else into
//! the frame. The first upload of a whole image that is not needed at once
//! ([`ImageDst::background`]) does the same; images are exclusive to one queue family, so
//! a transfer queue releases the image and the frame that first may use it acquires it.
//! An image too large for one burst streams in tiles over several batches of the copy
//! queue on either path ([`UploadRing::begin_stream`]); it stays private until the batch
//! with its last tiles has finished.
//!
//! **Worker-written staging.** For data a worker thread produces (meshes), the ring's
//! owner grants an exclusive slice of a second staging buffer ([`UploadRing::reserve`]),
//! the worker fills it and publishes that it is done, and the owner records the copy.
//! Slices come from a free list, not from the ring, so a slow worker holds only its own
//! slice and never the bytes behind it.
//!
//! Nothing here allocates per upload once the ring exists (a stream allocates when it
//! starts).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ash::vk;

use crate::Error;
use crate::budget::{Budget, PacingConfig, UploadPhase};
use crate::device::Gpu;
use crate::frame::FrameCtx;
use crate::memory::{Buffer, MemoryUse, texel_bytes};
use crate::reserve::{Reservation, ReservationState, Reservations, StagingWriter};
use crate::staging::{Batches, Ring};
use crate::stream::{ImageStream, Tile, TilePlan};
use crate::transfer::{COPY_SLOTS, CopyQueue};
use crate::users::{Completed, Users};

/// Buffer copy regions one batch can hold.
const MAX_BUFFER_COPIES: usize = 4096;
/// Buffer-to-image regions one batch can hold (one per mip level, sub-rectangle or tile).
const MAX_IMAGE_COPIES: usize = 1024;
/// Distinct images one batch can write.
const MAX_IMAGES: usize = 256;
/// Staging offsets are at least this aligned: a multiple of every texel size and of 4,
/// as image copies require.
const MIN_ALIGN: u64 = 16;
/// The largest alignment honoured; the ring's capacity is a multiple of it.
const MAX_ALIGN: u64 = 256;
/// Images released by the transfer queue and not yet acquired by the graphics queue.
const MAX_RELEASED: usize = MAX_IMAGES * COPY_SLOTS;
/// Images that can stream at once.
const MAX_STREAMS: usize = 16;
/// Writes at least this large are timed, to measure the speed of the mapped ring.
const TIMED_WRITE: usize = 64 << 10;

/// Why data could not be staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadError {
    /// No room now (the ring, the staging budget, the copy lists or the reservations are
    /// used up); try again later.
    Full,
    /// More than the ring or a batch's copy lists can ever hold; split it, or stream it.
    TooLarge,
    /// The texture's first upload has not arrived yet, or the worker has not finished
    /// writing the slice; try again in a later frame.
    NotReady,
    /// The slice was abandoned by its writer or already let go.
    Abandoned,
}

impl UploadError {
    /// Whether retrying in a later frame may succeed (transient fullness or readiness),
    /// as opposed to a permanent rejection (`TooLarge`, `Abandoned`).
    pub fn try_again(self) -> bool {
        matches!(self, UploadError::Full | UploadError::NotReady)
    }
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UploadError::Full => "the upload ring is full for now",
            UploadError::TooLarge => "the upload is larger than the upload ring",
            UploadError::NotReady => "the upload is not ready yet",
            UploadError::Abandoned => "the staging slice was abandoned",
        })
    }
}

impl std::error::Error for UploadError {}

/// The two kinds of batch a copy can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Lane {
    /// Recorded into a frame's command buffer.
    Frame,
    /// Submitted by the copy queue.
    Copy,
}

/// When an upload can be used: pass it to [`UploadRing::is_ready`]. It names the batch
/// the copy is in, which has no frame or sequence value until it is submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UploadTicket {
    lane: Lane,
    /// 0: nothing was staged, nothing to wait for.
    batch: u64,
}

/// How soon a buffer copy is needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Urgency {
    /// With the other uploads: on the transfer queue if there is one.
    #[default]
    Normal,
    /// In the next frame (a block edit the player is looking at): recorded into it on the
    /// graphics queue, and allowed the room kept free for such edits.
    Edit,
}

/// An image the ring writes to, as the copies and barriers need it.
#[derive(Clone, Copy, Debug)]
pub struct ImageDst {
    pub image: vk::Image,
    pub format: vk::Format,
    /// Size of mip level 0.
    pub extent: vk::Extent2D,
    pub mip_levels: u32,
    /// The image has never been written: its layout is still UNDEFINED and its contents
    /// may be discarded. Otherwise it is in SHADER_READ_ONLY_OPTIMAL and keeps the texels
    /// outside the written regions.
    pub fresh: bool,
    /// The image is not needed in the next frame: its first upload may go on the transfer
    /// queue and be ready a frame or more later.
    pub background: bool,
}

/// A rectangle of one mip level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageRegion {
    pub mip: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ImageRegion {
    /// All of mip level `mip` of an image whose level 0 is `extent`.
    pub fn whole_mip(extent: vk::Extent2D, mip: u32) -> ImageRegion {
        ImageRegion {
            mip,
            x: 0,
            y: 0,
            width: (extent.width >> mip).max(1),
            height: (extent.height >> mip).max(1),
        }
    }
}

/// Shader stages that sample uploaded images or read uploaded buffers.
const SHADERS: vk::PipelineStageFlags2 = vk::PipelineStageFlags2::from_raw(
    vk::PipelineStageFlags2::VERTEX_SHADER.as_raw()
        | vk::PipelineStageFlags2::FRAGMENT_SHADER.as_raw()
        | vk::PipelineStageFlags2::COMPUTE_SHADER.as_raw(),
);

fn whole(i: &ImageDst) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: i.mip_levels,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// A copy region for `width` × `height` texels of level `mip` at (`x`, `y`), read from
/// tightly packed rows at `buffer_offset`.
fn image_copy(buffer_offset: u64, r: &ImageRegion) -> vk::BufferImageCopy {
    vk::BufferImageCopy {
        buffer_offset,
        buffer_row_length: 0,
        buffer_image_height: 0,
        image_subresource: vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: r.mip,
            base_array_layer: 0,
            layer_count: 1,
        },
        image_offset: vk::Offset3D {
            x: r.x as i32,
            y: r.y as i32,
            z: 0,
        },
        image_extent: vk::Extent3D {
            width: r.width,
            height: r.height,
            depth: 1,
        },
    }
}

/// Buffer copies waiting for the next submission of their kind.
#[derive(Debug)]
struct BufferCopies {
    /// Source (the ring or the worker staging buffer) and destination of each region.
    ends: Vec<(vk::Buffer, vk::Buffer)>,
    regions: Vec<vk::BufferCopy>,
    bytes: u64,
}

impl BufferCopies {
    fn new() -> BufferCopies {
        BufferCopies {
            ends: Vec::with_capacity(MAX_BUFFER_COPIES),
            regions: Vec::with_capacity(MAX_BUFFER_COPIES),
            bytes: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn is_full(&self) -> bool {
        self.regions.len() == MAX_BUFFER_COPIES
    }

    fn push(&mut self, src: vk::Buffer, dst: vk::Buffer, region: vk::BufferCopy) {
        self.ends.push((src, dst));
        self.bytes += region.size;
        self.regions.push(region);
    }

    fn clear(&mut self) {
        self.ends.clear();
        self.regions.clear();
        self.bytes = 0;
    }

    /// # Safety
    /// `cmd` is recording; every source and destination is alive.
    unsafe fn record(&self, d: &ash::Device, cmd: vk::CommandBuffer) {
        for_runs(&self.ends, |(src, dst), range| {
            // SAFETY: guaranteed by the caller.
            unsafe { d.cmd_copy_buffer(cmd, src, dst, &self.regions[range]) };
        });
    }
}

/// Image copies of one kind of batch, waiting for its next submission.
#[derive(Debug)]
struct ImageCopies {
    /// The images written whole or in part by `upload_image` (not the streamed ones).
    images: Vec<ImageDst>,
    dsts: Vec<vk::Image>,
    regions: Vec<vk::BufferImageCopy>,
    bytes: u64,
}

impl ImageCopies {
    fn new() -> ImageCopies {
        ImageCopies {
            images: Vec::with_capacity(MAX_IMAGES),
            dsts: Vec::with_capacity(MAX_IMAGE_COPIES),
            regions: Vec::with_capacity(MAX_IMAGE_COPIES),
            bytes: 0,
        }
    }

    fn find(&self, image: vk::Image) -> Option<usize> {
        self.images.iter().position(|i| i.image == image)
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn push(&mut self, image: vk::Image, region: vk::BufferImageCopy, bytes: u64) {
        self.dsts.push(image);
        self.regions.push(region);
        self.bytes += bytes;
    }

    /// Drops every region that writes `image` (`bytes` of them in all).
    fn remove_image(&mut self, image: vk::Image, bytes: u64) {
        let mut kept = 0;
        for i in 0..self.dsts.len() {
            if self.dsts[i] != image {
                self.dsts[kept] = self.dsts[i];
                self.regions[kept] = self.regions[i];
                kept += 1;
            }
        }
        self.dsts.truncate(kept);
        self.regions.truncate(kept);
        self.bytes -= bytes;
    }

    fn clear(&mut self) {
        self.images.clear();
        self.dsts.clear();
        self.regions.clear();
        self.bytes = 0;
    }

    /// # Safety
    /// `cmd` is recording; `src` and every image are alive and in TRANSFER_DST_OPTIMAL.
    unsafe fn record(&self, d: &ash::Device, cmd: vk::CommandBuffer, src: vk::Buffer) {
        for_runs(&self.dsts, |dst, range| {
            // SAFETY: guaranteed by the caller.
            unsafe {
                d.cmd_copy_buffer_to_image(
                    cmd,
                    src,
                    dst,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &self.regions[range],
                )
            };
        });
    }
}

/// Calls `f` once per run of equal consecutive items, with the run's index range.
fn for_runs<T: Copy + PartialEq>(items: &[T], mut f: impl FnMut(T, std::ops::Range<usize>)) {
    let mut start = 0;
    for i in 1..=items.len() {
        if i == items.len() || items[i] != items[start] {
            f(items[start], start..i);
            start = i;
        }
    }
}

/// Where a stream has got to, for whoever started it.
const STREAMING: u8 = 0;
const PUBLISHED: u8 = 1;
const CANCELLED: u8 = 2;

/// What the ring and the owner of a stream share: the owner asks for cancellation and
/// reads progress here, without access to the ring.
#[derive(Debug)]
struct StreamShared {
    cancel: AtomicBool,
    outcome: AtomicU8,
    total: u64,
    staged: AtomicU64,
    submitted: AtomicU64,
    completed: AtomicU64,
    committed: AtomicBool,
    released: AtomicBool,
    /// The last copy batch whose submission wrote the image (0: none).
    last_batch: AtomicU64,
}

/// How a stream ended, or that it has not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamOutcome {
    /// Tiles are still being staged, submitted or copied; the image is private.
    Streaming,
    /// The image is whole, in SHADER_READ_ONLY_OPTIMAL and owned by the graphics queue
    /// family: the frame last recorded, and every later one, may sample it.
    Published,
    /// The stream was cancelled and the ring has let go of the image. It was never made
    /// samplable; it may be destroyed once these users have finished.
    Cancelled(Users),
}

/// How far a streamed image has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamProgress {
    /// Bytes of the whole image, and how many of them are staged, in a submission that
    /// went to the queue, and copied.
    pub total: u64,
    pub staged: u64,
    pub submitted: u64,
    pub completed: u64,
    /// The first submission went: the image has left UNDEFINED.
    pub committed: bool,
    /// The submission with the last tiles and the final transition went.
    pub released: bool,
    pub outcome: StreamOutcome,
}

/// An image being streamed ([`UploadRing::begin_stream`]), as its owner holds it.
#[derive(Clone, Debug)]
pub struct StreamHandle {
    shared: Arc<StreamShared>,
}

impl StreamHandle {
    /// Stops the stream. The ring acts on it before it stages or submits anything else:
    /// staged tiles are dropped, no more are scheduled and the image is never published.
    /// Watch [`Self::progress`] for [`StreamOutcome::Cancelled`] to learn when the image
    /// may be destroyed. A stream that was already published stays published.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Release);
    }

    pub fn progress(&self) -> StreamProgress {
        let s = &self.shared;
        let outcome = match s.outcome.load(Ordering::Acquire) {
            STREAMING => StreamOutcome::Streaming,
            PUBLISHED => StreamOutcome::Published,
            _ => StreamOutcome::Cancelled(Users {
                copy_batch: s.last_batch.load(Ordering::Relaxed),
                ..Users::NONE
            }),
        };
        StreamProgress {
            total: s.total,
            staged: s.staged.load(Ordering::Relaxed),
            submitted: s.submitted.load(Ordering::Relaxed),
            completed: s.completed.load(Ordering::Relaxed),
            committed: s.committed.load(Ordering::Relaxed),
            released: s.released.load(Ordering::Relaxed),
            outcome,
        }
    }
}

/// One image generation being streamed, as the ring holds it.
#[derive(Debug)]
struct StreamEntry {
    shared: Arc<StreamShared>,
    dst: ImageDst,
    state: ImageStream,
    /// The texels, level 0 first; dropped once every tile is staged.
    mips: Vec<Vec<u8>>,
}

impl StreamEntry {
    /// Copies the state into what the owner reads.
    fn mirror(&self) {
        let s = &self.shared;
        s.staged.store(self.state.staged_bytes(), Ordering::Relaxed);
        s.submitted
            .store(self.state.submitted_bytes(), Ordering::Relaxed);
        s.completed
            .store(self.state.completed_bytes(), Ordering::Relaxed);
        s.committed.store(self.state.committed(), Ordering::Relaxed);
        s.released
            .store(self.state.final_batch().is_some(), Ordering::Relaxed);
        s.last_batch
            .store(self.state.last_batch(), Ordering::Relaxed);
    }
}

/// The second staging buffer, for slices written by workers.
struct WorkerStaging {
    buffer: Option<Buffer>,
    raw: vk::Buffer,
    slices: Reservations,
}

/// What a flush of the copy batch did ([`UploadRing::drain`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drained {
    /// Nothing was staged for the copy queue.
    Nothing,
    /// The batch went to the queue; its submission signals this sequence value.
    Submitted(u64),
    /// Every command buffer was busy (or a deferral was forced): the batch keeps its
    /// staged ranges and no value, and the next attempt submits them.
    Deferred,
}

/// What the ring has done so far, and how full it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    /// Bytes the ring's owner copied into staging itself: the render thread's memcpy.
    pub render_memcpy_bytes: u64,
    /// Bytes workers wrote into slices granted to them (finished slices).
    pub worker_write_bytes: u64,
    /// Copy regions that went to the GPU, on either queue, and their bytes.
    pub gpu_copies: u64,
    pub gpu_copy_bytes: u64,
    /// Bytes of the ring in use now and at most, and times an allocation wrapped around.
    pub ring_occupied: u64,
    pub ring_peak: u64,
    pub ring_wraps: u64,
    /// How long the oldest bytes still in use, and the oldest slice still out, have been
    /// held, in calls of [`UploadRing::record`] and [`UploadRing::drain`].
    pub oldest_wait: u64,
    pub oldest_reservation_wait: u64,
    /// Submissions of the copy queue, and the sequence values last submitted and last
    /// seen finished.
    pub copy_submissions: u64,
    pub copy_submitted: u64,
    pub copy_completed: u64,
    /// Batches that had to wait for a later attempt, and how many of those were forced.
    pub deferred: u64,
    pub forced_deferrals: u64,
    /// Whole images uploaded on the transfer queue and released to the graphics queue.
    pub transferred_images: u64,
    /// Uploads refused because the staging budget was used up.
    pub budget_refusals: u64,
    /// The bytes the gameplay budget allowed the frame last recorded.
    pub allowance: u64,
    pub streams_begun: u64,
    pub streams_published: u64,
    pub streams_cancelled: u64,
    /// Tiles staged, their bytes, the bytes of tiles that went to the queue, and the
    /// bytes of staged tiles dropped by cancellations.
    pub tiles: u64,
    pub tile_bytes: u64,
    pub tile_bytes_submitted: u64,
    pub tile_bytes_dropped: u64,
    /// Slices granted to workers, refused for want of room, and abandoned by a writer.
    pub reservations: u64,
    pub reservations_refused: u64,
    pub reservations_abandoned: u64,
    /// Slices out now and at most, and their bytes.
    pub reservations_outstanding: u32,
    pub reservations_peak: u32,
    pub worker_bytes: u64,
    pub worker_bytes_peak: u64,
}

impl UploadStats {
    /// Every byte written into staging memory, by the render thread and by workers.
    pub fn host_write_bytes(&self) -> u64 {
        self.render_memcpy_bytes + self.worker_write_bytes
    }
}

/// The staging ring, the copy queue and the copies waiting for their next submission.
/// See the module docs.
pub struct UploadRing {
    gpu: Arc<Gpu>,
    buffer: Option<Buffer>,
    raw: vk::Buffer,
    mapped: *mut u8,
    ring: Ring,
    align: u64,
    budget: Budget,
    /// The owner of every upload submission outside frames (always there until Drop).
    copyq: Option<CopyQueue>,
    /// The batches of copies recorded into frames; a submission's value is its frame's
    /// number.
    frame_lane: Batches,
    /// The batches of the copy queue; a submission's value is its sequence value.
    copy_lane: Batches,
    /// The number the next recorded frame will have, at the least.
    next_frame: u64,
    /// Copy batches up to this are done and visible to the frame last recorded.
    visible_copy: u64,
    /// How far the GPU had got when last asked.
    completed: Completed,
    /// Counts calls of `record` and `drain`: the clock wait ages are measured on.
    clock: u64,
    frame_buffers: BufferCopies,
    copy_buffers: BufferCopies,
    /// Image copies recorded into the frame.
    frame_images: ImageCopies,
    /// First uploads of whole background images, for a transfer queue.
    copy_images: ImageCopies,
    /// Tiles of streamed images, for the copy queue.
    tiles: ImageCopies,
    /// Whole images a transfer queue released, with the batch that did, waiting for the
    /// graphics queue to acquire them.
    released: Vec<(u64, ImageDst)>,
    /// Streamed images, oldest first.
    streams: Vec<StreamEntry>,
    /// Development: streams are only staged when asked ([`Self::pump_stream`]).
    hold_streams: bool,
    worker: Option<WorkerStaging>,
    /// Staging offsets of the regions of the `upload_image` call in progress.
    staged_offsets: Vec<u64>,
    /// Development: every this many frames, act as if every copy command buffer were
    /// busy, so the deferred path runs (0: never).
    defer_every: u64,
    /// Development: the next batch with anything in it is deferred once.
    force_defer: bool,
    /// Scratch space for the barriers of a frame and of a copy submission.
    pre: Vec<vk::ImageMemoryBarrier2<'static>>,
    post: Vec<vk::ImageMemoryBarrier2<'static>>,
    copy_pre: Vec<vk::ImageMemoryBarrier2<'static>>,
    copy_post: Vec<vk::ImageMemoryBarrier2<'static>>,
    stats: UploadStats,
}

impl fmt::Debug for UploadRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadRing")
            .field("capacity", &self.ring.capacity())
            .field("phase", &self.budget.phase())
            .field("budget", &self.budget.fixed())
            .field("staged", &self.budget.staged())
            .field("transfer_queue", &self.uses_transfer_queue())
            .finish_non_exhaustive()
    }
}

// SAFETY: the mappings are only written through &mut self and through the writers of
// granted slices, which never overlap what the ring itself writes; Vulkan handles may
// move threads. The barrier scratch holds no pointers between calls.
unsafe impl Send for UploadRing {}

impl UploadRing {
    /// Creates a ring of `capacity` bytes (rounded up to 64 KiB) letting each frame stage
    /// up to `budget` bytes while playing ([`UploadPhase::Gameplay`], the phase it starts
    /// in).
    pub fn new(gpu: Arc<Gpu>, capacity: u64, budget: u64) -> Result<UploadRing, Error> {
        let capacity = capacity.max(1).next_multiple_of(64 << 10);
        let buffer = Buffer::new(
            &gpu,
            capacity,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryUse::Upload,
        )?;
        let mapped = buffer.memory.mapped;
        assert!(!mapped.is_null(), "upload memory is host-visible");
        // Touch every page now, so the first uploads in a frame don't page-fault.
        // SAFETY: the mapping covers `capacity` bytes; nothing else uses it yet.
        unsafe { std::ptr::write_bytes(mapped, 0, capacity as usize) };
        let align = gpu
            .caps()
            .copy_offset_alignment
            .clamp(MIN_ALIGN, MAX_ALIGN)
            .next_power_of_two();
        let copyq = match CopyQueue::new(&gpu) {
            Ok(q) => q,
            Err(e) => {
                // SAFETY: unused.
                unsafe { buffer.destroy(&gpu) };
                return Err(e);
            }
        };
        let pacing = if gpu.caps().unified_memory {
            PacingConfig::INTEGRATED
        } else {
            PacingConfig::DISCRETE
        };
        Ok(UploadRing {
            raw: buffer.raw,
            buffer: Some(buffer),
            mapped,
            ring: Ring::new(capacity),
            align,
            budget: Budget::new(budget, pacing),
            copyq: Some(copyq),
            frame_lane: Batches::new(),
            copy_lane: Batches::new(),
            next_frame: 1,
            visible_copy: 0,
            completed: Completed::default(),
            clock: 0,
            frame_buffers: BufferCopies::new(),
            copy_buffers: BufferCopies::new(),
            frame_images: ImageCopies::new(),
            copy_images: ImageCopies::new(),
            tiles: ImageCopies::new(),
            released: Vec::with_capacity(MAX_RELEASED),
            streams: Vec::with_capacity(MAX_STREAMS),
            hold_streams: false,
            worker: None,
            staged_offsets: Vec::with_capacity(MAX_IMAGE_COPIES),
            defer_every: 0,
            force_defer: false,
            pre: Vec::with_capacity(MAX_IMAGES + MAX_RELEASED + MAX_STREAMS),
            post: Vec::with_capacity(MAX_IMAGES),
            copy_pre: Vec::with_capacity(MAX_IMAGES + MAX_STREAMS),
            copy_post: Vec::with_capacity(MAX_IMAGES + MAX_STREAMS),
            stats: UploadStats::default(),
            gpu,
        })
    }

    fn queue(&self) -> &CopyQueue {
        self.copyq
            .as_ref()
            .expect("the copy queue lives until Drop")
    }

    pub fn capacity(&self) -> u64 {
        self.ring.capacity()
    }

    /// The bytes one frame may stage while playing, at most.
    pub fn budget(&self) -> u64 {
        self.budget.fixed()
    }

    /// Changes the bytes each frame may stage while playing.
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget.set_fixed(bytes);
    }

    /// The phase the staging budget is set for.
    pub fn phase(&self) -> UploadPhase {
        self.budget.phase()
    }

    /// Switches between bursts behind a loading screen and a ceiling per frame.
    pub fn set_phase(&mut self, phase: UploadPhase) {
        self.budget.set_phase(phase);
    }

    /// Tells the gameplay budget the pace of the frame about to be staged and recorded:
    /// the time since the frame before it started, the interval the frame loop aims for
    /// if it has one, and the time left before this frame's deadline if that is known.
    /// The frame's allowance is then the smaller of the fixed budget and a time-derived
    /// one (see [`crate::Pacing`]); without this call it is the fixed budget.
    pub fn pace(
        &mut self,
        interval: Duration,
        target: Option<Duration>,
        time_left: Option<Duration>,
    ) {
        self.budget.pace(interval, target, time_left);
    }

    /// Bytes staged so far against the current allowance.
    pub fn staged(&self) -> u64 {
        self.budget.staged()
    }

    /// Whether the copy queue is a dedicated transfer queue (else it is the graphics
    /// queue, and buffer copies and whole background images are recorded into frames).
    pub fn uses_transfer_queue(&self) -> bool {
        self.queue().dedicated()
    }

    /// Copy batches that had to wait for a later attempt.
    pub fn deferred_transfers(&self) -> u64 {
        self.stats.deferred
    }

    /// Development: from now on, every `n`th frame's copy batch waits for the next
    /// attempt, as when every copy command buffer is busy (0 turns it off).
    pub fn force_deferral_every(&mut self, n: u64) {
        self.defer_every = n;
    }

    /// Development: the next copy batch that has anything in it is deferred once.
    pub fn force_next_deferral(&mut self) {
        self.force_defer = true;
    }

    /// Development: streamed images are staged only by [`Self::pump_stream`] (`true`),
    /// or by every [`Self::record`] and [`Self::drain`] as the budget allows (`false`,
    /// the default).
    pub fn hold_streams(&mut self, hold: bool) {
        self.hold_streams = hold;
    }

    /// Whole images whose first upload went on the transfer queue.
    pub fn transferred_images(&self) -> u64 {
        self.stats.transferred_images
    }

    /// Whether the frame last passed to [`Self::record`] (and every later one) may read
    /// what the upload with this ticket wrote.
    pub fn is_ready(&self, ticket: UploadTicket) -> bool {
        match ticket.lane {
            Lane::Frame => ticket.batch <= self.frame_lane.submitted(),
            Lane::Copy => ticket.batch <= self.visible_copy,
        }
    }

    /// The batch a ticket's copy is in, as users of whatever it reads and writes: a
    /// destination range must not be reused, nor a destination destroyed, before they are
    /// done ([`Users::done`] with [`Self::completed`]). This also holds for a batch that
    /// has not been submitted yet.
    pub fn users_of(&self, ticket: UploadTicket) -> Users {
        match ticket.lane {
            Lane::Frame => Users {
                frame_batch: ticket.batch,
                ..Users::NONE
            },
            Lane::Copy => Users {
                copy_batch: ticket.batch,
                ..Users::NONE
            },
        }
    }

    /// How far the GPU had got at the last [`Self::record`] or [`Self::drain`].
    pub fn completed(&self) -> Completed {
        self.completed
    }

    /// Counters, and how full the ring is now.
    pub fn stats(&self) -> UploadStats {
        let q = self.queue();
        let mut s = UploadStats {
            ring_occupied: self.ring.occupied(),
            ring_peak: self.ring.peak(),
            ring_wraps: self.ring.wraps(),
            oldest_wait: self.ring.oldest_age(self.clock),
            copy_submitted: q.submitted(),
            copy_completed: q.completed(),
            budget_refusals: self.budget.refused,
            ..self.stats
        };
        if let Some(w) = &self.worker {
            let c = w.slices.counters();
            s.worker_write_bytes = c.written_bytes;
            s.reservations = c.granted;
            s.reservations_refused = c.refused;
            s.reservations_abandoned = c.abandoned;
            s.reservations_outstanding = c.outstanding;
            s.reservations_peak = c.peak_outstanding;
            s.worker_bytes = c.bytes;
            s.worker_bytes_peak = c.peak_bytes;
            s.oldest_reservation_wait = w.slices.oldest_age(self.clock);
        }
        s
    }

    fn open_users(&self, lane: Lane) -> Users {
        match lane {
            Lane::Frame => Users {
                frame_batch: self.frame_lane.open(),
                ..Users::NONE
            },
            Lane::Copy => Users {
                copy_batch: self.copy_lane.open(),
                ..Users::NONE
            },
        }
    }

    fn ticket(&self, lane: Lane) -> UploadTicket {
        let batch = match lane {
            Lane::Frame => self.frame_lane.open(),
            Lane::Copy => self.copy_lane.open(),
        };
        UploadTicket { lane, batch }
    }

    fn touch(&mut self, lane: Lane) {
        match lane {
            Lane::Frame => self.frame_lane.touch(),
            Lane::Copy => self.copy_lane.touch(),
        }
    }

    /// Where a buffer copy goes.
    fn buffer_lane(&self, urgency: Urgency) -> Lane {
        if urgency == Urgency::Normal && self.uses_transfer_queue() {
            Lane::Copy
        } else {
            Lane::Frame
        }
    }

    /// Reserves ring space for the open batch of `lane` and copies `data` into it. The
    /// caller accounts for the bytes ([`Self::spent`]) once its whole call has succeeded.
    fn stage(&mut self, data: &[u8], lane: Lane) -> Result<u64, UploadError> {
        let size = data.len() as u64;
        let users = self.open_users(lane);
        let offset = self.ring.alloc(size, self.align, users, self.clock)?;
        let timed = (data.len() >= TIMED_WRITE).then(Instant::now);
        // SAFETY: [offset, offset + size) lies inside the mapping and is not read by any
        // unfinished GPU work (the ring's invariant).
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.mapped.add(offset as usize),
                data.len(),
            )
        };
        if let Some(started) = timed {
            self.budget.pacing.wrote(size, started.elapsed());
        }
        Ok(offset)
    }

    /// `bytes` were staged by the render thread for the open batch of `lane`.
    fn spent(&mut self, bytes: u64, lane: Lane) {
        self.budget.spend(bytes);
        self.stats.render_memcpy_bytes += bytes;
        self.touch(lane);
    }

    /// Stages `data` to be copied to `dst` at `dst_offset`, and returns when the copy can
    /// be used ([`Self::is_ready`]): recorded into a frame, in the next frame; on a
    /// transfer queue, once its batch has finished there.
    ///
    /// The destination range must not be in use by unfinished frames (write to fresh
    /// ranges, and reuse a range only after the frames reading it have finished), and must
    /// not be read before the ticket is ready.
    pub fn upload_buffer(
        &mut self,
        dst: vk::Buffer,
        dst_offset: u64,
        data: &[u8],
    ) -> Result<UploadTicket, UploadError> {
        self.upload_buffer_with(dst, dst_offset, data, Urgency::Normal)
    }

    /// [`Self::upload_buffer`], saying how soon the copy is needed.
    pub fn upload_buffer_with(
        &mut self,
        dst: vk::Buffer,
        dst_offset: u64,
        data: &[u8],
        urgency: Urgency,
    ) -> Result<UploadTicket, UploadError> {
        if data.is_empty() {
            return Ok(UploadTicket {
                lane: Lane::Frame,
                batch: 0,
            });
        }
        let lane = self.buffer_lane(urgency);
        let size = data.len() as u64;
        let full = match lane {
            Lane::Frame => self.frame_buffers.is_full(),
            Lane::Copy => self.copy_buffers.is_full(),
        };
        if full || !self.budget.admit(size, self.next_frame) {
            return Err(UploadError::Full);
        }
        let ticket = self.ticket(lane);
        let src_offset = self.stage(data, lane)?;
        self.spent(size, lane);
        let copies = match lane {
            Lane::Frame => &mut self.frame_buffers,
            Lane::Copy => &mut self.copy_buffers,
        };
        copies.push(
            self.raw,
            dst,
            vk::BufferCopy {
                src_offset,
                dst_offset,
                size,
            },
        );
        Ok(ticket)
    }

    /// Stages `regions` of `dst` (tightly packed texels, rows top to bottom), and returns
    /// when the image can be sampled ([`Self::is_ready`]). Either every region is staged or
    /// none is. Once ready, the image is in SHADER_READ_ONLY_OPTIMAL.
    ///
    /// The first upload of a [`ImageDst::background`] image goes on the transfer queue if
    /// there is one, and is ready once it has finished there (usually a frame later);
    /// everything else is recorded into the next frame and ready in it. An image whose
    /// first upload is not ready yet must not be written again (only `fresh` writes may
    /// join it in the same batch). An image too large for the ring is
    /// [`UploadError::TooLarge`]: stream it ([`Self::begin_stream`]).
    ///
    /// # Panics
    /// If a region lies outside its mip level, or its data is not exactly its size.
    pub fn upload_image(
        &mut self,
        dst: &ImageDst,
        regions: &[(ImageRegion, &[u8])],
    ) -> Result<UploadTicket, UploadError> {
        if regions.is_empty() {
            return Ok(UploadTicket {
                lane: Lane::Frame,
                batch: 0,
            });
        }
        let texel = texel_bytes(dst.format)
            .unwrap_or_else(|| panic!("no upload path for {:?}", dst.format));
        for (r, data) in regions {
            let mip = ImageRegion::whole_mip(dst.extent, r.mip);
            assert!(
                r.mip < dst.mip_levels
                    && r.x + r.width <= mip.width
                    && r.y + r.height <= mip.height,
                "{r:?} lies outside the image"
            );
            assert_eq!(
                data.len() as u64,
                u64::from(r.width) * u64::from(r.height) * u64::from(texel),
                "region data size"
            );
        }
        if regions.len() > MAX_IMAGE_COPIES {
            return Err(UploadError::TooLarge);
        }
        let in_frame = self.frame_images.find(dst.image);
        let in_copy = self.copy_images.find(dst.image);
        debug_assert!(
            dst.fresh
                || (in_copy.is_none() && !self.released.iter().any(|(_, i)| i.image == dst.image)),
            "{:?} written before its first upload is ready",
            dst.image
        );
        debug_assert!(
            !self.streams.iter().any(|s| s.dst.image == dst.image),
            "{:?} written while it streams",
            dst.image
        );
        // Background first uploads (and fresh writes joining one) go to a transfer queue.
        let lane = if self.uses_transfer_queue()
            && in_frame.is_none()
            && (in_copy.is_some() || (dst.fresh && dst.background))
        {
            Lane::Copy
        } else {
            Lane::Frame
        };
        let (copies, known) = match lane {
            Lane::Copy => (&self.copy_images, in_copy),
            Lane::Frame => (&self.frame_images, in_frame),
        };
        if copies.regions.len() + regions.len() > MAX_IMAGE_COPIES
            || (known.is_none() && copies.images.len() == MAX_IMAGES)
            || (lane == Lane::Copy
                && known.is_none()
                && self.released.len() + copies.images.len() >= MAX_RELEASED)
        {
            return Err(UploadError::Full);
        }
        let total: u64 = regions.iter().map(|(_, d)| d.len() as u64).sum();
        // All of it has to be in the ring at once, each region at an aligned offset: more
        // than the ring holds never fits, however often it is tried.
        let aligned: u64 = regions
            .iter()
            .map(|(_, d)| (d.len() as u64).next_multiple_of(self.align))
            .sum();
        if aligned > self.ring.capacity() {
            return Err(UploadError::TooLarge);
        }
        if !self.budget.admit(total, self.next_frame) {
            return Err(UploadError::Full);
        }
        let ticket = self.ticket(lane);
        let before = self.ring.checkpoint();
        self.staged_offsets.clear();
        for (_, data) in regions {
            match self.stage(data, lane) {
                Ok(offset) => self.staged_offsets.push(offset),
                Err(e) => {
                    // Undo the regions staged so far: the bytes were not handed out yet.
                    self.ring.rollback(before);
                    return Err(e);
                }
            }
        }
        self.spent(total, lane);
        let copies = match lane {
            Lane::Copy => &mut self.copy_images,
            Lane::Frame => &mut self.frame_images,
        };
        for ((r, data), &buffer_offset) in regions.iter().zip(&self.staged_offsets) {
            copies.push(dst.image, image_copy(buffer_offset, r), data.len() as u64);
        }
        match known {
            // Written twice in this batch: if either write saw it unwritten, it is.
            Some(i) => copies.images[i].fresh |= dst.fresh,
            None => copies.images.push(*dst),
        }
        Ok(ticket)
    }

    /// Starts the first upload of an image in tiles: `mips` are its levels (level 0
    /// first; tightly packed rows, top first), which the ring keeps until they are
    /// staged. Tiles are staged as the budget allows by every [`Self::record`] and
    /// [`Self::drain`], in the order the streams were started, and go to the copy queue;
    /// the image stays private (never in SHADER_READ_ONLY_OPTIMAL, never acquired) until
    /// the batch with its last tiles has finished, and is published by the next frame
    /// recorded after that ([`StreamOutcome::Published`]).
    ///
    /// The image must be unwritten (layout UNDEFINED) and must stay alive until the
    /// stream is published or reports [`StreamOutcome::Cancelled`] and those users are
    /// done.
    ///
    /// [`UploadError::Full`]: too many images stream already; try again later.
    /// [`UploadError::TooLarge`]: not even its smallest tile fits the ring.
    ///
    /// # Panics
    /// If `mips` doesn't match the image's levels and sizes, or its format has no upload
    /// path.
    pub fn begin_stream(
        &mut self,
        dst: &ImageDst,
        mips: Vec<Vec<u8>>,
    ) -> Result<StreamHandle, UploadError> {
        // A transfer queue copies in units of its family's granularity; the graphics
        // queue copies any texels.
        let granularity = if self.uses_transfer_queue() {
            self.gpu.caps().transfer_granularity
        } else {
            vk::Extent3D {
                width: 1,
                height: 1,
                depth: 1,
            }
        };
        let plan = TilePlan::new(dst.format, dst.extent, dst.mip_levels, granularity)
            .unwrap_or_else(|| panic!("no upload path for {:?}", dst.format));
        assert_eq!(mips.len(), dst.mip_levels as usize, "mip levels");
        for (level, data) in mips.iter().enumerate() {
            assert_eq!(
                data.len() as u64,
                plan.level_bytes(level as u32).0,
                "size of mip level {level}"
            );
        }
        if plan.largest_step() > self.ring.capacity() {
            return Err(UploadError::TooLarge);
        }
        if self.streams.len() == MAX_STREAMS {
            return Err(UploadError::Full);
        }
        let state = ImageStream::new(plan);
        let shared = Arc::new(StreamShared {
            cancel: AtomicBool::new(false),
            outcome: AtomicU8::new(STREAMING),
            total: state.total(),
            staged: AtomicU64::new(0),
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            committed: AtomicBool::new(false),
            released: AtomicBool::new(false),
            last_batch: AtomicU64::new(0),
        });
        self.streams.push(StreamEntry {
            shared: shared.clone(),
            dst: ImageDst {
                fresh: true,
                background: true,
                ..*dst
            },
            state,
            mips,
        });
        self.stats.streams_begun += 1;
        Ok(StreamHandle { shared })
    }

    /// Development: stages tiles of `stream` now, at most `max_bytes` of them and outside
    /// the staging budget, and returns the bytes staged (0: the stream is done or gone,
    /// its next tile is larger than `max_bytes`, or the ring has no room).
    pub fn pump_stream(&mut self, stream: &StreamHandle, max_bytes: u64) -> u64 {
        let Some(i) = self
            .streams
            .iter()
            .position(|s| Arc::ptr_eq(&s.shared, &stream.shared))
        else {
            return 0;
        };
        let mut staged = 0;
        while let Some(bytes) = self.stage_tile(i, max_bytes - staged, false) {
            staged += bytes;
        }
        staged
    }

    /// Stages the next tile of stream `i`, of at most `max` bytes and, if `budgeted`,
    /// within the staging budget. Returns its bytes, or `None` if nothing was staged.
    fn stage_tile(&mut self, i: usize, max: u64, budgeted: bool) -> Option<u64> {
        let entry = &self.streams[i];
        if entry.shared.cancel.load(Ordering::Acquire)
            || self.tiles.regions.len() == MAX_IMAGE_COPIES
        {
            return None;
        }
        let (here, wrapped) = self.ring.room(self.align);
        let limit = if budgeted {
            max.min(self.budget.remaining())
        } else {
            max
        };
        // Up to the end of the buffer if a tile fits there, else from its start.
        let pick = |bytes: u64| {
            entry
                .state
                .next_tile(bytes.min(here))
                .or_else(|| entry.state.next_tile(bytes.min(wrapped)))
        };
        let tile = match pick(limit) {
            Some(tile) => tile,
            None => {
                // The allowance is below the smallest tile there is: that one still goes
                // if the budget admits it, which it does when nothing else is staged.
                let smallest = entry.state.smallest_tile()?;
                if !budgeted || smallest.bytes <= limit || smallest.bytes > max {
                    return None;
                }
                pick(smallest.bytes)?
            }
        };
        if budgeted && !self.budget.admit(tile.bytes, self.next_frame) {
            return None;
        }
        let Tile {
            mip,
            y,
            rows,
            width,
            bytes,
        } = tile;
        let users = self.open_users(Lane::Copy);
        let offset = self.ring.alloc(bytes, self.align, users, self.clock).ok()?;
        let entry = &mut self.streams[i];
        let from = entry.state.plan().offset_in_level(&tile) as usize;
        let data = &entry.mips[mip as usize][from..from + bytes as usize];
        let timed = (data.len() >= TIMED_WRITE).then(Instant::now);
        // SAFETY: [offset, offset + bytes) lies inside the mapping and is not read by any
        // unfinished GPU work (the ring's invariant).
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.mapped.add(offset as usize),
                data.len(),
            )
        };
        if let Some(started) = timed {
            self.budget.pacing.wrote(bytes, started.elapsed());
        }
        let region = ImageRegion {
            mip,
            x: 0,
            y,
            width,
            height: rows,
        };
        self.tiles
            .push(entry.dst.image, image_copy(offset, &region), bytes);
        entry.state.staged(&tile);
        if entry.state.all_staged() {
            // Every texel is in the ring: the CPU copy can go.
            entry.mips = Vec::new();
        }
        entry.mirror();
        self.stats.tiles += 1;
        self.stats.tile_bytes += bytes;
        self.spent(bytes, Lane::Copy);
        Some(bytes)
    }

    /// Stages what the budget allows of every stream, oldest first.
    fn pump_streams(&mut self) {
        for i in 0..self.streams.len() {
            while self.stage_tile(i, u64::MAX, true).is_some() {}
        }
    }

    /// Creates the second staging buffer, of `bytes` (rounded up to 64 KiB), that
    /// [`Self::reserve`] grants slices of to workers. An eighth of it is kept for urgent
    /// edits. Not on a frame's path: it allocates device memory and touches every page.
    pub fn enable_worker_staging(&mut self, bytes: u64) -> Result<(), Error> {
        if self.worker.is_some() {
            return Ok(());
        }
        let capacity = bytes.max(1).next_multiple_of(64 << 10);
        let buffer = Buffer::new(
            &self.gpu,
            capacity,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryUse::Upload,
        )?;
        let mapped = buffer.memory.mapped;
        assert!(!mapped.is_null(), "upload memory is host-visible");
        // SAFETY: the mapping covers `capacity` bytes; nothing else uses it yet.
        unsafe { std::ptr::write_bytes(mapped, 0, capacity as usize) };
        self.worker = Some(WorkerStaging {
            raw: buffer.raw,
            buffer: Some(buffer),
            // SAFETY: the mapping covers `capacity` bytes, only its slices' writers
            // write it, and it is unmapped (in Drop) only once no writer is left.
            slices: unsafe { Reservations::new(mapped, capacity, MIN_ALIGN) },
        });
        Ok(())
    }

    /// Grants a worker an exclusive slice of `size` bytes of the worker staging buffer:
    /// the writer goes to the worker, the reservation stays with the ring's owner, who
    /// records copies from the slice once it is written ([`Self::copy_reserved`]) and
    /// lets it go ([`Self::release`]).
    ///
    /// [`UploadError::Full`]: too many slices are out, or their bytes leave no room
    /// (ordinary reservations also leave some for urgent ones); defer the job.
    /// [`UploadError::TooLarge`]: larger than the buffer can ever grant.
    ///
    /// # Panics
    /// If worker staging is not enabled, or `size` is zero.
    pub fn reserve(
        &mut self,
        size: u64,
        urgency: Urgency,
    ) -> Result<(Reservation, StagingWriter), UploadError> {
        let w = self.worker.as_mut().expect("worker staging is enabled");
        w.slices
            .grant(size, self.align, urgency == Urgency::Edit, self.clock)
    }

    /// What the worker has done with the slice.
    pub fn reservation(&self, r: Reservation) -> ReservationState {
        match &self.worker {
            Some(w) => w.slices.state(r),
            None => ReservationState::Gone,
        }
    }

    /// Records a copy of the written bytes of a finished slice to `dst` at `dst_offset`,
    /// and returns when it can be used. Only the ring's owner records copies, and only
    /// from slices whose writer has finished; decide first whether the data is still
    /// wanted (a newer revision makes it stale: let the slice go instead). A slice may be
    /// copied more than once, on either queue; it is reclaimed when all of them are done.
    ///
    /// The destination range is under [`Self::upload_buffer`]'s contract.
    ///
    /// [`UploadError::NotReady`]: the worker still writes. [`UploadError::Abandoned`]:
    /// the worker dropped the slice, or the reservation is stale.
    pub fn copy_reserved(
        &mut self,
        r: Reservation,
        dst: vk::Buffer,
        dst_offset: u64,
        urgency: Urgency,
    ) -> Result<UploadTicket, UploadError> {
        let written = match self.reservation(r) {
            ReservationState::Writing => return Err(UploadError::NotReady),
            ReservationState::Complete { written } => written,
            ReservationState::Abandoned | ReservationState::Gone => {
                return Err(UploadError::Abandoned);
            }
        };
        let lane = self.buffer_lane(urgency);
        if written == 0 {
            return Ok(UploadTicket { lane, batch: 0 });
        }
        let ticket = self.ticket(lane);
        let users = self.open_users(lane);
        let copies = match lane {
            Lane::Frame => &mut self.frame_buffers,
            Lane::Copy => &mut self.copy_buffers,
        };
        if copies.is_full() {
            return Err(UploadError::Full);
        }
        let w = self.worker.as_mut().expect("the reservation is live");
        let src_offset = w
            .slices
            .copied_by(r, users)
            .expect("the reservation is live");
        copies.push(
            w.raw,
            dst,
            vk::BufferCopy {
                src_offset,
                dst_offset,
                size: written,
            },
        );
        self.touch(lane);
        Ok(ticket)
    }

    /// Lets a slice go: after its copies were recorded, or instead of recording any (the
    /// job was cancelled or its data is stale). The bytes are handed out again only once
    /// the writer has let go too and every recorded copy has finished on the GPU. Does
    /// nothing for a stale reservation.
    pub fn release(&mut self, r: Reservation) {
        if let Some(w) = &mut self.worker {
            w.slices.release(r);
        }
    }

    /// Reads how far both queues have got, frees what has finished, and acts on
    /// cancellations. `completed` is a frame number the GPU has finished.
    fn progress(&mut self, completed: u64) -> Result<(), Error> {
        self.clock += 1;
        let q = self
            .copyq
            .as_mut()
            .expect("the copy queue lives until Drop");
        let value = q.poll(&self.gpu)?;
        self.copy_lane.finish(value);
        self.frame_lane.finish(completed);
        self.completed = Completed {
            frame: self.completed.frame.max(completed),
            frame_batch: self.frame_lane.done(),
            copy_batch: self.copy_lane.done(),
        };
        self.ring.retire(&self.completed);
        if let Some(w) = &mut self.worker {
            w.slices.reclaim(&self.completed);
        }
        let done = self.copy_lane.done();
        let mut i = 0;
        while i < self.streams.len() {
            let entry = &mut self.streams[i];
            entry.state.finished(done);
            if entry.shared.cancel.load(Ordering::Acquire) {
                // Before anything else is staged or submitted: drop what is staged, and
                // tell the owner which submission wrote the image last.
                let cancelled = entry.state.cancel();
                self.tiles
                    .remove_image(entry.dst.image, cancelled.dropped_bytes);
                self.stats.tile_bytes_dropped += cancelled.dropped_bytes;
                self.stats.streams_cancelled += 1;
                entry.mirror();
                entry.shared.outcome.store(CANCELLED, Ordering::Release);
                self.streams.remove(i);
            } else {
                entry.mirror();
                i += 1;
            }
        }
        Ok(())
    }

    /// Submits the open batch of the copy queue, if it has anything and a command buffer
    /// is free. A batch that has to wait keeps everything: its copies, its staged bytes
    /// and its id; nothing about any image changes.
    ///
    /// # Safety
    /// Every destination of the staged copies is alive, and its written ranges are not in
    /// use by unfinished work; every slice copied from is written.
    unsafe fn flush(&mut self, forced: bool) -> Result<Drained, Error> {
        if self.copy_buffers.is_empty() && self.copy_images.is_empty() && self.tiles.is_empty() {
            if self.copy_lane.dirty() {
                // What was staged for it was cancelled: its bytes need no submission.
                self.copy_lane.close_empty();
            }
            return Ok(Drained::Nothing);
        }
        let forced = std::mem::take(&mut self.force_defer) || forced;
        let UploadRing {
            gpu,
            copyq,
            copy_buffers,
            copy_images,
            tiles,
            streams,
            copy_pre,
            copy_post,
            raw,
            ..
        } = self;
        let q = copyq.as_mut().expect("the copy queue lives until Drop");
        let trackable = streams
            .iter()
            .all(|s| !s.state.has_pending() || s.state.can_submit());
        if forced || !q.has_free_slot() || !trackable {
            self.stats.deferred += 1;
            self.stats.forced_deferrals += u64::from(forced);
            return Ok(Drained::Deferred);
        }
        // A transfer queue hands each finished image to the graphics queue family.
        let (from, to) = if q.dedicated() {
            (q.family(), gpu.queue_family)
        } else {
            (vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED)
        };
        let to_dst = |i: &ImageDst| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .image(i.image)
                .subresource_range(whole(i))
        };
        let to_sampling = |i: &ImageDst| {
            let barrier = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(from)
                .dst_queue_family_index(to)
                .image(i.image)
                .subresource_range(whole(i));
            if from == to {
                // The same queue goes on to sample it: the copy-to-sampling barrier.
                barrier
                    .dst_stage_mask(SHADERS)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            } else {
                // The release half of the queue family ownership transfer.
                barrier
                    .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                    .dst_access_mask(vk::AccessFlags2::NONE)
            }
        };
        copy_pre.clear();
        copy_post.clear();
        for i in &copy_images.images {
            copy_pre.push(to_dst(i));
            copy_post.push(to_sampling(i));
        }
        for s in streams.iter() {
            if let Some(t) = s.state.transitions() {
                if t.first {
                    copy_pre.push(to_dst(&s.dst));
                }
                if t.last {
                    copy_post.push(to_sampling(&s.dst));
                }
            }
        }
        let src = *raw;
        // SAFETY: the copies read staged bytes that are written and stay reserved until
        // this submission has finished (their slices carry the open batch); whole images
        // are fresh and streamed ones are in the layout their state says; the rest is the
        // caller's contract. One thread submits to the queue: the ring's owner, which on
        // the graphics queue is the frame loop's thread.
        let value = unsafe {
            q.submit(gpu, |cmd| {
                let d = &gpu.device;
                if !copy_pre.is_empty() {
                    gpu.cmd_pipeline_barrier2(
                        cmd,
                        &vk::DependencyInfo::default().image_memory_barriers(copy_pre),
                    );
                }
                copy_buffers.record(d, cmd);
                copy_images.record(d, cmd, src);
                tiles.record(d, cmd, src);
                if !copy_post.is_empty() {
                    gpu.cmd_pipeline_barrier2(
                        cmd,
                        &vk::DependencyInfo::default().image_memory_barriers(copy_post),
                    );
                }
            })?
        };
        let Some(value) = value else {
            self.stats.deferred += 1;
            return Ok(Drained::Deferred);
        };
        // Only now does the batch have a value, and do the images change state.
        let batch = self.copy_lane.submit(value);
        self.stats.copy_submissions += 1;
        self.stats.gpu_copies +=
            (copy_buffers.regions.len() + copy_images.regions.len() + tiles.regions.len()) as u64;
        self.stats.gpu_copy_bytes += copy_buffers.bytes + copy_images.bytes + tiles.bytes;
        self.stats.tile_bytes_submitted += tiles.bytes;
        self.stats.transferred_images += copy_images.images.len() as u64;
        for i in copy_images.images.drain(..) {
            self.released.push((batch, i));
        }
        for s in streams.iter_mut() {
            if s.state.has_pending() {
                s.state.submitted(batch);
                s.mirror();
            }
        }
        copy_buffers.clear();
        copy_images.clear();
        tiles.clear();
        self.budget.submitted();
        Ok(Drained::Submitted(value))
    }

    /// Outside a frame: frees what has finished, stages stream tiles as the budget
    /// allows and submits the copy batch on its own. Behind a loading screen, call it
    /// until the work is through, between servicing events: each submission makes room
    /// for the next burst as soon as it has finished. Copies recorded into frames, and
    /// the publication of finished images, wait for the next [`Self::record`].
    ///
    /// # Safety
    /// `completed` is a frame number the GPU has finished; every destination of the
    /// staged copies is still alive; the caller is the thread that submits frames.
    pub unsafe fn drain(&mut self, completed: u64) -> Result<Drained, Error> {
        profiling::scope!("drain_uploads");
        self.progress(completed)?;
        if !self.hold_streams {
            self.pump_streams();
        }
        // SAFETY: forwarded from the caller and the upload calls.
        unsafe { self.flush(false) }
    }

    /// Sends the staged copies on their way, frees the space of finished batches, and
    /// makes the finished copies visible to `ctx`'s frame. Call it once per frame, before
    /// the frame's passes:
    ///
    /// - the copy queue's batch is submitted (stream tiles are staged into it first, as
    ///   the budget allows); `ctx` gets a wait on the copy timeline, acquires the images a
    ///   transfer queue has finished and released, and publishes finished streams;
    /// - every other copy is recorded into `ctx.cmd`, with the barriers that make it
    ///   visible to this frame's and later frames' shaders.
    ///
    /// # Safety
    /// `ctx` is the frame being recorded on the graphics queue, and `completed` a frame
    /// number the GPU has finished; frame numbers grow with each call; every destination
    /// buffer and image is still alive.
    pub unsafe fn record(&mut self, ctx: &mut FrameCtx, completed: u64) -> Result<(), Error> {
        profiling::scope!("record_uploads");
        let frame = ctx.number;
        debug_assert!(frame >= self.next_frame, "frames recorded out of order");
        self.progress(completed)?;
        if !self.hold_streams {
            self.pump_streams();
        }
        let forced = self.defer_every > 0 && frame.is_multiple_of(self.defer_every);
        // SAFETY: forwarded from the caller and the upload calls.
        unsafe { self.flush(forced)? };
        self.stats.allowance = match self.budget.phase() {
            UploadPhase::Loading => 0,
            UploadPhase::Gameplay => self.budget.allowance(),
        };

        self.pre.clear();
        self.post.clear();
        let q = self
            .copyq
            .as_ref()
            .expect("the copy queue lives until Drop");
        let done = self.copy_lane.done();
        let (from, to) = (q.family(), self.gpu.queue_family);
        // The acquire half of the ownership transfer, for every image whose release has
        // finished. The frame's wait on the copy timeline orders it after the release.
        let acquire = |i: &ImageDst| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(SHADERS)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(from)
                .dst_queue_family_index(to)
                .image(i.image)
                .subresource_range(whole(i))
        };
        let mut i = 0;
        while i < self.released.len() {
            if self.released[i].0 > done {
                i += 1;
                continue;
            }
            let (_, image) = self.released.swap_remove(i);
            self.pre.push(acquire(&image));
        }
        // A streamed image is published by the first frame recorded after the batch with
        // its last tiles has finished, and by no frame before: until here it was never
        // in a layout or a queue family a frame could sample it in.
        let mut i = 0;
        while i < self.streams.len() {
            let entry = &self.streams[i];
            if entry.state.final_batch().is_none_or(|batch| batch > done) {
                i += 1;
                continue;
            }
            if q.dedicated() {
                self.pre.push(acquire(&entry.dst));
            }
            entry.shared.outcome.store(PUBLISHED, Ordering::Release);
            self.stats.streams_published += 1;
            self.streams.remove(i);
        }
        // Copies up to the finished value are done: this frame may use them. The wait is
        // already satisfied; it makes their writes visible to the frame.
        self.visible_copy = done;
        if q.completed() > 0 {
            ctx.wait_timeline(
                q.timeline(),
                q.completed(),
                vk::PipelineStageFlags2::ALL_COMMANDS,
            );
        }

        self.next_frame = frame + 1;
        let copies = !self.frame_buffers.is_empty() || !self.frame_images.is_empty();
        if copies || !self.pre.is_empty() {
            // SAFETY: forwarded from the caller and the upload calls.
            unsafe { self.record_into_frame(ctx.cmd) };
        }
        if copies {
            self.stats.gpu_copies +=
                (self.frame_buffers.regions.len() + self.frame_images.regions.len()) as u64;
            self.stats.gpu_copy_bytes += self.frame_buffers.bytes + self.frame_images.bytes;
            self.frame_buffers.clear();
            self.frame_images.clear();
            self.frame_lane.submit(frame);
        } else if self.frame_lane.dirty() {
            self.frame_lane.close_empty();
        }
        self.budget.frame_recorded();
        Ok(())
    }

    /// Records the acquires in `self.pre` and the frame's own copies, between the barriers
    /// they need.
    ///
    /// # Safety
    /// `cmd` is the command buffer of the frame being recorded on the graphics queue;
    /// every destination is alive; the staged bytes are written.
    unsafe fn record_into_frame(&mut self, cmd: vk::CommandBuffer) {
        for i in &self.frame_images.images {
            let (old, src_stage) = if i.fresh {
                (vk::ImageLayout::UNDEFINED, vk::PipelineStageFlags2::NONE)
            } else {
                // Earlier frames' reads must finish before the copy overwrites texels.
                (vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, SHADERS)
            };
            self.pre.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(src_stage)
                    .src_access_mask(vk::AccessFlags2::NONE)
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(old)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .image(i.image)
                    .subresource_range(whole(i)),
            );
            self.post.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(SHADERS)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(i.image)
                    .subresource_range(whole(i)),
            );
        }
        // Buffers are written only where no unfinished frame reads (upload_buffer's
        // contract), so they need a barrier after the copies only.
        let buffers_after = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(
                vk::PipelineStageFlags2::VERTEX_INPUT
                    | vk::PipelineStageFlags2::DRAW_INDIRECT
                    | SHADERS,
            )
            .dst_access_mask(
                vk::AccessFlags2::VERTEX_ATTRIBUTE_READ
                    | vk::AccessFlags2::INDEX_READ
                    | vk::AccessFlags2::INDIRECT_COMMAND_READ
                    | vk::AccessFlags2::UNIFORM_READ
                    | vk::AccessFlags2::SHADER_STORAGE_READ,
            )];
        let buffers = !self.frame_buffers.is_empty();
        let buffer_barriers = if buffers {
            &buffers_after[..]
        } else {
            &buffers_after[..0]
        };
        let gpu = &*self.gpu;
        // SAFETY: guaranteed by the caller; the staged bytes were written before this
        // frame's submit, which makes host writes visible to the device.
        unsafe {
            if !self.pre.is_empty() {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&self.pre),
                );
            }
            self.frame_buffers.record(&gpu.device, cmd);
            self.frame_images.record(&gpu.device, cmd, self.raw);
            if buffers || !self.post.is_empty() {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default()
                        .memory_barriers(buffer_barriers)
                        .image_memory_barriers(&self.post),
                );
            }
        }
    }
}

/// How long dropping the ring waits for workers to let go of their slices.
const DROP_WRITER_WAIT: Duration = Duration::from_secs(5);

impl Drop for UploadRing {
    fn drop(&mut self) {
        if let Some(mut q) = self.copyq.take() {
            // Copies on a transfer queue are not covered by the frame loop's idle wait.
            q.wait_idle(&self.gpu);
            // SAFETY: after the wait no submission of the queue is unfinished.
            unsafe { q.destroy(&self.gpu) };
        }
        if let Some(mut w) = self.worker.take() {
            // A worker may still be writing into the mapping: it goes only once every
            // writer has let go.
            let started = Instant::now();
            while w.slices.writers() > 0 && started.elapsed() < DROP_WRITER_WAIT {
                std::thread::sleep(Duration::from_millis(1));
            }
            if w.slices.writers() == 0 {
                if let Some(b) = w.buffer.take() {
                    // SAFETY: no writer holds a pointer into it, and no copy from it is
                    // unfinished (see below for the frames' copies).
                    unsafe { b.destroy(&self.gpu) };
                }
            } else {
                // Leaked on purpose: unmapping under a writer would be worse.
                eprintln!(
                    "[gpu] a staging slice is still held by a worker: its buffer is not freed"
                );
            }
        }
        if let Some(b) = self.buffer.take() {
            // SAFETY: the owner drops the ring only after the GPU is idle (the frame loop
            // waits in its Drop) or before any frame recorded its copies.
            unsafe { b.destroy(&self.gpu) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_of_equal_destinations() {
        let mut runs = Vec::new();
        for_runs(&[1, 1, 2, 3, 3, 3, 1], |v, r| runs.push((v, r)));
        assert_eq!(runs, vec![(1, 0..2), (2, 2..3), (3, 3..6), (1, 6..7)]);
        for_runs::<u8>(&[], |_, _| panic!("no runs in nothing"));
    }

    #[test]
    fn whole_mips_shrink_to_one_texel() {
        let e = vk::Extent2D {
            width: 64,
            height: 16,
        };
        assert_eq!(ImageRegion::whole_mip(e, 0).width, 64);
        let m = ImageRegion::whole_mip(e, 5);
        assert_eq!((m.width, m.height), (2, 1));
    }

    #[test]
    fn a_cancelled_image_leaves_the_other_images_of_the_batch() {
        use ash::vk::Handle;
        let (a, b) = (vk::Image::from_raw(1), vk::Image::from_raw(2));
        let region = |offset| image_copy(offset, &ImageRegion::default());
        let mut copies = ImageCopies::new();
        copies.push(a, region(0), 100);
        copies.push(b, region(100), 50);
        copies.push(a, region(150), 100);
        copies.push(b, region(250), 50);
        copies.remove_image(a, 200);
        assert_eq!(copies.dsts, vec![b, b]);
        let offsets: Vec<u64> = copies.regions.iter().map(|r| r.buffer_offset).collect();
        assert_eq!(offsets, vec![100, 250]);
        assert_eq!(copies.bytes, 100);
    }
}
