use std::sync::Arc;
use std::time::{Duration, Instant};

use ash::vk;
use ash::vk::{Handle, TaggedStructure};

use crate::device::{Gpu, PretendSrgb, SwapchainMaintenance};
use crate::swapchain::{PresentPolicy, Swapchain};
use crate::{Error, VkContext};

/// Frames the CPU may record while the GPU works on earlier ones. The governor in the
/// client usually waits for fewer (see [`FrameLoop::wait_for_gpu`]).
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// The longest one acquire may block before the frame is skipped, so a stalled compositor
/// never stalls the game loop (vanilla gives up only after 5 s). It can be made shorter
/// ([`FrameLoop::set_acquire_limit`]).
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_millis(100);

/// How long dropping the frame loop waits for presents that are still out.
const DROP_PRESENT_WAIT: Duration = Duration::from_millis(250);

/// Semaphore waits a frame can carry besides its acquire (see [`FrameCtx::wait_timeline`]).
pub const MAX_FRAME_WAITS: usize = 4;

/// How long before the GPU is expected to finish a frame the wait for it stops sleeping
/// and polls ([`FrameLoop::wait_for_gpu`]). On a GPU-bound laptop (RTX 4060, 2026-11-26)
/// a sleep to the end added 0.08 ms to the median frame and 0.11 ms to the 1% slowest:
/// the time to wake the core, and its work running slower after.
pub const DEFAULT_GPU_SPIN: Duration = Duration::from_micros(250);

/// The longest the wait for the GPU polls before it sleeps after all (the GPU is far
/// later than its last frame said it would be).
const MAX_GPU_POLL: Duration = Duration::from_millis(2);

/// Named GPU zones one frame can record (see [`FrameLoop::zone_begin`]); more are ignored.
pub const MAX_GPU_ZONES: usize = 16;
/// Timestamp queries per frame slot: the frame's start and end, then two per zone.
const QUERIES_PER_SLOT: u32 = 2 + 2 * MAX_GPU_ZONES as u32;

/// Per-frame-in-flight objects.
struct Slot {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    /// Signalled by acquire, waited by this slot's submit.
    acquire: vk::Semaphore,
    /// Timeline value of the last frame recorded in this slot (0: never used).
    frame: u64,
    /// When that frame was submitted.
    submitted: Option<Instant>,
    timed: bool,
    /// Zones recorded in the slot's frame.
    #[cfg(feature = "tracy")]
    zones: u32,
    /// Tracy query ids in the order their timestamps were recorded.
    #[cfg(feature = "tracy")]
    events: [u16; QUERIES_PER_SLOT as usize],
    #[cfg(feature = "tracy")]
    event_count: usize,
}

/// A swapchain that has been replaced, kept until nothing uses it: the frames that drew
/// to it and the presents of its images.
struct Retired {
    swapchain: Swapchain,
    /// The last frame that drew to one of its images (0: none did).
    last_frame: u64,
    /// Some of its presents had no fence when it was replaced, and never will: they are
    /// over once a present to a later swapchain is.
    chained: bool,
}

/// A present not known to be over, or the fence of an acquire still to signal.
///
/// With swapchain maintenance a present carries its own fence, which signals when the
/// presentation engine is done with the present's semaphore and image. Without it, the
/// fence is the one given to the acquire that next returns the same image (an image comes
/// back only after its present is over), unknown until then; and when a swapchain is
/// replaced first, its presents wait for a present to a later swapchain instead
/// ([`Retired::chained`]). This is the scheme of Khronos' swapchain recreation sample.
#[derive(Clone, Copy)]
struct PresentOp {
    generation: u32,
    image: u32,
    /// Null: not known yet (only without swapchain maintenance).
    fence: vk::Fence,
    /// False: an acquire of an image that had no present waiting; only its fence is kept,
    /// to be taken back.
    present: bool,
    /// The present was refused (out of date): its fence may never signal, and it proves
    /// nothing about earlier presents.
    rejected: bool,
}

/// GPU execution time of an earlier frame, read back from timestamp queries.
#[derive(Clone, Copy, Debug)]
pub struct GpuTime {
    pub frame: u64,
    pub nanos: u64,
}

/// How a frame's acquire ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Acquired {
    /// The frame has not reached its acquire.
    #[default]
    NotTried,
    Image,
    /// An image, from a swapchain that no longer matches the surface exactly.
    Suboptimal,
    /// No image within the acquire timeout.
    Timeout,
    NotReady,
    OutOfDate,
    /// There is no swapchain: the window has no area.
    NoSurface,
}

impl Acquired {
    /// A short name for logs.
    pub fn name(self) -> &'static str {
        match self {
            Acquired::NotTried => "none",
            Acquired::Image => "image",
            Acquired::Suboptimal => "suboptimal",
            Acquired::Timeout => "timeout",
            Acquired::NotReady => "not-ready",
            Acquired::OutOfDate => "out-of-date",
            Acquired::NoSurface => "no-surface",
        }
    }
}

/// Where one frame's time inside the frame loop went: the calls that can block, timed on
/// the calling thread. The frame number ties them to the frame's GPU time ([`GpuTime`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTimings {
    pub frame: u64,
    /// Waiting in [`FrameLoop::begin`] for the slot's previous frame.
    pub begin_wait_ns: u64,
    /// Rebuilding the swapchain before the acquire (0: it was not rebuilt).
    pub recreate_ns: u64,
    /// The `vkAcquireNextImageKHR` call (0: there was none), and how it ended.
    pub acquire_ns: u64,
    pub acquired: Acquired,
    /// The `vkQueueSubmit2` call.
    pub submit_ns: u64,
    /// The `vkQueuePresentKHR` call (0: nothing was presented).
    pub present_ns: u64,
    /// Swapchains created up to and including this frame: a change marks a rebuild.
    pub swapchains: u32,
}

