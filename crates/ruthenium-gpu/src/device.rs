use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicU32, AtomicU64};

use ash::vk;
use ash::vk::TaggedStructure;

use crate::instance::Instance;
use crate::{Error, VkContext};

/// Provisional in the registry, so ash only has it behind a feature; it is just a name here.
const PORTABILITY_SUBSET: &CStr = c"VK_KHR_portability_subset";

/// Texture-table slots Ruthenium asks for unless told otherwise
/// ([`GpuOptions::texture_slots`]); fewer if the device's descriptor limits are lower.
pub const DEFAULT_TEXTURE_SLOTS: u32 = 1_024;
/// The smallest table Ruthenium runs with: a GPU that cannot hold it is rejected.
pub const MIN_TEXTURE_SLOTS: u32 = 256;
/// The largest table that can be asked for.
pub const MAX_TEXTURE_SLOTS: u32 = 65_536;
/// Per-stage resources kept for everything that is not a texture of the table: its
/// samplers, the frame's data, later sets and the colour attachments, which count against
/// `maxPerStageResources` in the fragment stage.
const NON_TEXTURE_DESCRIPTORS: u32 = 16;

/// The kind of descriptor set the texture table is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureTablePath {
    /// Plain descriptor sets, within the device's ordinary sampled-image limits. A frame's
    /// copy of the set is only written before anything of that frame binds it, which is
    /// all the table does.
    #[default]
    Ordinary,
    /// Update-after-bind sets, for their (often far) higher limits; needs
    /// `descriptorBindingSampledImageUpdateAfterBind`.
    UpdateAfterBind,
}

impl TextureTablePath {
    /// A short name for logs and options.
    pub fn name(self) -> &'static str {
        match self {
            TextureTablePath::Ordinary => "ordinary",
            TextureTablePath::UpdateAfterBind => "update-after-bind",
        }
    }
}

/// The device's descriptor limits that bound the texture table, as it reports them (the
/// update-after-bind ones are only meaningful with that feature).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DescriptorLimits {
    pub per_stage_sampled_images: u32,
    pub set_sampled_images: u32,
    pub per_stage_resources: u32,
    pub update_after_bind_per_stage_sampled_images: u32,
    pub update_after_bind_set_sampled_images: u32,
    pub update_after_bind_per_stage_resources: u32,
    pub update_after_bind_descriptors_in_all_pools: u32,
}

impl DescriptorLimits {
    fn of(
        limits: &vk::PhysicalDeviceLimits,
        indexing: &vk::PhysicalDeviceDescriptorIndexingProperties<'_>,
    ) -> DescriptorLimits {
        DescriptorLimits {
            per_stage_sampled_images: limits.max_per_stage_descriptor_sampled_images,
            set_sampled_images: limits.max_descriptor_set_sampled_images,
            per_stage_resources: limits.max_per_stage_resources,
            update_after_bind_per_stage_sampled_images: indexing
                .max_per_stage_descriptor_update_after_bind_sampled_images,
            update_after_bind_set_sampled_images: indexing
                .max_descriptor_set_update_after_bind_sampled_images,
            update_after_bind_per_stage_resources: indexing
                .max_per_stage_update_after_bind_resources,
            update_after_bind_descriptors_in_all_pools: indexing
                .max_update_after_bind_descriptors_in_all_pools,
        }
    }
}

/// How many textures the table could have on each path, under every limit that applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureTableLimits {
    pub ordinary: u32,
    /// `None` without `descriptorBindingSampledImageUpdateAfterBind`.
    pub update_after_bind: Option<u32>,
}

impl TextureTableLimits {
    fn of(
        limits: &vk::PhysicalDeviceLimits,
        indexing: &vk::PhysicalDeviceDescriptorIndexingProperties<'_>,
        update_after_bind: bool,
    ) -> TextureTableLimits {
        let ordinary = limits
            .max_per_stage_descriptor_sampled_images
            .min(limits.max_descriptor_set_sampled_images)
            .min(
                limits
                    .max_per_stage_resources
                    .saturating_sub(NON_TEXTURE_DESCRIPTORS),
            );
        // Every frame in flight has its own copy of the set, all from one pool, and the
        // budget of update-after-bind pools counts every descriptor in them.
        let copies = crate::frame::MAX_FRAMES_IN_FLIGHT as u32;
        let update_after_bind = update_after_bind.then(|| {
            indexing
                .max_per_stage_descriptor_update_after_bind_sampled_images
                .min(indexing.max_descriptor_set_update_after_bind_sampled_images)
                .min(
                    indexing
                        .max_per_stage_update_after_bind_resources
                        .saturating_sub(NON_TEXTURE_DESCRIPTORS),
                )
                .min(
                    (indexing.max_update_after_bind_descriptors_in_all_pools / copies)
                        .saturating_sub(crate::textures::SAMPLER_COUNT),
                )
        });
        TextureTableLimits {
            ordinary,
            update_after_bind,
        }
    }

    /// The path and size of a table of `wanted` slots. Ordinary sets when they hold it;
    /// update-after-bind only when that gives a larger table than they can; a smaller
    /// ordinary table, down to [`MIN_TEXTURE_SLOTS`], when neither holds `wanted`.
    /// `only` allows one path. `Err` says why the device can't hold a table.
    fn choose(
        self,
        wanted: u32,
        only: Option<TextureTablePath>,
    ) -> Result<(TextureTablePath, u32), String> {
        let ordinary = self.ordinary.min(wanted);
        let uab = self.update_after_bind.map(|n| n.min(wanted));
        let (path, slots) = match (only, uab) {
            (Some(TextureTablePath::Ordinary), _) | (None, None) => {
                (TextureTablePath::Ordinary, ordinary)
            }
            (Some(TextureTablePath::UpdateAfterBind), None) => {
                return Err("no descriptorBindingSampledImageUpdateAfterBind".into());
            }
            (Some(TextureTablePath::UpdateAfterBind), Some(n)) => {
                (TextureTablePath::UpdateAfterBind, n)
            }
            (None, Some(n)) if n > ordinary => (TextureTablePath::UpdateAfterBind, n),
            (None, Some(_)) => (TextureTablePath::Ordinary, ordinary),
        };
        if slots < MIN_TEXTURE_SLOTS {
            return Err(format!(
                "only {slots} textures in one {} descriptor set (needs {MIN_TEXTURE_SLOTS})",
                path.name()
            ));
        }
        Ok((path, slots))
    }
}

/// How to pick among several GPUs.
#[derive(Clone, Debug, Default)]
pub struct GpuOptions {
    /// Case-insensitive substring of the device name (`"nvidia"`, `"intel"`, `"4060"`).
    /// A matching suitable GPU wins over the default order; `None` or `"auto"` keeps it.
    pub prefer: Option<String>,
    /// Don't use a dedicated transfer queue even if the GPU has one: every upload goes on
    /// the graphics queue (for A/B comparisons and driver problems).
    pub no_transfer_queue: bool,
    /// Use only this swapchain maintenance chain, or none of them, whatever the device
    /// has (`None`: KHR if it is whole, else EXT, else neither). For testing each path.
    pub swapchain_maintenance: Option<SwapchainMaintenance>,
    /// Development: treat the window's surface as offering only sRGB formats.
    pub pretend_srgb: PretendSrgb,
    /// Texture-table slots to ask for (`None`: [`DEFAULT_TEXTURE_SLOTS`]), between
    /// [`MIN_TEXTURE_SLOTS`] and [`MAX_TEXTURE_SLOTS`]. The device's limits may give fewer.
    pub texture_slots: Option<u32>,
    /// Make the texture table of this kind of descriptor set only (`None`: ordinary sets,
    /// update-after-bind when that gives a larger table). For testing each path.
    pub texture_table: Option<TextureTablePath>,
}

