//! The upload ring: one persistently mapped staging buffer that CPU data is copied into,
//! and the copies to buffers and images that the next frame records before its passes.
//!
//! - Space is handed out in frame order and comes back once the frame that copied it has
//!   finished on the GPU (the frame timeline), so staging never waits for the GPU.
//! - When the ring, the per-frame byte budget or the per-frame copy lists are used up,
//!   staging returns [`UploadError::Full`] and the caller tries again next frame: an upload
//!   burst spreads over frames instead of stalling one.
//! - All copies of a frame are recorded together, between one batch of barriers before and
//!   one after (vanilla puts a full pipeline barrier after every single copy).
//!
//! **Queues** (research 04 §4.3). On a GPU with a dedicated transfer queue (a copy engine:
//! NVIDIA, AMD), buffer copies are submitted there once per frame and signal the ring's own
//! transfer timeline, so a burst of mesh uploads never occupies the graphics queue. Their
//! data is usable from the first frame recorded after the copies have finished (usually
//! the next one): check with [`UploadRing::is_ready`]. That frame's submit waits on the
//! transfer timeline (already signalled, so the wait costs nothing), which makes the copies
//! visible to it. Buffers are created with concurrent sharing, so no ownership transfers
//! are needed. The first upload of an image that is not needed at once
//! ([`ImageDst::background`]: atlases, skins) goes there too; images are exclusive to one
//! queue family, so the transfer queue releases it and the frame that first may use it
//! acquires it. Later writes to an image (animated textures, partial updates) are needed
//! in the frame that makes them and stay on the graphics queue. Without a transfer queue
//! (Intel, Apple, mobile GPUs, `--no-transfer-queue`), every copy is recorded on the
//! graphics queue before the frame's passes and is ready in that frame.
//!
//! Nothing here allocates once the ring exists.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use ash::vk;
use ash::vk::{Handle, TaggedStructure};

use crate::device::Gpu;
use crate::frame::{FrameCtx, MAX_FRAMES_IN_FLIGHT};
use crate::memory::{Buffer, MemoryUse, texel_bytes};
use crate::{Error, VkContext};

/// Buffer copy regions one frame can record.
const MAX_BUFFER_COPIES: usize = 4096;
/// Buffer-to-image regions one frame can record (one per mip level or sub-rectangle).
const MAX_IMAGE_COPIES: usize = 1024;
/// Distinct images one frame can write.
const MAX_IMAGES: usize = 256;
/// Staging offsets are at least this aligned: a multiple of every texel size and of 4,
/// as image copies require.
const MIN_ALIGN: u64 = 16;
/// The largest alignment honoured; the ring's capacity is a multiple of it.
const MAX_ALIGN: u64 = 256;
/// Transfer submissions that may be unfinished at once. Normally at most two are: the copy
/// engine finishes a frame's copies within that frame. If all are busy, the frame's buffer
/// copies wait for the next frame instead of stalling.
const TRANSFER_SLOTS: usize = MAX_FRAMES_IN_FLIGHT + 2;

/// Why data could not be staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadError {
    /// No room this frame (the ring, the frame's byte budget or its copy lists are used
    /// up); try again next frame.
    Full,
    /// More than the whole ring or a frame's copy lists can ever hold; split it.
    TooLarge,
    /// The texture's first upload has not arrived yet; try again in a later frame.
    NotReady,
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UploadError::Full => "the upload ring is full for this frame",
            UploadError::TooLarge => "the upload is larger than the upload ring",
            UploadError::NotReady => "the texture's first upload has not arrived yet",
        })
    }
}

impl std::error::Error for UploadError {}

/// When a buffer upload can be used: pass it to [`UploadRing::is_ready`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UploadTicket(u64);

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

/// The end of one frame's bytes in the ring.
#[derive(Clone, Copy, Debug)]
struct Mark {
    /// The frame that recorded the copies (its graphics timeline value).
    frame: u64,
    /// The ring's head after them.
    head: u64,
    /// Some of the bytes before `head` are copied on the transfer queue, by a submission
    /// signalling a transfer timeline value of at least `frame`.
    transfer: bool,
}

