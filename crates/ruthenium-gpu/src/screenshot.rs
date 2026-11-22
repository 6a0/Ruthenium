//! Development screenshots: a copy of the swapchain image a frame drew, read back as
//! RGBA. Ruthenium's own pixels only, so checks never need a desktop capture.

use std::sync::Arc;

use ash::vk;

use crate::frame::{FrameCtx, FrameLoop, Target};
use crate::memory::{Buffer, MemoryUse};
use crate::swapchain::COLOR_RANGE;
use crate::{Error, Gpu};

/// A copy recorded into a frame, waiting for that frame to finish.
#[derive(Debug)]
pub struct Screenshot {
    gpu: Arc<Gpu>,
    buffer: Option<Buffer>,
    extent: vk::Extent2D,
    format: vk::Format,
    frame: u64,
}

impl Screenshot {
    /// Records a copy of `target` and its transition to `PRESENT_SRC_KHR`, in place of
    /// the frame's own final transition. `None` if the swapchain cannot be read or its
    /// format is not 8-bit RGBA or BGRA.
    ///
    /// # Safety
    /// `ctx.cmd` is recording, outside a render pass, after the frame's last draw into
    /// `target`, which is in `COLOR_ATTACHMENT_OPTIMAL`.
    pub unsafe fn record(
        gpu: &Arc<Gpu>,
        ctx: &FrameCtx,
        target: &Target,
    ) -> Result<Option<Screenshot>, Error> {
        let bgra = matches!(
            target.format,
            vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
        );
        let rgba = matches!(
            target.format,
            vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB
        );
        if !target.readable || !(bgra || rgba) {
            return Ok(None);
        }
        let size = u64::from(target.extent.width) * u64::from(target.extent.height) * 4;
        let buffer = Buffer::new(
            gpu,
            size,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUse::Readback,
        )?;
        let to_copy = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::COPY)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(target.image)
            .subresource_range(COLOR_RANGE)];
        let to_present = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COPY)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::NONE)
            .dst_access_mask(vk::AccessFlags2::NONE)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(target.image)
            .subresource_range(COLOR_RANGE)];
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: target.extent.width,
                height: target.extent.height,
                depth: 1,
            });
        // SAFETY: guaranteed by the caller; the buffer is fresh and large enough.
        unsafe {
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_copy),
            );
            gpu.device.cmd_copy_image_to_buffer(
                ctx.cmd,
                target.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.raw,
                &[region],
            );
            gpu.cmd_pipeline_barrier2(
                ctx.cmd,
                &vk::DependencyInfo::default().image_memory_barriers(&to_present),
            );
        }
        Ok(Some(Screenshot {
            gpu: gpu.clone(),
            buffer: Some(buffer),
            extent: target.extent,
            format: target.format,
            frame: ctx.number,
        }))
    }

    /// Whether the frame that copied it has finished.
    pub fn done(&self, frames: &FrameLoop) -> Result<bool, Error> {
        Ok(frames.completed()? >= self.frame)
    }

    /// The pixels as `(width, height, RGBA rows top first)`. Call once [`Self::done`].
    pub fn into_rgba(mut self) -> (u32, u32, Vec<u8>) {
        let buffer = self.buffer.take().expect("live");
        let n = self.extent.width as usize * self.extent.height as usize * 4;
        // SAFETY: the copy finished with its frame; Readback memory is host-visible and
        // coherent, and nothing else writes it.
        let mut px = unsafe { std::slice::from_raw_parts(buffer.memory.mapped, n) }.to_vec();
        if matches!(
            self.format,
            vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
        ) {
            for p in px.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
        }
        // SAFETY: the GPU is done with it.
        unsafe { buffer.destroy(&self.gpu) };
        (self.extent.width, self.extent.height, px)
    }
}

impl Drop for Screenshot {
    fn drop(&mut self) {
        if let Some(b) = self.buffer.take() {
            // Not read: wait for its frame before freeing it.
            // SAFETY: after the device is idle nothing uses the buffer.
            unsafe {
                let _ = self.gpu.device.device_wait_idle();
                b.destroy(&self.gpu);
            }
        }
    }
}
