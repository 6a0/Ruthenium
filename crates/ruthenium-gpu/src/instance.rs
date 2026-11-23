use std::borrow::Cow;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

use ash::vk;
use ash::vk::TaggedStructure;

use crate::{Error, VkContext};

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// Validation-layer errors and warnings reported so far (tests and CI assert zero).
static VALIDATION_MESSAGES: AtomicU32 = AtomicU32::new(0);

/// How many validation errors and warnings have been reported in this process.
pub fn validation_messages() -> u32 {
    VALIDATION_MESSAGES.load(Ordering::Relaxed)
}

/// Whether to load the Khronos validation layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validation {
    Off,
    /// Use it if it is installed (default in debug builds).
    IfAvailable,
    /// Fail if it is missing.
    Required,
}

/// What the platform layer supplies to create an instance.
#[derive(Debug)]
pub struct InstanceDesc<'a> {
    /// `vkGetInstanceProcAddr` of the Vulkan loader (SDL loads it, which on macOS finds the
    /// bundled MoltenVK).
    pub get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    /// Extensions the window system needs for surfaces.
    pub surface_extensions: &'a [&'a CStr],
    pub validation: Validation,
}

/// A Vulkan instance with the surface extension loaded.
pub struct Instance {
    /// Holds the loader's function table that `raw` was created through.
    _entry: ash::Entry,
    pub(crate) raw: ash::Instance,
    pub(crate) surface_fn: ash::khr::surface::Instance,
    debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub(crate) api_version: u32,
    /// `VK_KHR_surface_maintenance1` and `VK_EXT_surface_maintenance1` are enabled (each
    /// with `VK_KHR_get_surface_capabilities2`, which both need): what a device's
    /// swapchain maintenance extension of the same kind depends on.
    pub(crate) surface_maintenance_khr: bool,
    pub(crate) surface_maintenance_ext: bool,
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance")
            .field("api_version", &crate::version_string(self.api_version))
            .field("validation", &self.debug.is_some())
            .finish_non_exhaustive()
    }
}

