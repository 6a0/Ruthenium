//! Device memory: a memory type chosen for each use, and one dedicated allocation per
//! buffer or image.
//!
//! Ruthenium creates few, large GPU objects (the upload ring, atlases, later the terrain
//! arenas) and sub-allocates inside them itself, so a general-purpose allocator buys little.
//! This module is the seam where one would go (gpu-allocator 0.28 does not build against
//! the pinned ash, which dropped `push_next`).

use std::sync::atomic::Ordering;

use ash::vk;
use ash::vk::TaggedStructure;

use crate::device::Gpu;
use crate::{Error, VkContext};

/// What a buffer or image is for, which decides its memory type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryUse {
    /// Only the GPU touches it; filled by copies (textures, meshes).
    GpuOnly,
    /// Written once, sequentially, by the CPU and read by a GPU copy (staging). Kept out of
    /// device-local host-visible memory, which on discrete GPUs without resizable BAR is a
    /// 256 MiB window better left to per-frame data.
    Upload,
    /// Written by GPU copies and read by the CPU (readback, checks).
    Readback,
    /// Written by the CPU every frame, sequentially, and read by shaders: device-local
    /// host-visible memory (the BAR window on discrete GPUs) if there is any.
    Dynamic,
}

impl MemoryUse {
    /// Flags a type must have, flags it should have, and flags it should not have.
    fn flags(
        self,
    ) -> (
        vk::MemoryPropertyFlags,
        vk::MemoryPropertyFlags,
        vk::MemoryPropertyFlags,
    ) {
        type F = vk::MemoryPropertyFlags;
        match self {
            MemoryUse::GpuOnly => (F::empty(), F::DEVICE_LOCAL, F::HOST_VISIBLE),
            MemoryUse::Upload => (
                F::HOST_VISIBLE | F::HOST_COHERENT,
                F::empty(),
                F::DEVICE_LOCAL | F::HOST_CACHED,
            ),
            MemoryUse::Readback => (
                F::HOST_VISIBLE | F::HOST_COHERENT,
                F::HOST_CACHED,
                F::DEVICE_LOCAL,
            ),
            MemoryUse::Dynamic => (
                F::HOST_VISIBLE | F::HOST_COHERENT,
                F::DEVICE_LOCAL,
                F::HOST_CACHED,
            ),
        }
    }
}

/// Memory types Ruthenium never allocates from: they need features it does not enable.
const NEVER: vk::MemoryPropertyFlags = vk::MemoryPropertyFlags::from_raw(
    vk::MemoryPropertyFlags::PROTECTED.as_raw()
        | vk::MemoryPropertyFlags::LAZILY_ALLOCATED.as_raw()
        | vk::MemoryPropertyFlags::DEVICE_COHERENT_AMD.as_raw()
        | vk::MemoryPropertyFlags::DEVICE_UNCACHED_AMD.as_raw(),
);

/// The memory types in `type_bits` that suit `usage`, best first: types with every
/// preferred flag before others, then fewer unwanted flags, then the driver's order.
/// Returns the indices and how many there are.
fn ranked_types(
    props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    usage: MemoryUse,
) -> ([u32; vk::MAX_MEMORY_TYPES], usize) {
    let (required, preferred, unwanted) = usage.flags();
    let mut out = [0u32; vk::MAX_MEMORY_TYPES];
    let mut n = 0;
    for (i, t) in props.memory_types_as_slice().iter().enumerate() {
        let f = t.property_flags;
        if type_bits & (1 << i) != 0 && f.contains(required) && !f.intersects(NEVER) {
            out[n] = i as u32;
            n += 1;
        }
    }
    let types = props.memory_types_as_slice();
    out[..n].sort_by_key(|&i| {
        let f = types[i as usize].property_flags;
        (
            !f.contains(preferred),
            (f & unwanted).as_raw().count_ones(),
            i,
        )
    });
    (out, n)
}

