//! Ruthenium's out-of-game menus (for now the title screen): lays them out for the
//! window, prepares the glyph atlas and the logo in the background and uploads them, and
//! builds each frame's shapes and text.

use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_gpu::{Arrival, TextureDesc, TextureId, UploadError};
use ruthenium_render::{GuiFrame, Renderer};
use ruthenium_ui::logo::Logo;
use ruthenium_ui::modern::{Action, ButtonStates, Key, ShapeList, TitleScreen, UiScale, WEIGHTS};
use ruthenium_ui::text::{ATLAS_SIZE, Fonts, GlyphAtlas, TextList, TextStyle, Weight};

/// Side of the logo texture's top level.
const LOGO_SIZE: u32 = 512;
/// Side of the window icon.
pub const ICON_SIZE: u32 = 256;

/// SDL scancodes the menus use.
const SCANCODE_RETURN: u32 = 40;
const SCANCODE_TAB: u32 = 43;
const SCANCODE_SPACE: u32 = 44;
const SCANCODE_DOWN: u32 = 81;
const SCANCODE_UP: u32 = 82;
const SCANCODE_KP_ENTER: u32 = 88;
/// The longest step the menus' motion takes in one frame (after a stall or a hidden window).
const MAX_STEP: f32 = 0.1;

/// Every printable ASCII character: what the menus' text uses for now.
const ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// What the background thread prepares.
struct Prepared {
    atlas: GlyphAtlas,
    logo: Logo,
    icon: Vec<u8>,
    ms: f64,
}

enum Atlas {
    Building(JoinHandle<Prepared>),
    /// Built; the textures wait for room in the upload ring.
    Upload {
        atlas: GlyphAtlas,
        logo: Logo,
        atlas_texture: Option<TextureId>,
        logo_texture: Option<TextureId>,
    },
    Ready {
        atlas: GlyphAtlas,
        texture: TextureId,
        logo: TextureId,
    },
    Failed,
}

/// See the module docs.
pub struct Menu {
    fonts: Fonts,
    atlas: Atlas,
    title: Option<TitleScreen>,
    shapes: ShapeList,
    text: TextList,
    /// Mouse position in window coordinates.
    mouse: Option<(f32, f32)>,
    /// The title screen's buttons: pointer, focus and motion (kept across layouts).
    states: ButtonStates,
    last_frame: Option<Instant>,
}

impl std::fmt::Debug for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menu").finish_non_exhaustive()
    }
}

fn glyph_atlas() -> GlyphAtlas {
    let fonts = Fonts::new(&WEIGHTS);
    let mut atlas = GlyphAtlas::new(ATLAS_SIZE);
    // Every weight's glyphs in one batch, so they are generated in parallel.
    let glyphs: Vec<(Weight, u16)> = WEIGHTS
        .iter()
        .flat_map(|&w| {
            let line = fonts.shape(
                ASCII,
                TextStyle {
                    size: 1.0,
                    weight: w,
                    tracking: 0.0,
                },
            );
            line.glyphs.into_iter().map(move |(g, _, _)| (w, g))
        })
        .collect();
    if !atlas.add_many(&fonts, &glyphs) {
        eprintln!("[ruthenium] the glyph atlas is full; some menu text is missing");
    }
    atlas
}

/// The glyph atlas and the logo, side by side.
fn prepare() -> Prepared {
    let started = Instant::now();
    let (atlas, logo) = std::thread::scope(|s| {
        let logo = s.spawn(|| Logo::new(LOGO_SIZE, ICON_SIZE));
        let atlas = glyph_atlas();
        (atlas, logo.join().expect("the logo thread"))
    });
    let icon = logo.icon.clone();
    Prepared {
        atlas,
        logo,
        icon,
        ms: started.elapsed().as_secs_f64() * 1e3,
    }
}

impl Menu {
    /// Starts building the glyph atlas on another thread.
    pub fn new() -> Menu {
        let handle = std::thread::Builder::new()
            .name("menu-assets".into())
            .spawn(prepare)
            .expect("spawning a thread");
        Menu {
            fonts: Fonts::new(&WEIGHTS),
            atlas: Atlas::Building(handle),
            title: None,
            shapes: ShapeList::new(),
            text: TextList::new(),
            mouse: None,
            states: ButtonStates::new(0),
            last_frame: None,
        }
    }

