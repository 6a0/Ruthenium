//! A depth attachment: one image the size of the colour target it is used with, cleared
//! at the start of each pass and never kept after it.
//!
//! Depth is reversed: the near plane is 1 and the far distance 0, so a pass clears to 0
//! and nearer fragments have greater values. With a float format this spreads precision
//! evenly over distance.

use std::sync::Arc;

use ash::vk;

use crate::memory::Image;
use crate::{Error, Gpu};

/// The depth every pass clears to: the far distance.
pub const DEPTH_CLEAR: f32 = 0.0;

/// The depth aspect of a depth image's one level.
pub(crate) const DEPTH_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::DEPTH,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// See the module docs.
#[derive(Debug)]
pub struct DepthTarget {
    gpu: Arc<Gpu>,
    image: Option<Image>,
    format: vk::Format,
    extent: vk::Extent2D,
}

impl DepthTarget {
    /// A depth image of `extent` in the device's depth format ([`Gpu::depth_format`]).
    pub fn new(gpu: Arc<Gpu>, extent: vk::Extent2D) -> Result<DepthTarget, Error> {
        let format = gpu.depth_format();
        let image = Image::new_2d(
            &gpu,
            format,
            extent,
            1,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        )?;
        Ok(DepthTarget {
            gpu,
            image: Some(image),
            format,
            extent,
        })
    }

    pub fn format(&self) -> vk::Format {
        self.format
    }

    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// Bytes of the image, as the format stores a texel.
    pub fn bytes(&self) -> u64 {
        u64::from(self.extent.width) * u64::from(self.extent.height) * 4
    }

    pub(crate) fn image(&self) -> (vk::Image, vk::ImageView) {
        let i = self.image.as_ref().expect("live");
        (i.raw, i.view)
    }
}

impl Drop for DepthTarget {
    fn drop(&mut self) {
        // It is replaced when the window changes size, while frames that used it may
        // still be running: wait.
        // SAFETY: after the device is idle nothing uses the image.
        unsafe {
            let _ = self.gpu.device.device_wait_idle();
            if let Some(i) = self.image.take() {
                i.destroy(&self.gpu);
            }
        }
    }
}