/// The swapchain maintenance extension a device was created with. With one, every present
/// carries a fence that says when it is over; without, that is inferred from the fences of
/// later acquires (see [`crate::FrameLoop`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwapchainMaintenance {
    #[default]
    None,
    /// `VK_KHR_swapchain_maintenance1` over `VK_KHR_surface_maintenance1`.
    Khr,
    /// `VK_EXT_swapchain_maintenance1` over `VK_EXT_surface_maintenance1`.
    Ext,
}

impl SwapchainMaintenance {
    /// A short name for logs and options.
    pub fn name(self) -> &'static str {
        match self {
            SwapchainMaintenance::None => "none",
            SwapchainMaintenance::Khr => "khr",
            SwapchainMaintenance::Ext => "ext",
        }
    }

    /// The chain to use out of the whole ones, or the only one allowed.
    fn choose(khr: bool, ext: bool, only: Option<SwapchainMaintenance>) -> SwapchainMaintenance {
        match only {
            Some(SwapchainMaintenance::None) => SwapchainMaintenance::None,
            Some(SwapchainMaintenance::Khr) if khr => SwapchainMaintenance::Khr,
            Some(SwapchainMaintenance::Ext) if ext => SwapchainMaintenance::Ext,
            Some(_) => SwapchainMaintenance::None,
            None if khr => SwapchainMaintenance::Khr,
            None if ext => SwapchainMaintenance::Ext,
            None => SwapchainMaintenance::None,
        }
    }
}

/// When to pretend that the surface offers only sRGB formats ([`GpuOptions::pretend_srgb`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PretendSrgb {
    #[default]
    Never,
    /// From the start: no GPU can present to the window.
    Always,
    /// From the second swapchain on: the surface changes under a running client.
    OnRecreation,
}

/// A GPU that was not chosen because it misses part of the baseline.
#[derive(Clone, Debug)]
pub struct Rejected {
    pub name: String,
    pub reasons: Vec<String>,
}

/// What the chosen GPU and driver can do beyond the baseline. Every optional path is gated
/// on one of these bits and has a fallback.
#[derive(Clone, Debug, Default)]
pub struct DeviceCaps {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub device_type: vk::PhysicalDeviceType,
    pub driver_id: vk::DriverId,
    /// Driver name and version string as the driver reports them.
    pub driver: String,
    pub api_version: u32,
    pub pipeline_cache_uuid: [u8; vk::UUID_SIZE],
    /// Nanoseconds per timestamp tick, and whether the graphics queue writes timestamps.
    pub timestamp_period_ns: f32,
    pub timestamps: bool,
    /// Valid bits of the graphics queue's timestamps (`timestampValidBits`).
    pub timestamp_bits: u32,
    /// All device-local memory is also host-visible (integrated GPUs, Apple, ARM).
    pub unified_memory: bool,
    /// Slots in the shared texture table: what was asked for, capped by the descriptor
    /// limits of the path it is on.
    pub texture_slots: u32,
    /// The kind of descriptor set the table is made of.
    pub texture_table: TextureTablePath,
    /// What each path could hold on this device.
    pub texture_table_limits: TextureTableLimits,
    /// The limits that follows from.
    pub descriptor_limits: DescriptorLimits,
    pub max_image_dimension_2d: u32,
    /// `maxMemoryAllocationCount` (often 4096 on Windows).
    pub max_memory_allocations: u32,
    /// `optimalBufferCopyOffsetAlignment`: staging offsets are aligned to it.
    pub copy_offset_alignment: u64,
    /// `minStorageBufferOffsetAlignment`: per-frame data regions are aligned to it.
    pub storage_offset_alignment: u64,
    /// A queue family with transfer but no graphics, compute or video, for background
    /// uploads (whether it is used: [`Gpu::transfer_family`]).
    pub transfer_family: Option<u32>,
    pub portability_subset: bool,
    /// What a portability-subset device reports about itself.
    pub portability: Option<PortabilitySubset>,
    pub present_id: bool,
    pub present_wait: bool,
    pub present_id2: bool,
    pub present_wait2: bool,
    pub present_timing: bool,
    /// The KHR and the EXT swapchain maintenance chains that are whole: the device
    /// extension, its feature, and the instance extensions it depends on.
    pub swapchain_maintenance_khr: bool,
    pub swapchain_maintenance_ext: bool,
    /// The one the device was created with.
    pub swapchain_maintenance: SwapchainMaintenance,
    pub memory_budget: bool,
    pub calibrated_timestamps: bool,
    pub low_latency2: bool,
    pub anti_lag: bool,
    /// Features beyond the baseline that the device was created with.
    pub optional: OptionalFeatures,
    /// `maxSamplerAnisotropy` (meaningful with [`OptionalFeatures::sampler_anisotropy`]).
    pub max_sampler_anisotropy: f32,
}

/// The logical device, its graphics queue and the window surface. Shared (`Arc`) by
/// everything that creates GPU objects; it is destroyed after all of them.
pub struct Gpu {
    pub(crate) device: ash::Device,
    pub(crate) swapchain_fn: ash::khr::swapchain::Device,
    /// Present when the device is Vulkan 1.2; on 1.3 the core entry points are used.
    sync2_fn: Option<ash::khr::synchronization2::Device>,
    dynamic_rendering_fn: Option<ash::khr::dynamic_rendering::Device>,
    pub(crate) physical: vk::PhysicalDevice,
    pub(crate) queue: vk::Queue,
    pub(crate) queue_family: u32,
    /// The dedicated transfer queue and its family, if one is in use.
    pub(crate) transfer: Option<(vk::Queue, u32)>,
    pub(crate) surface: vk::SurfaceKHR,
    pub(crate) memory_properties: vk::PhysicalDeviceMemoryProperties,
    /// Live `vkAllocateMemory` allocations, kept under `maxMemoryAllocationCount`.
    pub(crate) allocations: AtomicU32,
    /// Bytes of live allocations, by [`crate::memory::MemoryUse`].
    pub(crate) allocated: [AtomicU64; crate::memory::MemoryUse::COUNT],
    pub(crate) pretend_srgb: PretendSrgb,
    caps: DeviceCaps,
    pub(crate) instance: Instance,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

// SAFETY: Vulkan handles and ash's function tables may be used from any thread; Ruthenium
// externally synchronises queue access (one submitting thread), as Vulkan requires.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

/// Everything learnt about one physical device while checking it against the baseline.
struct Candidate {
    physical: vk::PhysicalDevice,
    queue_family: u32,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    caps: DeviceCaps,
    /// Extensions to enable (all checked to be present).
    extensions: Vec<&'static CStr>,
    core13: bool,
    /// The transfer family to create a queue from (`caps.transfer_family` unless disabled).
    use_transfer: Option<u32>,
}

/// Features nothing needs yet, enabled wherever the driver has them so that their first
/// users (indirect terrain draws, the anisotropic terrain sampler) only have to look here.
/// Each of those users keeps a path that works without.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OptionalFeatures {
    /// `multiDrawIndirect`: one call draws a buffer of draw commands.
    pub multi_draw_indirect: bool,
    /// `drawIndirectFirstInstance`: those commands may start at a nonzero instance.
    pub draw_indirect_first_instance: bool,
    /// `shaderDrawParameters`: `gl_DrawID`, `gl_BaseInstance` and `gl_BaseVertex`.
    pub shader_draw_parameters: bool,
    /// `samplerAnisotropy`, up to [`DeviceCaps::max_sampler_anisotropy`].
    pub sampler_anisotropy: bool,
}