/// One `VkDeviceMemory`, mapped for its whole size if it is host-visible.
#[derive(Debug)]
pub(crate) struct Allocation {
    pub memory: vk::DeviceMemory,
    /// Start of the mapping, or null.
    pub mapped: *mut u8,
}

/// The object an allocation is dedicated to (`VkMemoryDedicatedAllocateInfo`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Dedicated {
    Buffer(vk::Buffer),
    Image(vk::Image),
}

impl Gpu {
    /// Allocates memory for one object, trying the suitable memory types best first (a
    /// full heap falls through to the next type).
    pub(crate) fn allocate(
        &self,
        req: &vk::MemoryRequirements,
        usage: MemoryUse,
        dedicated: Dedicated,
    ) -> Result<Allocation, Error> {
        let limit = self.caps().max_memory_allocations;
        if self.allocations.fetch_add(1, Ordering::Relaxed) >= limit {
            self.allocations.fetch_sub(1, Ordering::Relaxed);
            return Err(Error::Vk {
                call: "vkAllocateMemory (maxMemoryAllocationCount reached)",
                result: vk::Result::ERROR_TOO_MANY_OBJECTS,
            });
        }
        let result = self.allocate_counted(req, usage, dedicated);
        if result.is_err() {
            self.allocations.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }

    fn allocate_counted(
        &self,
        req: &vk::MemoryRequirements,
        usage: MemoryUse,
        dedicated: Dedicated,
    ) -> Result<Allocation, Error> {
        let (types, n) = ranked_types(&self.memory_properties, req.memory_type_bits, usage);
        let mut last = vk::Result::ERROR_OUT_OF_DEVICE_MEMORY;
        for &type_index in &types[..n] {
            let mut dedicated_info = match dedicated {
                Dedicated::Buffer(b) => vk::MemoryDedicatedAllocateInfo::default().buffer(b),
                Dedicated::Image(i) => vk::MemoryDedicatedAllocateInfo::default().image(i),
            };
            let info = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(type_index)
                .push(&mut dedicated_info);
            // SAFETY: valid allocate info; the dedicated object is live and unbound.
            let memory = match unsafe { self.device.allocate_memory(&info, None) } {
                Ok(m) => m,
                Err(
                    e @ (vk::Result::ERROR_OUT_OF_DEVICE_MEMORY
                    | vk::Result::ERROR_OUT_OF_HOST_MEMORY),
                ) => {
                    last = e;
                    continue;
                }
                Err(e) => {
                    return Err(Error::Vk {
                        call: "vkAllocateMemory",
                        result: e,
                    });
                }
            };
            let flags = self.memory_properties.memory_types[type_index as usize].property_flags;
            let mapped = if flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
                // SAFETY: host-visible memory, mapped once for its whole size.
                match unsafe {
                    self.device
                        .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                } {
                    Ok(p) => p.cast::<u8>(),
                    Err(e) => {
                        // SAFETY: just allocated, unused.
                        unsafe { self.device.free_memory(memory, None) };
                        return Err(Error::Vk {
                            call: "vkMapMemory",
                            result: e,
                        });
                    }
                }
            } else {
                std::ptr::null_mut()
            };
            return Ok(Allocation { memory, mapped });
        }
        Err(Error::Vk {
            call: "vkAllocateMemory",
            result: last,
        })
    }

