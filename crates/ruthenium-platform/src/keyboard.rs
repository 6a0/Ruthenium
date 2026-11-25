//! What the keyboard's layout calls its keys, for the names key bindings show.

use std::ffi::CStr;

use sdl3_sys::keyboard::{SDL_GetKeyFromScancode, SDL_GetKeyName};
use sdl3_sys::keycode::SDL_KMOD_NONE;
use sdl3_sys::scancode::SDL_Scancode;

/// SDL's scancodes run below this (`SDL_SCANCODE_COUNT`).
const SCANCODES: i32 = 512;

/// What the keyboard's layout calls the key at SDL scancode `scancode`, unshifted
/// (`SDL_GetKeyName` of `SDL_GetKeyFromScancode`, as vanilla's `InputConstants` asks):
/// `W`, `Left Shift`, or `Z` where a German layout has it. `None` for a key it has no
/// name for. Main thread only, after the platform is up.
pub fn key_name(scancode: i32) -> Option<String> {
    if !(0..SCANCODES).contains(&scancode) {
        return None;
    }
    // SAFETY: the scancode is in SDL's range; SDL_GetKeyName returns a NUL-terminated
    // string (empty for no name) that is copied before any other SDL call.
    let name = unsafe {
        let key = SDL_GetKeyFromScancode(SDL_Scancode(scancode), SDL_KMOD_NONE, false);
        let p = SDL_GetKeyName(key);
        if p.is_null() {
            return None;
        }
        CStr::from_ptr(p).to_string_lossy().into_owned()
    };
    Some(name).filter(|n| !n.is_empty())
}