impl Instance {
    pub fn new(desc: &InstanceDesc<'_>) -> Result<Instance, Error> {
        // SAFETY: the function pointer comes from a loaded Vulkan loader (InstanceDesc contract).
        let entry = unsafe {
            ash::Entry::from_static_fn(ash::StaticFn {
                get_instance_proc_addr: desc.get_instance_proc_addr,
            })
        };
        // SAFETY: plain loader queries.
        let loader_version = unsafe { entry.try_enumerate_instance_version() }
            .ctx("vkEnumerateInstanceVersion")?
            .unwrap_or(vk::API_VERSION_1_0);
        if loader_version < vk::API_VERSION_1_2 {
            return Err(Error::Unsupported(format!(
                "the Vulkan loader is version {}; Ruthenium needs 1.2 or newer (update the graphics driver)",
                crate::version_string(loader_version)
            )));
        }
        // The highest version Ruthenium is written against; devices may be lower (1.2).
        let api_version = loader_version.min(vk::API_VERSION_1_3);

        // SAFETY: plain loader queries.
        let available_exts = unsafe { entry.enumerate_instance_extension_properties(None) }
            .ctx("vkEnumerateInstanceExtensionProperties")?;
        let has_ext = |name: &CStr| {
            available_exts
                .iter()
                .any(|e| e.extension_name_as_c_str() == Ok(name))
        };
        // SAFETY: plain loader query.
        let layers = unsafe { entry.enumerate_instance_layer_properties() }
            .ctx("vkEnumerateInstanceLayerProperties")?;
        let has_validation = layers
            .iter()
            .any(|l| l.layer_name_as_c_str() == Ok(VALIDATION_LAYER));

        let mut extensions: Vec<*const c_char> = Vec::new();
        for &e in desc.surface_extensions {
            if !has_ext(e) {
                return Err(Error::Unsupported(format!(
                    "the Vulkan driver lacks {}, needed to show a window",
                    e.to_string_lossy()
                )));
            }
            extensions.push(e.as_ptr());
        }
        let mut flags = vk::InstanceCreateFlags::empty();
        // MoltenVK (and other non-conformant drivers) are only listed with this extension.
        if has_ext(ash::khr::portability_enumeration::NAME) {
            extensions.push(ash::khr::portability_enumeration::NAME.as_ptr());
            flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
        }
        // Swapchain maintenance is a device extension that depends on these. Both kinds are
        // enabled when the loader has them, so the device can use whichever chain is whole.
        let mut enable = |name: &'static CStr| {
            if !desc.surface_extensions.contains(&name) {
                extensions.push(name.as_ptr());
            }
        };
        let surface_caps2 = has_ext(ash::khr::get_surface_capabilities2::NAME);
        if surface_caps2 {
            enable(ash::khr::get_surface_capabilities2::NAME);
        }
        let surface_maintenance_khr =
            surface_caps2 && has_ext(ash::khr::surface_maintenance1::NAME);
        if surface_maintenance_khr {
            enable(ash::khr::surface_maintenance1::NAME);
        }
        let surface_maintenance_ext =
            surface_caps2 && has_ext(ash::ext::surface_maintenance1::NAME);
        if surface_maintenance_ext {
            enable(ash::ext::surface_maintenance1::NAME);
        }
        let validate = match desc.validation {
            Validation::Off => false,
            Validation::IfAvailable => has_validation,
            Validation::Required if has_validation => true,
            Validation::Required => {
                return Err(Error::Unsupported(
                    "validation was requested but VK_LAYER_KHRONOS_validation is not installed"
                        .into(),
                ));
            }
        };
        let debug_utils = validate && has_ext(ash::ext::debug_utils::NAME);
        if debug_utils {
            extensions.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        let layer_names: Vec<*const c_char> = if validate {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };
        // Synchronization validation is off by default in the layer; it is what catches
        // missing barriers and semaphore misuse, so turn it on whenever validating.
        let layer_settings_ext = validate
            && (has_ext(ash::ext::layer_settings::NAME)
                // SAFETY: plain loader query.
                || unsafe { entry.enumerate_instance_extension_properties(Some(VALIDATION_LAYER)) }
                    .unwrap_or_default()
                    .iter()
                    .any(|e| e.extension_name_as_c_str() == Ok(ash::ext::layer_settings::NAME)));
        if layer_settings_ext {
            extensions.push(ash::ext::layer_settings::NAME.as_ptr());
        }
        let on = [vk::TRUE];
        let settings = [vk::LayerSettingEXT::default()
            .layer_name(VALIDATION_LAYER)
            .setting_name(c"validate_sync")
            .ty(vk::LayerSettingTypeEXT::BOOL32)
            .values(&on)];
        let mut layer_settings = vk::LayerSettingsCreateInfoEXT::default().settings(&settings);

        let app = vk::ApplicationInfo::default()
            .application_name(c"Ruthenium")
            .application_version(0)
            .engine_name(c"Ruthenium")
            .engine_version(0)
            .api_version(api_version);
        let info = vk::InstanceCreateInfo::default()
            .flags(flags)
            .application_info(&app)
            .enabled_layer_names(&layer_names)
            .enabled_extension_names(&extensions);
        let info = if layer_settings_ext {
            info.push(&mut layer_settings)
        } else {
            info
        };
        // SAFETY: every pointer in info outlives the call; extensions/layers were checked.
        let raw = unsafe { entry.create_instance(&info, None) }.ctx("vkCreateInstance")?;
        let surface_fn = ash::khr::surface::Instance::load(&entry, &raw);

        let debug = if debug_utils {
            let loader = ash::ext::debug_utils::Instance::load(&entry, &raw);
            let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(on_debug_message));
            // SAFETY: valid create info; the messenger is destroyed before the instance.
            match unsafe { loader.create_debug_utils_messenger(&info, None) } {
                Ok(m) => Some((loader, m)),
                Err(e) => {
                    eprintln!("[gpu] could not install the validation message handler: {e}");
                    None
                }
            }
        } else {
            None
        };
        if validate {
            let sync = if layer_settings_ext {
                " with synchronization validation"
            } else {
                ""
            };
            eprintln!("[gpu] Vulkan validation layer enabled{sync}");
        }
        Ok(Instance {
            _entry: entry,
            raw,
            surface_fn,
            debug,
            api_version,
            surface_maintenance_khr,
            surface_maintenance_ext,
        })
    }

    /// The raw instance handle, for creating a window surface.
    pub fn handle(&self) -> vk::Instance {
        self.raw.handle()
    }

    /// Whether the validation layer is active.
    pub fn validating(&self) -> bool {
        self.debug.is_some()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: everything created from this instance (devices, surfaces) has been
        // destroyed by its owner (Gpu) before the instance is dropped.
        unsafe {
            if let Some((loader, messenger)) = self.debug.take() {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

unsafe extern "system" fn on_debug_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    types: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: the layer passes a valid callback-data struct for the duration of the call.
    let message = unsafe { data.as_ref() }
        .and_then(|d| unsafe { d.message_as_c_str() })
        .map(CStr::to_string_lossy)
        .unwrap_or(Cow::Borrowed("(no message)"));
    let level = if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        "error"
    } else {
        "warning"
    };
    if !types.contains(vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE) {
        VALIDATION_MESSAGES.fetch_add(1, Ordering::Relaxed);
    }
    eprintln!("[vulkan {level}] {message}");
    vk::FALSE
}
