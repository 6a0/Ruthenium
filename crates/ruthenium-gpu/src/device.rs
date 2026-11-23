use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicU32, AtomicU64};

use ash::vk;
use ash::vk::TaggedStructure;

use crate::instance::Instance;
use crate::{Error, VkContext};

/// Provisional in the registry, so ash only has it behind a feature; it is just a name here.
const PORTABILITY_SUBSET: &CStr = c"VK_KHR_portability_subset";

/// Texture-table slots Ruthenium asks for; fewer if the device's descriptor limits are lower.
const WANTED_TEXTURE_SLOTS: u32 = 16_384;
/// Below this many bindless textures a GPU is rejected. Every descriptor-indexing driver
/// we know of reports far more (hundreds of thousands).
const MIN_TEXTURE_SLOTS: u32 = 1_024;
/// Descriptors of the texture table's set that are not sampled images (its samplers), kept
/// out of the per-stage resource limit.
const NON_TEXTURE_DESCRIPTORS: u32 = 16;

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
    /// Slots in the shared texture table: what Ruthenium wants, capped by the
    /// update-after-bind descriptor limits.
    pub texture_slots: u32,
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

/// A baseline feature: its name for diagnostics and how to read it from the query.
type FeatureCheck = (&'static str, fn(&Features) -> bool);

#[derive(Default)]
struct Features {
    core: vk::PhysicalDeviceFeatures,
    shader_draw_parameters: bool,
    timeline_semaphore: bool,
    host_query_reset: bool,
    runtime_descriptor_array: bool,
    sampled_image_array_non_uniform_indexing: bool,
    descriptor_binding_partially_bound: bool,
    descriptor_binding_variable_descriptor_count: bool,
    descriptor_binding_sampled_image_update_after_bind: bool,
    dynamic_rendering: bool,
    synchronization2: bool,
}

const REQUIRED_FEATURES: &[FeatureCheck] = &[
    ("dynamicRendering", |f| f.dynamic_rendering),
    ("synchronization2", |f| f.synchronization2),
    ("timelineSemaphore", |f| f.timeline_semaphore),
    ("multiDrawIndirect", |f| {
        f.core.multi_draw_indirect == vk::TRUE
    }),
    ("drawIndirectFirstInstance", |f| {
        f.core.draw_indirect_first_instance == vk::TRUE
    }),
    ("shaderDrawParameters", |f| f.shader_draw_parameters),
    ("runtimeDescriptorArray", |f| f.runtime_descriptor_array),
    ("shaderSampledImageArrayNonUniformIndexing", |f| {
        f.sampled_image_array_non_uniform_indexing
    }),
    ("descriptorBindingPartiallyBound", |f| {
        f.descriptor_binding_partially_bound
    }),
    ("descriptorBindingVariableDescriptorCount", |f| {
        f.descriptor_binding_variable_descriptor_count
    }),
    ("descriptorBindingSampledImageUpdateAfterBind", |f| {
        f.descriptor_binding_sampled_image_update_after_bind
    }),
    ("samplerAnisotropy", |f| {
        f.core.sampler_anisotropy == vk::TRUE
    }),
    ("hostQueryReset", |f| f.host_query_reset),
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
    let texture_slots = texture_slots(&indexing);
    if texture_slots < MIN_TEXTURE_SLOTS {
        reasons.push(format!(
            "only {texture_slots} bindless textures (needs {MIN_TEXTURE_SLOTS})"
        ));
    }

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
        max_image_dimension_2d: props.limits.max_image_dimension2_d,
        max_memory_allocations: props.limits.max_memory_allocation_count,
        copy_offset_alignment: props.limits.optimal_buffer_copy_offset_alignment,
        storage_offset_alignment: props.limits.min_storage_buffer_offset_alignment,
        transfer_family,
        portability_subset,
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

/// Slots the texture table can have under the device's update-after-bind limits.
fn texture_slots(p: &vk::PhysicalDeviceDescriptorIndexingProperties<'_>) -> u32 {
    WANTED_TEXTURE_SLOTS
        .min(p.max_per_stage_descriptor_update_after_bind_sampled_images)
        .min(p.max_descriptor_set_update_after_bind_sampled_images)
        .min(
            p.max_per_stage_update_after_bind_resources
                .saturating_sub(NON_TEXTURE_DESCRIPTORS),
        )
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
        host_query_reset: t(f12.host_query_reset),
        runtime_descriptor_array: t(f12.runtime_descriptor_array),
        sampled_image_array_non_uniform_indexing: t(
            f12.shader_sampled_image_array_non_uniform_indexing
        ),
        descriptor_binding_partially_bound: t(f12.descriptor_binding_partially_bound),
        descriptor_binding_variable_descriptor_count: t(
            f12.descriptor_binding_variable_descriptor_count
        ),
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
    let core = vk::PhysicalDeviceFeatures::default()
        .multi_draw_indirect(true)
        .draw_indirect_first_instance(true)
        .sampler_anisotropy(true);
    let mut f11 = vk::PhysicalDeviceVulkan11Features::default().shader_draw_parameters(true);
    let mut f12 = vk::PhysicalDeviceVulkan12Features::default()
        .timeline_semaphore(true)
        .host_query_reset(true)
        .runtime_descriptor_array(true)
        .shader_sampled_image_array_non_uniform_indexing(true)
        .descriptor_binding_partially_bound(true)
        .descriptor_binding_variable_descriptor_count(true)
        .descriptor_binding_sampled_image_update_after_bind(true);
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
    // SAFETY: every enabled feature and extension was checked in evaluate().
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
