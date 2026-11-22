//! Ruthenium's out-of-game menus: the title screen, the server list and its forms. Lays
//! them out for the window, prepares the glyph atlas and the logo in the background and
//! uploads them, routes input to the screen showing, and builds each frame's shapes and
//! text.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_gpu::{Arrival, TextureDesc, TextureId, UploadError};
use ruthenium_net::server_list::{MAX_ADDRESS, MAX_NAME, PackStatus, ServerEntry};
use ruthenium_platform::Window;

/// Where copied text goes and pasted text comes from.
pub trait Clipboard {
    fn get(&self) -> Option<String>;
    fn set(&self, text: &str);
}

impl Clipboard for Window {
    fn get(&self) -> Option<String> {
        self.clipboard_text()
    }

    fn set(&self, text: &str) {
        self.set_clipboard_text(text);
    }
}
use ruthenium_render::{GuiFrame, Renderer};
use ruthenium_ui::logo::Logo;
use ruthenium_ui::modern::{
    Action, ButtonStates, Ease, Field, FormButton, FormScreen, FormSpec, Key, ShapeList,
    TitleScreen, UiScale, WEIGHTS,
};
use ruthenium_ui::text::{ATLAS_SIZE, Fonts, GlyphAtlas, TextList, TextStyle, Weight};

use crate::servers::{Servers, valid_address};

/// Side of the logo texture's top level.
const LOGO_SIZE: u32 = 512;
/// Side of the window icon.
pub const ICON_SIZE: u32 = 256;

/// SDL scancodes the menus use.
const SCANCODE_A: u32 = 4;
const SCANCODE_C: u32 = 6;
const SCANCODE_V: u32 = 25;
const SCANCODE_X: u32 = 27;
const SCANCODE_RETURN: u32 = 40;
const SCANCODE_ESCAPE: u32 = 41;
const SCANCODE_BACKSPACE: u32 = 42;
const SCANCODE_TAB: u32 = 43;
const SCANCODE_SPACE: u32 = 44;
const SCANCODE_F5: u32 = 62;
const SCANCODE_HOME: u32 = 74;
const SCANCODE_DELETE: u32 = 76;
const SCANCODE_END: u32 = 77;
const SCANCODE_RIGHT: u32 = 79;
const SCANCODE_LEFT: u32 = 80;
const SCANCODE_DOWN: u32 = 81;
const SCANCODE_UP: u32 = 82;
const SCANCODE_KP_ENTER: u32 = 88;
/// The longest step the menus' motion takes in one frame (after a stall or a hidden window).
const MAX_STEP: f32 = 0.1;
/// Time constant of the menus' fade-in once their text and logo are on the GPU.
const APPEAR_TAU: f32 = 0.09;

/// Every printable ASCII character: what the menus' text uses for now.
const ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

/// Vanilla's default name for a server added without one (`selectServer.defaultName`).
const DEFAULT_NAME: &str = "Minecraft Server";

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

/// Which form is showing, and what it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormKind {
    Add,
    Edit(usize),
    Direct,
    Delete(usize),
}

/// Form button indices.
const CONFIRM: usize = 0;
const CYCLE: usize = 0;

enum Screen {
    Title,
    Servers,
    Form(FormKind, Box<FormScreen>, PackStatus),
}