    /// # Safety
    /// The GPU no longer uses the memory, and the object bound to it is destroyed.
    pub(crate) unsafe fn free(&self, a: Allocation) {
        // SAFETY: guaranteed by the caller; freeing also unmaps.
        unsafe { self.device.free_memory(a.memory, None) };
        self.allocations.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A buffer with its own memory.
#[derive(Debug)]
pub(crate) struct Buffer {
    pub raw: vk::Buffer,
    pub memory: Allocation,
}

impl Buffer {
    pub(crate) fn new(
        gpu: &Gpu,
        size: u64,
        usage: vk::BufferUsageFlags,
        memory: MemoryUse,
    ) -> Result<Buffer, Error> {
        let families = gpu.buffer_families();
        let info = vk::BufferCreateInfo::default().size(size).usage(usage);
        // Shared with the transfer queue, if there is one, without ownership transfers.
        let info = match &families {
            Some(f) => info
                .sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(f),
            None => info.sharing_mode(vk::SharingMode::EXCLUSIVE),
        };
        let d = &gpu.device;
        // SAFETY: plain object creation; destroyed below on error, or by destroy().
        let raw = unsafe { d.create_buffer(&info, None) }.ctx("vkCreateBuffer")?;
        // SAFETY: live buffer.
        let req = unsafe { d.get_buffer_memory_requirements(raw) };
        let bound = gpu
            .allocate(&req, memory, Dedicated::Buffer(raw))
            .and_then(|m| {
                // SAFETY: fresh memory of a type the buffer accepts, of its required size.
                match unsafe { d.bind_buffer_memory(raw, m.memory, 0) } {
                    Ok(()) => Ok(m),
                    Err(e) => {
                        // SAFETY: unused.
                        unsafe { gpu.free(m) };
                        Err(Error::Vk {
                            call: "vkBindBufferMemory",
                            result: e,
                        })
                    }
                }
            });
        match bound {
            Ok(memory) => Ok(Buffer { raw, memory }),
            Err(e) => {
                // SAFETY: unused.
                unsafe { d.destroy_buffer(raw, None) };
                Err(e)
            }
        }
    }

    /// # Safety
    /// The GPU no longer uses the buffer.
    pub(crate) unsafe fn destroy(self, gpu: &Gpu) {
        // SAFETY: guaranteed by the caller.
        unsafe {
            gpu.device.destroy_buffer(self.raw, None);
            gpu.free(self.memory);
        }
    }
}

/// A 2D image with its own memory and a view of every mip level.
#[derive(Debug)]
pub(crate) struct Image {
    pub raw: vk::Image,
    pub view: vk::ImageView,
    pub memory: Allocation,
}

impl Image {
    /// A 2D image that copies write and shaders sample, in GPU-only memory.
    pub(crate) fn new_sampled(
        gpu: &Gpu,
        format: vk::Format,
        extent: vk::Extent2D,
        mip_levels: u32,
    ) -> Result<Image, Error> {
        // TRANSFER_SRC for reading textures back (checks, atlas dumps).
        let usage = vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_DST
            | vk::ImageUsageFlags::TRANSFER_SRC;
        Image::new_2d(gpu, format, extent, mip_levels, usage)
    }

    /// A 2D image with `usage`, in GPU-only memory, with a view of every mip level.
    pub(crate) fn new_2d(
        gpu: &Gpu,
        format: vk::Format,
        extent: vk::Extent2D,
        mip_levels: u32,
        usage: vk::ImageUsageFlags,
    ) -> Result<Image, Error> {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(extent.into())
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let d = &gpu.device;
        // SAFETY: plain object creation; everything is destroyed below on error.
        let raw = unsafe { d.create_image(&info, None) }.ctx("vkCreateImage")?;
        // SAFETY: live image.
        let req = unsafe { d.get_image_memory_requirements(raw) };
        let memory = match gpu.allocate(&req, MemoryUse::GpuOnly, Dedicated::Image(raw)) {
            Ok(m) => m,
            Err(e) => {
                // SAFETY: unused.
                unsafe { d.destroy_image(raw, None) };
                return Err(e);
            }
        };
        let view_info = vk::ImageViewCreateInfo::default()
            .image(raw)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: fresh memory of a type the image accepts; the view matches the image.
        let view = unsafe {
            d.bind_image_memory(raw, memory.memory, 0)
                .ctx("vkBindImageMemory")
                .and_then(|()| {
                    d.create_image_view(&view_info, None)
                        .ctx("vkCreateImageView")
                })
        };
        match view {
            Ok(view) => Ok(Image { raw, view, memory }),
            Err(e) => {
                // SAFETY: unused.
                unsafe {
                    d.destroy_image(raw, None);
                    gpu.free(memory);
                }
                Err(e)
            }
        }
    }

    /// # Safety
    /// The GPU no longer uses the image, and no descriptor that may be used refers to it.
    pub(crate) unsafe fn destroy(self, gpu: &Gpu) {
        // SAFETY: guaranteed by the caller.
        unsafe {
            gpu.device.destroy_image_view(self.view, None);
            gpu.device.destroy_image(self.raw, None);
            gpu.free(self.memory);
        }
    }
}

/// Bytes per texel of the uncompressed colour formats Ruthenium uploads.
pub fn texel_bytes(format: vk::Format) -> Option<u32> {
    use vk::Format as F;
    Some(match format {
        F::R8_UNORM => 1,
        F::R8G8_UNORM => 2,
        F::R8G8B8A8_UNORM | F::R8G8B8A8_SRGB | F::B8G8R8A8_UNORM | F::B8G8R8A8_SRGB => 4,
        F::R16G16B16A16_SFLOAT => 8,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type F = vk::MemoryPropertyFlags;

    fn props(types: &[F]) -> vk::PhysicalDeviceMemoryProperties {
        let mut p = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: types.len() as u32,
            memory_heap_count: 1,
            ..Default::default()
        };
        for (i, &f) in types.iter().enumerate() {
            p.memory_types[i] = vk::MemoryType {
                property_flags: f,
                heap_index: 0,
            };
        }
        p
    }

    fn ranked(p: &vk::PhysicalDeviceMemoryProperties, bits: u32, u: MemoryUse) -> Vec<u32> {
        let (t, n) = ranked_types(p, bits, u);
        t[..n].to_vec()
    }

    /// A discrete GPU without resizable BAR, in the order NVIDIA lists its types.
    fn discrete() -> vk::PhysicalDeviceMemoryProperties {
        props(&[
            F::empty(),
            F::DEVICE_LOCAL,
            F::HOST_VISIBLE | F::HOST_COHERENT,
            F::HOST_VISIBLE | F::HOST_COHERENT | F::HOST_CACHED,
            F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT,
        ])
    }

    #[test]
    fn discrete_gpu_keeps_staging_out_of_the_bar() {
        let p = discrete();
        assert_eq!(ranked(&p, !0, MemoryUse::GpuOnly)[0], 1);
        assert_eq!(ranked(&p, !0, MemoryUse::Upload), vec![2, 3, 4]);
        assert_eq!(ranked(&p, !0, MemoryUse::Readback), vec![3, 2, 4]);
        assert_eq!(ranked(&p, !0, MemoryUse::Dynamic), vec![4, 2, 3]);
    }

    #[test]
    fn unified_memory_uses_what_there_is() {
        let p = props(&[
            F::DEVICE_LOCAL,
            F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT,
            F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT | F::HOST_CACHED,
        ]);
        assert_eq!(ranked(&p, !0, MemoryUse::GpuOnly), vec![0, 1, 2]);
        assert_eq!(ranked(&p, !0, MemoryUse::Upload), vec![1, 2]);
    }

    #[test]
    fn only_types_the_object_accepts() {
        let p = discrete();
        assert_eq!(ranked(&p, 0b10000, MemoryUse::Upload), vec![4]);
        assert!(ranked(&p, 0b00010, MemoryUse::Upload).is_empty());
        // Device-local is preferred, not required.
        assert_eq!(ranked(&p, 0b01100, MemoryUse::GpuOnly), vec![2, 3]);
    }

    #[test]
    fn special_types_are_never_used() {
        let p = props(&[
            F::DEVICE_LOCAL | F::PROTECTED,
            F::DEVICE_LOCAL | F::DEVICE_COHERENT_AMD,
            F::DEVICE_LOCAL,
        ]);
        assert_eq!(ranked(&p, !0, MemoryUse::GpuOnly), vec![2]);
    }
}
