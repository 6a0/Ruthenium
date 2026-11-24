//! A development check of streamed uploads, worker-written staging and pooled buffer
//! ranges (`--exercise-streaming`). It needs the upload ring to itself.
//!
//! **A texture larger than the ring.** A 2048 × 2048 RGBA8 texture with all twelve mip
//! levels (21.3 MiB; the ring holds 16) is streamed in tiles, every level is read back
//! and every byte compared. Round after round the check walks through:
//!
//! - the first upload, staged by hand: the first batch is deferred before it is
//!   submitted, then two batches are submitted within one frame with a deferred batch
//!   between them, a staged batch is held back until staging has to go around the end of
//!   the ring, and the batch with the last tiles is deferred once more. A deferral
//!   must leave submitted coverage, layout and release as they were; each submission must
//!   have a higher sequence value than the one before; and the tiles must have gone to
//!   the queue exactly once, whatever was retried;
//! - a replacement of the visible texture, streamed by the ring itself (as a loading
//!   burst drained between presents, or under the gameplay budget): the old generation
//!   is read back, unchanged, while the new one streams, and the new one when it shows;
//! - a replacement cancelled part-way, after which the old generation is still intact;
//! - a second texture cancelled before its first submission (a small texture staged into
//!   the same batch must still arrive, intact), one cancelled during streaming, and one
//!   cancelled after the batch with its last tiles was submitted and before any frame
//!   acquired it. None of them may ever be published.
//!
//! In every frame of every stream the slot must show the missing texture until the
//! stream is published. A round ends by letting go of everything and waiting until the
//! ring, the worker staging buffer and the pool are empty again.
//!
//! **Worker-written staging and pooled ranges.** All the while, mesh-sized blocks get a
//! range from a pool of 1 MiB pages and a slice of the worker staging buffer; a worker
//! thread fills the slice, the check records the copy once the writer has finished,
//! copies the range on to a host-visible buffer when the ticket is ready and compares
//! the bytes. Some writers are slow, some abandon their slice, some slices are copied on
//! both queues, some are let go while the writer still has them (and must not be handed
//! out again before it lets go), and every retired range's handle must resolve to nothing.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ash::vk;

use crate::Error;
use crate::budget::UploadPhase;
use crate::buffer_pool::BufferPool;
use crate::device::Gpu;
use crate::frame::FrameCtx;
use crate::memory::{Buffer, MemoryUse};
use crate::pool::{PoolConfig, PoolError, RangeHandle};
use crate::reserve::{Reservation, ReservationState, StagingWriter};
use crate::textures::{Arrival, TextureDesc, TextureId, TextureTable};
use crate::upload::{
    Drained, StreamHandle, StreamOutcome, StreamProgress, UploadError, UploadRing, UploadTicket,
    Urgency,
};
use crate::users::{Completed, Users};

/// The streamed texture: RGBA8 with every mip level, larger than the ring.
const SIZE: u32 = 2048;
const MIPS: u32 = 12;
/// A texture small enough to stage whole next to the large one.
const SMALL_SIZE: u32 = 64;
/// One with mip levels that goes in a single batch (1.4 MiB).
const MEDIUM_SIZE: u32 = 512;
const MEDIUM_MIPS: u32 = 10;
/// Bytes staged by hand per step of the first upload.
const STEP: u64 = 512 << 10;
/// Bytes submitted, and bytes kept staged behind them, before the ring is made to wrap.
const PINNED: u64 = 6 << 20;
/// Drains per frame while a replacement streams as a loading burst.
const LOADING_DRAINS: usize = 3;
/// Frames a wait may take before it counts as stuck.
const PATIENCE: u32 = 2400;
/// Problems printed in detail; the rest are only counted.
const PRINTED: u64 = 12;

/// Blocks in flight at once, and the largest one.
const JOBS: usize = 24;
const JOB_BYTES: usize = 128 << 10;
/// Worker staging: room for every job at its largest and the part kept for urgent ones.
const WORKER_BYTES: u64 = 8 << 20;

fn pool_config() -> PoolConfig {
    PoolConfig {
        page_bytes: 1 << 20,
        budget_bytes: 6 << 20,
        low_watermark: 256 << 10,
        high_watermark: 3 << 19,
        release_after: 30,
        granule: 4,
    }
}

/// xorshift64*
fn next(rng: &mut u64) -> u64 {
    *rng ^= *rng >> 12;
    *rng ^= *rng << 25;
    *rng ^= *rng >> 27;
    rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Fills `out` with the bytes of `seed`.
fn pattern(seed: u64, out: &mut [u8]) {
    let mut x = seed | 1;
    let (words, tail) = out.as_chunks_mut::<8>();
    for word in words {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *word = x.to_le_bytes();
    }
    x ^= x << 13;
    let n = tail.len();
    tail.copy_from_slice(&x.to_le_bytes()[..n]);
}

/// The levels of a `size` × `size` RGBA8 texture, each filled from `seed`.
fn levels(size: u32, mips: u32, seed: u64) -> Vec<Vec<u8>> {
    (0..mips)
        .map(|m| {
            let side = (size >> m).max(1) as usize;
            let mut level = vec![0; side * side * 4];
            pattern(seed.wrapping_add(u64::from(m) * 0x9E37_79B9), &mut level);
            level
        })
        .collect()
}

/// Whether a deferred batch left a stream as it was: what is staged, what has been
/// submitted, the layout and the release. (Copies submitted earlier may finish meanwhile.)
fn unchanged(before: &StreamProgress, after: &StreamProgress) -> bool {
    (
        before.staged,
        before.submitted,
        before.committed,
        before.released,
        before.outcome,
    ) == (
        after.staged,
        after.submitted,
        after.committed,
        after.released,
        after.outcome,
    )
}

fn desc(size: u32, mips: u32) -> TextureDesc {
    TextureDesc {
        mip_levels: mips,
        ..TextureDesc::rgba8(size, size)
    }
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamCheckReport {
    /// Rounds finished, each with every reservation taken back at its end.
    pub rounds: u64,
    /// Whole read-backs of a texture compared, their bytes, and how many differed.
    pub readbacks: u64,
    pub readback_bytes: u64,
    pub mismatches: u64,
    /// Frames in which a slot showed a texture whose stream was not published.
    pub early_publications: u64,
    /// Deferrals of the first, a middle and the final batch that left the stream as it
    /// was.
    pub deferred_first: u64,
    pub deferred_middle: u64,
    pub deferred_final: u64,
    /// Steps with two submissions and a deferred batch between them.
    pub double_submits: u64,
    /// Times staging went around the end of the ring while bytes before it were held.
    pub wraps: u64,
    /// Replacements published, read-backs of the old generation while one streamed, and
    /// replacements cancelled part-way.
    pub replacements: u64,
    pub old_generation_reads: u64,
    pub cancelled_replacements: u64,
    /// Streams cancelled before the first submission, during streaming, and between
    /// the final submission and the acquire.
    pub cancelled_before: u64,
    pub cancelled_during: u64,
    pub cancelled_released: u64,
    /// Small textures that arrived intact from a batch another image was cancelled out
    /// of.
    pub survivors: u64,
    /// Blocks written by the worker, copied through pooled ranges and compared.
    pub blocks: u64,
    pub block_bytes: u64,
    pub block_mismatches: u64,
    /// Blocks copied on both queues, slices abandoned by the worker, slices let go while
    /// their writer still held them, and reservations and ranges refused for want of room.
    pub dual_blocks: u64,
    pub abandoned: u64,
    pub held_writers: u64,
    pub refused: u64,
    /// Retired handles that were checked to resolve to nothing.
    pub stale_handles: u64,
    /// Frames staged under the gameplay budget's time-derived ceiling.
    pub paced_frames: u64,
    /// Rules broken (each printed): a deferral that changed a stream, a sequence value
    /// that did not grow, tiles submitted twice, a slice reclaimed under its writer, a
    /// stale handle that resolved, a wait that never ended, something left reserved.
    pub violations: u64,
}

impl StreamCheckReport {
    /// What a run still lacks to count as passed, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        let seen = [
            (self.rounds, "a finished round"),
            (self.readbacks, "a read-back"),
            (self.deferred_first, "a deferred first batch"),
            (self.deferred_middle, "a deferred middle batch"),
            (self.deferred_final, "a deferred final batch"),
            (self.double_submits, "two submissions in one frame"),
            (self.wraps, "a wrap of the ring"),
            (self.replacements, "a replacement"),
            (self.old_generation_reads, "a read of the old generation"),
            (self.cancelled_replacements, "a cancelled replacement"),
            (self.cancelled_before, "a cancel before the first submit"),
            (self.cancelled_during, "a cancel during streaming"),
            (self.cancelled_released, "a cancel after the release"),
            (self.survivors, "a survivor of a cancelled batch"),
            (self.blocks, "a worker-written block"),
            (self.dual_blocks, "a block copied on both queues"),
            (self.abandoned, "an abandoned slice"),
            (self.held_writers, "a slice let go under its writer"),
            (self.stale_handles, "a stale handle"),
        ];
        seen.iter().find(|(n, _)| *n == 0).map(|&(_, what)| what)
    }

    pub fn passed(&self) -> bool {
        self.missing().is_none()
            && self.mismatches == 0
            && self.block_mismatches == 0
            && self.early_publications == 0
            && self.violations == 0
    }
}

