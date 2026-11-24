//! The copy queue: the one owner of upload submissions that are not part of a frame.
//!
//! On a GPU with a copy engine it submits to the dedicated transfer queue. Without one
//! (Intel, Apple, mobile GPUs, `--no-transfer-queue`) it submits copy-only command buffers
//! to the graphics queue, between the frames' own submissions, so batches can be drained
//! behind a loading screen without presenting.
//!
//! Either way its submissions have a sequence of their own: a `u64` that grows by one with
//! each submission that actually went to the queue, signalled on a timeline semaphore of
//! its own. A value is taken only by a successful `vkQueueSubmit2`; a batch that has to
//! wait (every command buffer busy, or a forced deferral) takes none. Frame numbers are a
//! separate domain: any number of copy submissions can go between two frames.

use ash::vk;
use ash::vk::{Handle, TaggedStructure};

use crate::device::Gpu;
use crate::frame::MAX_FRAMES_IN_FLIGHT;
use crate::{Error, VkContext};

/// Copy submissions that may be unfinished at once. Normally at most two are: the copy
/// engine finishes a frame's copies within that frame. If all are busy, the batch waits
/// for the next attempt instead of stalling.
pub(crate) const COPY_SLOTS: usize = MAX_FRAMES_IN_FLIGHT + 2;

/// One command buffer for copy submissions, reusable once its value is reached.
#[derive(Debug)]
struct Slot {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    /// The sequence value its last submission signals (0: never used).
    value: u64,
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct CopyQueue {
    queue: vk::Queue,
    family: u32,
    /// The queue is a transfer queue of its own, not the graphics queue.
    dedicated: bool,
    /// Signalled by each submission with its sequence value.
    timeline: vk::Semaphore,
    slots: [Slot; COPY_SLOTS],
    /// The timeline's value when last read.
    completed: u64,
    /// The last value a submission took.
    submitted: u64,
}

impl CopyQueue {
    /// The copy queue of `gpu`: its transfer queue if one is in use, else its graphics
    /// queue.
    pub(crate) fn new(gpu: &Gpu) -> Result<CopyQueue, Error> {
        let (queue, family, dedicated) = match gpu.transfer {
            Some((queue, family)) => (queue, family, true),
            None => (gpu.queue, gpu.queue_family, false),
        };
        let d = &gpu.device;
        let mut q = CopyQueue {
            queue,
            family,
            dedicated,
            timeline: vk::Semaphore::null(),
            slots: std::array::from_fn(|_| Slot {
                pool: vk::CommandPool::null(),
                cmd: vk::CommandBuffer::null(),
                value: 0,
            }),
            completed: 0,
            submitted: 0,
        };
        let created = (|| {
            let mut type_info = vk::SemaphoreTypeCreateInfo::default()
                .semaphore_type(vk::SemaphoreType::TIMELINE)
                .initial_value(0);
            let info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
            // SAFETY: plain object creation; destroyed by destroy(), also on error.
            q.timeline = unsafe { d.create_semaphore(&info, None) }
                .ctx("vkCreateSemaphore(copy timeline)")?;
            for s in &mut q.slots {
                let pool_info = vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                    .queue_family_index(family);
                // SAFETY: as above.
                unsafe {
                    s.pool = d
                        .create_command_pool(&pool_info, None)
                        .ctx("vkCreateCommandPool(copy)")?;
                    let alloc = vk::CommandBufferAllocateInfo::default()
                        .command_pool(s.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1);
                    s.cmd = d
                        .allocate_command_buffers(&alloc)
                        .ctx("vkAllocateCommandBuffers(copy)")?[0];
                }
            }
            Ok(())
        })();
        match created {
            Ok(()) => Ok(q),
            Err(e) => {
                // SAFETY: nothing was submitted.
                unsafe { q.destroy(gpu) };
                Err(e)
            }
        }
    }

    pub(crate) fn dedicated(&self) -> bool {
        self.dedicated
    }

