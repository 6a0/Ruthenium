use sdl3_sys::events::*;

/// The subset of SDL events the client handles so far. Timestamps are SDL's nanosecond
/// event timestamps (`SDL_GetTicksNS` clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// A key went down or up. `scancode` is the SDL scancode, the value vanilla stores in
    /// options.txt.
    Key {
        scancode: u32,
        down: bool,
        repeat: bool,
        timestamp_ns: u64,
    },
}

impl Event {
    pub(crate) fn from_sdl(e: &SDL_Event) -> Option<Event> {
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
                SDL_EVENT_KEY_DOWN | SDL_EVENT_KEY_UP => Event::Key {
                    scancode: e.key.scancode.0 as u32,
                    down: e.key.down,
                    repeat: e.key.repeat,
                    timestamp_ns: e.key.timestamp,
                },
                _ => return None,
            })
        }
    }
}