/// What the worker thread is asked to do with a slice.
#[derive(Debug)]
enum Work {
    /// Fill it with the bytes of `seed` and finish, after a pause if `slow`.
    Write {
        writer: StagingWriter,
        seed: u64,
        slow: bool,
    },
    /// Write half of it and drop the writer.
    Abandon { writer: StagingWriter, seed: u64 },
    /// Drop the writer of a slice its owner has already let go.
    LetGo(StagingWriter),
}

fn worker(jobs: mpsc::Receiver<Work>) {
    let mut scratch = vec![0u8; JOB_BYTES];
    for work in jobs {
        match work {
            Work::Write {
                mut writer,
                seed,
                slow,
            } => {
                let len = writer.capacity();
                pattern(seed, &mut scratch[..len]);
                // In pieces, front to back, as a mesh is packed.
                for piece in scratch[..len].chunks(24 << 10) {
                    assert!(writer.push(piece), "the slice has the size it was granted");
                }
                if slow {
                    std::thread::sleep(Duration::from_millis(3));
                }
                writer.finish();
            }
            Work::Abandon { mut writer, seed } => {
                let len = writer.capacity() / 2;
                pattern(seed, &mut scratch[..len]);
                writer.push(&scratch[..len]);
            }
            Work::LetGo(writer) => drop(writer),
        }
    }
}

/// One block on its way through a pooled range.
#[derive(Debug, Default)]
enum Job {
    #[default]
    Idle,
    /// The worker has the slice.
    Writing {
        reservation: Reservation,
        range: RangeHandle,
        /// A second range the same bytes go to, on the other queue.
        second: Option<RangeHandle>,
        seed: u64,
        len: usize,
        urgency: Urgency,
        abandoned: bool,
    },
    /// Let go by the check while it still holds the writer itself.
    Held {
        reservation: Reservation,
        writer: Option<StagingWriter>,
        frames: u32,
    },
    /// Let go and handed to the worker to drop: gone once it has.
    Dropping {
        reservation: Reservation,
        frames: u32,
    },
    Uploading {
        range: RangeHandle,
        second: Option<RangeHandle>,
        tickets: [UploadTicket; 2],
        seed: u64,
        len: usize,
    },
    Reading {
        range: RangeHandle,
        second: Option<RangeHandle>,
        tickets: [UploadTicket; 2],
        seed: u64,
        len: usize,
        frame: u64,
    },
}

/// Which texture a read-back is of, and what it must hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subject {
    /// The large texture: the current generation's levels.
    Large,
    /// The small texture staged beside a cancelled one.
    Small,
}

/// A cancelled stream the check still watches.
#[derive(Debug)]
struct Watched {
    stream: StreamHandle,
    kind: Cancel,
    frames: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cancel {
    Before,
    During,
    Released,
    Replacement,
}

/// Where the round is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Create the large texture and start its stream, staged by hand.
    Begin,
    /// Defer the first batch, then submit it.
    First,
    /// Two submissions in one step with a deferred batch between them.
    Middle,
    /// Submit a part, keep a part staged behind it until the first is done, stage on
    /// around the end of the ring, then defer the batch with the last tiles. `mark` is
    /// the coverage the first part's submission reached.
    Bulk {
        stage: u8,
        mark: u64,
    },
    /// Wait for the stream to be published, then read the texture back.
    Arrive,
    /// Start a replacement that the ring streams by itself.
    Replace,
    /// While it streams: read the old generation once; when it shows, read the new one.
    Replacing {
        read_old: bool,
    },
    /// Start another replacement and cancel it once part of it is submitted.
    CancelReplacement {
        started: bool,
    },
    /// The old generation after the cancelled replacement.
    AfterCancelledReplacement,
    CancelBefore,
    /// Wait for the small texture staged beside the cancelled one.
    Survivor,
    CancelDuring {
        started: bool,
    },
    CancelReleased {
        started: bool,
    },
    /// Let go of everything and wait until nothing is reserved.
    Quiesce,
}

/// See the module docs.
pub struct StreamCheck {
    gpu: Arc<Gpu>,
    rng: u64,
    report: StreamCheckReport,
    phase: Phase,
    /// Frames spent waiting in the current phase.
    waited: u32,
    round: u64,
    /// The large texture, and the levels its current generation holds.
    large: Option<TextureId>,
    current: Vec<Vec<u8>>,
    /// The levels of the generation that is streaming.
    incoming: Option<Vec<Vec<u8>>>,
    /// The stream being followed: the texture, its generation when the stream started,
    /// and whether it is a first upload.
    followed: Option<(StreamHandle, TextureId, u32, bool)>,
    /// The ring's count of tile bytes submitted when the followed first upload started.
    submitted_at_start: u64,
    /// The last sequence value a drain reported.
    last_value: u64,
    /// The texture of the cancel scenario in progress.
    other: Option<TextureId>,
    small: Option<TextureId>,
    small_pixels: Vec<u8>,
    watched: Vec<Watched>,
    readback: Option<Buffer>,
    /// The frame that copies a texture into the read-back buffer, what it is a copy of,
    /// and whether it is the old generation read while a replacement streams.
    reading: Option<(u64, Subject, bool)>,
    last_step: Instant,
    pool: Option<BufferPool>,
    jobs: [Job; JOBS],
    job_readback: Option<Buffer>,
    /// Handles of ranges retired a while ago: they must stay unresolvable.
    stale: [Option<RangeHandle>; 16],
    stale_next: usize,
    /// No new blocks are started (the round is ending).
    quiet: bool,
    sender: Option<mpsc::Sender<Work>>,
    thread: Option<JoinHandle<()>>,
    scratch: Vec<u8>,
    /// A reservation was refused for want of room while an urgent one still got some.
    pub(crate) limit_hits: u64,
}

