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
//! Copies run on the graphics queue for now; a dedicated transfer queue (research 04 §4.3)
//! slots in behind the same calls.
//!
//! Nothing here allocates once the ring exists.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use ash::vk;

use crate::Error;
use crate::device::Gpu;
use crate::frame::MAX_FRAMES_IN_FLIGHT;
use crate::memory::{Buffer, MemoryUse, texel_bytes};

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

/// Byte positions in the ring. Positions only grow (a `u64` does not wrap in practice);
/// position `p` is byte `p % capacity` of the buffer.
#[derive(Debug)]
struct Ring {
    capacity: u64,
    /// Where the next allocation starts.
    head: u64,
    /// Everything before this is free again.
    tail: u64,
    /// For each recorded frame not yet known to be finished: its number and the head after
    /// its copies. Oldest first; at most a few long.
    marks: VecDeque<(u64, u64)>,
}

impl Ring {
    fn new(capacity: u64) -> Ring {
        Ring {
            capacity,
            head: 0,
            tail: 0,
            marks: VecDeque::with_capacity(MAX_FRAMES_IN_FLIGHT + 2),
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

    /// Everything allocated so far is copied by `frame`.
    fn close(&mut self, frame: u64) {
        self.marks.push_back((frame, self.head));
    }

    /// Frees the space of every frame up to `completed`.
    fn retire(&mut self, completed: u64) {
        while let Some(&(frame, head)) = self.marks.front() {
            if frame > completed {
                break;
            }
            self.tail = head;
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
        Ok(UploadRing {
            raw: buffer.raw,
            buffer: Some(buffer),
            mapped,
            ring: Ring::new(capacity),
            align,
            budget,
            staged: 0,
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

    /// Stages `data` to be copied to `dst` at `dst_offset` at the start of the next frame.
    ///
    /// The destination range must not be in use by unfinished frames (write to fresh
    /// ranges, and reuse a range only after the frames reading it have finished).
    pub fn upload_buffer(
        &mut self,
        dst: vk::Buffer,
        dst_offset: u64,
        data: &[u8],
    ) -> Result<(), UploadError> {
        if data.is_empty() {
            return Ok(());
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
        Ok(())
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

    /// Records every staged copy into `cmd`, with the barriers that make them visible to
    /// this frame's and later frames' shaders, and frees the space of frames up to
    /// `completed`. Call it once per frame, before the frame's passes.
    ///
    /// # Safety
    /// `cmd` is recording frame `frame` (which signals that timeline value when done) on
    /// the graphics queue; `completed` is a frame number the GPU has finished; every
    /// destination buffer and image is still alive.
    pub unsafe fn record(&mut self, cmd: vk::CommandBuffer, frame: u64, completed: u64) {
        profiling::scope!("record_uploads");
        self.ring.retire(completed);
        if self.buffer_regions.is_empty() && self.image_regions.is_empty() {
            self.ring.close(frame);
            return;
        }
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
        let buffer_barriers = if self.buffer_regions.is_empty() {
            &buffers_after[..0]
        } else {
            &buffers_after[..]
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
            for_runs(&self.buffer_dsts, |dst, range| {
                d.cmd_copy_buffer(cmd, self.raw, dst, &self.buffer_regions[range]);
            });
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
        self.buffer_dsts.clear();
        self.buffer_regions.clear();
        self.image_dsts.clear();
        self.image_regions.clear();
        self.images.clear();
        self.staged = 0;
        self.ring.close(frame);
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
        r.close(1);
        r.alloc(300, 16).unwrap();
        r.close(2);
        assert_eq!(r.alloc(600, 16), Err(UploadError::Full));
        r.retire(0);
        assert_eq!(r.alloc(600, 16), Err(UploadError::Full));
        r.retire(1);
        // 608..908 is still in use by frame 2; the block wraps to the start.
        assert_eq!(r.alloc(600, 16), Ok(0));
        r.close(3);
        r.retire(2);
        // Free now: 608..908 only (908..1024 was skipped by frame 3, which uses 0..600).
        assert_eq!(r.alloc(304, 16), Err(UploadError::Full));
        assert_eq!(r.alloc(288, 16), Ok(608));
    }

    #[test]
    fn a_block_never_straddles_the_end() {
        let mut r = Ring::new(1024);
        r.alloc(900, 16).unwrap();
        r.close(1);
        r.alloc(16, 16).unwrap(); // 912..928
        r.close(2);
        r.retire(1);
        // 200 bytes don't fit in 928..1024: skip to 0 (free since frame 1 finished).
        assert_eq!(r.alloc(200, 16), Ok(0));
        // ...but not past the frame-2 bytes.
        assert_eq!(r.alloc(800, 16), Err(UploadError::Full));
    }

    #[test]
    fn an_idle_ring_rewinds_so_a_whole_ring_block_fits() {
        let mut r = Ring::new(1024);
        r.alloc(100, 16).unwrap();
        r.close(1);
        r.retire(1);
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
