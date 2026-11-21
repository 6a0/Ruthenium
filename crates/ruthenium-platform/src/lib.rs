//! Ruthenium's platform layer: SDL3 with vanilla's hints, the game window and its Vulkan
//! surface, allocation-free event delivery, and precise sleeping for the frame limiter.
//!
//! Vanilla 26.3 also runs on SDL3, so key codes, IME and relative-mouse behaviour match it.
//! Everything here must be used from the main thread: SDL's video subsystem requires it.

mod event;
mod sleep;
mod window;

pub use event::Event;
pub use sleep::{Sleeper, next_deadline};
pub use window::{DEFAULT_SIZE, Platform, Window};

use std::ffi::CStr;
use std::fmt;

/// An SDL call failed.
#[derive(Debug)]
pub struct Error {
    what: &'static str,
    detail: String,
}

impl Error {
    /// Takes SDL's message for the call that just failed.
    fn sdl(what: &'static str) -> Self {
        // SAFETY: SDL_GetError always returns a valid NUL-terminated string (possibly empty).
        let detail = unsafe { CStr::from_ptr(sdl3_sys::error::SDL_GetError()) }
            .to_string_lossy()
            .into_owned();
        Error { what, detail }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{} failed", self.what)
        } else {
            write!(f, "{} failed: {}", self.what, self.detail)
        }
    }
}

impl std::error::Error for Error {}