impl std::fmt::Debug for StreamCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamCheck")
            .field("phase", &self.phase)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

// SAFETY: the read-back mappings are only read through &mut self; handles may move
// threads.
unsafe impl Send for StreamCheck {}

fn large_bytes() -> u64 {
    (0..MIPS)
        .map(|m| u64::from((SIZE >> m).max(1)).pow(2) * 4)
        .sum()
}

impl StreamCheck {
    /// Creates the check's buffers and its worker thread, and turns on worker staging in
    /// `ring`.
    pub fn new(gpu: Arc<Gpu>, ring: &mut UploadRing) -> Result<StreamCheck, Error> {
        ring.enable_worker_staging(WORKER_BYTES)?;
        let readback = Buffer::new(
            &gpu,
            large_bytes(),
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        )?;
        let job_readback = match Buffer::new(
            &gpu,
            (JOBS * 2 * JOB_BYTES) as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: unused.
                unsafe { readback.destroy(&gpu) };
                return Err(e);
            }
        };
        let pool = BufferPool::new(
            gpu.clone(),
            pool_config(),
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
        );
        let (sender, jobs) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("stream-check".into())
            .spawn(move || worker(jobs));
        let thread = match thread {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: unused.
                unsafe {
                    readback.destroy(&gpu);
                    job_readback.destroy(&gpu);
                }
                return Err(Error::Unsupported(format!(
                    "no thread for the streaming check: {e}"
                )));
            }
        };
        Ok(StreamCheck {
            gpu,
            rng: 0x9E37_79B9_7F4A_7C15,
            report: StreamCheckReport::default(),
            phase: Phase::Begin,
            waited: 0,
            round: 0,
            large: None,
            current: Vec::new(),
            incoming: None,
            followed: None,
            submitted_at_start: 0,
            last_value: 0,
            other: None,
            small: None,
            small_pixels: Vec::new(),
            watched: Vec::new(),
            readback: Some(readback),
            reading: None,
            last_step: Instant::now(),
            pool: Some(pool),
            jobs: std::array::from_fn(|_| Job::Idle),
            job_readback: Some(job_readback),
            stale: [None; 16],
            stale_next: 0,
            quiet: false,
            sender: Some(sender),
            thread: Some(thread),
            scratch: vec![0; JOB_BYTES],
            limit_hits: 0,
        })
    }

    pub fn report(&self) -> StreamCheckReport {
        self.report
    }

    /// Times a reservation was refused for want of room while an urgent one still got
    /// some.
    pub fn limit_hits(&self) -> u64 {
        self.limit_hits
    }

    /// The pool the blocks go through, for its counters.
    pub fn pool_stats(&self) -> crate::pool::PoolStats {
        self.pool.as_ref().expect("live").stats()
    }

    fn violation(&mut self, what: std::fmt::Arguments<'_>) {
        if self.report.violations < PRINTED {
            eprintln!(
                "[stream-check] {what} (round {}, {:?})",
                self.round, self.phase
            );
        }
        self.report.violations += 1;
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.waited = 0;
    }

    /// Counts a frame of waiting; `false` once the wait has taken too long (said once).
    fn wait(&mut self, what: &str) -> bool {
        self.waited += 1;
        if self.waited == PATIENCE {
            self.violation(format_args!("{what} did not happen in {PATIENCE} frames"));
        }
        self.waited < PATIENCE
    }

    /// A drain's result: sequence values must grow.
    fn seen(&mut self, drained: Drained) -> Drained {
        if let Drained::Submitted(value) = drained {
            let last = self.last_value;
            if value <= last {
                self.violation(format_args!(
                    "a submission took sequence value {value} after {last}"
                ));
            }
            self.last_value = value;
        }
        drained
    }

    /// One frame of the check. Call it after `ring.record(ctx, …)` and
    /// `table.begin_frame` for the frame, before its passes.
    ///
    /// # Safety
    /// `ctx.cmd` is recording frame `ctx.number` on the graphics queue, on the thread that
    /// submits frames; `completed` is a frame the GPU has finished; `ring` is the ring
    /// recorded into this frame and `table` the table begun for it.
    pub unsafe fn step(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        ctx: &FrameCtx,
        completed: u64,
    ) -> Result<(), Error> {
        profiling::scope!("stream_check");
        // SAFETY: forwarded from the caller.
        unsafe {
            self.step_jobs(ring, ctx, completed);
            self.step_texture(ring, table, ctx, completed)?;
        }
        self.last_step = Instant::now();
        Ok(())
    }

    /// Watches the followed stream and the cancelled ones: nothing may show early, and a
    /// cancelled stream must end cancelled, with the users its kind implies.
    fn watch(&mut self, table: &TextureTable) {
        if let Some((stream, id, generation, first)) = &self.followed {
            let published = stream.progress().outcome == StreamOutcome::Published;
            let visible = if *first {
                table.is_ready(*id) || table.shows(id.index())
            } else {
                table.generation(*id) != *generation
            };
            if visible && !published {
                if self.report.early_publications < PRINTED {
                    eprintln!(
                        "[stream-check] {id:?} shows a stream that is not published: {:?}",
                        stream.progress()
                    );
                }
                self.report.early_publications += 1;
            }
        }
        let mut i = 0;
        while i < self.watched.len() {
            let p = self.watched[i].stream.progress();
            let kind = self.watched[i].kind;
            match p.outcome {
                StreamOutcome::Streaming => {
                    self.watched[i].frames += 1;
                    if self.watched[i].frames == PATIENCE {
                        self.violation(format_args!("a cancelled stream was never let go: {p:?}"));
                        self.watched.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
                StreamOutcome::Published => {
                    self.violation(format_args!("a cancelled stream was published: {p:?}"));
                    self.watched.swap_remove(i);
                }
                StreamOutcome::Cancelled(users) => {
                    let written = users.copy_batch > 0;
                    let as_expected = match kind {
                        Cancel::Before => !written && p.submitted == 0 && !p.committed,
                        Cancel::During => {
                            written && p.submitted > 0 && p.submitted < p.total && !p.released
                        }
                        Cancel::Released => written && p.submitted == p.total && p.released,
                        Cancel::Replacement => written && p.submitted > 0,
                    };
                    if as_expected {
                        match kind {
                            Cancel::Before => self.report.cancelled_before += 1,
                            Cancel::During => self.report.cancelled_during += 1,
                            Cancel::Released => self.report.cancelled_released += 1,
                            Cancel::Replacement => self.report.cancelled_replacements += 1,
                        }
                    } else {
                        self.violation(format_args!("a stream cancelled {kind:?} ended as {p:?}"));
                    }
                    self.watched.swap_remove(i);
                }
            }
        }
    }

    /// Compares a read-back that has finished. `true` while one is still on its way.
    fn compare(&mut self, table: &mut TextureTable, completed: u64) -> bool {
        let Some((frame, subject, old)) = self.reading else {
            return false;
        };
        if frame > completed {
            return true;
        }
        self.reading = None;
        let mapped = self.readback.as_ref().expect("live").memory.mapped;
        let small = [match subject {
            Subject::Small => std::mem::take(&mut self.small_pixels),
            Subject::Large => Vec::new(),
        }];
        let expected: &[Vec<u8>] = match subject {
            Subject::Large => &self.current,
            Subject::Small => &small,
        };
        let mut at = 0;
        let mut differs = None;
        for (level, want) in expected.iter().enumerate() {
            // SAFETY: the copy into it finished with its frame; host-coherent memory
            // nothing else writes; the buffer holds every level of the largest texture.
            let got = unsafe { std::slice::from_raw_parts(mapped.add(at), want.len()) };
            if got != &want[..] && differs.is_none() {
                differs = Some((level, got.iter().zip(want).position(|(a, b)| a != b)));
            }
            at += want.len();
        }
        self.report.readbacks += 1;
        self.report.readback_bytes += at as u64;
        match differs {
            Some((level, byte)) => {
                if self.report.mismatches < PRINTED {
                    eprintln!(
                        "[stream-check] {subject:?} texture read back in frame {frame} differs in level {level} (first at byte {byte:?})"
                    );
                }
                self.report.mismatches += 1;
            }
            None if subject == Subject::Small => self.report.survivors += 1,
            None if old => self.report.old_generation_reads += 1,
            None => {}
        }
        if subject == Subject::Small
            && let Some(id) = self.small.take()
        {
            table.destroy(id);
        }
        false
    }

    /// Creates a texture and starts its stream. `None`, with nothing left behind, if the
    /// table or the ring has no room now.
    fn start(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        desc: TextureDesc,
        mips: Vec<Vec<u8>>,
    ) -> Result<Option<(TextureId, StreamHandle)>, Error> {
        let id = match table.create(desc) {
            Ok(id) => id,
            Err(e) if e.try_again() => return Ok(None),
            Err(e) => return Err(e),
        };
        match table.stream(ring, id, mips) {
            Ok(()) => {
                let stream = table.stream_of(id).expect("just started");
                Ok(Some((id, stream)))
            }
            Err(UploadError::Full) => {
                table.destroy(id);
                Ok(None)
            }
            Err(e) => {
                table.destroy(id);
                Err(Error::Upload(e))
            }
        }
    }

    /// The texture half of [`Self::step`].
    ///
    /// # Safety
    /// As [`Self::step`].
    unsafe fn step_texture(
        &mut self,
        ring: &mut UploadRing,
        table: &mut TextureTable,
        ctx: &FrameCtx,
        completed: u64,
    ) -> Result<(), Error> {
        self.watch(table);
        if self.compare(table, completed) {
            return Ok(());
        }
        let total = large_bytes();
        match self.phase {
            Phase::Begin => {
                ring.hold_streams(true);
                ring.set_phase(UploadPhase::Gameplay);
                let seed = next(&mut self.rng);
                let pixels = levels(SIZE, MIPS, seed);
                // As any background texture is created: too large to stage at once, it
                // streams.
                let borrowed: Vec<&[u8]> = pixels.iter().map(Vec::as_slice).collect();
                let created =
                    table.create_with(ring, desc(SIZE, MIPS), &borrowed, Arrival::Background);
                let id = match created {
                    Ok(id) => id,
                    Err(e) if e.try_again() => {
                        self.wait("room to start the large texture");
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                let Some(stream) = table.stream_of(id) else {
                    self.violation(format_args!(
                        "a texture larger than the ring did not start to stream"
                    ));
                    table.destroy(id);
                    return Ok(());
                };
                self.current = pixels;
                self.large = Some(id);
                self.followed = Some((stream, id, 0, true));
                self.submitted_at_start = ring.stats().tile_bytes_submitted;
                self.round += 1;
                self.enter(Phase::First);
            }
            Phase::First => {
                let stream = self.followed.as_ref().expect("followed").0.clone();
                if stream.progress().staged == 0 {
                    if ring.pump_stream(&stream, STEP) == 0 {
                        self.wait("room for the first tiles");
                        return Ok(());
                    }
                    // The first batch waits: nothing about the image may change.
                    let before = stream.progress();
                    ring.force_next_deferral();
                    // SAFETY: forwarded from the caller; every destination is alive.
                    let drained = unsafe { ring.drain(completed)? };
                    let after = stream.progress();
                    if drained == Drained::Deferred
                        && unchanged(&before, &after)
                        && !after.committed
                        && after.submitted == 0
                    {
                        self.report.deferred_first += 1;
                    } else {
                        self.violation(format_args!(
                            "deferring the first batch gave {drained:?} and changed {before:?} to {after:?}"
                        ));
                    }
                }
                // The retry: the same tiles, now with a value.
                // SAFETY: as above.
                let drained = unsafe { ring.drain(completed)? };
                self.seen(drained);
                let p = stream.progress();
                if p.submitted > 0 {
                    if !p.committed || p.submitted != p.staged {
                        self.violation(format_args!("after the first submission: {p:?}"));
                    }
                    self.enter(Phase::Middle);
                } else {
                    self.wait("the first submission");
                }
            }
            Phase::Middle => {
                let stream = self.followed.as_ref().expect("followed").0.clone();
                if stream.progress().staged + 4 * STEP > total {
                    // Out of tiles before a step managed it: on with the round.
                    self.enter(Phase::Bulk { stage: 0, mark: 0 });
                    return Ok(());
                }
                ring.pump_stream(&stream, STEP);
                // SAFETY: forwarded from the caller; every destination is alive.
                let first = unsafe { ring.drain(completed)? };
                if !matches!(self.seen(first), Drained::Submitted(_)) {
                    self.wait("a free command buffer");
                    return Ok(());
                }
                if ring.pump_stream(&stream, STEP) == 0 {
                    self.wait("room for a middle tile");
                    return Ok(());
                }
                let before = stream.progress();
                ring.force_next_deferral();
                // SAFETY: as above.
                let deferred = unsafe { ring.drain(completed)? };
                let after = stream.progress();
                if deferred != Drained::Deferred || !unchanged(&before, &after) || !after.committed
                {
                    self.violation(format_args!(
                        "deferring a middle batch gave {deferred:?} and changed {before:?} to {after:?}"
                    ));
                    self.enter(Phase::Bulk { stage: 0, mark: 0 });
                    return Ok(());
                }
                self.report.deferred_middle += 1;
                // More tiles join the waiting batch; all of it goes once.
                ring.pump_stream(&stream, STEP);
                // SAFETY: as above.
                let second = unsafe { ring.drain(completed)? };
                if matches!(self.seen(second), Drained::Submitted(_)) {
                    // Two submissions since the last present, a deferred batch between.
                    self.report.double_submits += 1;
                    self.enter(Phase::Bulk { stage: 0, mark: 0 });
                } else {
                    self.wait("a second free command buffer");
                }
            }
            Phase::Bulk { stage: 0, .. } => {
                let stream = self.followed.as_ref().expect("followed").0.clone();
                // From here to the last tiles only this check submits: what a frame
                // stages stays reserved, so the ring cannot empty and start over.
                ring.force_deferral_every(1);
                let p = stream.progress();
                if p.submitted == p.staged {
                    ring.pump_stream(&stream, PINNED);
                }
                // SAFETY: forwarded from the caller; every destination is alive.
                let drained = unsafe { ring.drain(completed)? };
                if !matches!(self.seen(drained), Drained::Submitted(_)) {
                    self.wait("a free command buffer");
                    return Ok(());
                }
                let mark = stream.progress().submitted;
                // More behind it, staged and in no submission: it holds its bytes.
                ring.pump_stream(&stream, PINNED);
                self.enter(Phase::Bulk { stage: 1, mark });
            }
            Phase::Bulk { stage: 1, mark } => {
                let stream = self.followed.as_ref().expect("followed").0.clone();
                let p = stream.progress();
                if p.completed < mark {
                    self.wait("the bulk of the large texture");
                    return Ok(());
                }
                // The submitted part is done and its bytes are free; the staged part
                // behind it still holds the middle of the ring. Staging on runs into the
                // end of the buffer and goes on from its start: a wrap.
                let wraps = ring.stats().ring_wraps;
                ring.pump_stream(&stream, (total - p.staged).saturating_sub(STEP));
                if ring.stats().ring_wraps > wraps {
                    self.report.wraps += 1;
                }
                self.enter(Phase::Bulk { stage: 2, mark });
            }
            Phase::Bulk { mark, .. } => {
                let stream = self.followed.as_ref().expect("followed").0.clone();
                let p = stream.progress();
                if p.submitted < p.staged {
                    // SAFETY: forwarded from the caller; every destination is alive.
                    let drained = unsafe { ring.drain(completed)? };
                    self.seen(drained);
                    self.wait("a free command buffer");
                    return Ok(());
                }
                let left = total - p.staged;
                if left > 2 * STEP {
                    // There was no room for all of it: on as it comes back.
                    ring.pump_stream(&stream, left - STEP);
                    self.phase = Phase::Bulk { stage: 2, mark };
                    self.wait("room for the rest of the large texture");
                    return Ok(());
                }
                ring.pump_stream(&stream, left);
                let before = stream.progress();
                if before.staged < total {
                    self.wait("room for the last tiles");
                    return Ok(());
                }
                // The batch with the last tiles waits: no release, no ticket.
                ring.force_next_deferral();
                // SAFETY: as above.
                let drained = unsafe { ring.drain(completed)? };
                let after = stream.progress();
                if drained == Drained::Deferred && unchanged(&before, &after) && !after.released {
                    self.report.deferred_final += 1;
                } else {
                    self.violation(format_args!(
                        "deferring the final batch gave {drained:?} and changed {before:?} to {after:?}"
                    ));
                }
                // The next frame submits it.
                ring.force_deferral_every(0);
                ring.hold_streams(false);
                self.enter(Phase::Arrive);
            }
            Phase::Arrive => {
                let id = self.large.expect("large");
                if !table.is_ready(id) {
                    self.wait("the large texture's publication");
                    return Ok(());
                }
                // Whatever was deferred and retried, every tile went to the queue once.
                let went = ring.stats().tile_bytes_submitted - self.submitted_at_start;
                if went != total {
                    self.violation(format_args!(
                        "{went} bytes of tiles went to the queue for a texture of {total}"
                    ));
                }
                self.followed = None;
                // SAFETY: forwarded from the caller; the texture is ready.
                unsafe { self.read_texture(table, id, Subject::Large, false, ctx) };
                self.enter(Phase::Replace);
            }
            Phase::Replace => {
                let id = self.large.expect("large");
                // By the ring itself: as a loading burst in every other round.
                ring.hold_streams(false);
                ring.set_phase(if self.round.is_multiple_of(2) {
                    UploadPhase::Gameplay
                } else {
                    UploadPhase::Loading
                });
                let seed = next(&mut self.rng);
                let pixels = levels(SIZE, MIPS, seed);
                match table.replace_streamed(ring, id, desc(SIZE, MIPS), pixels.clone()) {
                    Ok(()) => {}
                    Err(e) if e.try_again() => {
                        self.wait("room to start a replacement");
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
                self.incoming = Some(pixels);
                let stream = table.stream_of(id).expect("just started");
                self.followed = Some((stream, id, table.generation(id), false));
                self.enter(Phase::Replacing { read_old: false });
            }
            Phase::Replacing { read_old } => {
                let id = self.large.expect("large");
                let (stream, _, generation, _) = self.followed.clone().expect("followed");
                if ring.phase() == UploadPhase::Loading {
                    // Batches drained between presents, as behind a loading screen.
                    for _ in 0..LOADING_DRAINS {
                        // SAFETY: forwarded from the caller; every destination is alive.
                        let drained = unsafe { ring.drain(completed)? };
                        self.seen(drained);
                    }
                }
                if table.generation(id) != generation {
                    if let Some(pixels) = self.incoming.take() {
                        self.current = pixels;
                    }
                    self.report.replacements += 1;
                    self.followed = None;
                    ring.set_phase(UploadPhase::Gameplay);
                    // SAFETY: forwarded from the caller; the texture is ready.
                    unsafe { self.read_texture(table, id, Subject::Large, false, ctx) };
                    self.enter(Phase::CancelReplacement { started: false });
                } else if !read_old && stream.progress().submitted >= total / 2 {
                    // Half of the next generation is on the GPU: the old one is still
                    // what the slot shows, whole.
                    // SAFETY: as above.
                    unsafe { self.read_texture(table, id, Subject::Large, true, ctx) };
                    self.phase = Phase::Replacing { read_old: true };
                } else {
                    self.wait("the replacement's publication");
                }
            }
            Phase::CancelReplacement { started: false } => {
                let id = self.large.expect("large");
                let seed = next(&mut self.rng);
                match table.replace_streamed(ring, id, desc(SIZE, MIPS), levels(SIZE, MIPS, seed)) {
                    Ok(()) => {}
                    Err(e) if e.try_again() => {
                        self.wait("room to start a replacement");
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
                let stream = table.stream_of(id).expect("just started");
                self.followed = Some((stream, id, table.generation(id), false));
                ring.pace(self.last_step.elapsed(), None, None);
                self.enter(Phase::CancelReplacement { started: true });
            }
            Phase::CancelReplacement { started: true } => {
                let id = self.large.expect("large");
                let (stream, ..) = self.followed.clone().expect("followed");
                // Under the gameplay budget's time-derived ceiling.
                ring.pace(self.last_step.elapsed(), None, None);
                self.report.paced_frames += 1;
                if stream.progress().submitted == 0 || self.waited < 6 {
                    self.wait("the replacement's first submissions");
                    return Ok(());
                }
                table.cancel_replacement(id);
                self.followed = None;
                self.watched.push(Watched {
                    stream,
                    kind: Cancel::Replacement,
                    frames: 0,
                });
                self.enter(Phase::AfterCancelledReplacement);
            }
            Phase::AfterCancelledReplacement => {
                let id = self.large.expect("large");
                // SAFETY: forwarded from the caller; the texture is ready.
                unsafe { self.read_texture(table, id, Subject::Large, false, ctx) };
                self.enter(Phase::CancelBefore);
            }
            Phase::CancelBefore => {
                ring.hold_streams(true);
                let seed = next(&mut self.rng);
                let Some((id, stream)) =
                    self.start(ring, table, desc(SIZE, MIPS), levels(SIZE, MIPS, seed))?
                else {
                    self.wait("room to start a texture");
                    return Ok(());
                };
                let staged = ring.pump_stream(&stream, STEP);
                // A small texture in the same batch, which must arrive all the same.
                let mut pixels = vec![0; (SMALL_SIZE * SMALL_SIZE * 4) as usize];
                pattern(next(&mut self.rng), &mut pixels);
                let small = self.start(ring, table, desc(SMALL_SIZE, 1), vec![pixels.clone()])?;
                let beside = match &small {
                    Some((_, s)) => ring.pump_stream(s, u64::MAX),
                    None => 0,
                };
                // Cancelled before anything of it was submitted.
                table.destroy(id);
                match small {
                    Some((small_id, small_stream)) if staged > 0 && beside > 0 => {
                        self.watched.push(Watched {
                            stream,
                            kind: Cancel::Before,
                            frames: 0,
                        });
                        self.small = Some(small_id);
                        self.small_pixels = pixels;
                        self.followed = Some((small_stream, small_id, 0, true));
                        self.enter(Phase::Survivor);
                    }
                    Some((small_id, _)) => {
                        // The ring had no room for both: try again in a later frame.
                        table.destroy(small_id);
                        self.wait("room for the tiles of two textures");
                    }
                    None => {
                        self.wait("room for the small texture");
                    }
                }
            }
            Phase::Survivor => {
                let id = self.small.expect("small");
                if !table.is_ready(id) {
                    self.wait("the small texture's publication");
                    return Ok(());
                }
                self.followed = None;
                // SAFETY: forwarded from the caller; the texture is ready.
                unsafe { self.read_texture(table, id, Subject::Small, false, ctx) };
                self.enter(Phase::CancelDuring { started: false });
            }
            Phase::CancelDuring { started: false } => {
                ring.hold_streams(true);
                let seed = next(&mut self.rng);
                let Some((id, stream)) =
                    self.start(ring, table, desc(SIZE, MIPS), levels(SIZE, MIPS, seed))?
                else {
                    self.wait("room to start a texture");
                    return Ok(());
                };
                self.other = Some(id);
                self.followed = Some((stream, id, 0, true));
                self.enter(Phase::CancelDuring { started: true });
            }
            Phase::CancelDuring { started: true } => {
                let id = self.other.expect("other");
                let (stream, ..) = self.followed.clone().expect("followed");
                if stream.progress().submitted == 0 {
                    if stream.progress().staged == 0 {
                        ring.pump_stream(&stream, 4 * STEP);
                    }
                    // SAFETY: forwarded from the caller; every destination is alive.
                    let drained = unsafe { ring.drain(completed)? };
                    self.seen(drained);
                    self.wait("the first submission of a texture to cancel");
                    return Ok(());
                }
                // Part of it is submitted, more is staged: cancelled in mid-stream.
                if ring.pump_stream(&stream, STEP) == 0 {
                    self.wait("room for a tile to drop");
                    return Ok(());
                }
                table.destroy(id);
                self.other = None;
                self.followed = None;
                self.watched.push(Watched {
                    stream,
                    kind: Cancel::During,
                    frames: 0,
                });
                self.enter(Phase::CancelReleased { started: false });
            }
            Phase::CancelReleased { started: false } => {
                ring.hold_streams(true);
                let seed = next(&mut self.rng);
                let Some((id, stream)) = self.start(
                    ring,
                    table,
                    desc(MEDIUM_SIZE, MEDIUM_MIPS),
                    levels(MEDIUM_SIZE, MEDIUM_MIPS, seed),
                )?
                else {
                    self.wait("room to start a texture");
                    return Ok(());
                };
                self.other = Some(id);
                self.followed = Some((stream, id, 0, true));
                self.enter(Phase::CancelReleased { started: true });
            }
            Phase::CancelReleased { started: true } => {
                let id = self.other.expect("other");
                let (stream, ..) = self.followed.clone().expect("followed");
                if stream.progress().staged < stream.progress().total {
                    ring.pump_stream(&stream, u64::MAX);
                }
                // SAFETY: forwarded from the caller; every destination is alive.
                let drained = unsafe { ring.drain(completed)? };
                self.seen(drained);
                let p = stream.progress();
                self.other = None;
                self.followed = None;
                if p.released && p.outcome == StreamOutcome::Streaming {
                    // Its last tiles and the release are submitted; no frame has
                    // acquired it. Cancelled here, none ever does.
                    table.destroy(id);
                    self.watched.push(Watched {
                        stream,
                        kind: Cancel::Released,
                        frames: 0,
                    });
                    self.enter(Phase::Quiesce);
                } else if p.outcome == StreamOutcome::Published {
                    // A frame got in between: an ordinary texture by now. Once more.
                    table.destroy(id);
                    self.enter(Phase::CancelReleased { started: false });
                } else {
                    self.other = Some(id);
                    self.followed = Some((stream, id, 0, true));
                    self.wait("the final submission of a texture to cancel");
                }
            }
            Phase::Quiesce => {
                if self.waited == 0 {
                    self.quiet = true;
                    ring.hold_streams(false);
                    if let Some(id) = self.large.take() {
                        table.destroy(id);
                    }
                    self.current = Vec::new();
                    self.burst(ring);
                }
                let s = ring.stats();
                let pool = self.pool_stats();
                let idle = self.jobs.iter().all(|j| matches!(j, Job::Idle));
                let reclaimed = idle
                    && self.watched.is_empty()
                    && s.ring_occupied == 0
                    && s.reservations_outstanding == 0
                    && s.worker_bytes == 0
                    && s.streams_begun == s.streams_published + s.streams_cancelled
                    && pool.live == 0
                    && pool.retiring == 0
                    && table.stats().retiring == 0;
                if reclaimed {
                    self.report.rounds += 1;
                    self.quiet = false;
                    self.enter(Phase::Begin);
                } else if !self.wait("everything to be taken back") {
                    eprintln!("[stream-check] still held: {s:?}; pool {pool:?}");
                    self.quiet = false;
                    self.enter(Phase::Begin);
                }
            }
        }
        Ok(())
    }

    /// Reserves ordinary slices until the ring refuses, checks that an urgent one still
    /// gets room, and gives everything back unwritten.
    fn burst(&mut self, ring: &mut UploadRing) {
        let mut taken: Vec<(Reservation, StagingWriter)> = Vec::new();
        let refused = loop {
            match ring.reserve(256 << 10, Urgency::Normal) {
                Ok(slice) => taken.push(slice),
                Err(e) => break e,
            }
            if taken.len() > 256 {
                break UploadError::TooLarge;
            }
        };
        match (refused, ring.reserve(64 << 10, Urgency::Edit)) {
            (UploadError::Full, Ok(urgent)) => {
                self.limit_hits += 1;
                taken.push(urgent);
            }
            (refused, urgent) => self.violation(format_args!(
                "after {} ordinary reservations: {refused:?}, then an urgent one: {:?}",
                taken.len(),
                urgent.map(|(r, _)| r)
            )),
        }
        for (reservation, writer) in taken {
            // Dropped unwritten: abandoned, and reclaimed once the owner lets go too.
            drop(writer);
            ring.release(reservation);
        }
    }

    /// Records a copy of every mip level of the texture into the read-back buffer.
    ///
    /// # Safety
    /// `ctx.cmd` is recording on the graphics queue; the texture is ready and its texels
    /// are visible to this frame; no read-back is on its way.
    unsafe fn read_texture(
        &mut self,
        table: &TextureTable,
        id: TextureId,
        subject: Subject,
        old: bool,
        ctx: &FrameCtx,
    ) {
        debug_assert!(self.reading.is_none(), "one read-back at a time");
        let (image, desc) = table.image(id);
        let readback = self.readback.as_ref().expect("live").raw;
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: desc.mip_levels,
            base_array_layer: 0,
            layer_count: 1,
        };
        let shaders = vk::PipelineStageFlags2::VERTEX_SHADER
            | vk::PipelineStageFlags2::FRAGMENT_SHADER
            | vk::PipelineStageFlags2::COMPUTE_SHADER;
        // After the ring's copies (COPY) and its or the acquire's transition (shaders).
        let before = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY | shaders)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(image)
            .subresource_range(range)];
        let after_image = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(shaders)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(range)];
        let after_host = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)];
        let mut copies = [vk::BufferImageCopy::default(); MIPS as usize];
        let mut at = 0;
        for (m, copy) in copies.iter_mut().enumerate().take(desc.mip_levels as usize) {
            let (width, height) = ((desc.width >> m).max(1), (desc.height >> m).max(1));
            *copy = vk::BufferImageCopy {
                buffer_offset: at,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: m as u32,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D::default(),
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };
            at += u64::from(width) * u64::from(height) * 4;
        }
        let gpu = &*self.gpu;
        // SAFETY: guaranteed by the caller; the read-back buffer holds every level of the
        // largest texture and is not in use (the last read-back was compared).
        unsafe {
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&before),
            );
            gpu.device.cmd_copy_image_to_buffer(
                ctx.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback,
                &copies[..desc.mip_levels as usize],
            );
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default()
                    .memory_barriers(&after_host)
                    .image_memory_barriers(&after_image),
            );
        }
        self.reading = Some((ctx.number, subject, old));
    }

    /// Retires a range and checks that its handle is gone from then on.
    fn retire(&mut self, range: RangeHandle, users: Users) {
        let pool = self.pool.as_mut().expect("live");
        let retired = pool.retire(range, users);
        if !retired || pool.resolve(range).is_some() {
            self.violation(format_args!(
                "retiring {range:?} gave {retired} and it still resolves"
            ));
            return;
        }
        self.report.stale_handles += 1;
        self.stale[self.stale_next % self.stale.len()] = Some(range);
        self.stale_next += 1;
    }

    /// The worker-staging and pool half of [`Self::step`].
    ///
    /// # Safety
    /// As [`Self::step`].
    unsafe fn step_jobs(&mut self, ring: &mut UploadRing, ctx: &FrameCtx, completed: u64) {
        let done = Completed {
            frame: completed,
            ..ring.completed()
        };
        self.pool.as_mut().expect("live").maintain(&done);
        // A handle retired long ago names nothing, whatever took its place since.
        for i in 0..self.stale.len() {
            if let Some(h) = self.stale[i]
                && self.pool.as_ref().expect("live").resolve(h).is_some()
            {
                self.violation(format_args!("the stale handle {h:?} resolves again"));
                self.stale[i] = None;
            }
        }
        let readback = self.job_readback.as_ref().expect("live");
        let (readback_raw, mapped) = (readback.raw, readback.memory.mapped);
        let mut barrier_open = false;
        let mut started = 0;
        for i in 0..JOBS {
            let regions = [i * 2 * JOB_BYTES, (i * 2 + 1) * JOB_BYTES];
            match std::mem::take(&mut self.jobs[i]) {
                Job::Reading {
                    range,
                    second,
                    tickets,
                    seed,
                    len,
                    frame,
                } if frame <= completed => {
                    pattern(seed, &mut self.scratch[..len]);
                    for &at in &regions[..1 + usize::from(second.is_some())] {
                        // SAFETY: the copy into this region finished with its frame; the
                        // memory is host-coherent and nothing writes it until the job's
                        // place is used again.
                        let got = unsafe { std::slice::from_raw_parts(mapped.add(at), len) };
                        if got != &self.scratch[..len] {
                            if self.report.block_mismatches < PRINTED {
                                let first = got.iter().zip(&self.scratch).position(|(a, b)| a != b);
                                eprintln!(
                                    "[stream-check] block {i}: {len} bytes from frame {frame} differ (first at byte {first:?})"
                                );
                            }
                            self.report.block_mismatches += 1;
                        }
                    }
                    self.report.blocks += 1;
                    self.report.block_bytes += len as u64;
                    self.report.dual_blocks += u64::from(second.is_some());
                    // The frame that read it and the batches that wrote it, on whichever
                    // queue: the range is free once all of them are done.
                    let users = Users::until_frame(frame)
                        .and(ring.users_of(tickets[0]))
                        .and(ring.users_of(tickets[1]));
                    self.retire(range, users);
                    if let Some(second) = second {
                        self.retire(second, users);
                    }
                }
                Job::Uploading {
                    range,
                    second,
                    tickets,
                    seed,
                    len,
                } if ring.is_ready(tickets[0]) && ring.is_ready(tickets[1]) => {
                    let gpu = &*self.gpu;
                    if !barrier_open {
                        // The ring's own barrier (frame copies) covers shader and vertex
                        // reads, not copies; for the copy queue's batches the frame's
                        // semaphore wait covers everything.
                        let before = [vk::MemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::COPY)
                            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)];
                        // SAFETY: guaranteed by the caller.
                        unsafe {
                            gpu.cmd_pipeline_barrier2(
                                ctx.cmd,
                                &vk::DependencyInfo::default().memory_barriers(&before),
                            )
                        };
                        barrier_open = true;
                    }
                    let pool = self.pool.as_ref().expect("live");
                    for (handle, at) in [Some(range), second].into_iter().flatten().zip(regions) {
                        let Some(r) = pool.resolve(handle) else {
                            continue;
                        };
                        let copy = [vk::BufferCopy {
                            src_offset: r.offset,
                            dst_offset: at as u64,
                            size: len as u64,
                        }];
                        // SAFETY: guaranteed by the caller; the range holds the block
                        // (its tickets are ready) and its region of the read-back buffer
                        // is this job's alone.
                        unsafe {
                            gpu.device
                                .cmd_copy_buffer(ctx.cmd, r.buffer, readback_raw, &copy)
                        };
                    }
                    self.jobs[i] = Job::Reading {
                        range,
                        second,
                        tickets,
                        seed,
                        len,
                        frame: ctx.number,
                    };
                }
                Job::Writing {
                    reservation,
                    range,
                    second,
                    seed,
                    len,
                    urgency,
                    abandoned,
                } => match ring.reservation(reservation) {
                    ReservationState::Complete { written } => {
                        if abandoned || written != len as u64 {
                            self.violation(format_args!(
                                "a slice of {len} bytes came back with {written} written"
                            ));
                        }
                        let pool = self.pool.as_ref().expect("live");
                        let first = pool.resolve(range).expect("live range");
                        match ring.copy_reserved(reservation, first.buffer, first.offset, urgency) {
                            Ok(ticket) => {
                                // The same bytes once more, on the other queue.
                                let other = match urgency {
                                    Urgency::Normal => Urgency::Edit,
                                    Urgency::Edit => Urgency::Normal,
                                };
                                let both = second.and_then(|s| {
                                    let r = pool.resolve(s).expect("live range");
                                    ring.copy_reserved(reservation, r.buffer, r.offset, other)
                                        .ok()
                                });
                                if let (Some(s), None) = (second, both) {
                                    self.retire(s, Users::NONE);
                                }
                                ring.release(reservation);
                                self.jobs[i] = Job::Uploading {
                                    range,
                                    second: second.filter(|_| both.is_some()),
                                    tickets: [ticket, both.unwrap_or(ticket)],
                                    seed,
                                    len,
                                };
                            }
                            // The copy list is full for now: next frame.
                            Err(UploadError::Full) => {
                                self.jobs[i] = Job::Writing {
                                    reservation,
                                    range,
                                    second,
                                    seed,
                                    len,
                                    urgency,
                                    abandoned,
                                }
                            }
                            Err(e) => {
                                self.violation(format_args!("copying a finished slice: {e}"));
                                ring.release(reservation);
                            }
                        }
                    }
                    ReservationState::Abandoned => {
                        if abandoned {
                            self.report.abandoned += 1;
                        } else {
                            self.violation(format_args!("a slice was abandoned unasked"));
                        }
                        // Nothing was copied: the slice and its ranges go back at once.
                        ring.release(reservation);
                        self.retire(range, Users::NONE);
                        if let Some(second) = second {
                            self.retire(second, Users::NONE);
                        }
                    }
                    ReservationState::Gone => {
                        self.violation(format_args!("a slice was reclaimed before it was let go"));
                    }
                    ReservationState::Writing => {
                        self.jobs[i] = Job::Writing {
                            reservation,
                            range,
                            second,
                            seed,
                            len,
                            urgency,
                            abandoned,
                        }
                    }
                },
                Job::Held {
                    reservation,
                    writer,
                    frames,
                } => {
                    // Let go by its owner, but the writer still has the memory: the
                    // ring must not have taken it back.
                    if ring.reservation(reservation) != ReservationState::Writing {
                        self.violation(format_args!(
                            "a slice let go under its writer is {:?}",
                            ring.reservation(reservation)
                        ));
                    } else if frames < 3 {
                        self.jobs[i] = Job::Held {
                            reservation,
                            writer,
                            frames: frames + 1,
                        };
                    } else {
                        if let (Some(writer), Some(sender)) = (writer, &self.sender) {
                            let _ = sender.send(Work::LetGo(writer));
                        }
                        self.jobs[i] = Job::Dropping {
                            reservation,
                            frames: 0,
                        };
                    }
                }
                Job::Dropping {
                    reservation,
                    frames,
                } => {
                    if ring.reservation(reservation) == ReservationState::Gone {
                        self.report.held_writers += 1;
                    } else if frames == PATIENCE {
                        self.violation(format_args!("a slice was never taken back"));
                    } else {
                        self.jobs[i] = Job::Dropping {
                            reservation,
                            frames: frames + 1,
                        };
                    }
                }
                Job::Idle if !self.quiet && started < 6 => {
                    started += 1;
                    self.jobs[i] = self.start_job(ring);
                }
                other => self.jobs[i] = other,
            }
        }
        if barrier_open {
            let after = [vk::MemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::HOST)
                .dst_access_mask(vk::AccessFlags2::HOST_READ)];
            // SAFETY: guaranteed by the caller.
            unsafe {
                self.gpu.cmd_pipeline_barrier2(
                    ctx.cmd,
                    &vk::DependencyInfo::default().memory_barriers(&after),
                )
            };
        }
    }

    /// Starts a block: a range of the pool, a slice for the worker, and the work.
    fn start_job(&mut self, ring: &mut UploadRing) -> Job {
        let kind = next(&mut self.rng) % 16;
        let len = 4 * (64 + (next(&mut self.rng) % (JOB_BYTES as u64 / 4 - 64)) as usize);
        let align = if next(&mut self.rng).is_multiple_of(2) {
            16
        } else {
            20
        };
        let urgency = if next(&mut self.rng).is_multiple_of(8) {
            Urgency::Edit
        } else {
            Urgency::Normal
        };
        let seed = next(&mut self.rng);
        let Some(sender) = self.sender.clone() else {
            return Job::Idle;
        };
        if kind == 0 {
            // Let go at once, while the check itself still holds the writer.
            return match ring.reserve(len as u64, Urgency::Normal) {
                Ok((reservation, writer)) => {
                    ring.release(reservation);
                    Job::Held {
                        reservation,
                        writer: Some(writer),
                        frames: 0,
                    }
                }
                Err(_) => {
                    self.report.refused += 1;
                    Job::Idle
                }
            };
        }
        let pool = self.pool.as_mut().expect("live");
        let range = match pool.alloc(len as u64, align) {
            Ok(range) => range,
            Err(PoolError::Growing | PoolError::Full) => {
                // A page is on its way, or the budget is used up: the block waits.
                self.report.refused += 1;
                return Job::Idle;
            }
            Err(PoolError::TooLarge) => {
                self.violation(format_args!("{len} bytes are too large for the pool"));
                return Job::Idle;
            }
        };
        let second = if kind == 2 || kind == 3 {
            pool.alloc(len as u64, align).ok()
        } else {
            None
        };
        let (reservation, writer) = match ring.reserve(len as u64, urgency) {
            Ok(slice) => slice,
            Err(_) => {
                self.report.refused += 1;
                self.retire(range, Users::NONE);
                if let Some(second) = second {
                    self.retire(second, Users::NONE);
                }
                return Job::Idle;
            }
        };
        let abandoned = kind == 1;
        let work = if abandoned {
            Work::Abandon { writer, seed }
        } else {
            Work::Write {
                writer,
                seed,
                slow: kind == 4,
            }
        };
        // If the worker is gone the writer is dropped here, which abandons the slice.
        let _ = sender.send(work);
        Job::Writing {
            reservation,
            range,
            second,
            seed,
            len,
            urgency,
            abandoned,
        }
    }
}

