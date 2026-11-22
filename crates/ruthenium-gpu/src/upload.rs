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
//! are needed. Image copies stay on the graphics queue (images are few, and an animated
//! texture is needed in the frame that writes it). Without a transfer queue (Intel, Apple,
//! mobile GPUs, `--no-transfer-queue`), every copy is recorded on the graphics queue
//! before the frame's passes and is ready in that frame.
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
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UploadError::Full => "the upload ring is full for this frame",
            UploadError::TooLarge => "the upload is larger than the upload ring",
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
    /// Signalled by each submission with the number of the frame that submitted it.
    timeline: vk::Semaphore,
    slots: [TransferSlot; TRANSFER_SLOTS],
    /// The timeline's value when last read.
    completed: u64,
    /// The last value submitted.
    submitted: u64,
    /// Frames whose buffer copies waited a frame because every slot was busy.
    deferred: u64,
}

impl Transfer {
    fn new(gpu: &Gpu, queue: vk::Queue, family: u32) -> Result<Transfer, Error> {
        let d = &gpu.device;
        let mut t = Transfer {
            queue,
            timeline: vk::Semaphore::null(),
            slots: std::array::from_fn(|_| TransferSlot {
                pool: vk::CommandPool::null(),
                cmd: vk::CommandBuffer::null(),
                value: 0,
            }),
            completed: 0,
            submitted: 0,
            deferred: 0,
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

    /// Records and submits the buffer copies, signalling `value`. Returns false, having
    /// done nothing, if every command buffer is still in use.
    ///
    /// # Safety
    /// `value` is above every value submitted before; the staged bytes are written; every
    /// destination is alive and its written ranges are not in use by unfinished work.
    unsafe fn submit(
        &mut self,
        gpu: &Gpu,
        src: vk::Buffer,
        dsts: &[vk::Buffer],
        regions: &[vk::BufferCopy],
        value: u64,
    ) -> Result<bool, Error> {
        let completed = self.completed;
        let Some(slot) = self.slots.iter_mut().find(|s| s.value <= completed) else {
            return Ok(false);
        };
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
            for_runs(dsts, |dst, range| {
                d.cmd_copy_buffer(cmd, src, dst, &regions[range]);
            });
            d.end_command_buffer(cmd)
                .ctx("vkEndCommandBuffer(transfer)")?;
            let cmds = [vk::CommandBufferSubmitInfo::default().command_buffer(cmd)];
            // The signal makes the copies' writes available; the frames that use them wait
            // on it, which makes them visible.
            let signals = [vk::SemaphoreSubmitInfo::default()
                .semaphore(self.timeline)
                .value(value)
                .stage_mask(vk::PipelineStageFlags2::COPY)];
            let submit = vk::SubmitInfo2::default()
                .command_buffer_infos(&cmds)
                .signal_semaphore_infos(&signals);
            gpu.submit2_to(self.queue, std::slice::from_ref(&submit))?;
        }
        slot.value = value;
        self.submitted = value;
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
    /// Buffer copies go here if the device has a transfer queue in use.
    transfer: Option<Transfer>,
    /// The ticket of uploads staged now: the number the next recorded frame will have, at
    /// the least.
    next_frame: u64,
    /// Tickets up to this are usable by the frame last recorded.
    ready: u64,
    buffer_dsts: Vec<vk::Buffer>,
    buffer_regions: Vec<vk::BufferCopy>,
    image_dsts: Vec<vk::Image>,
    image_regions: Vec<vk::BufferImageCopy>,
    images: Vec<ImageDst>,
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
            buffer_dsts: Vec::with_capacity(MAX_BUFFER_COPIES),
            buffer_regions: Vec::with_capacity(MAX_BUFFER_COPIES),
            image_dsts: Vec::with_capacity(MAX_IMAGE_COPIES),
            image_regions: Vec::with_capacity(MAX_IMAGE_COPIES),
            images: Vec::with_capacity(MAX_IMAGES),
            pre: Vec::with_capacity(MAX_IMAGES),
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

    /// Whether buffer copies run on a dedicated transfer queue.
    pub fn uses_transfer_queue(&self) -> bool {
        self.transfer.is_some()
    }

    /// Frames whose buffer copies had to wait a frame for a free transfer command buffer.
    pub fn deferred_transfers(&self) -> u64 {
        self.transfer.as_ref().map_or(0, |t| t.deferred)
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
        if self.buffer_regions.len() == MAX_BUFFER_COPIES || !self.within_budget(data.len() as u64)
        {
            return Err(UploadError::Full);
        }
        let src_offset = self.stage(data)?;
        self.buffer_dsts.push(dst);
        self.buffer_regions.push(vk::BufferCopy {
            src_offset,
            dst_offset,
            size: data.len() as u64,
        });
        Ok(ticket)
    }

    /// Stages `regions` of `dst` (tightly packed texels, rows top to bottom) to be copied
    /// at the start of the next frame. Either every region is staged or none is. Once the
    /// frame's copies are done the image is in SHADER_READ_ONLY_OPTIMAL.
    ///
    /// # Panics
    /// If a region lies outside its mip level, or its data is not exactly its size.
    pub fn upload_image(
        &mut self,
        dst: &ImageDst,
        regions: &[(ImageRegion, &[u8])],
    ) -> Result<(), UploadError> {
        if regions.is_empty() {
            return Ok(());
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
        let known = self.images.iter().position(|i| i.image == dst.image);
        if regions.len() > MAX_IMAGE_COPIES {
            return Err(UploadError::TooLarge);
        }
        if self.image_regions.len() + regions.len() > MAX_IMAGE_COPIES
            || (known.is_none() && self.images.len() == MAX_IMAGES)
        {
            return Err(UploadError::Full);
        }
        let total: u64 = regions.iter().map(|(_, d)| d.len() as u64).sum();
        if !self.within_budget(total) {
            return Err(UploadError::Full);
        }
        let (head, staged, copies) = (self.ring.head, self.staged, self.image_regions.len());
        for (r, data) in regions {
            match self.stage(data) {
                Ok(buffer_offset) => {
                    self.image_dsts.push(dst.image);
                    self.image_regions.push(vk::BufferImageCopy {
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
                Err(e) => {
                    // Undo the regions staged so far: the bytes were not handed out yet.
                    self.ring.head = head;
                    self.staged = staged;
                    self.image_dsts.truncate(copies);
                    self.image_regions.truncate(copies);
                    return Err(e);
                }
            }
        }
        match known {
            // Written twice this frame: if either write saw it unwritten, it is unwritten.
            Some(i) => self.images[i].fresh |= dst.fresh,
            None => self.images.push(*dst),
        }
        Ok(())
    }

    /// Sends the staged copies on their way, frees the space of finished frames, and makes
    /// the finished copies visible to `ctx`'s frame. Call it once per frame, before the
    /// frame's passes:
    ///
    /// - buffer copies are submitted to the transfer queue if there is one (and `ctx` gets
    ///   a wait on the transfer timeline), else recorded into `ctx.cmd`;
    /// - image copies are recorded into `ctx.cmd`, with the barriers that make them visible
    ///   to this frame's and later frames' shaders.
    ///
    /// # Safety
    /// `ctx` is the frame being recorded on the graphics queue, and `completed` a frame
    /// number the GPU has finished; frame numbers grow with each call; every destination
    /// buffer and image is still alive.
    pub unsafe fn record(&mut self, ctx: &mut FrameCtx, completed: u64) -> Result<(), Error> {
        profiling::scope!("record_uploads");
        let frame = ctx.number;
        debug_assert!(frame >= self.next_frame, "frames recorded out of order");
        let mut transfer_bytes = false;
        match &mut self.transfer {
            Some(t) => {
                let transfer_completed = t.poll(&self.gpu)?;
                self.ring.retire(completed, transfer_completed);
                if !self.buffer_regions.is_empty() {
                    // SAFETY: frame numbers grow, so `frame` is above every earlier value;
                    // the rest is the caller's contract and upload_buffer's.
                    let sent = unsafe {
                        t.submit(
                            &self.gpu,
                            self.raw,
                            &self.buffer_dsts,
                            &self.buffer_regions,
                            frame,
                        )?
                    };
                    if sent {
                        self.buffer_dsts.clear();
                        self.buffer_regions.clear();
                    } else {
                        // Kept for the next frame's submission, which signals a later value.
                        t.deferred += 1;
                    }
                    transfer_bytes = true;
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
        // On the transfer path the buffer lists hold only deferred copies here.
        let graphics_buffers = self.transfer.is_none() && !self.buffer_regions.is_empty();
        if !graphics_buffers && self.image_regions.is_empty() {
            self.staged = 0;
            self.ring.close(frame, transfer_bytes);
            return Ok(());
        }
        let cmd = ctx.cmd;
        let gpu = &*self.gpu;
        let d = &gpu.device;
        let range = |i: &ImageDst| vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: i.mip_levels,
            base_array_layer: 0,
            layer_count: 1,
        };
        self.pre.clear();
        self.post.clear();
        let shaders = vk::PipelineStageFlags2::VERTEX_SHADER
            | vk::PipelineStageFlags2::FRAGMENT_SHADER
            | vk::PipelineStageFlags2::COMPUTE_SHADER;
        for i in &self.images {
            let (old, src_stage) = if i.fresh {
                (vk::ImageLayout::UNDEFINED, vk::PipelineStageFlags2::NONE)
            } else {
                // Earlier frames' reads must finish before the copy overwrites texels.
                (vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, shaders)
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
                    .subresource_range(range(i)),
            );
            self.post.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::COPY)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(shaders)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .image(i.image)
                    .subresource_range(range(i)),
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
                    | shaders,
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
                for_runs(&self.buffer_dsts, |dst, range| {
                    d.cmd_copy_buffer(cmd, self.raw, dst, &self.buffer_regions[range]);
                });
            }
            for_runs(&self.image_dsts, |dst, range| {
                d.cmd_copy_buffer_to_image(
                    cmd,
                    self.raw,
                    dst,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &self.image_regions[range],
                );
            });
            gpu.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default()
                    .memory_barriers(buffer_barriers)
                    .image_memory_barriers(&self.post),
            );
        }
        if graphics_buffers {
            self.buffer_dsts.clear();
            self.buffer_regions.clear();
        }
        self.image_dsts.clear();
        self.image_regions.clear();
        self.images.clear();
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