/// See the module docs.
pub struct Menu {
    fonts: Fonts,
    atlas: Atlas,
    scale: Option<UiScale>,
    title: Option<TitleScreen>,
    /// The title screen's buttons: pointer, focus and motion (kept across layouts).
    title_states: ButtonStates,
    screen: Screen,
    /// The server list, while it or one of its forms shows.
    servers: Option<Servers>,
    servers_path: Option<PathBuf>,
    /// The last address typed into Direct Connect (vanilla keeps it in options.txt).
    last_direct: String,
    shapes: ShapeList,
    text: TextList,
    /// Mouse position in window coordinates.
    mouse: Option<(f32, f32)>,
    started: Instant,
    last_frame: Option<Instant>,
    /// How far the menus have faded in: nothing shows until the text and logo can, so
    /// they never pop in after the pills.
    appear: Ease,
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

fn pack_label(p: PackStatus) -> String {
    let v = match p {
        PackStatus::Enabled => "Enabled",
        PackStatus::Disabled => "Disabled",
        PackStatus::Prompt => "Prompt",
    };
    format!("Server Resource Packs: {v}")
}

fn button(label: &str, action: Action) -> FormButton {
    FormButton {
        label: label.to_owned(),
        action,
    }
}

impl Menu {
    /// Starts building the glyph atlas on another thread. The server list lives in
    /// `servers_path` (none: not saved).
    pub fn new(servers_path: Option<PathBuf>) -> Menu {
        let handle = std::thread::Builder::new()
            .name("menu-assets".into())
            .spawn(prepare)
            .expect("spawning a thread");
        Menu {
            fonts: Fonts::new(&WEIGHTS),
            atlas: Atlas::Building(handle),
            scale: None,
            title: None,
            title_states: ButtonStates::new(0),
            screen: Screen::Title,
            servers: None,
            servers_path,
            last_direct: String::new(),
            shapes: ShapeList::new(),
            text: TextList::new(),
            mouse: None,
            started: Instant::now(),
            last_frame: None,
            appear: Ease::default(),
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
        self.title_states.cancel();
        if let Some(s) = &mut self.servers {
            s.screen.states.cancel();
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.states.cancel();
        }
    }

    /// Whether a text field has focus (the client turns text input on for it).
    pub fn wants_text_input(&self) -> bool {
        matches!(&self.screen, Screen::Form(_, form, _) if form.focused_field().is_some())
    }

    /// Lays every live screen out for the window, if its size changed.
    fn layout(&mut self, pixel_size: (u32, u32)) -> UiScale {
        let scale = UiScale::new(pixel_size.0, pixel_size.1);
        if self.scale == Some(scale) {
            return scale;
        }
        self.scale = Some(scale);
        let version = concat!("Ruthenium ", env!("CARGO_PKG_VERSION"));
        let title = TitleScreen::new(scale, &self.fonts, version);
        if title.buttons.len() != self.title_states.len() {
            self.title_states = ButtonStates::new(title.buttons.len());
        }
        self.title = Some(title);
        if let Some(s) = &mut self.servers {
            s.screen.layout(scale, &self.fonts);
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.layout(scale, &self.fonts);
        }
        scale
    }

    fn open_form(&mut self, kind: FormKind, scale: UiScale) {
        let servers = self.servers.as_ref();
        let entry = |i: usize| servers.and_then(|s| s.list.servers.get(i)).cloned();
        let name_field = |v: &str| Field {
            label: "Server Name".into(),
            value: v.into(),
            max: MAX_NAME,
            hint: DEFAULT_NAME.into(),
        };
        let addr_field = |v: &str| Field {
            label: "Server Address".into(),
            value: v.into(),
            max: MAX_ADDRESS,
            hint: "play.example.com".into(),
        };
        let (spec, pack) = match kind {
            FormKind::Add | FormKind::Edit(_) => {
                let e = match kind {
                    FormKind::Edit(i) => match entry(i) {
                        Some(e) => e,
                        None => return,
                    },
                    _ => ServerEntry::default(),
                };
                let spec = FormSpec {
                    title: if kind == FormKind::Add {
                        "Add Server"
                    } else {
                        "Edit Server"
                    }
                    .into(),
                    message: Vec::new(),
                    fields: vec![name_field(&e.name), addr_field(&e.ip)],
                    buttons: vec![
                        button(&pack_label(e.pack_status), Action::Cycle),
                        button("Done", Action::Confirm),
                        button("Cancel", Action::Cancel),
                    ],
                };
                (spec, e.pack_status)
            }
            FormKind::Direct => (
                FormSpec {
                    title: "Direct Connect".into(),
                    message: Vec::new(),
                    fields: vec![addr_field(&self.last_direct)],
                    buttons: vec![
                        button("Join Server", Action::Confirm),
                        button("Cancel", Action::Cancel),
                    ],
                },
                PackStatus::Prompt,
            ),
            FormKind::Delete(i) => {
                let Some(e) = entry(i) else { return };
                (
                    FormSpec {
                        title: "Delete Server?".into(),
                        message: vec![
                            format!("'{}' will be removed from your list.", e.name),
                            "This can't be undone.".into(),
                        ],
                        fields: Vec::new(),
                        buttons: vec![
                            button("Delete", Action::Confirm),
                            button("Cancel", Action::Cancel),
                        ],
                    },
                    PackStatus::Prompt,
                )
            }
        };
        let form = FormScreen::new(scale, &self.fonts, spec);
        self.screen = Screen::Form(kind, Box::new(form), pack);
        self.validate_form();
    }

    /// Enables Done / Join when the address is one vanilla would accept.
    fn validate_form(&mut self) {
        let Screen::Form(kind, form, _) = &mut self.screen else {
            return;
        };
        let addr = match kind {
            FormKind::Add | FormKind::Edit(_) => Some(1),
            FormKind::Direct => Some(0),
            FormKind::Delete(_) => None,
        };
        if let Some(f) = addr {
            let ok = valid_address(form.field(f).value());
            let confirm = form
                .spec
                .buttons
                .iter()
                .position(|b| b.action == Action::Confirm)
                .unwrap_or(CONFIRM);
            form.set_button_enabled(confirm, ok);
        }
    }

    /// Carries out `action`; returns it if the client must act on it (Quit).
    fn act(&mut self, action: Action, renderer: Option<&mut Renderer>) -> Option<Action> {
        let scale = self.scale?;
        match (&mut self.screen, action) {
            (Screen::Title, Action::Multiplayer) => {
                self.servers = Some(Servers::open(self.servers_path.clone(), scale, &self.fonts));
                self.screen = Screen::Servers;
            }
            (Screen::Title, Action::Quit) => return Some(Action::Quit),
            (Screen::Title, other) => eprintln!("[ruthenium] {other:?}: not there yet"),
            (Screen::Servers, Action::Back) => {
                // Without a renderer at hand (a key), the next frame frees the icons.
                if let Some(r) = renderer
                    && let Some(s) = self.servers.take()
                {
                    s.close(r);
                }
                self.screen = Screen::Title;
            }
            (Screen::Servers, Action::Join) => {
                if let Some((_, s)) = self.servers.as_ref().and_then(Servers::selected) {
                    eprintln!("[ruthenium] join {} ({}): not there yet", s.name, s.ip);
                }
            }
            (Screen::Servers, Action::DirectConnect) => self.open_form(FormKind::Direct, scale),
            (Screen::Servers, Action::AddServer) => self.open_form(FormKind::Add, scale),
            (Screen::Servers, Action::EditServer) => {
                if let Some((i, _)) = self.servers.as_ref().and_then(Servers::selected) {
                    self.open_form(FormKind::Edit(i), scale);
                }
            }
            (Screen::Servers, Action::DeleteServer) => {
                if let Some((i, _)) = self.servers.as_ref().and_then(Servers::selected) {
                    self.open_form(FormKind::Delete(i), scale);
                }
            }
            (Screen::Servers, Action::Refresh) => {
                if let Some(s) = &mut self.servers {
                    s.refresh();
                    s.sync_rows(&self.fonts);
                }
            }
            (Screen::Form(_, form, pack), Action::Cycle) => {
                *pack = match pack {
                    PackStatus::Prompt => PackStatus::Enabled,
                    PackStatus::Enabled => PackStatus::Disabled,
                    PackStatus::Disabled => PackStatus::Prompt,
                };
                form.set_button_label(CYCLE, &self.fonts, &pack_label(*pack));
            }
            (Screen::Form(kind, form, pack), Action::Confirm) => {
                let (kind, pack) = (*kind, *pack);
                let value = |i: usize| form.field(i).value().to_owned();
                match kind {
                    FormKind::Add | FormKind::Edit(_) => {
                        let name = value(0);
                        let mut e = ServerEntry::new(
                            if name.is_empty() { DEFAULT_NAME } else { &name },
                            &value(1),
                        );
                        e.pack_status = pack;
                        if let Some(s) = &mut self.servers {
                            match kind {
                                FormKind::Edit(i) => s.edit(i, e),
                                _ => s.add(e),
                            }
                            s.sync_rows(&self.fonts);
                        }
                    }
                    FormKind::Direct => {
                        let ip = value(0);
                        self.last_direct = ip.clone();
                        if let Some(s) = &mut self.servers {
                            let e = s.direct(&ip);
                            eprintln!("[ruthenium] join {} ({}): not there yet", e.name, e.ip);
                        }
                    }
                    FormKind::Delete(i) => {
                        if let Some(s) = &mut self.servers {
                            s.delete(i);
                            s.sync_rows(&self.fonts);
                        }
                    }
                }
                self.screen = Screen::Servers;
            }
            (Screen::Form(kind, form, _), Action::Cancel | Action::Back) => {
                if *kind == FormKind::Direct {
                    self.last_direct = form.field(0).value().to_owned();
                }
                self.screen = Screen::Servers;
            }
            (_, other) => eprintln!("[ruthenium] {other:?}: not there yet"),
        }
        None
    }

    /// A key went down or up: the action the client must take, if any.
    pub fn key(
        &mut self,
        scancode: u32,
        down: bool,
        repeat: bool,
        shift: bool,
        shortcut: bool,
        clipboard: &dyn Clipboard,
    ) -> Option<Action> {
        self.scale?;
        let nav = match scancode {
            SCANCODE_TAB if shift => Some(Key::Previous),
            SCANCODE_TAB => Some(Key::Next),
            _ => None,
        };
        let enter = matches!(scancode, SCANCODE_RETURN | SCANCODE_KP_ENTER);
        let action = match &mut self.screen {
            Screen::Title => {
                let key = match scancode {
                    SCANCODE_DOWN => Some(Key::Next),
                    SCANCODE_UP => Some(Key::Previous),
                    SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE if !repeat => {
                        Some(Key::Activate)
                    }
                    _ => nav,
                }?;
                let i = self.title_states.key(key, down)?;
                Some(self.title.as_ref()?.buttons.get(i)?.action)
            }
            Screen::Servers => {
                let s = self.servers.as_mut()?;
                let screen = &mut s.screen;
                match scancode {
                    SCANCODE_ESCAPE if down && !repeat => Some(Action::Back),
                    SCANCODE_F5 if down && !repeat => Some(Action::Refresh),
                    SCANCODE_DELETE if down && !repeat && screen.selected().is_some() => {
                        Some(Action::DeleteServer)
                    }
                    SCANCODE_UP | SCANCODE_DOWN if down => {
                        screen.move_selection(if scancode == SCANCODE_UP { -1 } else { 1 });
                        screen.states.set_focus(None, false);
                        None
                    }
                    _ if nav.is_some() => {
                        let i = screen.states.key(nav?, down)?;
                        Some(screen.buttons[i].action)
                    }
                    SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE if !repeat => {
                        if screen.states.focus_visible() && screen.states.focused().is_some() {
                            let i = screen.states.key(Key::Activate, down)?;
                            Some(screen.buttons[i].action)
                        } else if enter && down && screen.selected().is_some() {
                            Some(Action::Join)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            Screen::Form(_, form, _) => {
                let nf = form.field_count();
                let in_field = form.focused_field().is_some();
                match scancode {
                    SCANCODE_ESCAPE if down && !repeat => Some(Action::Cancel),
                    _ if nav.is_some()
                        || (matches!(scancode, SCANCODE_UP | SCANCODE_DOWN) && down) =>
                    {
                        let key = nav.unwrap_or(if scancode == SCANCODE_UP {
                            Key::Previous
                        } else {
                            Key::Next
                        });
                        let i = form.states.key(key, down)?;
                        Some(form.buttons[i.checked_sub(nf)?].action)
                    }
                    _ if enter && in_field => {
                        let confirm = form
                            .spec
                            .buttons
                            .iter()
                            .position(|b| b.action == Action::Confirm)?;
                        (down && !repeat && form.button_enabled(confirm)).then_some(Action::Confirm)
                    }
                    SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE
                        if !in_field && !repeat =>
                    {
                        let i = form.states.key(Key::Activate, down)?;
                        Some(form.buttons[i.checked_sub(nf)?].action)
                    }
                    _ if in_field && down => {
                        let fonts = &self.fonts;
                        match scancode {
                            SCANCODE_BACKSPACE => form.edit(fonts, |f| f.backspace(shortcut)),
                            SCANCODE_DELETE => form.edit(fonts, |f| f.delete(shortcut)),
                            SCANCODE_LEFT => form.edit(fonts, |f| f.step(false, shortcut, shift)),
                            SCANCODE_RIGHT => form.edit(fonts, |f| f.step(true, shortcut, shift)),
                            SCANCODE_HOME => form.edit(fonts, |f| f.home(shift)),
                            SCANCODE_END => form.edit(fonts, |f| f.end(shift)),
                            SCANCODE_A if shortcut => form.edit(fonts, |f| f.select_all()),
                            SCANCODE_C | SCANCODE_X if shortcut => {
                                let i = form.focused_field()?;
                                let sel = form.field(i).selected_text().to_owned();
                                if !sel.is_empty() {
                                    clipboard.set(&sel);
                                    if scancode == SCANCODE_X {
                                        form.edit(fonts, |f| f.insert(""));
                                    }
                                }
                            }
                            SCANCODE_V if shortcut => {
                                if let Some(clip) = clipboard.get() {
                                    form.edit(fonts, |f| f.insert(&clip));
                                }
                            }
                            _ => {}
                        }
                        None
                    }
                    _ => None,
                }
            }
        };
        self.validate_form();
        self.act(action?, None)
    }

    /// Typed text, for the focused field.
    pub fn text_input(&mut self, text: &str) {
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.edit(&self.fonts, |f| f.insert(text));
        }
        self.validate_form();
    }

    /// The wheel turned `notches` (positive: up).
    pub fn wheel(&mut self, notches: f32) {
        if let (Screen::Servers, Some(s)) = (&self.screen, &mut self.servers) {
            s.screen.wheel(notches);
        }
    }

    /// The left mouse button went down or up at `(x, y)` (window coordinates): the action
    /// the client must take, if any (buttons act on release).
    #[allow(clippy::too_many_arguments)]
    pub fn mouse_button(
        &mut self,
        down: bool,
        x: f32,
        y: f32,
        shift: bool,
        pixel_size: (u32, u32),
        window_size: (u32, u32),
        renderer: &mut Renderer,
    ) -> Option<Action> {
        self.mouse = Some((x, y));
        let scale = self.layout(pixel_size);
        let (ux, uy) = scale.from_window(x, y, window_size, pixel_size);
        let time = self.started.elapsed().as_secs_f64();
        let action = match &mut self.screen {
            Screen::Title => {
                let title = self.title.as_ref()?;
                let hit = title.hit(ux, uy);
                if down {
                    self.title_states.mouse_down(hit);
                    None
                } else {
                    let i = self.title_states.mouse_up(hit)?;
                    Some(title.buttons[i].action)
                }
            }
            Screen::Servers => {
                let screen = &mut self.servers.as_mut()?.screen;
                let hit = screen.hit_button(ux, uy);
                if down {
                    screen.states.mouse_down(hit);
                    (hit.is_none() && screen.click_row(ux, uy, time)).then_some(Action::Join)
                } else {
                    let i = screen.states.mouse_up(hit)?;
                    Some(screen.buttons[i].action)
                }
            }
            Screen::Form(_, form, _) => {
                if down {
                    form.mouse_down(ux, uy, shift);
                    None
                } else {
                    form.mouse_up(ux, uy)
                }
            }
        };
        self.act(action?, Some(renderer))
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
        let scale = self.layout(pixel_size);
        let pointer = self
            .mouse
            .map(|(x, y)| scale.from_window(x, y, window_size, pixel_size));
        // Leaving the server list (Escape) has no renderer at hand: free its icons now.
        if matches!(self.screen, Screen::Title)
            && let Some(s) = self.servers.take()
        {
            s.close(renderer);
        }
        if let Some(s) = &mut self.servers {
            s.poll();
            s.prepare(renderer, &self.fonts);
        }
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
        // Only the backdrop until the text and the logo can be drawn too.
        if atlas.is_none() && !matches!(self.atlas, Atlas::Failed) {
            return GuiFrame::default();
        }
        self.appear.step(1.0, dt, APPEAR_TAU, APPEAR_TAU);
        let time = self.started.elapsed().as_secs_f64();
        match &mut self.screen {
            Screen::Title => {
                let title = self.title.as_ref().unwrap();
                let hit = pointer.and_then(|(x, y)| title.hit(x, y));
                self.title_states.hover(hit);
                self.title_states.update(dt);
                title.draw(
                    &mut self.shapes,
                    &mut self.text,
                    atlas,
                    logo,
                    &self.title_states,
                );
            }
            Screen::Servers => {
                if let Some(s) = &mut self.servers {
                    s.screen.pointer(pointer);
                    s.screen.update(dt);
                    s.screen.draw(&mut self.shapes, &mut self.text, atlas, time);
                }
            }
            Screen::Form(_, form, _) => {
                let hit = pointer.and_then(|(x, y)| form.hit(x, y));
                form.states.hover(hit);
                form.update(dt);
                form.draw(&mut self.shapes, &mut self.text, atlas);
            }
        }
        if self.appear.value < 1.0 {
            self.shapes.fade_all(self.appear.value);
            self.text.fade_all(self.appear.value);
        }
        GuiFrame {
            shapes: self.shapes.shapes(),
            glyphs: self.text.quads(),
            font_atlas: texture,
            ..GuiFrame::default()
        }
    }

    /// Whether the menus have fully faded in (for screenshots).
    pub fn text_ready(&self) -> bool {
        self.appear.value >= 1.0
    }

    /// Development: opens the server list straight away (for screenshots).
    pub fn open_servers(&mut self, pixel_size: (u32, u32)) {
        let scale = self.layout(pixel_size);
        self.servers = Some(Servers::open(self.servers_path.clone(), scale, &self.fonts));
        self.screen = Screen::Servers;
    }

    /// Development: opens a form over the server list (for screenshots): "add",
    /// "direct" or "delete".
    pub fn open_form_named(&mut self, name: &str, pixel_size: (u32, u32)) {
        let scale = self.layout(pixel_size);
        let kind = match name {
            "add" => FormKind::Add,
            "direct" => FormKind::Direct,
            "delete" => FormKind::Delete(0),
            _ => return,
        };
        self.open_form(kind, scale);
    }

    /// Development: selects server `i` in the list.
    pub fn select_server(&mut self, i: usize) {
        if let Some(s) = &mut self.servers {
            s.screen.select(Some(i));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ruthenium_net::server_list::ServerList;

    use super::*;

    #[derive(Default)]
    struct FakeClipboard(RefCell<Option<String>>);

    impl Clipboard for FakeClipboard {
        fn get(&self) -> Option<String> {
            self.0.borrow().clone()
        }

        fn set(&self, text: &str) {
            *self.0.borrow_mut() = Some(text.to_owned());
        }
    }

    fn press(
        m: &mut Menu,
        code: u32,
        shift: bool,
        shortcut: bool,
        clip: &FakeClipboard,
    ) -> Option<Action> {
        let a = m.key(code, true, false, shift, shortcut, clip);
        let b = m.key(code, false, false, shift, shortcut, clip);
        a.or(b)
    }

    #[test]
    fn add_edit_and_delete_servers_from_the_keyboard() {
        let dir = std::env::temp_dir().join(format!("ruthenium-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("servers.dat");
        let mut list = ServerList::default();
        list.add(ServerEntry::new("One", "127.0.0.1:1"), false);
        list.save(&path).unwrap();
        let clip = FakeClipboard::default();

        let mut m = Menu::new(Some(path.clone()));
        m.layout((1600, 900));
        assert_eq!(m.act(Action::Multiplayer, None), None);
        assert!(matches!(m.screen, Screen::Servers));
        // Down selects the first server; Enter would join it.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        assert_eq!(m.servers.as_ref().unwrap().screen.selected(), Some(0));

        // Add Server: the name field has focus and takes text; V is just a letter.
        m.act(Action::AddServer, None);
        assert!(m.wants_text_input());
        m.text_input("Vanilla");
        // Done stays off until the address is valid.
        press(&mut m, SCANCODE_TAB, false, false, &clip);
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Form(..)), "no address yet");
        m.text_input("localhost:25570");
        // Copy and paste with the shortcut modifier.
        press(&mut m, SCANCODE_A, false, true, &clip);
        press(&mut m, SCANCODE_C, false, true, &clip);
        assert_eq!(clip.get().as_deref(), Some("localhost:25570"));
        press(&mut m, SCANCODE_END, false, false, &clip);
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Servers));
        let saved = ServerList::load(&path);
        assert_eq!(saved.servers.len(), 2);
        assert_eq!(saved.servers[1].name, "Vanilla");
        assert_eq!(saved.servers[1].ip, "localhost:25570");

        // Edit it: the resource pack choice cycles.
        m.servers.as_mut().unwrap().screen.select(Some(1));
        m.act(Action::EditServer, None);
        m.act(Action::Cycle, None);
        m.act(Action::Confirm, None);
        assert_eq!(
            ServerList::load(&path).servers[1].pack_status,
            PackStatus::Enabled
        );

        // Delete asks first; Escape cancels, confirming removes it.
        m.servers.as_mut().unwrap().screen.select(Some(0));
        press(&mut m, SCANCODE_DELETE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Form(FormKind::Delete(0), ..)));
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Servers));
        press(&mut m, SCANCODE_DELETE, false, false, &clip);
        m.act(Action::Confirm, None);
        let saved = ServerList::load(&path);
        assert_eq!(saved.servers.len(), 1);
        assert_eq!(saved.servers[0].name, "Vanilla");

        // Direct Connect keeps the address as a hidden server; Escape goes back.
        m.act(Action::DirectConnect, None);
        m.text_input("direct.example");
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert_eq!(ServerList::load(&path).hidden[0].ip, "direct.example");
        assert_eq!(m.last_direct, "direct.example");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Title));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