/// Byte positions in the ring. Positions only grow (a `u64` does not wrap in practice);
/// position `p` is byte `p % capacity` of the buffer.
#[derive(Debug)]
struct Ring {
    capacity: u64,
    /// Where the next allocation starts.
    head: u64,
    /// Everything before this is free again.
    tail: u64,
    /// For each recorded frame not yet known to be finished, oldest first; at most a few.
    marks: VecDeque<Mark>,
}

impl Ring {
    fn new(capacity: u64) -> Ring {
        Ring {
            capacity,
            head: 0,
            tail: 0,
            marks: VecDeque::with_capacity(MAX_FRAMES_IN_FLIGHT + TRANSFER_SLOTS + 2),
        }
    }

    /// Reserves `size` contiguous bytes aligned to `align` (a power of two dividing the
    /// capacity) and returns their offset in the buffer. A block never wraps around the end:
    /// if it does not fit before it, the rest of the buffer is skipped.
    fn alloc(&mut self, size: u64, align: u64) -> Result<u64, UploadError> {
        debug_assert!(align.is_power_of_two() && self.capacity.is_multiple_of(align));
        if size > self.capacity {
            return Err(UploadError::TooLarge);
        }
        let mut start = self.head.next_multiple_of(align);
        let offset = start % self.capacity;
        if offset + size > self.capacity {
            start += self.capacity - offset;
        }
        if start + size - self.tail > self.capacity {
            return Err(UploadError::Full);
        }
        self.head = start + size;
        Ok(start % self.capacity)
    }

    /// Everything allocated so far is copied by `frame` and, if `transfer`, partly by a
    /// transfer submission signalling `frame` or later.
    fn close(&mut self, frame: u64, transfer: bool) {
        self.marks.push_back(Mark {
            frame,
            head: self.head,
            transfer,
        });
    }

    /// Frees the space of every frame up to `completed` on the graphics timeline whose
    /// transfer copies, if any, are done (`transfer_completed` has reached the frame).
    fn retire(&mut self, completed: u64, transfer_completed: u64) {
        while let Some(&m) = self.marks.front() {
            if m.frame > completed || (m.transfer && m.frame > transfer_completed) {
                break;
            }
            self.tail = m.head;
            self.marks.pop_front();
        }
        // Nothing in flight: start again at the beginning, so a large block fits whole.
        if self.marks.is_empty() && self.head == self.tail {
            let rewound = self.head.next_multiple_of(self.capacity);
            self.head = rewound;
            self.tail = rewound;
        }
    }
}

/// Images released by the transfer queue and not yet acquired by the graphics queue.
const MAX_RELEASED: usize = MAX_IMAGES * TRANSFER_SLOTS;

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

/// Buffer copies waiting for the next submission.
#[derive(Debug)]
struct BufferCopies {
    dsts: Vec<vk::Buffer>,
    regions: Vec<vk::BufferCopy>,
}

impl BufferCopies {
    fn new() -> BufferCopies {
        BufferCopies {
            dsts: Vec::with_capacity(MAX_BUFFER_COPIES),
            regions: Vec::with_capacity(MAX_BUFFER_COPIES),
        }
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn clear(&mut self) {
        self.dsts.clear();
        self.regions.clear();
    }

    /// # Safety
    /// `cmd` is recording; `src` and every destination are alive.
    unsafe fn record(&self, d: &ash::Device, cmd: vk::CommandBuffer, src: vk::Buffer) {
        for_runs(&self.dsts, |dst, range| {
            // SAFETY: guaranteed by the caller.
            unsafe { d.cmd_copy_buffer(cmd, src, dst, &self.regions[range]) };
        });
    }
}

/// Image copies bound for one queue, waiting for the next submission.
#[derive(Debug)]
struct ImageCopies {
    images: Vec<ImageDst>,
    dsts: Vec<vk::Image>,
    regions: Vec<vk::BufferImageCopy>,
}

impl ImageCopies {
    fn new() -> ImageCopies {
        ImageCopies {
            images: Vec::with_capacity(MAX_IMAGES),
            dsts: Vec::with_capacity(MAX_IMAGE_COPIES),
            regions: Vec::with_capacity(MAX_IMAGE_COPIES),
        }
    }