impl Drop for StreamCheck {
    fn drop(&mut self) {
        // The worker ends when its channel closes; every writer is let go with it.
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        for job in &mut self.jobs {
            *job = Job::Idle;
        }
        // SAFETY: the owner drops the check only after the GPU is idle (after the frame
        // loop, whose Drop waits for every queue).
        unsafe {
            if let Some(b) = self.readback.take() {
                b.destroy(&self.gpu);
            }
            if let Some(b) = self.job_readback.take() {
                b.destroy(&self.gpu);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_large_texture_is_larger_than_the_ring() {
        assert_eq!(large_bytes(), 22_369_620);
        let all = levels(SIZE, MIPS, 7);
        assert_eq!(
            all.iter().map(|l| l.len() as u64).sum::<u64>(),
            large_bytes()
        );
        assert_ne!(all[0][..64], levels(SIZE, 1, 8)[0][..64]);
        assert_eq!(all[11].len(), 4);
    }

    #[test]
    fn a_run_passes_only_when_every_case_was_seen_and_nothing_was_wrong() {
        let mut r = StreamCheckReport::default();
        assert_eq!(r.missing(), Some("a finished round"));
        for field in [
            &mut r.rounds,
            &mut r.readbacks,
            &mut r.deferred_first,
            &mut r.deferred_middle,
            &mut r.deferred_final,
            &mut r.double_submits,
            &mut r.wraps,
            &mut r.replacements,
            &mut r.old_generation_reads,
            &mut r.cancelled_replacements,
            &mut r.cancelled_before,
            &mut r.cancelled_during,
            &mut r.cancelled_released,
            &mut r.survivors,
            &mut r.blocks,
            &mut r.dual_blocks,
            &mut r.abandoned,
            &mut r.held_writers,
            &mut r.stale_handles,
        ] {
            *field = 1;
        }
        assert!(r.passed());
        r.cancelled_released = 0;
        assert_eq!(r.missing(), Some("a cancel after the release"));
        r.cancelled_released = 1;
        r.early_publications = 1;
        assert!(!r.passed());
        r.early_publications = 0;
        r.violations = 1;
        assert!(!r.passed());
    }
}