/// What has happened at acquire and present since the frame loop was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// Acquires that gave an image (suboptimal ones included).
    pub images: u64,
    pub suboptimal: u64,
    pub timeouts: u64,
    pub not_ready: u64,
    pub out_of_date: u64,
    /// Frames with no swapchain to acquire from.
    pub no_surface: u64,
    pub present_suboptimal: u64,
    pub present_out_of_date: u64,
    pub swapchains: u32,
    /// Replaced swapchains destroyed, and the most that were waiting for it at once.
    pub swapchains_destroyed: u32,
    pub retired_peak: u32,
}

/// The swapchain frames are presented to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapchainInfo {
    /// Which of the window's swapchains it is, counted from 1.
    pub generation: u32,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    pub present_mode: vk::PresentModeKHR,
    pub images: u32,
}

/// The frame being recorded.
#[derive(Debug)]
pub struct FrameCtx {
    /// Frame number, starting at 1; also the timeline value its submit signals.
    pub number: u64,
    pub cmd: vk::CommandBuffer,
    slot: usize,
    waits: [vk::SemaphoreSubmitInfo<'static>; MAX_FRAME_WAITS],
    wait_count: usize,
}

impl FrameCtx {
    /// Makes this frame's submit wait, before `stages`, until timeline `semaphore`
    /// reaches `value` (another queue's work, such as uploads on the transfer queue). A
    /// second wait on the same semaphore raises the first one's value.
    ///
    /// # Panics
    /// If the frame already waits on [`MAX_FRAME_WAITS`] other semaphores.
    pub fn wait_timeline(
        &mut self,
        semaphore: vk::Semaphore,
        value: u64,
        stages: vk::PipelineStageFlags2,
    ) {
        if let Some(w) = self.waits[..self.wait_count]
            .iter_mut()
            .find(|w| w.semaphore == semaphore)
        {
            w.value = w.value.max(value);
            w.stage_mask |= stages;
            return;
        }
        assert!(self.wait_count < MAX_FRAME_WAITS, "too many frame waits");
        self.waits[self.wait_count] = vk::SemaphoreSubmitInfo::default()
            .semaphore(semaphore)
            .value(value)
            .stage_mask(stages);
        self.wait_count += 1;
    }

    /// Which frame-in-flight slot this frame uses (`number % MAX_FRAMES_IN_FLIGHT`).
    /// Per-slot objects, such as the texture table's descriptor sets, are free to update:
    /// the slot's previous frame has finished.
    pub fn slot(&self) -> usize {
        self.slot
    }
}

/// The acquired swapchain image to draw into this frame. Its layout is UNDEFINED; the
/// frame must leave it in PRESENT_SRC_KHR.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    /// The image may be copied from (see [`crate::Screenshot`]).
    pub readable: bool,
    /// Its swapchain ([`SwapchainInfo::generation`]), and whether it is the first image
    /// acquired from that one.
    pub generation: u32,
    pub first: bool,
    index: u32,
}

/// A GPU zone being recorded (see [`FrameLoop::zone_begin`]).
#[derive(Debug)]
#[must_use = "end it with FrameLoop::zone_end"]
pub struct GpuZone {
    /// Query index of its start; the end is the next one. None: not recorded.
    #[cfg(feature = "tracy")]
    query: Option<u32>,
}

/// What happened to a submitted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    Yes,
    /// Nothing to present to (minimised, out of date, or acquire timed out).
    Skipped,
}

/// Command recording, submission and presentation for one window. Frames are numbered and
/// tracked with one timeline semaphore; nothing in here allocates per frame.
pub struct FrameLoop {
    gpu: Arc<Gpu>,
    slots: [Slot; MAX_FRAMES_IN_FLIGHT],
    timeline: vk::Semaphore,
    /// Number of the last frame begun.
    frame: u64,
    query_pool: vk::QueryPool,
    swapchain: Option<Swapchain>,
    /// The last frame that drew to the swapchain, and the images acquired from it.
    drawn_frame: u64,
    acquired_images: u64,
    retired: Vec<Retired>,
    presents: Vec<PresentOp>,
    /// Fences free to use: unsignalled and in no pending operation.
    fences: Vec<vk::Fence>,
    /// Fences of refused presents that never signalled, kept until the frame loop goes.
    stuck_fences: Vec<vk::Fence>,
    /// The newest swapchain with a present known to be over (0: none yet).
    completed_generation: u32,
    acquire_timeout: Duration,
    acquire_deadline: Option<Instant>,
    /// How long before the GPU is expected to finish [`Self::wait_for_gpu`] polls instead
    /// of sleeping (zero: it sleeps until woken), and the GPU time of the last frame
    /// measured, which it expects the next to take too.
    gpu_spin: Duration,
    last_gpu: Duration,
    policy: PresentPolicy,
    pixel_size: (u32, u32),
    stale: bool,
    /// The frame being recorded, or the last one submitted.
    timings: FrameTimings,
    counters: FrameCounters,
    /// The graphics queue's GPU context in Tracy, if the profiler is running.
    #[cfg(feature = "tracy")]
    tracy: Option<tracy_client::GpuContext>,
}