impl DeviceCaps {
    /// The stride a vertex binding of `bytes` bytes per vertex has to have on this
    /// device: `bytes`, or on a portability-subset device the next multiple of its
    /// stride alignment.
    pub fn vertex_stride(&self, bytes: u32) -> u32 {
        let align = self
            .portability
            .map_or(1, |p| p.min_vertex_stride_alignment.max(1));
        bytes.next_multiple_of(align)
    }
}

impl OptionalFeatures {
    fn of(f: &Features) -> OptionalFeatures {
        OptionalFeatures {
            multi_draw_indirect: f.core.multi_draw_indirect == vk::TRUE,
            draw_indirect_first_instance: f.core.draw_indirect_first_instance == vk::TRUE,
            shader_draw_parameters: f.shader_draw_parameters,
            sampler_anisotropy: f.core.sampler_anisotropy == vk::TRUE,
        }
    }

    /// Each feature's Vulkan name and whether the device has it enabled.
    pub fn named(&self) -> [(&'static str, bool); 4] {
        [
            ("multiDrawIndirect", self.multi_draw_indirect),
            (
                "drawIndirectFirstInstance",
                self.draw_indirect_first_instance,
            ),
            ("shaderDrawParameters", self.shader_draw_parameters),
            ("samplerAnisotropy", self.sampler_anisotropy),
        ]
    }
}

/// What a device of the Vulkan portability subset (MoltenVK over Metal) has of the things
/// the subset makes optional, and the one limit it adds. Ruthenium's pipelines stay
/// inside the subset (triangle lists, no constant blend factors, samplers without LOD
/// bias or comparison, views in the image's own format and component order), so none of
/// these is required; a pipeline that needs one has to check it here first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortabilitySubset {
    pub constant_alpha_color_blend_factors: bool,
    pub events: bool,
    pub image_view_format_reinterpretation: bool,
    pub image_view_format_swizzle: bool,
    pub image_view_2d_on_3d_image: bool,
    pub multisample_array_image: bool,
    pub mutable_comparison_samplers: bool,
    pub point_polygons: bool,
    pub sampler_mip_lod_bias: bool,
    pub separate_stencil_mask_ref: bool,
    pub shader_sample_rate_interpolation_functions: bool,
    pub tessellation_isolines: bool,
    pub tessellation_point_mode: bool,
    pub triangle_fans: bool,
    pub vertex_attribute_access_beyond_stride: bool,
    /// `minVertexInputBindingStrideAlignment`: a vertex binding's stride must be a
    /// multiple of it ([`DeviceCaps::vertex_stride`]).
    pub min_vertex_stride_alignment: u32,
}

impl PortabilitySubset {
    /// Asks a device that has `VK_KHR_portability_subset`.
    fn query(raw: &ash::Instance, pd: vk::PhysicalDevice) -> PortabilitySubset {
        let mut f = vk::PhysicalDevicePortabilitySubsetFeaturesKHR::default();
        let mut p = vk::PhysicalDevicePortabilitySubsetPropertiesKHR::default();
        let mut features = vk::PhysicalDeviceFeatures2::default().push(&mut f);
        let mut props = vk::PhysicalDeviceProperties2::default().push(&mut p);
        // SAFETY: the device has the extension that defines both chained structures.
        unsafe {
            raw.get_physical_device_features2(pd, &mut features);
            raw.get_physical_device_properties2(pd, &mut props);
        }
        let t = |b: vk::Bool32| b == vk::TRUE;
        PortabilitySubset {
            constant_alpha_color_blend_factors: t(f.constant_alpha_color_blend_factors),
            events: t(f.events),
            image_view_format_reinterpretation: t(f.image_view_format_reinterpretation),
            image_view_format_swizzle: t(f.image_view_format_swizzle),
            image_view_2d_on_3d_image: t(f.image_view2_d_on3_d_image),
            multisample_array_image: t(f.multisample_array_image),
            mutable_comparison_samplers: t(f.mutable_comparison_samplers),
            point_polygons: t(f.point_polygons),
            sampler_mip_lod_bias: t(f.sampler_mip_lod_bias),
            separate_stencil_mask_ref: t(f.separate_stencil_mask_ref),
            shader_sample_rate_interpolation_functions: t(
                f.shader_sample_rate_interpolation_functions
            ),
            tessellation_isolines: t(f.tessellation_isolines),
            tessellation_point_mode: t(f.tessellation_point_mode),
            triangle_fans: t(f.triangle_fans),
            vertex_attribute_access_beyond_stride: t(f.vertex_attribute_access_beyond_stride),
            min_vertex_stride_alignment: p.min_vertex_input_binding_stride_alignment,
        }
    }

    /// Each feature's Vulkan name and whether the device has it.
    pub fn named(&self) -> [(&'static str, bool); 15] {
        [
            (
                "constantAlphaColorBlendFactors",
                self.constant_alpha_color_blend_factors,
            ),
            ("events", self.events),
            (
                "imageViewFormatReinterpretation",
                self.image_view_format_reinterpretation,
            ),
            ("imageViewFormatSwizzle", self.image_view_format_swizzle),
            ("imageView2DOn3DImage", self.image_view_2d_on_3d_image),
            ("multisampleArrayImage", self.multisample_array_image),
            (
                "mutableComparisonSamplers",
                self.mutable_comparison_samplers,
            ),
            ("pointPolygons", self.point_polygons),
            ("samplerMipLodBias", self.sampler_mip_lod_bias),
            ("separateStencilMaskRef", self.separate_stencil_mask_ref),
            (
                "shaderSampleRateInterpolationFunctions",
                self.shader_sample_rate_interpolation_functions,
            ),
            ("tessellationIsolines", self.tessellation_isolines),
            ("tessellationPointMode", self.tessellation_point_mode),
            ("triangleFans", self.triangle_fans),
            (
                "vertexAttributeAccessBeyondStride",
                self.vertex_attribute_access_beyond_stride,
            ),
        ]
    }
}

/// One attribute of a vertex layout: where it starts in the vertex and how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexAttribute {
    pub offset: u32,
    pub format: vk::Format,
}

/// Whether a device can draw from a vertex layout, and with which stride
/// ([`Gpu::check_vertex_layout`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexLayoutCheck {
    /// Bytes per vertex as packed.
    pub bytes: u32,
    /// The stride its binding gets on this device: `bytes`, or more where a
    /// portability-subset device wants strides aligned. Packing, pipeline keys and
    /// memory figures have to use this one.
    pub stride: u32,
    /// Attribute formats the device cannot read from a vertex buffer.
    pub unsupported: Vec<vk::Format>,
    /// An attribute reaches past the vertex's bytes, which this device does not allow.
    pub beyond_stride: bool,
}

