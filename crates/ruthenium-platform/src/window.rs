use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr;
use std::rc::Rc;

use ash::vk;
use sdl3_sys::events::{
    SDL_EVENT_FIRST, SDL_EVENT_LAST, SDL_Event, SDL_GETEVENT, SDL_PeepEvents, SDL_PumpEvents,
};
use sdl3_sys::hints::SDL_SetHint;
use sdl3_sys::init::{SDL_INIT_VIDEO, SDL_Init, SDL_Quit, SDL_SetAppMetadata};
use sdl3_sys::video::{
    SDL_CreateWindow, SDL_DestroyWindow, SDL_GetCurrentDisplayMode, SDL_GetDisplayForWindow,
    SDL_GetWindowSize, SDL_GetWindowSizeInPixels, SDL_MinimizeWindow, SDL_RestoreWindow,
    SDL_SetWindowFullscreen, SDL_SetWindowSize, SDL_WINDOW_HIGH_PIXEL_DENSITY,
    SDL_WINDOW_RESIZABLE, SDL_WINDOW_VULKAN, SDL_Window,
};
use sdl3_sys::vulkan::{
    SDL_Vulkan_CreateSurface, SDL_Vulkan_GetInstanceExtensions,
    SDL_Vulkan_GetVkGetInstanceProcAddr, SDL_Vulkan_LoadLibrary, SDL_Vulkan_UnloadLibrary,
};

use crate::{Error, Event};

/// Vanilla's default window size (`Window.java`, `DisplayData` defaults).
pub const DEFAULT_SIZE: (u32, u32) = (854, 480);

/// Hints vanilla sets before `SDL_Init` (`RenderSystem.java:207-212`). Mouse-scaling
/// hints are deliberately left at SDL's defaults, as vanilla does, so sensitivity matches.
const VANILLA_HINTS: [(&CStr, &CStr); 6] = [
    (c"SDL_NO_SIGNAL_HANDLERS", c"1"),
    (c"SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", c"0"),
    (c"SDL_QUIT_ON_LAST_WINDOW_CLOSE", c"0"),
    (c"SDL_MOUSE_FOCUS_CLICKTHROUGH", c"1"),
    (c"SDL_ENABLE_SCREEN_KEYBOARD", c"0"),
    (c"SDL_IME_IMPLEMENTED_UI", c"composition"),
];

/// The MoltenVK an application bundle carries for the executable `exe`
/// (`Ruthenium.app/Contents/MacOS/ruthenium`): `Contents/Frameworks/libMoltenVK.dylib`, if
/// it is there (`cargo xtask bundle` puts it there).
fn bundled_vulkan_library(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let library = exe.parent()?.parent()?.join("Frameworks/libMoltenVK.dylib");
    library.is_file().then_some(library)
}

/// Quits SDL when the last `Platform` or `Window` handle is dropped.
struct SdlGuard;

impl Drop for SdlGuard {
    fn drop(&mut self) {
        // SAFETY: SDL was initialised by Platform::init, and nothing that uses it outlives
        // this guard (every handle holds an Rc to it).
        unsafe {
            SDL_Vulkan_UnloadLibrary();
            SDL_Quit();
        }
    }
}

/// SDL, initialised with vanilla's hints and the video subsystem. Main thread only.
pub struct Platform {
    sdl: Rc<SdlGuard>,
    _not_send: PhantomData<*const ()>,
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Platform").finish_non_exhaustive()
    }
}