    fn find(&self, image: vk::Image) -> Option<usize> {
        self.images.iter().position(|i| i.image == image)
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn clear(&mut self) {
        self.images.clear();
        self.dsts.clear();
        self.regions.clear();
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

/// One command buffer for transfer submissions, reusable once its value is reached.
#[derive(Debug)]
struct TransferSlot {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    /// Transfer timeline value its last submission signals (0: never used).
    value: u64,
}

/// The dedicated transfer queue's side of the ring.
#[derive(Debug)]
struct Transfer {
    queue: vk::Queue,
    family: u32,
    /// Signalled by each submission with the number of the frame that submitted it.
    timeline: vk::Semaphore,
    slots: [TransferSlot; TRANSFER_SLOTS],
    /// The timeline's value when last read.
    completed: u64,
    /// The last value submitted.
    submitted: u64,
    /// Frames whose transfer copies waited a frame because every slot was busy.
    deferred: u64,
    /// Images uploaded and released to the graphics queue.
    images: u64,
    pre: Vec<vk::ImageMemoryBarrier2<'static>>,
    post: Vec<vk::ImageMemoryBarrier2<'static>>,
}

impl Transfer {
    fn new(gpu: &Gpu, queue: vk::Queue, family: u32) -> Result<Transfer, Error> {
        let d = &gpu.device;
        let mut t = Transfer {
            queue,
            family,
            timeline: vk::Semaphore::null(),
            slots: std::array::from_fn(|_| TransferSlot {
                pool: vk::CommandPool::null(),
                cmd: vk::CommandBuffer::null(),
                value: 0,
            }),
            completed: 0,
            submitted: 0,
            deferred: 0,
            images: 0,
            pre: Vec::with_capacity(MAX_IMAGES),
            post: Vec::with_capacity(MAX_IMAGES),
        };
        let created = (|| {
            let mut type_info = vk::SemaphoreTypeCreateInfo::default()
                .semaphore_type(vk::SemaphoreType::TIMELINE)
                .initial_value(0);
            let info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
            // SAFETY: plain object creation; destroyed by destroy(), also on error.
            t.timeline = unsafe { d.create_semaphore(&info, None) }
                .ctx("vkCreateSemaphore(transfer timeline)")?;
            for s in &mut t.slots {
                let pool_info = vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                    .queue_family_index(family);
                // SAFETY: as above.
                unsafe {
                    s.pool = d
                        .create_command_pool(&pool_info, None)
                        .ctx("vkCreateCommandPool(transfer)")?;
                    let alloc = vk::CommandBufferAllocateInfo::default()
                        .command_pool(s.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1);
                    s.cmd = d
                        .allocate_command_buffers(&alloc)
                        .ctx("vkAllocateCommandBuffers(transfer)")?[0];
                }
            }
            Ok(())
        })();
        match created {
            Ok(()) => Ok(t),
            Err(e) => {
                // SAFETY: nothing was submitted.
                unsafe { t.destroy(gpu) };
                Err(e)
            }
        }
    }

    /// Reads how far the transfer queue has got.
    fn poll(&mut self, gpu: &Gpu) -> Result<u64, Error> {
        // SAFETY: live timeline semaphore.
        self.completed = unsafe { gpu.device.get_semaphore_counter_value(self.timeline) }
            .ctx("vkGetSemaphoreCounterValue(transfer)")?;
        Ok(self.completed)
    }

    /// Records and submits the copies, signalling `value`. Images go from UNDEFINED to
    /// TRANSFER_DST_OPTIMAL, are written, and are released to the graphics family in
    /// SHADER_READ_ONLY_OPTIMAL (the graphics queue acquires them). Returns false, having
    /// done nothing, if every command buffer is still in use.
    ///
    /// # Safety
    /// `value` is above every value submitted before; the staged bytes are written; every
    /// destination is alive, its written ranges are not in use by unfinished work, and
    /// every image is fresh (its old contents may be discarded).
    unsafe fn submit(
        &mut self,
        gpu: &Gpu,
        src: vk::Buffer,
        buffers: &BufferCopies,
        images: &ImageCopies,
        value: u64,
    ) -> Result<bool, Error> {
        let completed = self.completed;
        let Some(slot) = self.slots.iter_mut().find(|s| s.value <= completed) else {
            return Ok(false);
        };
        self.pre.clear();
        self.post.clear();
        for i in &images.images {
            self.pre.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::NONE)
                    .src_access_mask(vk::AccessFlags2::NONE)
                    .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .image(i.image)
                    .subresource_range(whole(i)),
            );
            // The release half of the queue family ownership transfer.
            self.post.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                    .dst_access_mask(vk::AccessFlags2::NONE)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(self.family)
                    .dst_queue_family_index(gpu.queue_family)
                    .image(i.image)
                    .subresource_range(whole(i)),
            );
        }
        let d = &gpu.device;
        let cmd = slot.cmd;
        // SAFETY: the slot's last submission has finished (its value is reached); one thread
        // submits to the transfer queue (the ring's owner, through &mut self).
        unsafe {
            d.reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty())
                .ctx("vkResetCommandPool(transfer)")?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            d.begin_command_buffer(cmd, &begin)
                .ctx("vkBeginCommandBuffer(transfer)")?;
            if !self.pre.is_empty() {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&self.pre),
                );
            }
            buffers.record(d, cmd, src);
            images.record(d, cmd, src);
            if !self.post.is_empty() {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default().image_memory_barriers(&self.post),
                );
            }
            d.end_command_buffer(cmd)
                .ctx("vkEndCommandBuffer(transfer)")?;
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
        self.images += images.images.len() as u64;
        Ok(true)
    }

    /// # Safety
    /// No submission of this transfer is unfinished.
    unsafe fn destroy(&mut self, gpu: &Gpu) {
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

/// The staging ring and the copies waiting for the next frame. See the module docs.
pub struct UploadRing {
    gpu: Arc<Gpu>,
    buffer: Option<Buffer>,
    raw: vk::Buffer,
    mapped: *mut u8,
    ring: Ring,
    align: u64,
    /// Bytes one frame may stage; one upload larger than this still goes, alone.
    budget: u64,
    /// Bytes staged for the next frame.
    staged: u64,
    /// The dedicated transfer queue, if the device has one in use.
    transfer: Option<Transfer>,
    /// The ticket of uploads staged now: the number the next recorded frame will have, at
    /// the least.
    next_frame: u64,
    /// Tickets up to this are usable by the frame last recorded.
    ready: u64,
    /// Buffer copies, for the transfer queue if there is one, else the graphics queue.
    buffers: BufferCopies,
    /// Image copies recorded into the frame.
    graphics_images: ImageCopies,
    /// First uploads of background images, for the transfer queue.
    transfer_images: ImageCopies,
    /// Images the transfer queue releases, with the submission's timeline value, waiting
    /// for the graphics queue to acquire them.
    released: Vec<(u64, ImageDst)>,
    /// Staging offsets of the regions of the `upload_image` call in progress.
    staged_offsets: Vec<u64>,
    /// Scratch space for the frame's barriers.
    pre: Vec<vk::ImageMemoryBarrier2<'static>>,
    post: Vec<vk::ImageMemoryBarrier2<'static>>,
}

impl fmt::Debug for UploadRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadRing")
            .field("capacity", &self.ring.capacity)
            .field("budget", &self.budget)
            .field("staged", &self.staged)
            .field("transfer_queue", &self.transfer.is_some())
            .finish_non_exhaustive()
    }
}