impl VertexLayoutCheck {
    pub fn usable(&self) -> bool {
        self.unsupported.is_empty() && !self.beyond_stride
    }

    fn of(
        bytes: u32,
        attributes: &[VertexAttribute],
        portability: Option<&PortabilitySubset>,
        reads: impl Fn(vk::Format) -> bool,
    ) -> VertexLayoutCheck {
        let align = portability.map_or(1, |p| p.min_vertex_stride_alignment.max(1));
        let mut unsupported: Vec<vk::Format> = Vec::new();
        let mut beyond = false;
        for a in attributes {
            match vertex_format_bytes(a.format) {
                Some(size) if reads(a.format) => beyond |= a.offset + size > bytes,
                _ if !unsupported.contains(&a.format) => unsupported.push(a.format),
                _ => {}
            }
        }
        VertexLayoutCheck {
            bytes,
            stride: bytes.next_multiple_of(align),
            unsupported,
            beyond_stride: beyond
                && portability.is_some_and(|p| !p.vertex_attribute_access_beyond_stride),
        }
    }
}

/// Bytes of the formats vertex attributes are packed in; `None` for any other.
fn vertex_format_bytes(format: vk::Format) -> Option<u32> {
    use vk::Format as F;
    Some(match format {
        F::R8G8_UNORM | F::R8G8_SNORM | F::R8G8_UINT | F::R8G8_SINT => 2,
        F::R8G8B8A8_UNORM | F::R8G8B8A8_SNORM | F::R8G8B8A8_UINT | F::R8G8B8A8_SINT => 4,
        F::R16G16_UNORM | F::R16G16_SNORM | F::R16G16_UINT | F::R16G16_SINT => 4,
        F::R16G16B16A16_UNORM
        | F::R16G16B16A16_SNORM
        | F::R16G16B16A16_UINT
        | F::R16G16B16A16_SINT => 8,
        F::R32_UINT | F::R32_SINT | F::R32_SFLOAT => 4,
        F::R32G32_SFLOAT => 8,
        F::R32G32B32_SFLOAT => 12,
        F::R32G32B32A32_SFLOAT => 16,
        _ => return None,
    })
}

