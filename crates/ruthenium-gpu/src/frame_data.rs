//! Per-frame data: what the CPU writes for one frame and its shaders read (GUI and overlay
//! quads, the world's records; later instance transforms, particles), and the draw
//! commands of the frame's indirect draws. One host-visible
//! buffer, in device-local memory where the GPU has some the CPU can write (the BAR window
//! on discrete GPUs), split into one region per frame in flight. A frame bump-allocates
//! from its own region, which the frame that last used it has finished with, so writes
//! never wait and nothing allocates.
//!
//! Shaders see the frame's region at descriptor set 1, binding 0, as a read-only storage
//! buffer (a dynamic offset selects the region; `glsl/include/frame.glsl`). A draw finds
//! its data by an element index passed in push constants.

use std::fmt;
use std::sync::Arc;

use ash::vk;

use crate::device::Gpu;
use crate::frame::MAX_FRAMES_IN_FLIGHT;
use crate::memory::{Buffer, MemoryUse};
use crate::{Error, VkContext};

/// Descriptor set index of the per-frame data in the shared pipeline layout.
pub const FRAME_DATA_SET: u32 = 1;
/// Bytes of one indexed draw command (`VkDrawIndexedIndirectCommand`): the index
/// count, the instance count, the first index, the vertex offset and the first instance,
/// a word each.
pub const DRAW_COMMAND_BYTES: u32 = 20;

/// Stages that read per-frame data.
const STAGES: vk::ShaderStageFlags = vk::ShaderStageFlags::from_raw(
    vk::ShaderStageFlags::VERTEX.as_raw()
        | vk::ShaderStageFlags::FRAGMENT.as_raw()
        | vk::ShaderStageFlags::COMPUTE.as_raw(),
);

/// The frame data's part of the pipelines' interface, for the pipeline cache's name.
pub(crate) fn hash_interface(h: &mut crate::pipeline::InterfaceHash) {
    h.u32(FRAME_DATA_SET)
        .u32(0)
        .u32(vk::DescriptorType::STORAGE_BUFFER_DYNAMIC.as_raw() as u32)
        .u32(STAGES.as_raw());
}

/// See the module docs.
pub struct FrameData {
    gpu: Arc<Gpu>,
    buffer: Option<Buffer>,
    mapped: *mut u8,
    /// Bytes per frame region (a multiple of the storage offset alignment).
    region: u64,
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    slot: usize,
    used: u64,
}

impl fmt::Debug for FrameData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameData")
            .field("region", &self.region)
            .field("used", &self.used)
            .finish_non_exhaustive()
    }
}

// SAFETY: the mapping is only written through &mut self; Vulkan handles may move threads.
unsafe impl Send for FrameData {}

