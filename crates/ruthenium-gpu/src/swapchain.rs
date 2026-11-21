use ash::vk;

use crate::device::Gpu;
use crate::{Error, VkContext};

/// The user-facing presentation choice; the frame limiter and governor sit on top of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentPolicy {
    /// FIFO: no tearing, paced by the display.
    VSync,
    /// IMMEDIATE (tearing), else MAILBOX, else FIFO: vanilla's no-vsync order.
    NoVSync,
}

impl PresentPolicy {
    fn preference(self) -> &'static [vk::PresentModeKHR] {
        match self {
            PresentPolicy::VSync => &[vk::PresentModeKHR::FIFO],
            PresentPolicy::NoVSync => &[
                vk::PresentModeKHR::IMMEDIATE,
                vk::PresentModeKHR::MAILBOX,
                vk::PresentModeKHR::FIFO,
            ],
        }
    }
}

/// Formats Ruthenium renders to, in order of preference. Like vanilla, rendering is in
/// UNORM with no sRGB conversion on write; colours are already display-encoded.
const FORMATS: [vk::Format; 2] = [vk::Format::B8G8R8A8_UNORM, vk::Format::R8G8B8A8_UNORM];

pub(crate) struct Swapchain {
    pub raw: vk::SwapchainKHR,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    pub present_mode: vk::PresentModeKHR,
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    /// One per image: signalled by the frame's submit, waited by its present. Indexed by
    /// image, because an image is only re-acquired once its previous present is done.
    pub render_done: Vec<vk::Semaphore>,
}

impl Swapchain {
    /// Creates a swapchain for the window's current pixel size, retiring `old` (which the
    /// caller destroys later). Returns `None` when the window has no area (minimised).
    pub fn create(
        gpu: &Gpu,
        pixel_size: (u32, u32),
        policy: PresentPolicy,
        old: vk::SwapchainKHR,
    ) -> Result<Option<Swapchain>, Error> {
        let sf = &gpu.instance.surface_fn;
        // SAFETY: plain queries on a live physical device and surface.
        let (caps, formats, modes) = unsafe {
            (
                sf.get_physical_device_surface_capabilities(gpu.physical, gpu.surface)
                    .ctx("vkGetPhysicalDeviceSurfaceCapabilitiesKHR")?,
                sf.get_physical_device_surface_formats(gpu.physical, gpu.surface)
                    .ctx("vkGetPhysicalDeviceSurfaceFormatsKHR")?,
                sf.get_physical_device_surface_present_modes(gpu.physical, gpu.surface)
                    .ctx("vkGetPhysicalDeviceSurfacePresentModesKHR")?,
            )
        };
        let extent = if caps.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: pixel_size
                    .0
                    .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: pixel_size
                    .1
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        } else {
            caps.current_extent
        };
        if extent.width == 0 || extent.height == 0 {
            return Ok(None);
        }
        let format = FORMATS
            .iter()
            .find_map(|&f| {
                formats
                    .iter()
                    .find(|s| s.format == f && s.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR)
            })
            .copied()
            .unwrap_or(formats[0]);
        let present_mode = policy
            .preference()
            .iter()
            .copied()
            .find(|m| modes.contains(m))
            .unwrap_or(vk::PresentModeKHR::FIFO);
        // Three images; the driver may give more, so never assume the count asked for.
        let mut image_count = caps.min_image_count.max(3);
        if caps.max_image_count != 0 {
            image_count = image_count.min(caps.max_image_count);
        }
        let composite = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::INHERIT,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
        ]
        .into_iter()
        .find(|&a| caps.supported_composite_alpha.contains(a))
        .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
        let usage = (vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
            & caps.supported_usage_flags;

        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(gpu.surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(usage | vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(composite)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old);
        // SAFETY: valid create info for this surface; `old` is a live swapchain of it or null.
        let raw = unsafe { gpu.swapchain_fn.create_swapchain(&info, None) }
            .ctx("vkCreateSwapchainKHR")?;
        let mut sc = Swapchain {
            raw,
            format: format.format,
            extent,
            present_mode,
            images: Vec::new(),
            views: Vec::new(),
            render_done: Vec::new(),
        };
        if let Err(e) = sc.create_image_objects(gpu) {
            // SAFETY: nothing has used the new swapchain yet.
            unsafe { sc.destroy(gpu) };
            return Err(e);
        }
        Ok(Some(sc))
    }

    fn create_image_objects(&mut self, gpu: &Gpu) -> Result<(), Error> {
        let d = &gpu.device;
        // SAFETY: plain query on a live swapchain; views and semaphores are destroyed in
        // destroy().
        unsafe {
            self.images = gpu
                .swapchain_fn
                .get_swapchain_images(self.raw)
                .ctx("vkGetSwapchainImagesKHR")?;
            for &image in &self.images {
                let info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.format)
                    .subresource_range(COLOR_RANGE);
                self.views
                    .push(d.create_image_view(&info, None).ctx("vkCreateImageView")?);
                self.render_done.push(
                    d.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                        .ctx("vkCreateSemaphore")?,
                );
            }
        }
        Ok(())
    }

    /// # Safety
    /// No submitted or pending work may still use the swapchain's images or semaphores.
    pub unsafe fn destroy(&mut self, gpu: &Gpu) {
        // SAFETY: guaranteed by the caller.
        unsafe {
            for v in self.views.drain(..) {
                gpu.device.destroy_image_view(v, None);
            }
            for s in self.render_done.drain(..) {
                gpu.device.destroy_semaphore(s, None);
            }
            gpu.swapchain_fn.destroy_swapchain(self.raw, None);
        }
        self.images.clear();
        self.raw = vk::SwapchainKHR::null();
    }
}

/// The colour aspect of mip 0, layer 0: every swapchain image.
pub const COLOR_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