/// A baseline feature: its name for diagnostics and how to read it from the query.
type FeatureCheck = (&'static str, fn(&Features) -> bool);

#[derive(Default)]
struct Features {
    core: vk::PhysicalDeviceFeatures,
    shader_draw_parameters: bool,
    timeline_semaphore: bool,
    runtime_descriptor_array: bool,
    sampled_image_array_non_uniform_indexing: bool,
    descriptor_binding_partially_bound: bool,
    descriptor_binding_sampled_image_update_after_bind: bool,
    dynamic_rendering: bool,
    synchronization2: bool,
}

const REQUIRED_FEATURES: &[FeatureCheck] = &[
    ("dynamicRendering", |f| f.dynamic_rendering),
    ("synchronization2", |f| f.synchronization2),
    ("timelineSemaphore", |f| f.timeline_semaphore),
    ("runtimeDescriptorArray", |f| f.runtime_descriptor_array),
    ("shaderSampledImageArrayNonUniformIndexing", |f| {
        f.sampled_image_array_non_uniform_indexing
    }),
    ("descriptorBindingPartiallyBound", |f| {
        f.descriptor_binding_partially_bound
    }),
];

impl Gpu {
    /// Picks a GPU that meets the baseline and can present to `surface`, and creates the
    /// device. Takes ownership of `surface` (destroyed with the Gpu), whose window must
    /// outlive the returned Gpu.
    pub fn new(
        instance: Instance,
        surface: vk::SurfaceKHR,
        options: &GpuOptions,
    ) -> Result<Gpu, Error> {
        let result = Self::create(&instance, surface, options);
        match result {
            Ok(parts) => Ok(Gpu::assemble(
                instance,
                surface,
                parts,
                options.pretend_srgb,
            )),
            Err(e) => {
                // SAFETY: nothing else uses the surface; it was created from this instance.
                unsafe { instance.surface_fn.destroy_surface(surface, None) };
                Err(e)
            }
        }
    }

    fn create(
        instance: &Instance,
        surface: vk::SurfaceKHR,
        options: &GpuOptions,
    ) -> Result<(Candidate, ash::Device), Error> {
        // SAFETY: plain instance query.
        let physicals = unsafe { instance.raw.enumerate_physical_devices() }
            .ctx("vkEnumeratePhysicalDevices")?;
        let mut rejected = Vec::new();
        let mut candidates = Vec::new();
        for pd in physicals {
            match evaluate(instance, pd, surface, options) {
                Ok(c) => candidates.push(c),
                Err(r) => rejected.push(r),
            }
        }
        let prefer = options
            .prefer
            .as_deref()
            .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("auto"));
        let mut chosen =
            pick(&mut candidates, prefer).ok_or(Error::NoSuitableDevice(rejected.clone()))?;
        chosen.use_transfer = chosen
            .caps
            .transfer_family
            .filter(|_| !options.no_transfer_queue);
        let maintenance = SwapchainMaintenance::choose(
            chosen.caps.swapchain_maintenance_khr,
            chosen.caps.swapchain_maintenance_ext,
            options.swapchain_maintenance,
        );
        chosen.caps.swapchain_maintenance = maintenance;
        match maintenance {
            SwapchainMaintenance::Khr => chosen
                .extensions
                .push(ash::khr::swapchain_maintenance1::NAME),
            SwapchainMaintenance::Ext => chosen
                .extensions
                .push(ash::ext::swapchain_maintenance1::NAME),
            SwapchainMaintenance::None => {}
        }
        if let Some(p) = prefer
            && !chosen.caps.name.to_lowercase().contains(&p.to_lowercase())
        {
            eprintln!(
                "[gpu] no suitable GPU matches \"{p}\"; using {}",
                chosen.caps.name
            );
        }
        for r in &rejected {
            eprintln!("[gpu] skipped {}: {}", r.name, r.reasons.join(", "));
        }
        let device = create_device(instance, &chosen)?;
        Ok((chosen, device))
    }

    fn assemble(
        instance: Instance,
        surface: vk::SurfaceKHR,
        (c, device): (Candidate, ash::Device),
        pretend_srgb: PretendSrgb,
    ) -> Gpu {
        // SAFETY: the device was created with one queue in this family.
        let queue = unsafe { device.get_device_queue(c.queue_family, 0) };
        // SAFETY: as above, one queue in the transfer family if it was requested.
        let transfer = c
            .use_transfer
            .map(|f| (unsafe { device.get_device_queue(f, 0) }, f));
        let swapchain_fn = ash::khr::swapchain::Device::load(&instance.raw, &device);
        let (sync2_fn, dynamic_rendering_fn) = if c.core13 {
            (None, None)
        } else {
            (
                Some(ash::khr::synchronization2::Device::load(
                    &instance.raw,
                    &device,
                )),
                Some(ash::khr::dynamic_rendering::Device::load(
                    &instance.raw,
                    &device,
                )),
            )
        };
        Gpu {
            device,
            swapchain_fn,
            sync2_fn,
            dynamic_rendering_fn,
            physical: c.physical,
            queue,
            queue_family: c.queue_family,
            transfer,
            surface,
            memory_properties: c.memory_properties,
            allocations: AtomicU32::new(0),
            allocated: std::array::from_fn(|_| AtomicU64::new(0)),
            pretend_srgb,
            caps: c.caps,
            instance,
        }
    }

    pub fn caps(&self) -> &DeviceCaps {
        &self.caps
    }

    /// Whether this device can draw from vertices of `bytes` bytes with `attributes`:
    /// every format readable from a vertex buffer, no attribute past the vertex where
    /// that is not allowed, and the stride the binding has to have here.
    pub fn check_vertex_layout(
        &self,
        bytes: u32,
        attributes: &[VertexAttribute],
    ) -> VertexLayoutCheck {
        VertexLayoutCheck::of(
            bytes,
            attributes,
            self.caps.portability.as_ref(),
            |format| {
                // SAFETY: plain query on the device this Gpu was created from.
                let props = unsafe {
                    self.instance
                        .raw
                        .get_physical_device_format_properties(self.physical, format)
                };
                props
                    .buffer_features
                    .contains(vk::FormatFeatureFlags::VERTEX_BUFFER)
            },
        )
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn queue_family(&self) -> u32 {
        self.queue_family
    }

    /// The family of the dedicated transfer queue, if uploads use one.
    pub fn transfer_family(&self) -> Option<u32> {
        self.transfer.map(|(_, f)| f)
    }

    /// The queue families buffers are shared between (`VK_SHARING_MODE_CONCURRENT`): the
    /// graphics and transfer families when a transfer queue is in use, else none
    /// (exclusive). Concurrent buffers need no ownership transfers; images stay exclusive
    /// to the graphics queue.
    pub(crate) fn buffer_families(&self) -> Option<[u32; 2]> {
        self.transfer.map(|(_, f)| [self.queue_family, f])
    }

    /// Whether the validation layer is active.
    pub fn validating(&self) -> bool {
        self.instance.validating()
    }

    /// `vkCmdPipelineBarrier2` (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkCmdPipelineBarrier2`.
    pub unsafe fn cmd_pipeline_barrier2(
        &self,
        cmd: vk::CommandBuffer,
        info: &vk::DependencyInfo<'_>,
    ) {
        // SAFETY: forwarded to the caller.
        unsafe {
            match &self.sync2_fn {
                Some(f) => f.cmd_pipeline_barrier2(cmd, info),
                None => self.device.cmd_pipeline_barrier2(cmd, info),
            }
        }
    }

    /// `vkCmdWriteTimestamp2` (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkCmdWriteTimestamp2`.
    pub unsafe fn cmd_write_timestamp2(
        &self,
        cmd: vk::CommandBuffer,
        stage: vk::PipelineStageFlags2,
        pool: vk::QueryPool,
        query: u32,
    ) {
        // SAFETY: forwarded to the caller.
        unsafe {
            match &self.sync2_fn {
                Some(f) => f.cmd_write_timestamp2(cmd, stage, pool, query),
                None => self.device.cmd_write_timestamp2(cmd, stage, pool, query),
            }
        }
    }

    /// `vkQueueSubmit2` on the graphics queue (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkQueueSubmit2`; the caller is the only thread submitting to this queue.
    pub(crate) unsafe fn queue_submit2(
        &self,
        submits: &[vk::SubmitInfo2<'_>],
    ) -> Result<(), Error> {
        // SAFETY: forwarded to the caller.
        unsafe { self.submit2_to(self.queue, submits) }
    }

    /// `vkQueueSubmit2` on `queue` (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkQueueSubmit2`; `queue` is one of this device's queues and the caller is the
    /// only thread submitting to it.
    pub(crate) unsafe fn submit2_to(
        &self,
        queue: vk::Queue,
        submits: &[vk::SubmitInfo2<'_>],
    ) -> Result<(), Error> {
        // SAFETY: forwarded to the caller.
        unsafe {
            match &self.sync2_fn {
                Some(f) => f.queue_submit2(queue, submits, vk::Fence::null()),
                None => self.device.queue_submit2(queue, submits, vk::Fence::null()),
            }
        }
        .ctx("vkQueueSubmit2")
    }

    /// `vkCmdBeginRendering` (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkCmdBeginRendering`.
    pub unsafe fn cmd_begin_rendering(&self, cmd: vk::CommandBuffer, info: &vk::RenderingInfo<'_>) {
        // SAFETY: forwarded to the caller.
        unsafe {
            match &self.dynamic_rendering_fn {
                Some(f) => f.cmd_begin_rendering(cmd, info),
                None => self.device.cmd_begin_rendering(cmd, info),
            }
        }
    }

    /// `vkCmdEndRendering` (core 1.3 or KHR).
    ///
    /// # Safety
    /// As `vkCmdEndRendering`.
    pub unsafe fn cmd_end_rendering(&self, cmd: vk::CommandBuffer) {
        // SAFETY: forwarded to the caller.
        unsafe {
            match &self.dynamic_rendering_fn {
                Some(f) => f.cmd_end_rendering(cmd),
                None => self.device.cmd_end_rendering(cmd),
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: every object created from this device is owned by something holding an
        // Arc<Gpu>, so all of them are gone; waiting idle covers work still queued.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.surface_fn.destroy_surface(self.surface, None);
        }
    }
}

/// Default order: discrete, then integrated, then virtual, then software; a preferred name
/// match comes first. Ties keep enumeration order (the OS's GPU preference on Windows).
fn pick(candidates: &mut Vec<Candidate>, prefer: Option<&str>) -> Option<Candidate> {
    let rank = |t: vk::PhysicalDeviceType| match t {
        vk::PhysicalDeviceType::DISCRETE_GPU => 0,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
        vk::PhysicalDeviceType::OTHER => 3,
        _ => 4,
    };
    let prefer = prefer.map(str::to_lowercase);
    let best = (0..candidates.len()).min_by_key(|&i| {
        let c = &candidates[i].caps;
        let matches = prefer
            .as_ref()
            .is_some_and(|p| c.name.to_lowercase().contains(p.as_str()));
        (!matches, rank(c.device_type), i)
    })?;
    Some(candidates.swap_remove(best))
}

fn evaluate(
    instance: &Instance,
    pd: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    options: &GpuOptions,
) -> Result<Candidate, Rejected> {
    let raw = &instance.raw;
    let mut driver_props = vk::PhysicalDeviceDriverProperties::default();
    let mut indexing = vk::PhysicalDeviceDescriptorIndexingProperties::default();
    let mut props2 = vk::PhysicalDeviceProperties2::default()
        .push(&mut driver_props)
        .push(&mut indexing);
    // SAFETY: the instance is at least 1.2 (checked at creation); the chain is valid.
    unsafe { raw.get_physical_device_properties2(pd, &mut props2) };
    let props = props2.properties;
    let name = props
        .device_name_as_c_str()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut reasons = Vec::new();
    let reject = |name: String, reasons: Vec<String>| Rejected { name, reasons };

    if props.api_version < vk::API_VERSION_1_2 {
        reasons.push(format!(
            "driver supports Vulkan {} (needs 1.2)",
            crate::version_string(props.api_version)
        ));
        return Err(reject(name, reasons));
    }
    let core13 = props.api_version >= vk::API_VERSION_1_3;

    // SAFETY: plain query.
    let exts = unsafe { raw.enumerate_device_extension_properties(pd) }.unwrap_or_default();
    let has = |n: &CStr| exts.iter().any(|e| e.extension_name_as_c_str() == Ok(n));

    let mut extensions: Vec<&'static CStr> = vec![ash::khr::swapchain::NAME];
    if !has(ash::khr::swapchain::NAME) {
        reasons.push("no VK_KHR_swapchain".into());
    }
    let has_dynamic_rendering = core13 || has(ash::khr::dynamic_rendering::NAME);
    let has_sync2 = core13 || has(ash::khr::synchronization2::NAME);
    if !core13 {
        for n in [
            ash::khr::dynamic_rendering::NAME,
            ash::khr::synchronization2::NAME,
        ] {
            if has(n) {
                extensions.push(n);
            }
        }
    }
    let portability_subset = has(PORTABILITY_SUBSET);
    if portability_subset {
        extensions.push(PORTABILITY_SUBSET);
    }
    // No features of its own: it makes the per-heap usage and budget query valid.
    if has(ash::ext::memory_budget::NAME) {
        extensions.push(ash::ext::memory_budget::NAME);
    }

    let features = query_features(raw, pd, core13, has_dynamic_rendering, has_sync2);
    for (feature, check) in REQUIRED_FEATURES {
        if !check(&features) {
            reasons.push(format!("no {feature}"));
        }
    }
    let texture_table_limits = TextureTableLimits::of(
        &props.limits,
        &indexing,
        features.descriptor_binding_sampled_image_update_after_bind,
    );
    let wanted_slots = options
        .texture_slots
        .unwrap_or(DEFAULT_TEXTURE_SLOTS)
        .clamp(MIN_TEXTURE_SLOTS, MAX_TEXTURE_SLOTS);
    let (texture_table, texture_slots) =
        match texture_table_limits.choose(wanted_slots, options.texture_table) {
            Ok(chosen) => chosen,
            Err(why) => {
                reasons.push(why);
                (TextureTablePath::Ordinary, 0)
            }
        };

    // A queue family that does graphics and compute and can present to our surface.
    // SAFETY: plain queries.
    let families = unsafe { raw.get_physical_device_queue_family_properties(pd) };
    let gfx = vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE;
    let queue_family = families.iter().enumerate().find_map(|(i, f)| {
        let i = i as u32;
        let presents = unsafe {
            instance
                .surface_fn
                .get_physical_device_surface_support(pd, i, surface)
        }
        .unwrap_or(false);
        (f.queue_flags.contains(gfx) && presents).then_some(i)
    });
    let Some(queue_family) = queue_family else {
        reasons.push("cannot present to this window".into());
        return Err(reject(name, reasons));
    };
    let formats = crate::swapchain::offered_formats(
        instance,
        pd,
        surface,
        options.pretend_srgb == PretendSrgb::Always,
    )
    .unwrap_or_default();
    if let Err(why) = crate::swapchain::choose_format(&formats) {
        reasons.push(why);
    }
    if !reasons.is_empty() {
        return Err(reject(name, reasons));
    }

    // A swapchain maintenance chain is whole with its device extension, the instance
    // extensions that one depends on, and the feature (one structure for both kinds).
    let maintenance_khr =
        has(ash::khr::swapchain_maintenance1::NAME) && instance.surface_maintenance_khr;
    let maintenance_ext =
        has(ash::ext::swapchain_maintenance1::NAME) && instance.surface_maintenance_ext;
    let maintenance_feature = (maintenance_khr || maintenance_ext) && {
        let mut m = vk::PhysicalDeviceSwapchainMaintenance1FeaturesKHR::default();
        let mut f2 = vk::PhysicalDeviceFeatures2::default().push(&mut m);
        // SAFETY: the device has an extension that defines the chained structure.
        unsafe { raw.get_physical_device_features2(pd, &mut f2) };
        m.swapchain_maintenance1 == vk::TRUE
    };

    let transfer_family = transfer_family(&families);
    // SAFETY: plain query.
    let memory = unsafe { raw.get_physical_device_memory_properties(pd) };
    let unified_memory = memory.memory_types_as_slice().iter().all(|t| {
        !t.property_flags
            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            || t.property_flags
                .contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
    });
    let driver_name = driver_props
        .driver_name_as_c_str()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let driver_info = driver_props
        .driver_info_as_c_str()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let caps = DeviceCaps {
        name,
        vendor_id: props.vendor_id,
        device_id: props.device_id,
        device_type: props.device_type,
        driver_id: driver_props.driver_id,
        driver: format!("{driver_name} {driver_info}").trim().to_owned(),
        api_version: props.api_version,
        pipeline_cache_uuid: props.pipeline_cache_uuid,
        timestamp_period_ns: props.limits.timestamp_period,
        timestamps: families[queue_family as usize].timestamp_valid_bits > 0,
        timestamp_bits: families[queue_family as usize].timestamp_valid_bits,
        unified_memory,
        texture_slots,
        texture_table,
        texture_table_limits,
        descriptor_limits: DescriptorLimits::of(&props.limits, &indexing),
        max_image_dimension_2d: props.limits.max_image_dimension2_d,
        max_memory_allocations: props.limits.max_memory_allocation_count,
        copy_offset_alignment: props.limits.optimal_buffer_copy_offset_alignment,
        storage_offset_alignment: props.limits.min_storage_buffer_offset_alignment,
        transfer_family,
        portability_subset,
        portability: portability_subset.then(|| PortabilitySubset::query(raw, pd)),
        present_id: has(ash::khr::present_id::NAME),
        present_wait: has(ash::khr::present_wait::NAME),
        present_id2: has(ash::khr::present_id2::NAME),
        present_wait2: has(ash::khr::present_wait2::NAME),
        present_timing: has(ash::ext::present_timing::NAME),
        swapchain_maintenance_khr: maintenance_khr && maintenance_feature,
        swapchain_maintenance_ext: maintenance_ext && maintenance_feature,
        // Chosen once the device is picked.
        swapchain_maintenance: SwapchainMaintenance::None,
        memory_budget: has(ash::ext::memory_budget::NAME),
        calibrated_timestamps: has(ash::khr::calibrated_timestamps::NAME)
            || has(ash::ext::calibrated_timestamps::NAME),
        low_latency2: has(ash::nv::low_latency2::NAME),
        anti_lag: has(ash::amd::anti_lag::NAME),
        optional: OptionalFeatures::of(&features),
        max_sampler_anisotropy: props.limits.max_sampler_anisotropy,
    };
    Ok(Candidate {
        physical: pd,
        queue_family,
        memory_properties: memory,
        caps,
        extensions,
        core13,
        use_transfer: None,
    })
}

/// A family of copy-engine queues: transfer without graphics or compute. Families that also
/// do video or optical flow are skipped (NVIDIA lists those after its copy family; they
/// are separate engines with fewer queues).
fn transfer_family(families: &[vk::QueueFamilyProperties]) -> Option<u32> {
    let not = vk::QueueFlags::GRAPHICS
        | vk::QueueFlags::COMPUTE
        | vk::QueueFlags::VIDEO_DECODE_KHR
        | vk::QueueFlags::VIDEO_ENCODE_KHR
        | vk::QueueFlags::OPTICAL_FLOW_NV;
    families
        .iter()
        .position(|f| {
            f.queue_count > 0
                && f.queue_flags.contains(vk::QueueFlags::TRANSFER)
                && !f.queue_flags.intersects(not)
        })
        .map(|i| i as u32)
}

fn query_features(
    raw: &ash::Instance,
    pd: vk::PhysicalDevice,
    core13: bool,
    dyn_ext: bool,
    sync2_ext: bool,
) -> Features {
    let mut f11 = vk::PhysicalDeviceVulkan11Features::default();
    let mut f12 = vk::PhysicalDeviceVulkan12Features::default();
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default();
    let mut dynr = vk::PhysicalDeviceDynamicRenderingFeatures::default();
    let mut sync2 = vk::PhysicalDeviceSynchronization2Features::default();
    let core = {
        let mut f2 = vk::PhysicalDeviceFeatures2::default()
            .push(&mut f11)
            .push(&mut f12);
        // Only chain structs the device knows (core 1.3, or its extension is present).
        if core13 {
            f2 = f2.push(&mut f13);
        } else {
            if dyn_ext {
                f2 = f2.push(&mut dynr);
            }
            if sync2_ext {
                f2 = f2.push(&mut sync2);
            }
        }
        // SAFETY: the chain only holds structs valid for this device.
        unsafe { raw.get_physical_device_features2(pd, &mut f2) };
        f2.features
    };
    let t = |b: vk::Bool32| b == vk::TRUE;
    Features {
        core,
        shader_draw_parameters: t(f11.shader_draw_parameters),
        timeline_semaphore: t(f12.timeline_semaphore),
        runtime_descriptor_array: t(f12.runtime_descriptor_array),
        sampled_image_array_non_uniform_indexing: t(
            f12.shader_sampled_image_array_non_uniform_indexing
        ),
        descriptor_binding_partially_bound: t(f12.descriptor_binding_partially_bound),
        descriptor_binding_sampled_image_update_after_bind: t(
            f12.descriptor_binding_sampled_image_update_after_bind
        ),
        dynamic_rendering: if core13 {
            t(f13.dynamic_rendering)
        } else {
            t(dynr.dynamic_rendering)
        },
        synchronization2: if core13 {
            t(f13.synchronization2)
        } else {
            t(sync2.synchronization2)
        },
    }
}

fn create_device(instance: &Instance, c: &Candidate) -> Result<ash::Device, Error> {
    let optional = c.caps.optional;
    let core = vk::PhysicalDeviceFeatures::default()
        .multi_draw_indirect(optional.multi_draw_indirect)
        .draw_indirect_first_instance(optional.draw_indirect_first_instance)
        .sampler_anisotropy(optional.sampler_anisotropy);
    let mut f11 = vk::PhysicalDeviceVulkan11Features::default()
        .shader_draw_parameters(optional.shader_draw_parameters);
    let mut f12 = vk::PhysicalDeviceVulkan12Features::default()
        .timeline_semaphore(true)
        .runtime_descriptor_array(true)
        .shader_sampled_image_array_non_uniform_indexing(true)
        .descriptor_binding_partially_bound(true)
        // Only for a table that is made of update-after-bind sets.
        .descriptor_binding_sampled_image_update_after_bind(
            c.caps.texture_table == TextureTablePath::UpdateAfterBind,
        );
    let mut f13 = vk::PhysicalDeviceVulkan13Features::default()
        .dynamic_rendering(true)
        .synchronization2(true);
    let mut dynr = vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);
    let mut sync2 = vk::PhysicalDeviceSynchronization2Features::default().synchronization2(true);
    let mut f2 = vk::PhysicalDeviceFeatures2::default()
        .features(core)
        .push(&mut f11)
        .push(&mut f12);
    f2 = if c.core13 {
        f2.push(&mut f13)
    } else {
        f2.push(&mut dynr).push(&mut sync2)
    };
    let mut maintenance =
        vk::PhysicalDeviceSwapchainMaintenance1FeaturesKHR::default().swapchain_maintenance1(true);
    if c.caps.swapchain_maintenance != SwapchainMaintenance::None {
        f2 = f2.push(&mut maintenance);
    }

    let priorities = [1.0f32];
    let mut queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(c.queue_family)
        .queue_priorities(&priorities); 2];
    let queue_count = match c.use_transfer {
        Some(f) => {
            queues[1] = queues[1].queue_family_index(f);
            2
        }
        None => 1,
    };
    let queues = &queues[..queue_count];
    let ext_ptrs: Vec<*const c_char> = c.extensions.iter().map(|e| e.as_ptr()).collect();
    // SAFETY: f2's chain is made of live, valid feature structs (built just above).
    let info = unsafe {
        vk::DeviceCreateInfo::default()
            .queue_create_infos(queues)
            .enabled_extension_names(&ext_ptrs)
            .extend(&mut f2)
    };
    // SAFETY: every enabled feature and extension was checked in evaluate(); the optional
    // features are enabled only where the query reported them.
    unsafe { instance.raw.create_device(c.physical, &info, None) }.ctx("vkCreateDevice")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, device_type: vk::PhysicalDeviceType) -> Candidate {
        Candidate {
            physical: vk::PhysicalDevice::null(),
            queue_family: 0,
            memory_properties: vk::PhysicalDeviceMemoryProperties::default(),
            caps: DeviceCaps {
                name: name.into(),
                device_type,
                ..DeviceCaps::default()
            },
            extensions: Vec::new(),
            core13: true,
            use_transfer: None,
        }
    }

    fn lab() -> Vec<Candidate> {
        vec![
            candidate(
                "llvmpipe (LLVM 23.1.5, 256 bits)",
                vk::PhysicalDeviceType::CPU,
            ),
            candidate(
                "Intel(R) UHD Graphics (ADL-S GT0.5)",
                vk::PhysicalDeviceType::INTEGRATED_GPU,
            ),
            candidate(
                "NVIDIA GeForce RTX 4060 Laptop GPU",
                vk::PhysicalDeviceType::DISCRETE_GPU,
            ),
        ]
    }

    #[test]
    fn discrete_first_then_integrated_then_software() {
        let mut c = lab();
        assert!(pick(&mut c, None).unwrap().caps.name.starts_with("NVIDIA"));
        assert!(pick(&mut c, None).unwrap().caps.name.starts_with("Intel"));
        assert!(
            pick(&mut c, None)
                .unwrap()
                .caps
                .name
                .starts_with("llvmpipe")
        );
        assert!(pick(&mut c, None).is_none());
    }

    #[test]
    fn transfer_family_is_a_copy_engine() {
        let family = |queue_flags, queue_count| vk::QueueFamilyProperties {
            queue_flags,
            queue_count,
            ..Default::default()
        };
        type Q = vk::QueueFlags;
        // NVIDIA's order: graphics, copy, compute, then video decode and encode.
        let nvidia = [
            family(
                Q::GRAPHICS | Q::COMPUTE | Q::TRANSFER | Q::SPARSE_BINDING,
                16,
            ),
            family(Q::TRANSFER | Q::SPARSE_BINDING, 2),
            family(Q::COMPUTE | Q::TRANSFER | Q::SPARSE_BINDING, 8),
            family(Q::TRANSFER | Q::VIDEO_DECODE_KHR, 1),
        ];
        assert_eq!(transfer_family(&nvidia), Some(1));
        // Video-only transfer families don't count.
        let video = [
            family(Q::GRAPHICS | Q::COMPUTE | Q::TRANSFER, 1),
            family(Q::TRANSFER | Q::VIDEO_ENCODE_KHR, 1),
        ];
        assert_eq!(transfer_family(&video), None);
        assert_eq!(transfer_family(&nvidia[..1]), None);
    }

    #[test]
    fn the_khr_maintenance_chain_comes_first_and_an_override_allows_one() {
        use SwapchainMaintenance::{Ext, Khr, None as Neither};
        let choose = SwapchainMaintenance::choose;
        assert_eq!(choose(true, true, None), Khr);
        assert_eq!(choose(false, true, None), Ext);
        assert_eq!(choose(false, false, None), Neither);
        assert_eq!(choose(true, true, Some(Ext)), Ext);
        assert_eq!(choose(true, true, Some(Neither)), Neither);
        // An override never falls back to the other chain.
        assert_eq!(choose(false, true, Some(Khr)), Neither);
        assert_eq!(choose(true, false, Some(Ext)), Neither);
    }

    #[test]
    fn the_table_is_ordinary_unless_only_update_after_bind_holds_it() {
        use TextureTablePath::{Ordinary, UpdateAfterBind};
        let limits = |ordinary, update_after_bind| TextureTableLimits {
            ordinary,
            update_after_bind,
        };
        // Both hold it: ordinary sets.
        let roomy = limits(1 << 20, Some(1 << 20));
        assert_eq!(roomy.choose(1024, None), Ok((Ordinary, 1024)));
        assert_eq!(roomy.choose(16_384, None), Ok((Ordinary, 16_384)));
        // Ordinary sets are too small for what is wanted: update-after-bind, if there.
        let tight = limits(512, Some(500_000));
        assert_eq!(tight.choose(1024, None), Ok((UpdateAfterBind, 1024)));
        assert_eq!(tight.choose(512, None), Ok((Ordinary, 512)));
        assert_eq!(limits(512, None).choose(1024, None), Ok((Ordinary, 512)));
        // Update-after-bind that is no larger gains nothing.
        assert_eq!(
            limits(512, Some(512)).choose(1024, None),
            Ok((Ordinary, 512))
        );
        assert_eq!(
            limits(512, Some(800)).choose(1024, None),
            Ok((UpdateAfterBind, 800))
        );
        // Below the floor on every path allowed: no table.
        assert!(limits(128, None).choose(1024, None).is_err());
        assert!(limits(128, Some(200)).choose(1024, None).is_err());
        assert_eq!(
            limits(128, Some(500_000)).choose(1024, None),
            Ok((UpdateAfterBind, 1024))
        );
        // An override never falls back to the other path.
        assert_eq!(tight.choose(1024, Some(Ordinary)), Ok((Ordinary, 512)));
        assert_eq!(
            roomy.choose(1024, Some(UpdateAfterBind)),
            Ok((UpdateAfterBind, 1024))
        );
        assert!(
            limits(1 << 20, None)
                .choose(1024, Some(UpdateAfterBind))
                .is_err()
        );
        assert!(
            limits(128, Some(500_000))
                .choose(1024, Some(Ordinary))
                .is_err()
        );
    }

    #[test]
    fn table_limits_count_every_copy_of_the_set() {
        let core = vk::PhysicalDeviceLimits {
            max_per_stage_descriptor_sampled_images: 4000,
            max_descriptor_set_sampled_images: 3000,
            max_per_stage_resources: 2000,
            ..Default::default()
        };
        let indexing = vk::PhysicalDeviceDescriptorIndexingProperties {
            max_per_stage_descriptor_update_after_bind_sampled_images: 1 << 20,
            max_descriptor_set_update_after_bind_sampled_images: 1 << 20,
            max_per_stage_update_after_bind_resources: 1 << 20,
            max_update_after_bind_descriptors_in_all_pools: 10_000,
            ..Default::default()
        };
        let l = TextureTableLimits::of(&core, &indexing, true);
        assert_eq!(l.ordinary, 2000 - NON_TEXTURE_DESCRIPTORS);
        // Two copies of the set, samplers included, share the pools' budget.
        assert_eq!(
            l.update_after_bind,
            Some(
                10_000 / crate::frame::MAX_FRAMES_IN_FLIGHT as u32 - crate::textures::SAMPLER_COUNT
            )
        );
        assert_eq!(
            TextureTableLimits::of(&core, &indexing, false).update_after_bind,
            None
        );
    }

    #[test]
    fn vertex_strides_are_padded_to_the_portability_alignment() {
        // Eight bytes at the start and four that end at `end`.
        let layout = |end: u32| {
            [
                VertexAttribute {
                    offset: 0,
                    format: vk::Format::R16G16B16A16_UINT,
                },
                VertexAttribute {
                    offset: end - 4,
                    format: vk::Format::R16G16_UINT,
                },
            ]
        };
        let every = |_| true;
        // No portability subset: any stride, attributes may reach past it.
        let c = VertexLayoutCheck::of(16, &layout(16), None, every);
        assert_eq!((c.stride, c.usable()), (16, true));
        assert!(VertexLayoutCheck::of(18, &layout(20), None, every).usable());
        // Metal's alignment of four: 16 and 20 bytes stay, 18 is padded to 20.
        let metal = PortabilitySubset {
            min_vertex_stride_alignment: 4,
            ..PortabilitySubset::default()
        };
        for (bytes, stride) in [(16, 16), (20, 20), (18, 20)] {
            let c = VertexLayoutCheck::of(bytes, &layout(bytes), Some(&metal), every);
            assert_eq!((c.stride, c.usable()), (stride, true), "{bytes} bytes");
        }
        // A coarser alignment pads the 20-byte layout too.
        let coarse = PortabilitySubset {
            min_vertex_stride_alignment: 8,
            ..metal
        };
        assert_eq!(
            VertexLayoutCheck::of(20, &layout(20), Some(&coarse), every).stride,
            24
        );
        // An attribute past the vertex needs the subset's feature.
        let c = VertexLayoutCheck::of(16, &layout(20), Some(&metal), every);
        assert!(c.beyond_stride && !c.usable());
        let lenient = PortabilitySubset {
            vertex_attribute_access_beyond_stride: true,
            ..metal
        };
        assert!(VertexLayoutCheck::of(16, &layout(20), Some(&lenient), every).usable());
        // A format the device cannot read, or that is no vertex format at all.
        let c = VertexLayoutCheck::of(16, &layout(16), None, |f| f != vk::Format::R16G16_UINT);
        assert_eq!(c.unsupported, vec![vk::Format::R16G16_UINT]);
        let odd = [VertexAttribute {
            offset: 0,
            format: vk::Format::D32_SFLOAT,
        }];
        assert!(!VertexLayoutCheck::of(4, &odd, None, every).usable());
    }

    #[test]
    fn a_name_match_wins() {
        assert!(
            pick(&mut lab(), Some("intel"))
                .unwrap()
                .caps
                .name
                .starts_with("Intel")
        );
        assert!(
            pick(&mut lab(), Some("LLVMPIPE"))
                .unwrap()
                .caps
                .name
                .starts_with("llvmpipe")
        );
        // No match: the default order.
        assert!(
            pick(&mut lab(), Some("radeon"))
                .unwrap()
                .caps
                .name
                .starts_with("NVIDIA")
        );
    }
}