impl FrameData {
    /// Creates the buffer with `bytes_per_frame` (rounded up) for each frame in flight.
    pub fn new(gpu: Arc<Gpu>, bytes_per_frame: u64) -> Result<FrameData, Error> {
        let align = gpu.caps().storage_offset_alignment.max(16);
        let region = bytes_per_frame.max(align).next_multiple_of(align);
        let mut fd = FrameData {
            buffer: None,
            mapped: std::ptr::null_mut(),
            region,
            layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            slot: 0,
            used: 0,
            gpu,
        };
        // Drop destroys whatever was created if a step fails.
        let buffer = Buffer::new(
            &fd.gpu,
            region * MAX_FRAMES_IN_FLIGHT as u64,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
            MemoryUse::Dynamic,
        )?;
        fd.mapped = buffer.memory.mapped;
        assert!(!fd.mapped.is_null(), "dynamic memory is host-visible");
        let raw = buffer.raw;
        fd.buffer = Some(buffer);
        let d = &fd.gpu.device;
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER_DYNAMIC)
            .descriptor_count(1)
            .stage_flags(STAGES)];
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        // SAFETY: plain object creation; destroyed in Drop.
        fd.layout = unsafe { d.create_descriptor_set_layout(&info, None) }
            .ctx("vkCreateDescriptorSetLayout(frame data)")?;
        let sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER_DYNAMIC,
            descriptor_count: 1,
        }];
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes);
        // SAFETY: as above.
        fd.pool = unsafe { d.create_descriptor_pool(&info, None) }
            .ctx("vkCreateDescriptorPool(frame data)")?;
        let layouts = [fd.layout];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(fd.pool)
            .set_layouts(&layouts);
        // SAFETY: the pool has room for exactly this set; it is freed with the pool.
        fd.set = unsafe { d.allocate_descriptor_sets(&info) }
            .ctx("vkAllocateDescriptorSets(frame data)")?[0];
        let buffers = [vk::DescriptorBufferInfo::default()
            .buffer(raw)
            .offset(0)
            .range(region)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(fd.set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER_DYNAMIC)
            .buffer_info(&buffers);
        // SAFETY: the set is not in use; the buffer is alive.
        unsafe { d.update_descriptor_sets(std::slice::from_ref(&write), &[]) };
        Ok(fd)
    }

    /// The layout of set 1, for the shared pipeline layout.
    pub fn layout(&self) -> vk::DescriptorSetLayout {
        self.layout
    }

    /// Bytes each frame may write.
    pub fn capacity(&self) -> u64 {
        self.region
    }

    /// Bytes written so far this frame.
    pub fn used(&self) -> u64 {
        self.used
    }

    /// Starts writing the region of frame slot `slot`. Call it once per frame, after
    /// `FrameLoop::begin` (which waited for the slot's previous frame).
    pub fn begin_frame(&mut self, slot: usize) {
        assert!(slot < MAX_FRAMES_IN_FLIGHT, "frame slot {slot}");
        self.slot = slot;
        self.used = 0;
    }

    /// Reserves `count` elements of `size` bytes for this frame and returns the index of
    /// the first (the shader's array index for elements of that size) and the bytes to
    /// write. The memory is write-combined: write it once, front to back, never read it.
    /// `None` if this frame's region is full.
    pub fn alloc(&mut self, count: u32, size: u32) -> Option<(u32, &mut [u8])> {
        assert!(size > 0 && size.is_multiple_of(4), "element size {size}");
        let start = self.used.next_multiple_of(u64::from(size));
        let bytes = u64::from(count) * u64::from(size);
        if start + bytes > self.region {
            return None;
        }
        self.used = start + bytes;
        let offset = self.slot as u64 * self.region + start;
        // SAFETY: the range lies inside this frame's region of the mapping, which no
        // unfinished frame reads (the slot's previous frame finished before begin_frame),
        // and `&mut self` keeps it unaliased for the slice's lifetime.
        let slice = unsafe {
            std::slice::from_raw_parts_mut(self.mapped.add(offset as usize), bytes as usize)
        };
        Some(((start / u64::from(size)) as u32, slice))
    }

    /// Reserves `count` indexed draw commands ([`DRAW_COMMAND_BYTES`] each) for this
    /// frame: where they start in [`Self::buffer`], in bytes, and the bytes to write
    /// (once, front to back). `None` if this frame's region is full.
    pub fn alloc_draw_commands(&mut self, count: u32) -> Option<(u64, &mut [u8])> {
        let base = self.slot as u64 * self.region;
        let (first, bytes) = self.alloc(count, DRAW_COMMAND_BYTES)?;
        Some((
            base + u64::from(first) * u64::from(DRAW_COMMAND_BYTES),
            bytes,
        ))
    }

    /// The buffer every frame's region is in: what indirect draws read their commands
    /// from ([`Self::alloc_draw_commands`]).
    pub fn buffer(&self) -> vk::Buffer {
        self.buffer.as_ref().map_or(vk::Buffer::null(), |b| b.raw)
    }

    /// Binds this frame's region at set 1 for graphics and compute.
    ///
    /// # Safety
    /// `cmd` is recording the frame passed to [`Self::begin_frame`]; `layout` was created
    /// with [`Self::layout`] as set 1.
    pub(crate) unsafe fn bind(&self, cmd: vk::CommandBuffer, layout: vk::PipelineLayout) {
        let sets = [self.set];
        let offsets = [(self.slot as u64 * self.region) as u32];
        for point in [
            vk::PipelineBindPoint::GRAPHICS,
            vk::PipelineBindPoint::COMPUTE,
        ] {
            // SAFETY: guaranteed by the caller.
            unsafe {
                self.gpu.device.cmd_bind_descriptor_sets(
                    cmd,
                    point,
                    layout,
                    FRAME_DATA_SET,
                    &sets,
                    &offsets,
                )
            };
        }
    }
}

impl Drop for FrameData {
    fn drop(&mut self) {
        let d = &self.gpu.device;
        // SAFETY: the owner drops it once the GPU is idle (after the frame loop).
        unsafe {
            if self.pool != vk::DescriptorPool::null() {
                d.destroy_descriptor_pool(self.pool, None);
            }
            if self.layout != vk::DescriptorSetLayout::null() {
                d.destroy_descriptor_set_layout(self.layout, None);
            }
            if let Some(b) = self.buffer.take() {
                b.destroy(&self.gpu);
            }
        }
    }
}