impl std::fmt::Debug for FrameLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameLoop")
            .field("frame", &self.frame)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl FrameLoop {
    pub fn new(
        gpu: Arc<Gpu>,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
    ) -> Result<FrameLoop, Error> {
        let d = &gpu.device;
        let mut type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let timeline_info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
        // SAFETY: plain object creation; everything is destroyed in Drop (or below on error).
        let timeline = unsafe { d.create_semaphore(&timeline_info, None) }
            .ctx("vkCreateSemaphore(timeline)")?;
        let query_pool = if gpu.caps().timestamps {
            let info = vk::QueryPoolCreateInfo::default()
                .query_type(vk::QueryType::TIMESTAMP)
                .query_count(QUERIES_PER_SLOT * MAX_FRAMES_IN_FLIGHT as u32);
            // SAFETY: as above.
            unsafe { d.create_query_pool(&info, None) }.ctx("vkCreateQueryPool")?
        } else {
            vk::QueryPool::null()
        };
        let mut fl = FrameLoop {
            slots: std::array::from_fn(|_| Slot {
                pool: vk::CommandPool::null(),
                cmd: vk::CommandBuffer::null(),
                acquire: vk::Semaphore::null(),
                frame: 0,
                submitted: None,
                timed: false,
                #[cfg(feature = "tracy")]
                zones: 0,
                #[cfg(feature = "tracy")]
                events: [0; QUERIES_PER_SLOT as usize],
                #[cfg(feature = "tracy")]
                event_count: 0,
            }),
            gpu,
            timeline,
            frame: 0,
            query_pool,
            swapchain: None,
            drawn_frame: 0,
            acquired_images: 0,
            retired: Vec::with_capacity(8),
            presents: Vec::with_capacity(32),
            fences: Vec::with_capacity(8),
            stuck_fences: Vec::new(),
            completed_generation: 0,
            acquire_timeout: ACQUIRE_TIMEOUT,
            acquire_deadline: None,
            gpu_spin: DEFAULT_GPU_SPIN,
            last_gpu: Duration::ZERO,
            policy,
            pixel_size,
            stale: true,
            timings: FrameTimings::default(),
            counters: FrameCounters::default(),
            #[cfg(feature = "tracy")]
            tracy: None,
        };
        // Drop cleans up whatever was created if this fails part-way.
        for i in 0..MAX_FRAMES_IN_FLIGHT {
            let d = &fl.gpu.device;
            let pool_info = vk::CommandPoolCreateInfo::default()
                .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                .queue_family_index(fl.gpu.queue_family);
            // SAFETY: plain object creation.
            unsafe {
                fl.slots[i].pool = d
                    .create_command_pool(&pool_info, None)
                    .ctx("vkCreateCommandPool")?;
                let alloc = vk::CommandBufferAllocateInfo::default()
                    .command_pool(fl.slots[i].pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);
                fl.slots[i].cmd = d
                    .allocate_command_buffers(&alloc)
                    .ctx("vkAllocateCommandBuffers")?[0];
                fl.slots[i].acquire = d
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .ctx("vkCreateSemaphore")?;
            }
        }
        // No reset of the query pool here: a frame resets its slot's queries in its command
        // buffer before writing them, and a slot's results are read only once it was timed.
        #[cfg(feature = "tracy")]
        if !fl.query_pool.is_null() && tracy_client::Client::running().is_some() {
            fl.tracy = fl.tracy_context()?;
        }
        fl.recreate_swapchain()?;
        Ok(fl)
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// The colour format frames are drawn in: the swapchain's, or the one a swapchain for
    /// this window would have.
    pub fn color_format(&self) -> Result<vk::Format, Error> {
        match &self.swapchain {
            Some(sc) => Ok(sc.format),
            None => crate::swapchain::surface_format(&self.gpu),
        }
    }

    /// The present mode in use, if there is a swapchain.
    pub fn present_mode(&self) -> Option<vk::PresentModeKHR> {
        self.swapchain.as_ref().map(|s| s.present_mode)
    }

    /// The swapchain in use, if there is one.
    pub fn swapchain_info(&self) -> Option<SwapchainInfo> {
        self.swapchain.as_ref().map(|s| SwapchainInfo {
            generation: s.generation,
            extent: s.extent,
            format: s.format,
            present_mode: s.present_mode,
            images: s.images.len() as u32,
        })
    }

    pub fn policy(&self) -> PresentPolicy {
        self.policy
    }

    /// Timings of the frame being recorded, complete once it is submitted.
    pub fn timings(&self) -> &FrameTimings {
        &self.timings
    }

    pub fn counters(&self) -> &FrameCounters {
        &self.counters
    }

    /// Switches VSync; the swapchain is rebuilt (with `oldSwapchain`, no idle wait) when the
    /// next frame is prepared.
    pub fn set_policy(&mut self, policy: PresentPolicy) {
        if policy != self.policy {
            self.policy = policy;
            self.stale = true;
        }
    }

    /// Records a new drawable size; the swapchain is rebuilt at most once per frame, when
    /// the next frame is prepared, however many resize events arrive.
    pub fn resize(&mut self, pixel_size: (u32, u32)) {
        if pixel_size != self.pixel_size {
            self.pixel_size = pixel_size;
            self.stale = true;
        }
    }

    /// Has the swapchain rebuilt when the next frame is prepared although nothing changed
    /// (development: to exercise recreation).
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// How long the acquires from now on may block: `timeout` each ([`ACQUIRE_TIMEOUT`] at
    /// most, and by default), and not past `deadline` if there is one. Past the deadline an
    /// acquire only takes an image that is free.
    pub fn set_acquire_limit(&mut self, timeout: Duration, deadline: Option<Instant>) {
        self.acquire_timeout = timeout.min(ACQUIRE_TIMEOUT);
        self.acquire_deadline = deadline;
    }

    /// Number of the last frame the GPU has finished.
    pub fn completed(&self) -> Result<u64, Error> {
        // SAFETY: live timeline semaphore.
        unsafe { self.gpu.device.get_semaphore_counter_value(self.timeline) }
            .ctx("vkGetSemaphoreCounterValue")
    }

    /// Blocks until at most `frames_ahead` submitted frames are unfinished on the GPU. The
    /// client calls this *before* sampling input: `0` holds the next frame until the GPU
    /// has finished the previous one, so a GPU-bound game never queues stale input.
    pub fn wait_for_gpu(&self, frames_ahead: u64) -> Result<(), Error> {
        profiling::scope!("wait_for_gpu");
        let target = self.frame.saturating_sub(frames_ahead);
        if target == 0 || self.gpu_spin.is_zero() {
            return self.wait_timeline(target);
        }
        if self.completed()? >= target {
            return Ok(());
        }
        // Sleep through what the GPU should still take, but for the margin: waking from
        // a sleep takes the core out of its idle state, and the time that takes would be
        // added to the frame.
        let slot = &self.slots[(target % MAX_FRAMES_IN_FLIGHT as u64) as usize];
        if slot.frame == target
            && let Some(submitted) = slot.submitted
            && let Some(sleep) = (submitted + self.last_gpu)
                .checked_duration_since(Instant::now())
                .and_then(|left| left.checked_sub(self.gpu_spin))
            && self.wait_timeline_for(target, sleep)?
        {
            return Ok(());
        }
        // Then poll, the core kept awake, until the frame is done; a GPU much later than
        // its last frame is slept for after all.
        let polling = Instant::now();
        loop {
            if self.completed()? >= target {
                return Ok(());
            }
            if polling.elapsed() >= MAX_GPU_POLL {
                return self.wait_timeline(target);
            }
            std::hint::spin_loop();
        }
    }

    /// How long before the GPU is expected to finish [`Self::wait_for_gpu`] stops sleeping
    /// and polls instead ([`DEFAULT_GPU_SPIN`] unless set); zero: it sleeps until woken.
    pub fn set_gpu_spin(&mut self, margin: Duration) {
        self.gpu_spin = margin;
    }

    /// Waits at most `timeout` for the timeline to reach `value`; whether it did.
    fn wait_timeline_for(&self, value: u64, timeout: Duration) -> Result<bool, Error> {
        let semaphores = [self.timeline];
        let values = [value];
        let info = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: live timeline semaphore; every waited value has been submitted.
        match unsafe { self.gpu.device.wait_semaphores(&info, nanos) } {
            Ok(()) => Ok(true),
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(e) => Err(e).ctx("vkWaitSemaphores"),
        }
    }

    fn wait_timeline(&self, value: u64) -> Result<(), Error> {
        if value == 0 {
            return Ok(());
        }
        let semaphores = [self.timeline];
        let values = [value];
        let info = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        // SAFETY: live timeline semaphore; every waited value has been submitted.
        unsafe { self.gpu.device.wait_semaphores(&info, u64::MAX) }.ctx("vkWaitSemaphores")
    }

    /// Starts recording the next frame. Waits until its slot's previous frame is done
    /// (normally already true after [`Self::wait_for_gpu`]) and returns that frame's GPU time.
    pub fn begin(&mut self) -> Result<(FrameCtx, Option<GpuTime>), Error> {
        profiling::scope!("begin_frame");
        self.frame += 1;
        let number = self.frame;
        let slot_index = (number % MAX_FRAMES_IN_FLIGHT as u64) as usize;
        let previous = self.slots[slot_index].frame;
        let waited = Instant::now();
        self.wait_timeline(previous)?;
        self.timings = FrameTimings {
            frame: number,
            begin_wait_ns: waited.elapsed().as_nanos() as u64,
            swapchains: self.counters.swapchains,
            ..FrameTimings::default()
        };
        let gpu_time = self.read_timestamps(slot_index);
        if let Some(t) = gpu_time {
            self.last_gpu = Duration::from_nanos(t.nanos);
        }
        #[cfg(feature = "tracy")]
        self.upload_zones(slot_index);
        self.reap()?;

        let d = &self.gpu.device;
        let slot = &mut self.slots[slot_index];
        slot.frame = number;
        let cmd = slot.cmd;
        // SAFETY: the slot's previous submission has completed (waited above).
        unsafe {
            d.reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty())
                .ctx("vkResetCommandPool")?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            d.begin_command_buffer(cmd, &begin)
                .ctx("vkBeginCommandBuffer")?;
            #[cfg(feature = "tracy")]
            {
                slot.zones = 0;
                slot.event_count = 0;
            }
            if !self.query_pool.is_null() {
                let q = QUERIES_PER_SLOT * slot_index as u32;
                d.cmd_reset_query_pool(cmd, self.query_pool, q, QUERIES_PER_SLOT);
                self.gpu.cmd_write_timestamp2(
                    cmd,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    self.query_pool,
                    q,
                );
                slot.timed = true;
                #[cfg(feature = "tracy")]
                if let Some(t) = &self.tracy {
                    t.begin_span_alloc("frame", "gpu", file!(), line!(), q as u16);
                    slot.events[0] = q as u16;
                    slot.event_count = 1;
                }
            }
        }
        Ok((
            FrameCtx {
                number,
                cmd,
                slot: slot_index,
                waits: [vk::SemaphoreSubmitInfo::default(); MAX_FRAME_WAITS],
                wait_count: 0,
            },
            gpu_time,
        ))
    }

    fn read_timestamps(&self, slot_index: usize) -> Option<GpuTime> {
        let slot = &self.slots[slot_index];
        if !slot.timed || self.query_pool.is_null() {
            return None;
        }
        let mut ticks = [0u64; 2];
        // SAFETY: the slot's frame has completed, so both queries are available.
        unsafe {
            self.gpu
                .device
                .get_query_pool_results(
                    self.query_pool,
                    QUERIES_PER_SLOT * slot_index as u32,
                    &mut ticks,
                    vk::QueryResultFlags::TYPE_64,
                )
                .ok()?;
        }
        let elapsed = elapsed_ticks(ticks[0], ticks[1], self.gpu.caps().timestamp_bits);
        let ns = elapsed as f64 * f64::from(self.gpu.caps().timestamp_period_ns);
        Some(GpuTime {
            frame: slot.frame,
            nanos: ns as u64,
        })
    }

    /// Makes the swapchain match the window, as the first thing a frame does after
    /// [`Self::begin`]: what it returns is what this frame's acquire will draw to, so
    /// pipelines and off-screen images can be chosen for its format and size before anything
    /// is recorded. `None`: the window has no area. A swapchain the acquire or the present
    /// finds out of date is replaced here in the next frame.
    pub fn prepare(&mut self) -> Result<Option<SwapchainInfo>, Error> {
        // Without a swapchain, try again each frame: not every window system says when a
        // window has an area again.
        if self.stale || self.swapchain.is_none() {
            let started = Instant::now();
            self.recreate_swapchain()?;
            self.timings.recreate_ns += started.elapsed().as_nanos() as u64;
            self.timings.swapchains = self.counters.swapchains;
        }
        Ok(self.swapchain_info())
    }

    /// Acquires the swapchain image for this frame, from the swapchain [`Self::prepare`]
    /// returned. Call it as late as possible: after all work that doesn't write to the
    /// window has been recorded. `None`: nothing to draw to this frame (still submit it).
    pub fn acquire(&mut self, ctx: &FrameCtx) -> Result<Option<Target>, Error> {
        profiling::scope!("acquire");
        if self.swapchain.is_none() {
            self.timings.acquired = Acquired::NoSurface;
            self.counters.no_surface += 1;
            return Ok(None);
        }
        // Without present fences, the acquire's fence says when the image's last present
        // was over.
        let fence = match self.gpu.caps().swapchain_maintenance {
            SwapchainMaintenance::None => take_fence(&self.gpu, &mut self.fences)?,
            _ => vk::Fence::null(),
        };
        let sc = self.swapchain.as_ref().expect("checked above");
        let acquire = self.slots[ctx.slot].acquire;
        let started = Instant::now();
        let timeout = match self.acquire_deadline {
            Some(d) => d
                .saturating_duration_since(started)
                .min(self.acquire_timeout),
            None => self.acquire_timeout,
        };
        // SAFETY: the acquire semaphore is unsignalled (its last wait has completed); the
        // fence, if any, is unsignalled and in no pending operation.
        let result = unsafe {
            self.gpu.swapchain_fn.acquire_next_image(
                sc.raw,
                timeout.as_nanos() as u64,
                acquire,
                fence,
            )
        };
        self.timings.acquire_ns = started.elapsed().as_nanos() as u64;
        if result.is_err() && !fence.is_null() {
            // No image, so the fence was not taken.
            self.fences.push(fence);
        }
        match result {
            Ok((index, suboptimal)) => {
                self.counters.images += 1;
                self.timings.acquired = Acquired::Image;
                if suboptimal {
                    self.stale = true;
                    self.counters.suboptimal += 1;
                    self.timings.acquired = Acquired::Suboptimal;
                }
                if !fence.is_null() {
                    let waiting = self.presents.iter_mut().find(|p| {
                        p.present
                            && p.fence.is_null()
                            && p.generation == sc.generation
                            && p.image == index
                    });
                    match waiting {
                        Some(p) => p.fence = fence,
                        None => self.presents.push(PresentOp {
                            generation: sc.generation,
                            image: index,
                            fence,
                            present: false,
                            rejected: false,
                        }),
                    }
                }
                self.drawn_frame = ctx.number;
                self.acquired_images += 1;
                let i = index as usize;
                Ok(Some(Target {
                    image: sc.images[i],
                    view: sc.views[i],
                    extent: sc.extent,
                    format: sc.format,
                    readable: sc.readable,
                    generation: sc.generation,
                    first: self.acquired_images == 1,
                    index,
                }))
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                self.counters.out_of_date += 1;
                self.timings.acquired = Acquired::OutOfDate;
                Ok(None)
            }
            Err(vk::Result::TIMEOUT) => {
                self.counters.timeouts += 1;
                self.timings.acquired = Acquired::Timeout;
                Ok(None)
            }
            Err(vk::Result::NOT_READY) => {
                self.counters.not_ready += 1;
                self.timings.acquired = Acquired::NotReady;
                Ok(None)
            }
            Err(e) => Err(Error::Vk {
                call: "vkAcquireNextImageKHR",
                result: e,
            }),
        }
    }

    /// Ends recording, submits (signalling the frame's timeline value) and presents.
    pub fn submit(&mut self, ctx: FrameCtx, target: Option<Target>) -> Result<Presented, Error> {
        profiling::scope!("submit_present");
        let gpu = &self.gpu;
        let d = &gpu.device;
        let slot = &mut self.slots[ctx.slot];
        // SAFETY: ctx.cmd is in the recording state (begin()).
        unsafe {
            if slot.timed {
                let q = QUERIES_PER_SLOT * ctx.slot as u32 + 1;
                gpu.cmd_write_timestamp2(
                    ctx.cmd,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    self.query_pool,
                    q,
                );
                #[cfg(feature = "tracy")]
                if let Some(t) = &self.tracy {
                    t.end_span(q as u16);
                    slot.events[slot.event_count] = q as u16;
                    slot.event_count += 1;
                }
            }
            d.end_command_buffer(ctx.cmd).ctx("vkEndCommandBuffer")?;
        }
        let render_done = target.and_then(|t| {
            self.swapchain
                .as_ref()
                .map(|s| s.render_done[t.index as usize])
        });
        let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(ctx.cmd)];
        // The acquire (if an image was acquired), then the frame's own waits.
        let mut waits = [vk::SemaphoreSubmitInfo::default(); 1 + MAX_FRAME_WAITS];
        let mut wait_count = 0;
        if render_done.is_some() {
            waits[0] = vk::SemaphoreSubmitInfo::default()
                .semaphore(slot.acquire)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);
            wait_count = 1;
        }
        for w in &ctx.waits[..ctx.wait_count] {
            waits[wait_count] = *w;
            wait_count += 1;
        }
        let mut signals = [
            vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(ctx.number)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS),
            vk::SemaphoreSubmitInfo::default(),
        ];
        let mut signal_count = 1;
        if let Some(sem) = render_done {
            signals[1] = vk::SemaphoreSubmitInfo::default()
                .semaphore(sem)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);
            signal_count = 2;
        }
        let submit = vk::SubmitInfo2::default()
            .command_buffer_infos(&cmds)
            .wait_semaphore_infos(&waits[..wait_count])
            .signal_semaphore_infos(&signals[..signal_count]);
        let started = Instant::now();
        // SAFETY: one submitting thread; all handles are live; the waited semaphore was
        // signalled by this frame's acquire.
        unsafe { gpu.queue_submit2(std::slice::from_ref(&submit))? };
        self.timings.submit_ns = started.elapsed().as_nanos() as u64;
        self.slots[ctx.slot].submitted = Some(started);

        let (Some(t), Some(sem), Some(sc)) = (target, render_done, &self.swapchain) else {
            return Ok(Presented::Skipped);
        };
        // With swapchain maintenance, a fence that signals when this present is over.
        let fence = match gpu.caps().swapchain_maintenance {
            SwapchainMaintenance::None => vk::Fence::null(),
            _ => take_fence(gpu, &mut self.fences)?,
        };
        let swapchains = [sc.raw];
        let indices = [t.index];
        let wait = [sem];
        let fences = [fence];
        let mut fence_info = vk::SwapchainPresentFenceInfoKHR::default().fences(&fences);
        let mut present = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait)
            .swapchains(&swapchains)
            .image_indices(&indices);
        if !fence.is_null() {
            present = present.push(&mut fence_info);
        }
        let started = Instant::now();
        // SAFETY: the image was acquired this frame and is in PRESENT_SRC_KHR after the
        // submitted commands (the Target contract); the fence, if any, is unsignalled and
        // in no pending operation, and the device has the extension its structure is from.
        let result = unsafe { gpu.swapchain_fn.queue_present(gpu.queue, &present) };
        self.timings.present_ns = started.elapsed().as_nanos() as u64;
        let rejected = match result {
            Ok(_) => false,
            // Refused, but queued as far as its semaphore goes.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => true,
            Err(e) => {
                if !fence.is_null() {
                    self.stuck_fences.push(fence);
                }
                return Err(Error::Vk {
                    call: "vkQueuePresentKHR",
                    result: e,
                });
            }
        };
        self.presents.push(PresentOp {
            generation: sc.generation,
            image: t.index,
            fence,
            present: true,
            rejected,
        });
        match result {
            Ok(true) => {
                self.stale = true;
                self.counters.present_suboptimal += 1;
                Ok(Presented::Yes)
            }
            Ok(false) => Ok(Presented::Yes),
            Err(_) => {
                self.stale = true;
                self.counters.present_out_of_date += 1;
                Ok(Presented::Skipped)
            }
        }
    }

    /// Starts a named GPU zone in `ctx`'s command buffer, shown in the Tracy profiler when
    /// it is running (built with the `tracy` feature); otherwise it records nothing. Zones
    /// may nest; each must be ended with [`Self::zone_end`] in the same frame, outside or
    /// inside the same render pass it started in. A frame records at most
    /// [`MAX_GPU_ZONES`]; more are ignored.
    #[track_caller]
    pub fn zone_begin(&mut self, ctx: &FrameCtx, name: &'static str) -> GpuZone {
        #[cfg(feature = "tracy")]
        if let Some(t) = &self.tracy {
            let slot = &mut self.slots[ctx.slot];
            if slot.timed && (slot.zones as usize) < MAX_GPU_ZONES {
                let q = QUERIES_PER_SLOT * ctx.slot as u32 + 2 + 2 * slot.zones;
                slot.zones += 1;
                // SAFETY: ctx.cmd is recording; the query was reset at the frame's start.
                unsafe {
                    self.gpu.cmd_write_timestamp2(
                        ctx.cmd,
                        vk::PipelineStageFlags2::ALL_COMMANDS,
                        self.query_pool,
                        q,
                    )
                };
                let at = std::panic::Location::caller();
                t.begin_span_alloc(name, "gpu", at.file(), at.line(), q as u16);
                slot.events[slot.event_count] = q as u16;
                slot.event_count += 1;
                return GpuZone { query: Some(q) };
            }
        }
        let _ = (ctx, name);
        GpuZone {
            #[cfg(feature = "tracy")]
            query: None,
        }
    }

    /// Ends a zone started with [`Self::zone_begin`] in the same frame.
    pub fn zone_end(&mut self, ctx: &FrameCtx, zone: GpuZone) {
        #[cfg(feature = "tracy")]
        if let (Some(t), Some(q)) = (&self.tracy, zone.query) {
            let slot = &mut self.slots[ctx.slot];
            // SAFETY: as in zone_begin.
            unsafe {
                self.gpu.cmd_write_timestamp2(
                    ctx.cmd,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    self.query_pool,
                    q + 1,
                )
            };
            t.end_span((q + 1) as u16);
            slot.events[slot.event_count] = (q + 1) as u16;
            slot.event_count += 1;
        }
        let _ = (ctx, zone);
    }

    /// Sends the slot's finished frame's timestamps to Tracy, in GPU-time order (as it
    /// requires; recording order breaks ties, so an outer zone starts first and ends last).
    #[cfg(feature = "tracy")]
    fn upload_zones(&mut self, slot_index: usize) {
        let Some(t) = &self.tracy else {
            return;
        };
        let slot = &self.slots[slot_index];
        let n = slot.event_count;
        if n == 0 {
            return;
        }
        let base = QUERIES_PER_SLOT * slot_index as u32;
        let used = 2 + 2 * slot.zones as usize;
        let mut ticks = [0u64; QUERIES_PER_SLOT as usize];
        // SAFETY: the slot's frame has finished, so every query it wrote is available.
        let read = unsafe {
            self.gpu.device.get_query_pool_results(
                self.query_pool,
                base,
                &mut ticks[..used],
                vk::QueryResultFlags::TYPE_64,
            )
        };
        if read.is_err() {
            return;
        }
        let bits = self.gpu.caps().timestamp_bits;
        let mask = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let mut order: [(u64, usize, u16); QUERIES_PER_SLOT as usize] =
            [(0, 0, 0); QUERIES_PER_SLOT as usize];
        for (i, &id) in slot.events[..n].iter().enumerate() {
            order[i] = (ticks[(u32::from(id) - base) as usize] & mask, i, id);
        }
        order[..n].sort_unstable();
        for &(ts, _, id) in &order[..n] {
            t.upload_gpu_timestamp(id, ts as i64);
        }
    }

    /// Creates the Tracy GPU context, calibrated with one timestamp read back right away.
    #[cfg(feature = "tracy")]
    fn tracy_context(&mut self) -> Result<Option<tracy_client::GpuContext>, Error> {
        let Some(client) = tracy_client::Client::running() else {
            return Ok(None);
        };
        let d = &self.gpu.device;
        let slot = &self.slots[0];
        // SAFETY: nothing has been submitted yet; the queue is idle after the wait, and the
        // slot's pool is reset again by its first frame.
        let ticks = unsafe {
            d.reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty())
                .ctx("vkResetCommandPool")?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            d.begin_command_buffer(slot.cmd, &begin)
                .ctx("vkBeginCommandBuffer")?;
            d.cmd_reset_query_pool(slot.cmd, self.query_pool, 0, 1);
            self.gpu.cmd_write_timestamp2(
                slot.cmd,
                vk::PipelineStageFlags2::ALL_COMMANDS,
                self.query_pool,
                0,
            );
            d.end_command_buffer(slot.cmd).ctx("vkEndCommandBuffer")?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(slot.cmd)];
            let submit = vk::SubmitInfo2::default().command_buffer_infos(&cmds);
            self.gpu.queue_submit2(std::slice::from_ref(&submit))?;
            d.queue_wait_idle(self.gpu.queue).ctx("vkQueueWaitIdle")?;
            let mut ticks = [0u64; 1];
            d.get_query_pool_results(
                self.query_pool,
                0,
                &mut ticks,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
            )
            .ctx("vkGetQueryPoolResults")?;
            ticks[0]
        };
        let name = format!("{} graphics", self.gpu.caps().name);
        match client.new_gpu_context(
            Some(&name),
            tracy_client::GpuContextType::Vulkan,
            ticks as i64,
            self.gpu.caps().timestamp_period_ns,
        ) {
            Ok(c) => Ok(Some(c)),
            Err(e) => {
                eprintln!("[gpu] no Tracy GPU context: {e}");
                Ok(None)
            }
        }
    }

    fn recreate_swapchain(&mut self) -> Result<(), Error> {
        profiling::scope!("recreate_swapchain");
        let old = self.swapchain.take();
        let old_raw = old.as_ref().map_or(vk::SwapchainKHR::null(), |s| s.raw);
        let srgb_only = match self.gpu.pretend_srgb {
            PretendSrgb::Never => false,
            PretendSrgb::Always => true,
            PretendSrgb::OnRecreation => self.counters.swapchains > 0,
        };
        let new = Swapchain::create(
            &self.gpu,
            self.pixel_size,
            self.policy,
            old_raw,
            self.counters.swapchains + 1,
            srgb_only,
        );
        if let Some(swapchain) = old {
            // Frames and presents may still use the old one: see reap(). Its presents
            // without a fence get none now that no image of it is acquired again.
            let generation = swapchain.generation;
            let mut chained = false;
            self.presents.retain(|p| {
                let orphan = p.generation == generation && p.fence.is_null();
                chained |= orphan;
                !orphan
            });
            self.retired.push(Retired {
                swapchain,
                last_frame: self.drawn_frame,
                chained,
            });
            self.counters.retired_peak = self.counters.retired_peak.max(self.retired.len() as u32);
        }
        self.swapchain = new?;
        self.stale = false;
        if let Some(sc) = &self.swapchain {
            self.counters.swapchains += 1;
            self.drawn_frame = 0;
            self.acquired_images = 0;
            eprintln!(
                "[gpu] swapchain {}x{} {:?} {:?}, {} images",
                sc.extent.width,
                sc.extent.height,
                sc.format,
                sc.present_mode,
                sc.images.len()
            );
        }
        Ok(())
    }

    /// Takes back the fences of presents that are over, and destroys the replaced
    /// swapchains nothing uses any more: the frames that drew to it have finished, and its
    /// presents are over, which their fences say or, for those without one, a finished
    /// present to a later swapchain.
    fn reap(&mut self) -> Result<(), Error> {
        if self.presents.is_empty() && self.retired.is_empty() {
            return Ok(());
        }
        let d = &self.gpu.device;
        let mut i = 0;
        while i < self.presents.len() {
            let op = self.presents[i];
            // SAFETY: a live fence.
            let over = !op.fence.is_null()
                && unsafe { d.get_fence_status(op.fence) }.ctx("vkGetFenceStatus")?;
            if !over {
                i += 1;
                continue;
            }
            if op.present && !op.rejected {
                self.completed_generation = self.completed_generation.max(op.generation);
            }
            // SAFETY: signalled, so the operation it was given to is over.
            unsafe { d.reset_fences(&[op.fence]) }.ctx("vkResetFences")?;
            self.fences.push(op.fence);
            self.presents.swap_remove(i);
        }
        if self.retired.is_empty() {
            return Ok(());
        }
        let done = self.completed()?;
        let mut i = 0;
        while i < self.retired.len() {
            let r = &self.retired[i];
            let generation = r.swapchain.generation;
            let succeeded = self.completed_generation > generation;
            // A refused present's fence may never signal; a later present being over
            // covers it too.
            let presenting = self
                .presents
                .iter()
                .any(|p| p.generation == generation && !(p.rejected && succeeded));
            if done < r.last_frame || presenting || (r.chained && !succeeded) {
                i += 1;
                continue;
            }
            let mut r = self.retired.swap_remove(i);
            // SAFETY: the frames that drew to it have finished and its presents are over.
            unsafe { r.swapchain.destroy(&self.gpu) };
            self.counters.swapchains_destroyed += 1;
            let stuck = &mut self.stuck_fences;
            self.presents.retain(|p| {
                if p.generation == generation {
                    stuck.push(p.fence);
                }
                p.generation != generation
            });
        }
        Ok(())
    }
}