    pub(crate) fn family(&self) -> u32 {
        self.family
    }

    pub(crate) fn timeline(&self) -> vk::Semaphore {
        self.timeline
    }

    /// The last sequence value a submission took (0: none yet).
    pub(crate) fn submitted(&self) -> u64 {
        self.submitted
    }

    /// The sequence value the queue had finished when last polled.
    pub(crate) fn completed(&self) -> u64 {
        self.completed
    }

    /// Reads how far the queue has got.
    pub(crate) fn poll(&mut self, gpu: &Gpu) -> Result<u64, Error> {
        // SAFETY: live timeline semaphore.
        self.completed = unsafe { gpu.device.get_semaphore_counter_value(self.timeline) }
            .ctx("vkGetSemaphoreCounterValue(copy)")?;
        Ok(self.completed)
    }

    /// Whether a submission could go now: a command buffer is free.
    pub(crate) fn has_free_slot(&self) -> bool {
        self.slots.iter().any(|s| s.value <= self.completed)
    }

    /// Records `record`'s commands into a free command buffer and submits it, signalling
    /// the next sequence value, which it returns. `None`, with nothing recorded,
    /// submitted or changed, if every command buffer is still in use.
    ///
    /// # Safety
    /// What `record` records is valid on this queue, and everything it uses stays alive
    /// and unmodified by other work until the returned value is reached. One thread
    /// submits to the queue (on the graphics queue: the frame loop's thread).
    pub(crate) unsafe fn submit(
        &mut self,
        gpu: &Gpu,
        record: impl FnOnce(vk::CommandBuffer),
    ) -> Result<Option<u64>, Error> {
        let completed = self.completed;
        let Some(slot) = self.slots.iter_mut().find(|s| s.value <= completed) else {
            return Ok(None);
        };
        // The value exists only once the submission does: a failure below leaves the
        // sequence where it was.
        let value = self.submitted + 1;
        let d = &gpu.device;
        let cmd = slot.cmd;
        // SAFETY: the slot's last submission has finished (its value is reached); the
        // rest is the caller's contract.
        unsafe {
            d.reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty())
                .ctx("vkResetCommandPool(copy)")?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            d.begin_command_buffer(cmd, &begin)
                .ctx("vkBeginCommandBuffer(copy)")?;
            record(cmd);
            d.end_command_buffer(cmd).ctx("vkEndCommandBuffer(copy)")?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            // The signal (after everything, releases included) makes the writes available;
            // the frames that use them wait on it, which makes them visible.
            let signals = [vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(value)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let submit = vk::SubmitInfo2::default()
                .command_buffer_infos(&cmds)
                .signal_semaphore_infos(&signals);
            gpu.submit2_to(self.queue, std::slice::from_ref(&submit))?;
        }
        slot.value = value;
        self.submitted = value;
        Ok(Some(value))
    }

    /// Blocks until every submission has finished.
    pub(crate) fn wait_idle(&mut self, gpu: &Gpu) {
        if self.submitted == 0 || self.timeline.is_null() {
            return;
        }
        let semaphores = [self.timeline];
        let values = [self.submitted];
        let info = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        // SAFETY: live semaphore; the waited value was submitted.
        if unsafe { gpu.device.wait_semaphores(&info, u64::MAX) }.is_ok() {
            self.completed = self.submitted;
        }
    }

    /// # Safety
    /// No submission of this queue is unfinished.
    pub(crate) unsafe fn destroy(&mut self, gpu: &Gpu) {
        let d = &gpu.device;
        // SAFETY: guaranteed by the caller; null handles are skipped.
        unsafe {
            for s in &mut self.slots {
                if !s.pool.is_null() {
                    d.destroy_command_pool(s.pool, None);
                    s.pool = vk::CommandPool::null();
                }
            }
            if !self.timeline.is_null() {
                d.destroy_semaphore(self.timeline, None);
                self.timeline = vk::Semaphore::null();
            }
        }
    }
}