    pub fn mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse = Some((x, y));
    }

    /// The mouse left the window.
    pub fn mouse_left(&mut self) {
        self.mouse = None;
    }

    /// The window lost focus: drops any press.
    pub fn cancel(&mut self) {
        self.states.cancel();
    }

    /// A key went down or up: the action it triggers, if any.
    pub fn key(&mut self, scancode: u32, down: bool, repeat: bool, shift: bool) -> Option<Action> {
        let key = match scancode {
            SCANCODE_DOWN => Key::Next,
            SCANCODE_UP => Key::Previous,
            SCANCODE_TAB if shift => Key::Previous,
            SCANCODE_TAB => Key::Next,
            SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE if !repeat => Key::Activate,
            _ => return None,
        };
        let i = self.states.key(key, down)?;
        Some(self.title.as_ref()?.buttons.get(i)?.action)
    }

    fn layout(&mut self, pixel_size: (u32, u32)) -> &TitleScreen {
        let scale = UiScale::new(pixel_size.0, pixel_size.1);
        if self.title.as_ref().is_none_or(|t| t.scale != scale) {
            let version = concat!("Ruthenium ", env!("CARGO_PKG_VERSION"));
            let title = TitleScreen::new(scale, &self.fonts, version);
            if title.buttons.len() != self.states.len() {
                self.states = ButtonStates::new(title.buttons.len());
            }
            self.title = Some(title);
        }
        self.title.as_ref().unwrap()
    }

    /// The left mouse button went down or up at `(x, y)` (window coordinates): the action
    /// of the button it released over, if any (buttons act on release).
    pub fn mouse_button(
        &mut self,
        down: bool,
        x: f32,
        y: f32,
        pixel_size: (u32, u32),
        window_size: (u32, u32),
    ) -> Option<Action> {
        self.mouse = Some((x, y));
        let title = self.layout(pixel_size);
        let (ux, uy) = title.scale.from_window(x, y, window_size, pixel_size);
        let hit = title.hit(ux, uy);
        if down {
            self.states.mouse_down(hit);
            None
        } else {
            let i = self.states.mouse_up(hit)?;
            Some(self.title.as_ref()?.buttons[i].action)
        }
    }

    /// Moves the menus' textures along: picks them up when prepared, then uploads them.
    /// Returns the window icon once, when it is ready.
    fn advance(&mut self, renderer: &mut Renderer) -> Option<Vec<u8>> {
        let mut icon = None;
        self.atlas = match std::mem::replace(&mut self.atlas, Atlas::Failed) {
            Atlas::Building(h) if !h.is_finished() => Atlas::Building(h),
            Atlas::Building(h) => match h.join() {
                Ok(p) => {
                    eprintln!(
                        "[ruthenium] menu assets: {} glyphs of Nunito and the logo in {:.0} ms",
                        p.atlas.len(),
                        p.ms
                    );
                    icon = Some(p.icon);
                    Atlas::Upload {
                        atlas: p.atlas,
                        logo: p.logo,
                        atlas_texture: None,
                        logo_texture: None,
                    }
                }
                Err(_) => {
                    eprintln!("[ruthenium] no menu text or logo: their thread panicked");
                    Atlas::Failed
                }
            },
            Atlas::Upload {
                atlas,
                logo,
                mut atlas_texture,
                mut logo_texture,
            } => {
                let mut failed = false;
                if atlas_texture.is_none() {
                    let desc = TextureDesc::rgba8(atlas.size, atlas.size);
                    match renderer.create_texture(desc, &[&atlas.pixels], Arrival::Background) {
                        Ok(t) => atlas_texture = Some(t),
                        Err(ruthenium_gpu::Error::Upload(UploadError::Full)) => {}
                        Err(e) => {
                            eprintln!("[ruthenium] no menu text: glyph atlas upload: {e}");
                            failed = true;
                        }
                    }
                }
                if logo_texture.is_none() && !failed {
                    let desc = TextureDesc {
                        mip_levels: logo.levels.len() as u32,
                        ..TextureDesc::rgba8(logo.size, logo.size)
                    };
                    let levels: Vec<&[u8]> = logo.levels.iter().map(Vec::as_slice).collect();
                    match renderer.create_texture(desc, &levels, Arrival::Background) {
                        Ok(t) => logo_texture = Some(t),
                        Err(ruthenium_gpu::Error::Upload(UploadError::Full)) => {}
                        Err(e) => {
                            eprintln!("[ruthenium] no logo: upload: {e}");
                            failed = true;
                        }
                    }
                }
                match (atlas_texture, logo_texture) {
                    _ if failed => Atlas::Failed,
                    (Some(texture), Some(logo)) => Atlas::Ready {
                        atlas,
                        texture,
                        logo,
                    },
                    _ => Atlas::Upload {
                        atlas,
                        logo,
                        atlas_texture,
                        logo_texture,
                    },
                }
            }
            other => other,
        };
        icon
    }

    /// This frame's GUI.
    /// `icon` receives the window icon (`ICON_SIZE` square RGBA) the frame it is ready.
    pub fn frame(
        &mut self,
        renderer: &mut Renderer,
        pixel_size: (u32, u32),
        window_size: (u32, u32),
        icon: impl FnOnce(&[u8]),
    ) -> GuiFrame<'_> {
        if let Some(rgba) = self.advance(renderer) {
            icon(&rgba);
        }
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| (now - t).as_secs_f32().min(MAX_STEP));
        self.last_frame = Some(now);
        let mouse = self.mouse;
        self.layout(pixel_size);
        let title = self.title.as_ref().unwrap();
        let hit = mouse.and_then(|(x, y)| {
            let (ux, uy) = title.scale.from_window(x, y, window_size, pixel_size);
            title.hit(ux, uy)
        });
        self.states.hover(hit);
        self.states.update(dt);
        let (atlas, texture, logo) = match &self.atlas {
            Atlas::Ready {
                atlas,
                texture,
                logo,
            } if renderer.texture_ready(*texture) && renderer.texture_ready(*logo) => {
                (Some(atlas), texture.index(), Some(logo.index()))
            }
            _ => (None, 0, None),
        };
        self.shapes.clear();
        self.text.clear();
        title.draw(&mut self.shapes, &mut self.text, atlas, logo, &self.states);
        GuiFrame {
            shapes: self.shapes.shapes(),
            glyphs: self.text.quads(),
            font_atlas: texture,
            ..GuiFrame::default()
        }
    }

    /// Whether the menus' text is showing (for screenshots).
    pub fn text_ready(&self) -> bool {
        matches!(self.atlas, Atlas::Ready { .. })
    }
}
