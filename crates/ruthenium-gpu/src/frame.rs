use std::sync::Arc;
use std::time::Duration;

use ash::vk;
use ash::vk::{Handle, TaggedStructure};

use crate::device::Gpu;
use crate::swapchain::{PresentPolicy, Swapchain};
use crate::{Error, VkContext};

/// Frames the CPU may record while the GPU works on earlier ones. The governor in the
/// client usually waits for fewer (see [`FrameLoop::wait_for_gpu`]).
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// How long one acquire may block before the frame is skipped, so a stalled compositor
/// never stalls the game loop (vanilla gives up only after 5 s).
const ACQUIRE_TIMEOUT: Duration = Duration::from_millis(100);

/// Per-frame-in-flight objects.
struct Slot {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    /// Signalled by acquire, waited by this slot's submit.
    acquire: vk::Semaphore,
    /// Timeline value of the last frame recorded in this slot (0: never used).
    frame: u64,
    timed: bool,
}

/// A swapchain kept alive until the frames that used it are surely done.
struct Retired {
    swapchain: Swapchain,
    destroy_after: u64,
}

/// GPU execution time of an earlier frame, read back from timestamp queries.
#[derive(Clone, Copy, Debug)]
pub struct GpuTime {
    pub frame: u64,
    pub nanos: u64,
}

/// The frame being recorded.
#[derive(Debug)]
pub struct FrameCtx {
    /// Frame number, starting at 1; also the timeline value its submit signals.
    pub number: u64,
    pub cmd: vk::CommandBuffer,
    slot: usize,
}