impl Platform {
    /// Initialises SDL's video subsystem and loads Vulkan through SDL: the system's
    /// loader, or on macOS the MoltenVK bundled in the app, if the executable runs from
    /// a bundle that has one and `SDL_VULKAN_LIBRARY` doesn't name another.
    pub fn init(version: &str) -> Result<Platform, Error> {
        let version = CString::new(version).unwrap_or_default();
        let bundled = (cfg!(target_os = "macos")
            && std::env::var_os("SDL_VULKAN_LIBRARY").is_none())
        .then(|| bundled_vulkan_library(&std::env::current_exe().ok()?))
        .flatten()
        .and_then(|p| CString::new(p.into_os_string().into_encoded_bytes()).ok());
        if let Some(path) = &bundled {
            eprintln!("[platform] Vulkan: bundled {}", path.to_string_lossy());
        }
        // SAFETY: plain SDL setup calls with valid NUL-terminated strings, made before any
        // other SDL use, on the thread that will own SDL.
        unsafe {
            SDL_SetAppMetadata(
                c"Ruthenium".as_ptr(),
                version.as_ptr(),
                c"dev.ruthenium.Ruthenium".as_ptr(),
            );
            for (name, value) in VANILLA_HINTS {
                SDL_SetHint(name.as_ptr(), value.as_ptr());
            }
            if !SDL_Init(SDL_INIT_VIDEO) {
                return Err(Error::sdl("SDL_Init(VIDEO)"));
            }
            // A null path leaves the choice to SDL (its hint, then the system's loader).
            if !SDL_Vulkan_LoadLibrary(bundled.as_ref().map_or(ptr::null(), |p| p.as_ptr())) {
                let e = Error::sdl("SDL_Vulkan_LoadLibrary");
                SDL_Quit();
                return Err(e);
            }
        }
        Ok(Platform {
            sdl: Rc::new(SdlGuard),
            _not_send: PhantomData,
        })
    }

    /// Creates a resizable, high-density window able to host a Vulkan surface.
    pub fn create_window(&self, title: &str, width: u32, height: u32) -> Result<Window, Error> {
        let title = CString::new(title).unwrap_or_default();
        let flags = SDL_WINDOW_VULKAN | SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY;
        let (w, h) = (
            width.min(i32::MAX as u32) as i32,
            height.min(i32::MAX as u32) as i32,
        );
        // SAFETY: SDL is initialised; title is a valid C string.
        let raw = unsafe { SDL_CreateWindow(title.as_ptr(), w, h, flags) };
        if raw.is_null() {
            return Err(Error::sdl("SDL_CreateWindow"));
        }
        Ok(Window {
            raw,
            _sdl: self.sdl.clone(),
        })
    }

    /// The instance extensions SDL needs for `Window::create_surface` on this platform.
    pub fn vulkan_instance_extensions(&self) -> Vec<&'static CStr> {
        let mut count = 0u32;
        // SAFETY: the Vulkan library is loaded; SDL returns an array it owns for the rest of
        // the program (until SDL_Vulkan_UnloadLibrary, which only runs at SDL shutdown).
        unsafe {
            let names: *const *const c_char = SDL_Vulkan_GetInstanceExtensions(&mut count);
            if names.is_null() {
                return Vec::new();
            }
            (0..count as usize)
                .map(|i| CStr::from_ptr(*names.add(i)))
                .collect()
        }
    }

    /// `vkGetInstanceProcAddr` from the Vulkan loader SDL loaded.
    pub fn vk_get_instance_proc_addr(&self) -> Result<vk::PFN_vkGetInstanceProcAddr, Error> {
        // SAFETY: the Vulkan library is loaded.
        unsafe { SDL_Vulkan_GetVkGetInstanceProcAddr() }
            // SAFETY: both are vkGetInstanceProcAddr's signature; they differ only in
            // sdl3-sys's and ash's spelling of the handle and return types (same ABI).
            .map(|f| unsafe {
                std::mem::transmute::<
                    unsafe extern "system" fn(
                        sdl3_sys::vulkan::VkInstance,
                        *const c_char,
                    )
                        -> Option<unsafe extern "system" fn()>,
                    vk::PFN_vkGetInstanceProcAddr,
                >(f)
            })
            .ok_or_else(|| Error::sdl("SDL_Vulkan_GetVkGetInstanceProcAddr"))
    }

    /// Drains every queued event, oldest first, without allocating: events are read from
    /// SDL's queue 64 at a time into a stack buffer.
    pub fn poll_events(&mut self, mut handle: impl FnMut(Event)) {
        const BATCH: usize = 64;
        let mut buf = [const { MaybeUninit::<SDL_Event>::uninit() }; BATCH];
        // SAFETY: SDL is initialised and we are on its main thread; PeepEvents writes at
        // most BATCH events and returns how many it wrote.
        unsafe {
            SDL_PumpEvents();
            loop {
                let n = SDL_PeepEvents(
                    buf.as_mut_ptr().cast(),
                    BATCH as i32,
                    SDL_GETEVENT,
                    SDL_EVENT_FIRST.0,
                    SDL_EVENT_LAST.0,
                );
                if n <= 0 {
                    break;
                }
                for e in &buf[..n as usize] {
                    Event::dispatch(e.assume_init_ref(), &mut handle);
                }
                if (n as usize) < BATCH {
                    break;
                }
            }
        }
    }
}