// SAFETY: the mapping is only written through &mut self; Vulkan handles may move threads.
unsafe impl Send for UploadRing {}

impl UploadRing {
    /// Creates a ring of `capacity` bytes (rounded up to 64 KiB) letting each frame stage
    /// up to `budget` bytes.
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
        let transfer = match gpu.transfer {
            Some((queue, family)) => match Transfer::new(&gpu, queue, family) {
                Ok(t) => Some(t),
                Err(e) => {
                    // SAFETY: unused.
                    unsafe { buffer.destroy(&gpu) };
                    return Err(e);
                }
            },
            None => None,
        };
        Ok(UploadRing {
            raw: buffer.raw,
            buffer: Some(buffer),
            mapped,
            ring: Ring::new(capacity),
            align,
            budget,
            staged: 0,
            transfer,
            next_frame: 1,
            ready: 0,
            buffers: BufferCopies::new(),
            graphics_images: ImageCopies::new(),
            transfer_images: ImageCopies::new(),
            released: Vec::with_capacity(MAX_RELEASED),
            staged_offsets: Vec::with_capacity(MAX_IMAGE_COPIES),
            pre: Vec::with_capacity(MAX_IMAGES + MAX_RELEASED),
            post: Vec::with_capacity(MAX_IMAGES),
            gpu,
        })
    }

    pub fn capacity(&self) -> u64 {
        self.ring.capacity
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Changes the bytes each frame may stage.
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
    }

    /// Bytes staged so far for the next frame.
    pub fn staged(&self) -> u64 {
        self.staged
    }

    /// Whether buffer copies and background image uploads run on a dedicated transfer
    /// queue.
    pub fn uses_transfer_queue(&self) -> bool {
        self.transfer.is_some()
    }

    /// Frames whose transfer copies had to wait a frame for a free command buffer.
    pub fn deferred_transfers(&self) -> u64 {
        self.transfer.as_ref().map_or(0, |t| t.deferred)
    }

    /// Images whose first upload went on the transfer queue.
    pub fn transferred_images(&self) -> u64 {
        self.transfer.as_ref().map_or(0, |t| t.images)
    }

    /// Whether the frame last passed to [`Self::record`] (and every later one) may read
    /// what the upload with this ticket wrote.
    pub fn is_ready(&self, ticket: UploadTicket) -> bool {
        ticket.0 <= self.ready
    }

    /// Whether `bytes` more fit in this frame's budget. Anything fits in an empty frame, so
    /// an upload larger than the budget still goes, alone.
    fn within_budget(&self, bytes: u64) -> bool {
        self.staged == 0 || self.staged + bytes <= self.budget
    }

    /// Reserves space for the next frame and copies `data` into it.
    fn stage(&mut self, data: &[u8]) -> Result<u64, UploadError> {
        let size = data.len() as u64;
        let offset = self.ring.alloc(size, self.align)?;
        // SAFETY: [offset, offset + size) lies inside the mapping and is not read by any
        // unfinished GPU work (the ring's invariant).
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.mapped.add(offset as usize),
                data.len(),
            )
        };
        self.staged += size;
        Ok(offset)
    }

    /// Stages `data` to be copied to `dst` at `dst_offset`, and returns when the copy can
    /// be used ([`Self::is_ready`]): on the graphics queue, in the next frame; on a transfer
    /// queue, once that frame's copies have finished.
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
        let ticket = UploadTicket(self.next_frame);
        if data.is_empty() {
            return Ok(ticket);
        }
        if self.buffers.regions.len() == MAX_BUFFER_COPIES || !self.within_budget(data.len() as u64)
        {
            return Err(UploadError::Full);
        }
        let src_offset = self.stage(data)?;
        self.buffers.dsts.push(dst);
        self.buffers.regions.push(vk::BufferCopy {
            src_offset,
            dst_offset,
            size: data.len() as u64,
        });
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
    /// join it in the same frame).
    ///
    /// # Panics
    /// If a region lies outside its mip level, or its data is not exactly its size.
    pub fn upload_image(
        &mut self,
        dst: &ImageDst,
        regions: &[(ImageRegion, &[u8])],
    ) -> Result<UploadTicket, UploadError> {
        let ticket = UploadTicket(self.next_frame);
        if regions.is_empty() {
            return Ok(ticket);
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
        let in_graphics = self.graphics_images.find(dst.image);
        let in_transfer = self.transfer_images.find(dst.image);
        debug_assert!(
            dst.fresh
                || (in_transfer.is_none()
                    && !self.released.iter().any(|(_, i)| i.image == dst.image)),
            "{:?} written before its first upload is ready",
            dst.image
        );
        // Background first uploads (and fresh writes joining one) go to the transfer queue.
        let to_transfer = self.transfer.is_some()
            && in_graphics.is_none()
            && (in_transfer.is_some() || (dst.fresh && dst.background));
        let (copies, known) = if to_transfer {
            (&self.transfer_images, in_transfer)
        } else {
            (&self.graphics_images, in_graphics)
        };
        if copies.regions.len() + regions.len() > MAX_IMAGE_COPIES
            || (known.is_none() && copies.images.len() == MAX_IMAGES)
            || (to_transfer
                && known.is_none()
                && self.released.len() + copies.images.len() >= MAX_RELEASED)
        {
            return Err(UploadError::Full);
        }
        let total: u64 = regions.iter().map(|(_, d)| d.len() as u64).sum();
        if !self.within_budget(total) {
            return Err(UploadError::Full);
        }
        let (head, staged) = (self.ring.head, self.staged);
        self.staged_offsets.clear();
        for (_, data) in regions {
            match self.stage(data) {
                Ok(offset) => self.staged_offsets.push(offset),
                Err(e) => {
                    // Undo the regions staged so far: the bytes were not handed out yet.
                    self.ring.head = head;
                    self.staged = staged;
                    return Err(e);
                }
            }
        }
        let copies = if to_transfer {
            &mut self.transfer_images
        } else {
            &mut self.graphics_images
        };
        for ((r, _), &buffer_offset) in regions.iter().zip(&self.staged_offsets) {
            copies.dsts.push(dst.image);
            copies.regions.push(vk::BufferImageCopy {
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
            });
        }
        match known {
            // Written twice this frame: if either write saw it unwritten, it is unwritten.
            Some(i) => copies.images[i].fresh |= dst.fresh,
            None => copies.images.push(*dst),
        }
        Ok(ticket)
    }

    /// Sends the staged copies on their way, frees the space of finished frames, and makes
    /// the finished copies visible to `ctx`'s frame. Call it once per frame, before the
    /// frame's passes:
    ///
    /// - with a transfer queue, buffer copies and background first uploads are submitted
    ///   there; `ctx` gets a wait on the transfer timeline, and acquires the images whose
    ///   transfer has finished;
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
        self.pre.clear();
        self.post.clear();
        let mut transfer_bytes = false;
        match &mut self.transfer {
            Some(t) => {
                let transfer_completed = t.poll(&self.gpu)?;
                self.ring.retire(completed, transfer_completed);
                if !self.buffers.is_empty() || !self.transfer_images.is_empty() {
                    // SAFETY: frame numbers grow, so `frame` is above every earlier value;
                    // transfer images are all fresh (upload_image's routing); the rest is
                    // the caller's contract and the upload calls'.
                    let sent = unsafe {
                        t.submit(
                            &self.gpu,
                            self.raw,
                            &self.buffers,
                            &self.transfer_images,
                            frame,
                        )?
                    };
                    if sent {
                        for i in &self.transfer_images.images {
                            self.released.push((frame, *i));
                        }
                        self.buffers.clear();
                        self.transfer_images.clear();
                    } else {
                        // Kept for the next frame's submission, which signals a later value.
                        t.deferred += 1;
                    }
                    transfer_bytes = true;
                }
                // The acquire half of the ownership transfer, for every image whose release
                // has finished. The frame's wait on the transfer timeline orders it after
                // the release.
                let mut i = 0;
                while i < self.released.len() {
                    if self.released[i].0 > transfer_completed {
                        i += 1;
                        continue;
                    }
                    let (_, img) = self.released.swap_remove(i);
                    self.pre.push(
                        vk::ImageMemoryBarrier2::default()
                            .src_stage_mask(vk::PipelineStageFlags2::NONE)
                            .src_access_mask(vk::AccessFlags2::NONE)
                            .dst_stage_mask(SHADERS)
                            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                            .src_queue_family_index(t.family)
                            .dst_queue_family_index(self.gpu.queue_family)
                            .image(img.image)
                            .subresource_range(whole(&img)),
                    );
                }
                // Copies up to the completed value are done: this frame may use them. The
                // wait is already satisfied; it makes their writes visible to the frame.
                self.ready = transfer_completed;
                if transfer_completed > 0 {
                    ctx.wait_timeline(
                        t.timeline,
                        transfer_completed,
                        vk::PipelineStageFlags2::ALL_COMMANDS,
                    );
                }
            }
            None => {
                self.ring.retire(completed, 0);
                self.ready = frame;
            }
        }
        self.next_frame = frame + 1;
        // On the transfer path the buffer list holds only deferred copies here.
        let graphics_buffers = self.transfer.is_none() && !self.buffers.is_empty();
        if !graphics_buffers && self.graphics_images.is_empty() && self.pre.is_empty() {
            self.staged = 0;
            self.ring.close(frame, transfer_bytes);
            return Ok(());
        }
        for i in &self.graphics_images.images {
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
        let buffer_barriers = if graphics_buffers {
            &buffers_after[..]
        } else {
            &buffers_after[..0]
        };
        let cmd = ctx.cmd;
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
            if graphics_buffers {
                self.buffers.record(&gpu.device, cmd, self.raw);
            }
            self.graphics_images.record(&gpu.device, cmd, self.raw);
            if graphics_buffers || !self.post.is_empty() {
                gpu.cmd_pipeline_barrier2(
                    cmd,
                    &vk::DependencyInfo::default()
                        .memory_barriers(buffer_barriers)
                        .image_memory_barriers(&self.post),
                );
            }
        }
        if graphics_buffers {
            self.buffers.clear();
        }
        self.graphics_images.clear();
        self.staged = 0;
        self.ring.close(frame, transfer_bytes);
        Ok(())
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

impl Drop for UploadRing {
    fn drop(&mut self) {
        if let Some(mut t) = self.transfer.take() {
            // Copies on the transfer queue are not covered by the frame loop's idle wait.
            let semaphores = [t.timeline];
            let values = [t.submitted];
            let info = vk::SemaphoreWaitInfo::default()
                .semaphores(&semaphores)
                .values(&values);
            // SAFETY: live semaphore, every waited value was submitted; after the wait no
            // transfer submission is unfinished.
            unsafe {
                let _ = self.gpu.device.wait_semaphores(&info, u64::MAX);
                t.destroy(&self.gpu);
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
    fn hands_out_aligned_blocks_until_full() {
        let mut r = Ring::new(1024);
        assert_eq!(r.alloc(100, 16), Ok(0));
        assert_eq!(r.alloc(100, 16), Ok(112));
        assert_eq!(r.alloc(800, 16), Ok(224));
        assert_eq!(r.alloc(1, 16), Err(UploadError::Full));
        assert_eq!(r.alloc(2000, 16), Err(UploadError::TooLarge));
    }

    #[test]
    fn space_returns_when_its_frame_finishes() {
        let mut r = Ring::new(1024);
        r.alloc(600, 16).unwrap();
        r.close(1, false);
        r.alloc(300, 16).unwrap();
        r.close(2, false);
        assert_eq!(r.alloc(600, 16), Err(UploadError::Full));
        r.retire(0, 0);
        assert_eq!(r.alloc(600, 16), Err(UploadError::Full));
        r.retire(1, 0);
        // 608..908 is still in use by frame 2; the block wraps to the start.
        assert_eq!(r.alloc(600, 16), Ok(0));
        r.close(3, false);
        r.retire(2, 0);
        // Free now: 608..908 only (908..1024 was skipped by frame 3, which uses 0..600).
        assert_eq!(r.alloc(304, 16), Err(UploadError::Full));
        assert_eq!(r.alloc(288, 16), Ok(608));
    }

    #[test]
    fn a_block_never_straddles_the_end() {
        let mut r = Ring::new(1024);
        r.alloc(900, 16).unwrap();
        r.close(1, false);
        r.alloc(16, 16).unwrap(); // 912..928
        r.close(2, false);
        r.retire(1, 0);
        // 200 bytes don't fit in 928..1024: skip to 0 (free since frame 1 finished).
        assert_eq!(r.alloc(200, 16), Ok(0));
        // ...but not past the frame-2 bytes.
        assert_eq!(r.alloc(800, 16), Err(UploadError::Full));
    }

    #[test]
    fn an_idle_ring_rewinds_so_a_whole_ring_block_fits() {
        let mut r = Ring::new(1024);
        r.alloc(100, 16).unwrap();
        r.close(1, false);
        r.retire(1, 0);
        assert_eq!(r.alloc(1024, 16), Ok(0));
    }

    #[test]
    fn transfer_bytes_wait_for_both_timelines() {
        let mut r = Ring::new(1024);
        r.alloc(512, 16).unwrap();
        r.close(1, true);
        r.alloc(256, 16).unwrap();
        r.close(2, false);
        // Graphics has finished both frames, but frame 1's transfer copies have not run:
        // nothing is free, not even frame 2's bytes behind them.
        r.retire(2, 0);
        assert_eq!(r.alloc(512, 16), Err(UploadError::Full));
        // A transfer value at or above the frame (it may have been deferred to a later
        // submission) frees it, and the frames after it.
        r.retire(2, 1);
        assert_eq!(r.alloc(1024, 16), Ok(0));
    }

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
}