/// The acquired swapchain image to draw into this frame. Its layout is UNDEFINED; the
/// frame must leave it in PRESENT_SRC_KHR.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    index: u32,
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
    retired: Vec<Retired>,
    policy: PresentPolicy,
    pixel_size: (u32, u32),
    stale: bool,
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
                .query_count(2 * MAX_FRAMES_IN_FLIGHT as u32);
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
                timed: false,
            }),
            gpu,
            timeline,
            frame: 0,
            query_pool,
            swapchain: None,
            retired: Vec::with_capacity(4),
            policy,
            pixel_size,
            stale: true,
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
        if !fl.query_pool.is_null() {
            // SAFETY: hostQueryReset is enabled; the pool is unused.
            unsafe {
                fl.gpu
                    .device
                    .reset_query_pool(fl.query_pool, 0, 2 * MAX_FRAMES_IN_FLIGHT as u32)
            };
        }
        fl.recreate_swapchain()?;
        Ok(fl)
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// The present mode in use, if there is a swapchain.
    pub fn present_mode(&self) -> Option<vk::PresentModeKHR> {
        self.swapchain.as_ref().map(|s| s.present_mode)
    }

    pub fn policy(&self) -> PresentPolicy {
        self.policy
    }

    /// Switches VSync; the swapchain is rebuilt (with `oldSwapchain`, no idle wait) at the
    /// next acquire.
    pub fn set_policy(&mut self, policy: PresentPolicy) {
        if policy != self.policy {
            self.policy = policy;
            self.stale = true;
        }
    }

    /// Records a new drawable size; the swapchain is rebuilt at most once per frame, at the
    /// next acquire, however many resize events arrive.
    pub fn resize(&mut self, pixel_size: (u32, u32)) {
        if pixel_size != self.pixel_size {
            self.pixel_size = pixel_size;
            self.stale = true;
        }
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
        self.wait_timeline(target)
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
        self.wait_timeline(previous)?;
        let gpu_time = self.read_timestamps(slot_index);
        self.destroy_retired()?;

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
            if !self.query_pool.is_null() {
                let q = 2 * slot_index as u32;
                d.cmd_reset_query_pool(cmd, self.query_pool, q, 2);
                self.gpu.cmd_write_timestamp2(
                    cmd,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    self.query_pool,
                    q,
                );
                slot.timed = true;
            }
        }
        Ok((
            FrameCtx {
                number,
                cmd,
                slot: slot_index,
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
                    2 * slot_index as u32,
                    &mut ticks,
                    vk::QueryResultFlags::TYPE_64,
                )
                .ok()?;
        }
        let ns =
            ticks[1].wrapping_sub(ticks[0]) as f64 * f64::from(self.gpu.caps().timestamp_period_ns);
        Some(GpuTime {
            frame: slot.frame,
            nanos: ns as u64,
        })
    }

    /// Acquires the swapchain image for this frame. Call it as late as possible: after all
    /// work that doesn't write to the window has been recorded. Rebuilds the swapchain first
    /// if the window changed. `None`: nothing to draw to this frame (still submit it).
    pub fn acquire(&mut self, ctx: &FrameCtx) -> Result<Option<Target>, Error> {
        profiling::scope!("acquire");
        if self.stale {
            self.recreate_swapchain()?;
        }
        let Some(sc) = &self.swapchain else {
            return Ok(None);
        };
        let acquire = self.slots[ctx.slot].acquire;
        // SAFETY: the acquire semaphore is unsignalled (its last wait has completed).
        let result = unsafe {
            self.gpu.swapchain_fn.acquire_next_image(
                sc.raw,
                ACQUIRE_TIMEOUT.as_nanos() as u64,
                acquire,
                vk::Fence::null(),
            )
        };
        match result {
            Ok((index, suboptimal)) => {
                if suboptimal {
                    self.stale = true;
                }
                let i = index as usize;
                Ok(Some(Target {
                    image: sc.images[i],
                    view: sc.views[i],
                    extent: sc.extent,
                    format: sc.format,
                    index,
                }))
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                Ok(None)
            }
            Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => Ok(None),
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
        let slot = &self.slots[ctx.slot];
        // SAFETY: ctx.cmd is in the recording state (begin()).
        unsafe {
            if slot.timed {
                gpu.cmd_write_timestamp2(
                    ctx.cmd,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    self.query_pool,
                    2 * ctx.slot as u32 + 1,
                );
            }
            d.end_command_buffer(ctx.cmd).ctx("vkEndCommandBuffer")?;
        }
        let render_done = target.and_then(|t| {
            self.swapchain
                .as_ref()
                .map(|s| s.render_done[t.index as usize])
        });
        let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(ctx.cmd)];
        let waits = [vk::SemaphoreSubmitInfo::default()
            .semaphore(slot.acquire)
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
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
        let wait_count = usize::from(render_done.is_some());
        let submit = vk::SubmitInfo2::default()
            .command_buffer_infos(&cmds)
            .wait_semaphore_infos(&waits[..wait_count])
            .signal_semaphore_infos(&signals[..signal_count]);
        // SAFETY: one submitting thread; all handles are live; the waited semaphore was
        // signalled by this frame's acquire.
        unsafe { gpu.queue_submit2(std::slice::from_ref(&submit))? };

        let (Some(t), Some(sem), Some(sc)) = (target, render_done, &self.swapchain) else {
            return Ok(Presented::Skipped);
        };
        let swapchains = [sc.raw];
        let indices = [t.index];
        let wait = [sem];
        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait)
            .swapchains(&swapchains)
            .image_indices(&indices);
        // SAFETY: the image was acquired this frame and is in PRESENT_SRC_KHR after the
        // submitted commands (the Target contract).
        match unsafe { gpu.swapchain_fn.queue_present(gpu.queue, &present) } {
            Ok(false) => Ok(Presented::Yes),
            Ok(true) => {
                self.stale = true;
                Ok(Presented::Yes)
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                Ok(Presented::Skipped)
            }
            Err(e) => Err(Error::Vk {
                call: "vkQueuePresentKHR",
                result: e,
            }),
        }
    }

    fn recreate_swapchain(&mut self) -> Result<(), Error> {
        profiling::scope!("recreate_swapchain");
        let old = self.swapchain.take();
        let old_raw = old.as_ref().map_or(vk::SwapchainKHR::null(), |s| s.raw);
        let new = Swapchain::create(&self.gpu, self.pixel_size, self.policy, old_raw);
        if let Some(swapchain) = old {
            // The old one may still be read by queued frames and presents: keep it until a
            // few frames after the last one that used it have completed.
            self.retired.push(Retired {
                swapchain,
                destroy_after: self.frame + MAX_FRAMES_IN_FLIGHT as u64 + 1,
            });
        }
        self.swapchain = new?;
        self.stale = false;
        if let Some(sc) = &self.swapchain {
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

    fn destroy_retired(&mut self) -> Result<(), Error> {
        if self.retired.is_empty() {
            return Ok(());
        }
        let done = self.completed()?;
        let gpu = &self.gpu;
        self.retired.retain_mut(|r| {
            if done >= r.destroy_after {
                // SAFETY: every frame that used the swapchain, and the frames after it, are done.
                unsafe { r.swapchain.destroy(gpu) };
                false
            } else {
                true
            }
        });
        Ok(())
    }
}

impl Drop for FrameLoop {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        // SAFETY: after the idle wait nothing uses any of these objects.
        unsafe {
            let _ = d.device_wait_idle();
            for mut r in self.retired.drain(..) {
                r.swapchain.destroy(&self.gpu);
            }
            if let Some(mut sc) = self.swapchain.take() {
                sc.destroy(&self.gpu);
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
