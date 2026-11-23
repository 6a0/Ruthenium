//! An off-screen colour target the CPU reads back. Development checks draw into it with
//! the real pipelines and compare the bytes that come out, so they need neither a desktop
//! capture nor a presented frame.

use std::sync::Arc;

use ash::vk;

use crate::frame::FrameCtx;
use crate::memory::{Buffer, Image, MemoryUse};
use crate::swapchain::COLOR_RANGE;
use crate::{Error, Gpu};

/// See the module docs. One copy can be on its way at a time.
#[derive(Debug)]
pub struct Readback {
    gpu: Arc<Gpu>,
    image: Option<Image>,
    buffer: Option<Buffer>,
    extent: vk::Extent2D,
    format: vk::Format,
    /// The frame that recorded the copy the buffer holds, until it is taken.
    copied: Option<u64>,
    /// The last copy taken, as RGBA.
    pixels: Vec<u8>,
}

// SAFETY: the mapping is only read through &mut self; handles may move between threads.
unsafe impl Send for Readback {}

impl Readback {
    /// A target of `extent` in `format`, which must be 8-bit RGBA or BGRA (the formats a
    /// swapchain has, so the target can stand in for a window).
    pub fn new(gpu: Arc<Gpu>, format: vk::Format, extent: vk::Extent2D) -> Result<Readback, Error> {
        if !matches!(
            format,
            vk::Format::B8G8R8A8_UNORM
                | vk::Format::B8G8R8A8_SRGB
                | vk::Format::R8G8B8A8_UNORM
                | vk::Format::R8G8B8A8_SRGB
        ) {
            return Err(Error::Unsupported(format!(
                "a {format:?} target cannot be read back"
            )));
        }
        let bytes = extent.width as usize * extent.height as usize * 4;
        let image = Image::new_2d(
            &gpu,
            format,
            extent,
            1,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
        )?;
        let buffer = match Buffer::new(
            &gpu,
            bytes as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        ) {
            Ok(b) => b,
            Err(e) => {
                // SAFETY: unused.
                unsafe { image.destroy(&gpu) };
                return Err(e);
            }
        };
        Ok(Readback {
            gpu,
            image: Some(image),
            buffer: Some(buffer),
            extent,
            format,
            copied: None,
            pixels: vec![0; bytes],
        })
    }

    pub fn format(&self) -> vk::Format {
        self.format
    }

    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// The image and its view, to draw into. A pass that starts from `UNDEFINED` (its
    /// old contents are never needed) works whatever layout the last copy left.
    pub fn image(&self) -> (vk::Image, vk::ImageView) {
        let i = self.image.as_ref().expect("live");
        (i.raw, i.view)
    }

    /// Whether a copy is recorded and not taken yet.
    pub fn pending(&self) -> bool {
        self.copied.is_some()
    }

    /// Records a copy of the target into the read-back buffer.
    ///
    /// # Safety
    /// `ctx.cmd` is recording on the graphics queue, outside a render pass, after the
    /// frame's last draw into the target, which is in `COLOR_ATTACHMENT_OPTIMAL`; no copy
    /// is pending.
    pub unsafe fn record_copy(&mut self, ctx: &FrameCtx) {
        debug_assert!(self.copied.is_none(), "one copy at a time");
        let image = self.image.as_ref().expect("live").raw;
        let buffer = self.buffer.as_ref().expect("live").raw;
        let to_copy = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(image)
            .subresource_range(COLOR_RANGE)];
        let to_host = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::HOST)
            .dst_access_mask(vk::AccessFlags2::HOST_READ)];
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(self.extent.into());
        // SAFETY: guaranteed by the caller; the buffer holds the whole target and the CPU
        // has finished with the copy before this one.
        unsafe {
            self.gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            self.gpu.device.cmd_copy_image_to_buffer(
                ctx.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[region],
            );
            self.gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().memory_barriers(&to_host),
            );
        }
        self.copied = Some(ctx.number);
    }

    /// The pending copy once its frame is among the `completed` ones, as RGBA rows, top
    /// first: `(frame, pixels)`. `None` while it is not finished, or with no copy pending.
    pub fn take(&mut self, completed: u64) -> Option<(u64, &[u8])> {
        let frame = self.copied.filter(|&f| f <= completed)?;
        self.copied = None;
        let mapped = self.buffer.as_ref().expect("live").memory.mapped;
        // SAFETY: the copy finished with its frame; read-back memory is host-visible and
        // coherent, and nothing writes it until the next copy is recorded.
        let bytes = unsafe { std::slice::from_raw_parts(mapped, self.pixels.len()) };
        self.pixels.copy_from_slice(bytes);
        if matches!(
            self.format,
            vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
        ) {
            for p in self.pixels.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
        }
        Some((frame, &self.pixels))
    }
}

impl Drop for Readback {
    fn drop(&mut self) {
        // It can be replaced while frames that drew into it are still running: wait.
        // SAFETY: after the device is idle nothing uses the image or the buffer.
        unsafe {
            let _ = self.gpu.device.device_wait_idle();
            if let Some(i) = self.image.take() {
                i.destroy(&self.gpu);
            }
            if let Some(b) = self.buffer.take() {
                b.destroy(&self.gpu);
            }
        }
    }
}