/// A fence from `pool`, or a new one: unsignalled and in no pending operation.
fn take_fence(gpu: &Gpu, pool: &mut Vec<vk::Fence>) -> Result<vk::Fence, Error> {
    match pool.pop() {
        Some(f) => Ok(f),
        // SAFETY: plain object creation; the frame loop destroys it in Drop.
        None => unsafe {
            gpu.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .ctx("vkCreateFence"),
    }
}

/// Ticks from `start` to `end`, two timestamps of which only the low `valid_bits` count
/// (`timestampValidBits`): the counter may have wrapped in between.
fn elapsed_ticks(start: u64, end: u64, valid_bits: u32) -> u64 {
    let mask = if valid_bits >= 64 {
        u64::MAX
    } else {
        (1u64 << valid_bits) - 1
    };
    end.wrapping_sub(start) & mask
}

impl Drop for FrameLoop {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        // SAFETY: after the idle wait no queue uses any of these objects. Presents still
        // out get a moment to finish; then the swapchains go first, which ends them, and
        // the fences after.
        unsafe {
            let _ = d.device_wait_idle();
            let out: Vec<vk::Fence> = self
                .presents
                .iter()
                .map(|p| p.fence)
                .filter(|f| !f.is_null())
                .collect();
            if !out.is_empty() {
                let _ = d.wait_for_fences(&out, true, DROP_PRESENT_WAIT.as_nanos() as u64);
            }
            for mut r in self.retired.drain(..) {
                r.swapchain.destroy(&self.gpu);
            }
            if let Some(mut sc) = self.swapchain.take() {
                sc.destroy(&self.gpu);
            }
            for f in out
                .into_iter()
                .chain(self.fences.drain(..))
                .chain(self.stuck_fences.drain(..))
            {
                d.destroy_fence(f, None);
            }
            for s in &self.slots {
                if !s.acquire.is_null() {
                    d.destroy_semaphore(s.acquire, None);
                }
                if !s.pool.is_null() {
                    d.destroy_command_pool(s.pool, None);
                }
            }
            if !self.query_pool.is_null() {
                d.destroy_query_pool(self.query_pool, None);
            }
            d.destroy_semaphore(self.timeline, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_wrap_at_their_valid_bits() {
        assert_eq!(elapsed_ticks(10, 25, 64), 15);
        assert_eq!(elapsed_ticks(u64::MAX - 1, 3, 64), 5);
        // A 36-bit counter that wrapped: the bits above it are not part of the value.
        let top = (1u64 << 36) - 1;
        assert_eq!(elapsed_ticks(top - 1, 3, 36), 5);
        assert_eq!(elapsed_ticks(7, 9, 36), 2);
    }
}
