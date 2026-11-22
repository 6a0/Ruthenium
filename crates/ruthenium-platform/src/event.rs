use std::ffi::CStr;

use sdl3_sys::events::*;
use sdl3_sys::keycode::{SDL_KMOD_CTRL, SDL_KMOD_GUI, SDL_KMOD_SHIFT};
use sdl3_sys::mouse::SDL_MOUSEWHEEL_FLIPPED;

/// The longest piece of typed text one [`Event::TextInput`] carries; longer input
/// arrives as several events.
pub const TEXT_INPUT_BYTES: usize = 32;

/// The subset of SDL events the client handles so far. Timestamps are SDL's nanosecond
/// event timestamps (`SDL_GetTicksNS` clock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The application was asked to quit (for example SIGTERM, or the last window closed).
    Quit,
    /// The window manager asked to close the window.
    CloseRequested,
    /// The drawable size changed; the swapchain must be rebuilt.
    PixelSizeChanged {
        width: u32,
        height: u32,
    },
    Minimized,
    Restored,
    Shown,
    Hidden,
    /// Fully covered by other windows: the compositor may stop releasing frames.
    Occluded,
    Exposed,
    FocusGained,
    FocusLost,
    /// The mouse moved to `(x, y)`, in window coordinates (points, not pixels).
    MouseMotion {
        x: f32,
        y: f32,
    },
    /// A mouse button (1 left, 2 middle, 3 right, as SDL numbers them) went down or up at
    /// `(x, y)` in window coordinates.
    MouseButton {
        button: u8,
        down: bool,
        x: f32,
        y: f32,
        timestamp_ns: u64,
    },
    /// The mouse left the window.
    MouseLeft,
    /// The wheel turned: `y` notches, positive away from the user (scroll up), with the
    /// system's natural-scrolling setting already applied.
    MouseWheel {
        y: f32,
    },
    /// A key went down or up. `scancode` is the SDL scancode, the value vanilla stores in
    /// options.txt. `shift` is whether a Shift key was held; `shortcut` whether the
    /// shortcut modifier was (Ctrl, or Cmd on macOS).
    Key {
        scancode: u32,
        down: bool,
        repeat: bool,
        shift: bool,
        shortcut: bool,
        timestamp_ns: u64,
    },
    /// Text was typed (while text input is on, see `Window::start_text_input`): UTF-8,
    /// whole characters, read with [`Event::text`].
    TextInput {
        bytes: [u8; TEXT_INPUT_BYTES],
        len: u8,
    },
}

impl Event {
    /// The text of a [`Event::TextInput`] (empty for other events).
    pub fn text(&self) -> &str {
        match self {
            Event::TextInput { bytes, len } => {
                std::str::from_utf8(&bytes[..usize::from(*len)]).unwrap_or("")
            }
            _ => "",
        }
    }

    /// Hands `e` to `handle` as zero or more events (text input in pieces).
    pub(crate) fn dispatch(e: &SDL_Event, handle: &mut impl FnMut(Event)) {
        // SAFETY: `type` is valid for every event, and text input events carry a NUL-
        // terminated string (or null) that lives until the next event poll.
        let text = unsafe {
            if SDL_EventType(e.r#type) == SDL_EVENT_TEXT_INPUT && !e.text.text.is_null() {
                Some(CStr::from_ptr(e.text.text))
            } else {
                None
            }
        };
        if let Some(text) = text {
            for piece in text_pieces(&text.to_string_lossy()) {
                handle(piece);
            }
        } else if let Some(ev) = Event::from_sdl(e) {
            handle(ev);
        }
    }

    fn from_sdl(e: &SDL_Event) -> Option<Event> {
        // SAFETY: `type` is valid for every event; each arm reads only the union member
        // that SDL documents for that event type.
        unsafe {
            let ty = SDL_EventType(e.r#type);
            Some(match ty {
                SDL_EVENT_QUIT => Event::Quit,
                SDL_EVENT_WINDOW_CLOSE_REQUESTED => Event::CloseRequested,
                SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED => Event::PixelSizeChanged {
                    width: e.window.data1.max(0) as u32,
                    height: e.window.data2.max(0) as u32,
                },
                SDL_EVENT_WINDOW_MINIMIZED => Event::Minimized,
                SDL_EVENT_WINDOW_RESTORED => Event::Restored,
                SDL_EVENT_WINDOW_SHOWN => Event::Shown,
                SDL_EVENT_WINDOW_HIDDEN => Event::Hidden,
                SDL_EVENT_WINDOW_OCCLUDED => Event::Occluded,
                SDL_EVENT_WINDOW_EXPOSED => Event::Exposed,
                SDL_EVENT_WINDOW_FOCUS_GAINED => Event::FocusGained,
                SDL_EVENT_WINDOW_FOCUS_LOST => Event::FocusLost,
                SDL_EVENT_WINDOW_MOUSE_LEAVE => Event::MouseLeft,
                SDL_EVENT_MOUSE_MOTION => Event::MouseMotion {
                    x: e.motion.x,
                    y: e.motion.y,
                },
                SDL_EVENT_MOUSE_BUTTON_DOWN | SDL_EVENT_MOUSE_BUTTON_UP => Event::MouseButton {
                    button: e.button.button,
                    down: e.button.down,
                    x: e.button.x,
                    y: e.button.y,
                    timestamp_ns: e.button.timestamp,
                },
                SDL_EVENT_KEY_DOWN | SDL_EVENT_KEY_UP => Event::Key {
                    scancode: e.key.scancode.0 as u32,
                    down: e.key.down,
                    repeat: e.key.repeat,
                    shift: e.key.r#mod.0 & SDL_KMOD_SHIFT.0 != 0,
                    shortcut: e.key.r#mod.0
                        & if cfg!(target_os = "macos") {
                            SDL_KMOD_GUI.0
                        } else {
                            SDL_KMOD_CTRL.0
                        }
                        != 0,
                    timestamp_ns: e.key.timestamp,
                },
                SDL_EVENT_MOUSE_WHEEL => Event::MouseWheel {
                    y: if e.wheel.direction == SDL_MOUSEWHEEL_FLIPPED {
                        -e.wheel.y
                    } else {
                        e.wheel.y
                    },
                },
                _ => return None,
            })
        }
    }
}

/// Splits typed text into events of at most [`TEXT_INPUT_BYTES`] bytes, at character
/// boundaries.
fn text_pieces(text: &str) -> impl Iterator<Item = Event> + '_ {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = rest.len().min(TEXT_INPUT_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (piece, tail) = rest.split_at(end);
        rest = tail;
        let mut bytes = [0u8; TEXT_INPUT_BYTES];
        bytes[..piece.len()].copy_from_slice(piece.as_bytes());
        Some(Event::TextInput {
            bytes,
            len: piece.len() as u8,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_arrives_in_whole_characters() {
        let text = "héllo wörld, ünïcödé ïs splït ïntö pïècés";
        let pieces: Vec<String> = text_pieces(text).map(|e| e.text().to_owned()).collect();
        assert!(pieces.len() > 1);
        assert!(pieces.iter().all(|p| p.len() <= TEXT_INPUT_BYTES));
        assert_eq!(pieces.concat(), text);
        assert_eq!(Event::Quit.text(), "");
    }
}