/// The game window. Drop it after everything that uses its Vulkan surface.
pub struct Window {
    raw: *mut SDL_Window,
    _sdl: Rc<SdlGuard>,
}

impl std::fmt::Debug for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Window").finish_non_exhaustive()
    }
}

impl Window {
    /// The drawable size in pixels (not points): what the swapchain must match.
    pub fn pixel_size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0, 0);
        // SAFETY: raw is a live window.
        unsafe { SDL_GetWindowSizeInPixels(self.raw, &mut w, &mut h) };
        (w.max(0) as u32, h.max(0) as u32)
    }

    /// The window size in points (window coordinates, as mouse events report them).
    pub fn size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0, 0);
        // SAFETY: raw is a live window.
        unsafe { SDL_GetWindowSize(self.raw, &mut w, &mut h) };
        (w.max(0) as u32, h.max(0) as u32)
    }

    /// Sets the window's icon from `size` x `size` RGBA8 texels (straight alpha, rows
    /// top first). Some window managers (Wayland compositors) take icons from the desktop
    /// entry instead and ignore it.
    pub fn set_icon(&self, rgba: &[u8], size: u32) {
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        let mut pixels = rgba.to_vec();
        let side = size.min(i32::MAX as u32 / 4) as i32;
        // SAFETY: raw is a live window; the surface borrows `pixels`, which outlives it (SDL
        // copies the icon in SDL_SetWindowIcon), and is destroyed here.
        unsafe {
            let surface = sdl3_sys::surface::SDL_CreateSurfaceFrom(
                side,
                side,
                sdl3_sys::pixels::SDL_PIXELFORMAT_RGBA32,
                pixels.as_mut_ptr().cast(),
                side * 4,
            );
            if !surface.is_null() {
                sdl3_sys::video::SDL_SetWindowIcon(self.raw, surface);
                sdl3_sys::surface::SDL_DestroySurface(surface);
            }
        }
    }

    /// Starts delivering typed text as [`Event::TextInput`] (and shows the IME when
    /// there is one), for a focused text field.
    pub fn start_text_input(&self) {
        // SAFETY: raw is a live window.
        unsafe { sdl3_sys::keyboard::SDL_StartTextInput(self.raw) };
    }

    pub fn stop_text_input(&self) {
        // SAFETY: raw is a live window.
        unsafe { sdl3_sys::keyboard::SDL_StopTextInput(self.raw) };
    }

    /// The clipboard's text, if it holds any.
    pub fn clipboard_text(&self) -> Option<String> {
        // SAFETY: SDL is initialised (this window holds it); the returned string is ours
        // to free with SDL_free, and is NUL-terminated.
        unsafe {
            let p = sdl3_sys::clipboard::SDL_GetClipboardText();
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            sdl3_sys::stdinc::SDL_free(p.cast());
            (!s.is_empty()).then_some(s)
        }
    }

    pub fn set_clipboard_text(&self, text: &str) {
        let Ok(c) = CString::new(text) else { return };
        // SAFETY: SDL is initialised; SDL copies the string.
        unsafe { sdl3_sys::clipboard::SDL_SetClipboardText(c.as_ptr()) };
    }

    /// Grabs the mouse for turning the camera (on: relative mode, the pointer hidden and
    /// held in the window) or lets it go, warping it to the window's centre first, as
    /// vanilla does (`InputConstants.grabMouse`, `releaseMouse`). False if SDL refused.
    pub fn set_mouse_grabbed(&self, on: bool) -> bool {
        let (w, h) = self.size();
        // SAFETY: raw is a live window.
        unsafe {
            sdl3_sys::mouse::SDL_WarpMouseInWindow(self.raw, w as f32 / 2.0, h as f32 / 2.0);
            sdl3_sys::mouse::SDL_SetWindowRelativeMouseMode(self.raw, on)
        }
    }

    /// Asks the window manager for a new window size in points (it may refuse or adjust;
    /// the result arrives as [`Event::PixelSizeChanged`]).
    pub fn request_size(&self, width: u32, height: u32) {
        let (w, h) = (
            width.min(i32::MAX as u32) as i32,
            height.min(i32::MAX as u32) as i32,
        );
        // SAFETY: raw is a live window.
        unsafe { SDL_SetWindowSize(self.raw, w, h) };
    }

    /// The refresh rate of the display the window is on, in hertz, if SDL knows it.
    pub fn refresh_rate(&self) -> Option<f32> {
        // SAFETY: raw is a live window; the mode SDL returns is its own and is only read
        // here, before any other SDL call.
        let rate = unsafe {
            let mode = SDL_GetCurrentDisplayMode(SDL_GetDisplayForWindow(self.raw));
            mode.as_ref().map(|m| m.refresh_rate)
        };
        rate.filter(|r| r.is_finite() && *r >= 1.0)
    }

    /// Asks the window manager to minimise the window (it may refuse).
    pub fn minimize(&self) {
        // SAFETY: raw is a live window.
        unsafe { SDL_MinimizeWindow(self.raw) };
    }

    /// Asks the window manager to bring a minimised window back. Wayland has no request
    /// for it, so there nothing happens.
    pub fn restore(&self) {
        // SAFETY: raw is a live window.
        unsafe { SDL_RestoreWindow(self.raw) };
    }

    /// Asks for borderless fullscreen on the window's display (on) or the window back
    /// (off). The window manager confirms with [`Event::Fullscreen`]; the new size
    /// arrives as [`Event::PixelSizeChanged`].
    pub fn set_fullscreen(&self, on: bool) {
        // SAFETY: raw is a live window. With no fullscreen mode set, SDL uses the
        // desktop's (borderless fullscreen, no mode switch).
        unsafe { SDL_SetWindowFullscreen(self.raw, on) };
    }

    /// Creates a Vulkan surface for this window.
    ///
    /// # Safety
    /// `instance` must be a live instance created with
    /// [`Platform::vulkan_instance_extensions`] enabled. The surface must be destroyed
    /// (with `vkDestroySurfaceKHR`) before this window and before the instance.
    pub unsafe fn create_surface(&self, instance: vk::Instance) -> Result<vk::SurfaceKHR, Error> {
        use ash::vk::Handle;
        // sdl3-sys's own handle types: VkInstance is a pointer and VkSurfaceKHR a pointer on
        // 64-bit targets (the only ones Ruthenium supports), both equal to ash's raw u64.
        let raw_instance = instance.as_raw() as usize as sdl3_sys::vulkan::VkInstance;
        let mut surface: sdl3_sys::vulkan::VkSurfaceKHR = ptr::null_mut();
        // SAFETY: guaranteed by the caller; raw is a live window created with SDL_WINDOW_VULKAN.
        if unsafe { SDL_Vulkan_CreateSurface(self.raw, raw_instance, ptr::null(), &mut surface) } {
            Ok(vk::SurfaceKHR::from_raw(surface as usize as u64))
        } else {
            Err(Error::sdl("SDL_Vulkan_CreateSurface"))
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: raw is a live window owned by this handle.
        unsafe { SDL_DestroyWindow(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_carries_its_vulkan_library_in_frameworks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let contents = root.join("Ruthenium.app/Contents");
        let exe = contents.join("MacOS/ruthenium");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Frameworks")).unwrap();
        // No library yet: the system's loader is used.
        assert_eq!(bundled_vulkan_library(&exe), None);
        let library = contents.join("Frameworks/libMoltenVK.dylib");
        std::fs::write(&library, b"").unwrap();
        assert_eq!(bundled_vulkan_library(&exe), Some(library));
        // An executable outside a bundle (target/release/ruthenium) has none.
        assert_eq!(bundled_vulkan_library(&root.join("ruthenium")), None);
        assert_eq!(
            bundled_vulkan_library(std::path::Path::new("ruthenium")),
            None
        );
    }
}
