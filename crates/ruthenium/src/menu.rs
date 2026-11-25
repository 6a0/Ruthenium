//! Ruthenium's out-of-game menus: the title screen, the server list and its forms, the
//! accounts and signing in, the settings, and joining a server (connecting, the code of
//! conduct, and why a session ended). Lays them out for the window, prepares the glyph atlas and the logo in the background and
//! uploads them, routes input to the screen showing, and builds each frame's shapes and
//! text.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_gpu::{Arrival, TextureDesc, TextureId};
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
    AccountView, AccountsScreen, Action, ButtonStates, Ease, Field, FormButton, FormScreen,
    FormSpec, Key, Presence, SettingsEvent, SettingsScreen, ShapeList, TitleScreen, UiScale,
    WEIGHTS,
};
use ruthenium_ui::text::{ATLAS_SIZE, Fonts, GlyphAtlas, TextList, TextStyle, Weight};

use ruthenium_ui::face::Face;

use ruthenium_net::connection::Stage as NetStage;
use ruthenium_sim::session::{Session, SessionEvent};

use crate::account::{AccountStatus, Accounts, Stage, failure_lines};
use crate::ingame::InGame;
use crate::join::Joiner;
use crate::route::RoutePlayer;
use crate::servers::{Servers, valid_address};
use crate::settings::{FRAME_STATS, FULLSCREEN, Settings, SettingsFile};

/// The characters a form's message line holds.
const MESSAGE_WIDTH: usize = 52;
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
/// Settings are saved once they have stayed put this long (a slider being dragged
/// changes many times), on leaving the Settings screen, and at exit.
const SAVE_AFTER: std::time::Duration = std::time::Duration::from_millis(500);

/// Letter pairs Nunito joins into one glyph (its `liga` feature), which the characters
/// one by one never produce.
const LIGATURES: &str = "ff fi fl ffi ffl";

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

/// The title's account chip for `status`.
fn chip(status: &AccountStatus) -> String {
    match status {
        AccountStatus::None => "Sign in".into(),
        AccountStatus::Checking(n) => format!("Signing in as {n}..."),
        AccountStatus::SignedIn {
            name,
            may_play_online: true,
        } => name.clone(),
        AccountStatus::SignedIn { name, .. } => format!("{name}: multiplayer is off"),
        AccountStatus::Failed { name, reason } => format!("{name}: {reason}"),
    }
}

/// The face of the account in use, on its way to the GPU.
#[derive(Debug, Default)]
enum FaceTexture {
    #[default]
    None,
    /// Waiting for room in the upload ring.
    Pending(Face),
    Ready(TextureId),
}

/// The Accounts screen's line under the account in use.
fn detail(status: &AccountStatus) -> (String, Presence) {
    match status {
        AccountStatus::None => ("Not signed in".into(), Presence::SignedOut),
        AccountStatus::Checking(_) => ("Signing in...".into(), Presence::Checking),
        AccountStatus::SignedIn {
            may_play_online: true,
            ..
        } => ("Signed in".into(), Presence::SignedIn),
        AccountStatus::SignedIn { .. } => (
            "Signed in; multiplayer is turned off for this account".into(),
            Presence::Problem,
        ),
        AccountStatus::Failed { reason, .. } => {
            let mut c = reason.chars();
            let line = c.next().map_or_else(String::new, |f| {
                f.to_uppercase().chain(c).collect::<String>()
            });
            (line, Presence::Problem)
        }
    }
}

/// A sign-in page's address as people type it: no scheme, no `www.`.
fn short_url(url: &str) -> &str {
    let u = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    u.strip_prefix("www.").unwrap_or(u).trim_end_matches('/')
}

/// The sign-in screen for `stage`.
fn sign_in_spec(stage: &Stage) -> FormSpec {
    let lines = |l: &[&str]| l.iter().map(|&s| s.to_owned()).collect::<Vec<_>>();
    let (title, message, code, buttons) = match stage {
        Stage::Starting { device: false } | Stage::Browser { .. } => (
            "Sign in with Microsoft",
            lines(&[
                "Continue in the browser window that opened.",
                "Ruthenium never sees your password.",
            ]),
            None,
            vec![
                button("Open Again", Action::OpenPage),
                button("Use a Code", Action::UseCode),
                button("Cancel", Action::Cancel),
            ],
        ),
        Stage::Starting { device: true } => (
            "Sign in with a code",
            lines(&["Getting a code from Microsoft..."]),
            None,
            vec![button("Cancel", Action::Cancel)],
        ),
        Stage::Device(c) => (
            "Sign in with a code",
            vec![
                format!("On any device, go to {}", short_url(&c.verification_uri)),
                "and enter this code:".into(),
            ],
            Some(c.user_code.clone()),
            vec![
                button("Open Page", Action::OpenPage),
                button("Copy Code", Action::CopyCode),
                button("Cancel", Action::Cancel),
            ],
        ),
        Stage::Failed { error, .. } => (
            "Couldn't sign in",
            failure_lines(error),
            None,
            vec![
                button("Try Again", Action::Confirm),
                button("Back", Action::Cancel),
            ],
        ),
    };
    FormSpec {
        title: title.into(),
        message,
        code,
        fields: Vec::new(),
        buttons,
    }
}

/// Which form is showing, and what it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormKind {
    Add,
    Edit(usize),
    Direct,
    Delete(usize),
    /// Signing in a new account (the form follows the sign-in's stage).
    SignIn,
    /// Confirming that account `i` (in the Accounts list) signs out.
    SignOut(usize),
    /// Joining a server: where the connection is, and Cancel.
    Connecting,
    /// The server's code of conduct, to accept or decline.
    CodeOfConduct,
    /// Configured, the level loading, and Disconnect.
    Playing,
    /// The Game Menu over the world: Back to Game and Disconnect.
    Paused,
    /// Why the session ended.
    Disconnected,
    /// The death screen: Respawn (Spectate World in hardcore) and Title Screen, working
    /// 20 ticks after it opened.
    Died,
    /// The death screen's Title Screen, confirmed (`DeathScreen.TitleConfirmScreen`).
    QuitConfirm,
    /// The bed screen (`InBedChatScreen`): Leave Bed, until the server wakes the player.
    InBed,
}

impl FormKind {
    /// The screen the form goes back to.
    fn parent(self) -> Screen {
        match self {
            FormKind::SignIn | FormKind::SignOut(_) => Screen::Accounts,
            _ => Screen::Servers,
        }
    }
}

/// The death screen's state, kept to show it again after the quit confirmation.
#[derive(Debug, Clone)]
struct DeathForm {
    title: &'static str,
    lines: Vec<String>,
    hardcore: bool,
    /// The buttons work (20 ticks after the screen opened).
    ready: bool,
    /// Respawn was pressed (`button.active = false`).
    respawn_pressed: bool,
}

/// Form button indices.
const CONFIRM: usize = 0;
const CYCLE: usize = 0;

enum Screen {
    Title,
    Servers,
    Accounts,
    Settings,
    Form(FormKind, Box<FormScreen>, PackStatus),
    /// In the world, no screen open: the mouse turns the camera.
    Game,
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
    /// The settings, and the file they're kept in.
    settings: SettingsFile,
    /// When a setting last changed, while that change isn't saved.
    settings_changed: Option<Instant>,
    /// The Settings screen, while it shows.
    settings_screen: Option<SettingsScreen>,
    accounts: Accounts,
    /// The Accounts screen, while it or one of its forms shows.
    accounts_screen: Option<AccountsScreen>,
    /// The accounts' revision the screens show.
    accounts_seen: Option<u64>,
    /// The sign-in stage the sign-in form shows.
    stage_shown: Option<Stage>,
    /// Development: a sign-in stage to show without signing in (screenshots).
    preview_stage: Option<Stage>,
    /// The players' faces, by skin address (`None`: Steve).
    faces: Vec<(Option<String>, FaceTexture)>,
    shapes: ShapeList,
    text: TextList,
    /// Mouse position in window coordinates.
    mouse: Option<(f32, f32)>,
    started: Instant,
    last_frame: Option<Instant>,
    /// How far the menus have faded in: nothing shows until the text and logo can, so
    /// they never pop in after the pills.
    appear: Ease,
    /// Starts sessions with servers (none: joining isn't possible, as in tests).
    joiner: Option<Joiner>,
    /// The session with a server, while there is one.
    session: Option<Session>,
    /// The server the session is with, as the list names it.
    server_name: String,
    /// The player left the session (Cancel, Disconnect): its end needs no screen.
    leaving: bool,
    /// The death screen while the player is dead: its texts and buttons as shown.
    death: Option<DeathForm>,
    /// Input in the world.
    game: InGame,
    /// The session the terrain's meshes are of ended or was replaced: the renderer
    /// forgets them in the next frame.
    forget_world: bool,
    /// Meshing threads to start with the world's assets.
    mesh_workers: usize,
    /// Whether the crosshair is drawn over the world, and the HUD at all.
    crosshair: bool,
    hud: bool,
    /// Whether the frame's corners darken in the dark (vanilla's Vignette option).
    vignette: bool,
    /// The route played once the session is in the world (`--route`).
    route: Option<RoutePlayer>,
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
            let style = TextStyle {
                size: 1.0,
                weight: w,
                tracking: 0.0,
            };
            let fonts = &fonts;
            [ASCII, LIGATURES]
                .into_iter()
                .flat_map(move |t| fonts.shape(t, style).glyphs)
                .map(move |(g, _, _)| (w, g))
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
    pub fn new(servers_path: Option<PathBuf>, accounts: Accounts, settings: SettingsFile) -> Menu {
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
            settings,
            settings_changed: None,
            settings_screen: None,
            accounts,
            accounts_screen: None,
            accounts_seen: None,
            stage_shown: None,
            preview_stage: None,
            faces: Vec::new(),
            shapes: ShapeList::new(),
            text: TextList::new(),
            mouse: None,
            started: Instant::now(),
            last_frame: None,
            appear: Ease::default(),
            joiner: None,
            session: None,
            server_name: String::new(),
            leaving: false,
            death: None,
            game: InGame::default(),
            forget_world: false,
            mesh_workers: default_mesh_workers(),
            crosshair: true,
            hud: true,
            vignette: true,
            route: None,
        }
    }

    /// Has `route` played by the session that is joined next: from then on input in
    /// the world is the route's, the mouse is not grabbed and nothing pauses the game.
    pub fn set_route(&mut self, route: RoutePlayer) {
        self.game.set_scripted();
        self.route = Some(route);
    }

    pub fn route(&self) -> Option<&RoutePlayer> {
        self.route.as_ref()
    }

    /// Threads that mesh the world's sections.
    pub fn mesh_workers(&self) -> usize {
        self.mesh_workers
    }

    /// How the handoff to the game thread and the queue of changed sections have fared,
    /// while there is a session.
    pub fn session_stats(
        &self,
    ) -> Option<(
        ruthenium_sim::handoff::HandoffStats,
        ruthenium_sim::scene::QueueStats,
    )> {
        let s = self.session.as_ref()?;
        Some((s.handoff_stats(), s.scene().queue_stats()))
    }

    /// Sets how many threads mesh the world's sections.
    pub fn set_mesh_workers(&mut self, workers: usize) {
        self.mesh_workers = workers.max(1);
    }

    /// Whether the crosshair is drawn over the world (it is unless said otherwise).
    pub fn set_crosshair(&mut self, on: bool) {
        self.crosshair = on;
    }

    /// Whether the HUD is drawn over the world (it is unless said otherwise).
    pub fn set_hud(&mut self, on: bool) {
        self.hud = on;
    }

    /// Whether the vignette is drawn (it is unless said otherwise).
    pub fn set_vignette(&mut self, on: bool) {
        self.vignette = on;
    }

    /// How this frame's HUD is to be drawn: over the world while no screen is open,
    /// unless it is switched off.
    pub fn hud_params(&self) -> Option<ruthenium_render::HudParams> {
        (self.hud && matches!(self.screen, Screen::Game)).then(|| ruthenium_render::HudParams {
            gui_scale: 0,
            vignette: self.vignette,
            options: ruthenium_ui::hud::HudOptions {
                crosshair: self.crosshair,
                ..Default::default()
            },
            millis: self.started.elapsed().as_millis() as i64,
        })
    }

    /// The world to draw this frame, if the session is in one.
    pub fn world_view(&self) -> Option<crate::ingame::WorldView> {
        crate::ingame::world_view(self.session.as_ref()?, Instant::now())
    }

    /// Hands the world's drawing assets to the renderer once they are built (they
    /// start building when the first session starts) and the skins it asks for, and has
    /// it forget the meshes of a session that is over.
    fn sync_world(&mut self, renderer: &mut Renderer) {
        if std::mem::take(&mut self.forget_world) {
            renderer.clear_world();
        }
        let Some(joiner) = self.joiner.as_mut().filter(|_| self.session.is_some()) else {
            return;
        };
        if renderer.has_world() {
            let wanted = renderer.take_skin_requests();
            let skins = joiner.skins();
            for source in wanted {
                skins.request(source);
            }
            while let Some((source, skin)) = skins.take() {
                renderer.skin_found(&source, skin);
            }
            return;
        }
        joiner.start_world_assets(renderer.gpu().caps().max_image_dimension_2d);
        if let Some(assets) = joiner.take_world_assets()
            && let Err(e) = renderer.load_world(assets, self.mesh_workers)
        {
            eprintln!("[ruthenium] the world is not drawn: {e}");
        }
    }

    /// Lets the menus join servers.
    pub fn set_joiner(&mut self, joiner: Joiner) {
        self.joiner = Some(joiner);
    }

    /// Shows a form about the session: `lines` wrapped to the form's width.
    fn session_form(
        &mut self,
        kind: FormKind,
        title: &str,
        lines: &[&str],
        buttons: Vec<FormButton>,
    ) {
        let Some(scale) = self.scale else { return };
        let message = lines
            .iter()
            .flat_map(|l| crate::join::wrap(l, MESSAGE_WIDTH))
            .collect();
        let spec = FormSpec {
            title: title.into(),
            message,
            code: None,
            fields: Vec::new(),
            buttons,
        };
        let form = FormScreen::new(scale, &self.fonts, spec);
        self.screen = Screen::Form(kind, Box::new(form), PackStatus::Prompt);
    }

    fn connecting_form(&mut self, stage: NetStage) {
        let title = self.server_name.clone();
        self.session_form(
            FormKind::Connecting,
            &title,
            &[crate::join::stage_text(stage)],
            vec![button("Cancel", Action::Cancel)],
        );
    }

    /// Starts joining `name` at `address`.
    fn start_join(&mut self, name: &str, address: &str) {
        self.server_name = name.to_owned();
        self.leaving = false;
        // A session still closing is dropped (and so disconnected) here.
        self.session = None;
        self.forget_world = true;
        let started = match &mut self.joiner {
            Some(j) => j.join(address, &self.accounts),
            None => Err("Joining servers isn't possible here".into()),
        };
        match started {
            Ok(s) => {
                eprintln!("[ruthenium] joining {name} ({address})");
                self.session = Some(s);
                self.connecting_form(NetStage::Connecting);
            }
            Err(e) => {
                eprintln!("[ruthenium] can't join {name}: {e}");
                self.session_form(
                    FormKind::Disconnected,
                    "Couldn't connect to the server",
                    &[&e],
                    vec![button("Back to Server List", Action::Confirm)],
                );
            }
        }
    }

    /// Into the world, no screen open (`setScreen(null)`, which grabs the mouse).
    fn enter_game(&mut self) {
        self.game.enter(self.session.as_ref());
        self.screen = Screen::Game;
    }

    /// The Game Menu (`pauseGame`): keys let go, the mouse freed.
    fn pause(&mut self) {
        self.game.release(self.session.as_ref());
        if let Some(s) = &self.session {
            s.pause();
        }
        let title = self.server_name.clone();
        self.session_form(
            FormKind::Paused,
            "Game Menu",
            &[&title],
            vec![
                button("Back to Game", Action::Confirm),
                button("Disconnect", Action::Cancel),
            ],
        );
    }

    /// Shows the death screen as it stands: its buttons work once it has been open 20
    /// ticks, Respawn not again once pressed.
    fn death_form(&mut self) {
        let Some(d) = self.death.clone() else { return };
        let lines: Vec<&str> = d.lines.iter().map(String::as_str).collect();
        let respawn = if d.hardcore {
            "Spectate World"
        } else {
            "Respawn"
        };
        self.session_form(
            FormKind::Died,
            d.title,
            &lines,
            vec![
                button(respawn, Action::Confirm),
                button("Title Screen", Action::Cancel),
            ],
        );
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.set_button_enabled(0, d.ready && !d.respawn_pressed);
            form.set_button_enabled(1, d.ready);
        }
    }

    /// The death screen's Respawn (`LocalPlayer.respawn`).
    fn press_respawn(&mut self) {
        if let Some(s) = &self.session {
            s.respawn();
        }
        if let Some(d) = &mut self.death {
            d.respawn_pressed = true;
        }
    }

    /// The game's part of the frame, every frame the client runs (also while the window
    /// is hidden): the mouse's turn, then the session's frame (`runTick`: packets, the
    /// frame's input, ticks, the turn). A route takes its step first; `terrain` tells
    /// it what is drawn when it has to know.
    pub fn game_frame(&mut self, terrain: &dyn Fn() -> Option<crate::route::Terrain>) {
        let in_world = matches!(self.screen, Screen::Game);
        if in_world {
            self.game
                .frame(&self.settings.settings.mouse, self.session.as_ref());
        }
        if let Some(s) = &self.session {
            if let Some(r) = &mut self.route {
                if r.running() && !in_world {
                    r.fail("a screen opened over the world while the route played".into());
                }
                r.step(s, s.view().as_ref(), terrain);
            }
            s.set_screen_open(!in_world);
            s.frame();
        }
    }

    /// Whether the mouse should be grabbed: in the world, the window focused, and the
    /// input not a route's.
    pub fn wants_mouse_grab(&self) -> bool {
        matches!(self.screen, Screen::Game) && self.game.focused() && self.route.is_none()
    }

    /// The window gained or lost focus.
    pub fn focus(&mut self, on: bool) {
        if !on {
            self.cancel();
        }
        self.game.set_focus(on, self.session.as_ref());
    }

    /// Leaves the session (Cancel, Disconnect, declining the code of conduct).
    fn leave(&mut self) {
        if let Some(s) = &self.session {
            s.disconnect();
        }
        self.death = None;
        self.leaving = true;
        self.screen = Screen::Servers;
    }

    /// Takes in the session's news and shows it.
    fn poll_session(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let events: Vec<SessionEvent> = session.poll().collect();
        let ours = matches!(
            self.screen,
            Screen::Form(
                FormKind::Connecting
                    | FormKind::CodeOfConduct
                    | FormKind::Playing
                    | FormKind::Paused,
                ..
            )
        );
        for e in events {
            match e {
                SessionEvent::Stage(s) if ours && !self.leaving => self.connecting_form(s),
                SessionEvent::CodeOfConduct(text) if !self.leaving => {
                    self.session_form(
                        FormKind::CodeOfConduct,
                        "Code of Conduct",
                        &[&text],
                        vec![
                            button("Accept", Action::Confirm),
                            button("Decline", Action::Cancel),
                        ],
                    );
                }
                SessionEvent::Playing if !self.leaving => {
                    let title = self.server_name.clone();
                    self.session_form(
                        FormKind::Playing,
                        &title,
                        &["Loading the world..."],
                        vec![button("Disconnect", Action::Cancel)],
                    );
                }
                SessionEvent::Loaded if !self.leaving => self.enter_game(),
                SessionEvent::Died {
                    cause,
                    hardcore,
                    score,
                } if !self.leaving => {
                    if let Some(r) = &mut self.route {
                        r.fail("the player died".into());
                    }
                    let lang = self.joiner.as_mut().map(|j| j.data());
                    let empty = std::collections::HashMap::new();
                    let lang = lang.as_ref().map_or(&empty, |d| &d.lang);
                    let mut lines = Vec::new();
                    if let Some(c) = &cause {
                        lines.push(crate::join::plain_text(c, lang));
                    }
                    lines.push(format!("Score: {score}"));
                    self.game.release(self.session.as_ref());
                    self.death = Some(DeathForm {
                        title: if hardcore { "Game Over!" } else { "You Died!" },
                        lines,
                        hardcore,
                        ready: false,
                        respawn_pressed: false,
                    });
                    self.death_form();
                }
                SessionEvent::InBed if !self.leaving => {
                    self.game.release(self.session.as_ref());
                    self.session_form(
                        FormKind::InBed,
                        "Sleeping",
                        &[],
                        vec![button("Leave Bed", Action::Confirm)],
                    );
                }
                SessionEvent::WokeUp => {
                    if matches!(self.screen, Screen::Form(FormKind::InBed, ..)) {
                        self.enter_game();
                    }
                }
                SessionEvent::RespawnReady => {
                    if let Some(d) = &mut self.death {
                        d.ready = true;
                    }
                    if matches!(self.screen, Screen::Form(FormKind::Died, ..)) {
                        self.death_form();
                    }
                }
                SessionEvent::Respawned if !self.leaving => {
                    // The level loading screen, until the player is back in the world.
                    self.death = None;
                    let title = self.server_name.clone();
                    self.session_form(
                        FormKind::Playing,
                        &title,
                        &["Loading the world..."],
                        vec![button("Disconnect", Action::Cancel)],
                    );
                }
                SessionEvent::Reconfiguring if !self.leaving => {
                    self.connecting_form(NetStage::Joining);
                }
                SessionEvent::Closed(c) => {
                    self.session = None;
                    self.forget_world = true;
                    if self.leaving || crate::join::left_on_purpose(&c) {
                        self.leaving = false;
                        if let Some(r) = &mut self.route {
                            r.fail("the session was left".into());
                        }
                    } else {
                        let lang = self.joiner.as_mut().map(|j| j.data());
                        let empty = std::collections::HashMap::new();
                        let (title, reason) =
                            crate::join::closed_text(&c, lang.as_ref().map_or(&empty, |d| &d.lang));
                        if let Some(r) = &mut self.route {
                            r.fail(format!("{title}: {reason}"));
                        }
                        self.session_form(
                            FormKind::Disconnected,
                            &title,
                            &[&reason],
                            vec![button("Back to Server List", Action::Confirm)],
                        );
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    /// The mouse moved to `(x, y)` (window coordinates), by `(dx, dy)`.
    pub fn mouse_moved(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
        self.mouse = Some((x, y));
        if matches!(self.screen, Screen::Game) {
            self.game.mouse_moved(dx, dy);
        }
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
        if let Some(a) = &mut self.accounts_screen {
            a.states.cancel();
        }
        if let Some(s) = &mut self.settings_screen {
            s.cancel();
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.states.cancel();
        }
    }

    /// The settings as they are now (the client applies them).
    pub fn settings(&self) -> &Settings {
        &self.settings.settings
    }

    /// Changes setting `i` (a Settings screen row) from outside the screen, which shows
    /// it.
    fn set_setting(&mut self, i: usize, value: usize) {
        let before = self.settings.settings.clone();
        self.settings.settings.set(i, value);
        if self.settings.settings != before {
            self.settings_changed = Some(Instant::now());
        }
        if let Some(s) = &mut self.settings_screen {
            s.set_value(i, self.settings.settings.get(i));
        }
    }

    /// F3: shows or hides the frame stats.
    pub fn toggle_frame_stats(&mut self) {
        self.set_setting(
            FRAME_STATS,
            usize::from(!self.settings.settings.frame_stats),
        );
    }

    /// F11: fullscreen on or off.
    pub fn toggle_fullscreen(&mut self) {
        self.set_fullscreen(!self.settings.settings.fullscreen);
    }

    /// The window went in or out of fullscreen (F11, or the window manager).
    pub fn set_fullscreen(&mut self, on: bool) {
        self.set_setting(FULLSCREEN, usize::from(on));
    }

    /// Saves the settings if they changed.
    pub fn save_settings(&mut self) {
        self.settings.save();
        self.settings_changed = None;
    }

    /// A change on the Settings screen, or leaving it.
    fn settings_event(&mut self, e: SettingsEvent) {
        match e {
            SettingsEvent::Changed { setting, value } => {
                self.settings.settings.set(setting, value);
                self.settings_changed = Some(Instant::now());
            }
            SettingsEvent::Done => {
                self.settings_screen = None;
                self.screen = Screen::Title;
                self.save_settings();
            }
        }
    }

    /// Opens the Settings screen.
    fn open_settings(&mut self, scale: UiScale) {
        let mut screen = SettingsScreen::new(scale, &self.fonts, self.settings.settings.sections());
        screen.set_reduce_motion(self.settings.settings.reduce_motion);
        self.settings_screen = Some(screen);
        self.screen = Screen::Settings;
    }

    /// Reduced motion for every screen alive.
    fn apply_motion(&mut self) {
        let still = self.settings.settings.reduce_motion;
        self.title_states.set_reduce_motion(still);
        if let Some(s) = &mut self.servers {
            s.screen.states.set_reduce_motion(still);
            s.screen.list.set_reduce_motion(still);
        }
        if let Some(a) = &mut self.accounts_screen {
            a.states.set_reduce_motion(still);
            a.list.set_reduce_motion(still);
        }
        if let Some(s) = &mut self.settings_screen {
            s.set_reduce_motion(still);
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.states.set_reduce_motion(still);
        }
    }

    /// Whether a text field has focus (the client turns text input on for it).
    pub fn wants_text_input(&self) -> bool {
        matches!(&self.screen, Screen::Form(_, form, _) if form.focused_field().is_some())
    }

    /// Lays every live screen out for the window, if its size changed.
    fn layout(&mut self, pixel_size: (u32, u32)) -> UiScale {
        let factor = self.settings.settings.menu_size.factor();
        let scale = UiScale::sized(pixel_size.0, pixel_size.1, factor);
        if self.scale == Some(scale) {
            return scale;
        }
        self.scale = Some(scale);
        let version = concat!("Ruthenium ", env!("CARGO_PKG_VERSION"));
        let title = TitleScreen::new(scale, &self.fonts, version);
        if title.controls() != self.title_states.len() {
            self.title_states = ButtonStates::new(title.controls());
        }
        self.title = Some(title);
        if let Some(s) = &mut self.servers {
            s.screen.layout(scale, &self.fonts);
        }
        if let Some(a) = &mut self.accounts_screen {
            a.layout(scale, &self.fonts);
        }
        if let Some(s) = &mut self.settings_screen {
            s.layout(scale, &self.fonts);
        }
        self.accounts_seen = None;
        self.sync_accounts();
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.layout(scale, &self.fonts);
        }
        scale
    }

    /// Takes in the accounts' news and brings the screens up to date with it: the title's
    /// chip and Multiplayer button, the Accounts list, and the sign-in form, which follows
    /// the sign-in's stage and gives way to the Accounts list when it ends.
    fn sync_accounts(&mut self) {
        self.accounts.poll();
        if self.accounts_seen == Some(self.accounts.revision()) {
            return;
        }
        self.accounts_seen = Some(self.accounts.revision());
        self.apply_account();
        if let Some(screen) = &mut self.accounts_screen {
            let status = self.accounts.status();
            let shown = detail(status);
            let views: Vec<AccountView> = self
                .accounts
                .list()
                .iter()
                .enumerate()
                .map(|(i, a)| AccountView {
                    name: &a.name,
                    current: i == 0,
                    detail: (i == 0).then_some((shown.0.as_str(), shown.1)),
                })
                .collect();
            screen.set_rows(&self.fonts, &views);
        }
        if let Screen::Form(FormKind::SignIn, ..) = self.screen {
            let stage = self
                .accounts
                .stage()
                .or(self.preview_stage.as_ref())
                .cloned();
            match stage {
                None => self.screen = Screen::Accounts,
                Some(s) if self.stage_shown.as_ref() != Some(&s) => {
                    if let Some(scale) = self.scale {
                        let form = FormScreen::new(scale, &self.fonts, sign_in_spec(&s));
                        self.screen =
                            Screen::Form(FormKind::SignIn, Box::new(form), PackStatus::Prompt);
                    }
                    self.stage_shown = Some(s);
                }
                Some(_) => {}
            }
        }
        if let Screen::Form(FormKind::SignIn, form, _) = &mut self.screen
            && matches!(self.stage_shown, Some(Stage::Starting { device: false }))
        {
            // Nothing to open again until the browser was sent somewhere.
            form.set_button_enabled(0, false);
        }
    }

    /// Opens the Accounts screen.
    fn open_accounts(&mut self, scale: UiScale) {
        self.accounts_screen = Some(AccountsScreen::new(scale, &self.fonts));
        self.screen = Screen::Accounts;
        self.accounts_seen = None;
        self.sync_accounts();
    }

    /// Shows the sign-in form for the sign-in just started.
    fn open_sign_in(&mut self) {
        self.stage_shown = None;
        if let Some(scale) = self.scale {
            let form = FormScreen::new(
                scale,
                &self.fonts,
                FormSpec {
                    title: String::new(),
                    message: Vec::new(),
                    code: None,
                    fields: Vec::new(),
                    buttons: Vec::new(),
                },
            );
            self.screen = Screen::Form(FormKind::SignIn, Box::new(form), PackStatus::Prompt);
        }
        self.accounts_seen = None;
        self.sync_accounts();
    }

    fn apply_account(&mut self) {
        let status = self.accounts.status();
        let line = chip(status);
        if let Some(t) = &mut self.title {
            t.set_account(&self.fonts, &line);
        }
        // Parental controls or a multiplayer ban turn Multiplayer off, as in vanilla.
        let allowed = !matches!(
            status,
            AccountStatus::SignedIn {
                may_play_online: false,
                ..
            }
        );
        if let Some(i) = self.title.as_ref().and_then(|t| {
            t.buttons
                .iter()
                .position(|b| b.action == Action::Multiplayer)
        }) {
            self.title_states.set_enabled(i, allowed);
        }
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
                    code: None,
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
                    code: None,
                    fields: vec![addr_field(&self.settings.settings.last_server)],
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
                        code: None,
                        fields: Vec::new(),
                        buttons: vec![
                            button("Delete", Action::Confirm),
                            button("Cancel", Action::Cancel),
                        ],
                    },
                    PackStatus::Prompt,
                )
            }
            FormKind::SignOut(i) => {
                let Some(a) = self.accounts.list().get(i) else {
                    return;
                };
                (
                    FormSpec {
                        title: "Remove Account?".into(),
                        message: vec![
                            format!("{} will be signed out of Ruthenium.", a.name),
                            "You can sign in again at any time.".into(),
                        ],
                        code: None,
                        fields: Vec::new(),
                        buttons: vec![
                            button("Remove", Action::Confirm),
                            button("Cancel", Action::Cancel),
                        ],
                    },
                    PackStatus::Prompt,
                )
            }
            FormKind::SignIn => return self.open_sign_in(),
            // Opened by the session as it goes.
            FormKind::Connecting
            | FormKind::CodeOfConduct
            | FormKind::Playing
            | FormKind::Paused
            | FormKind::Disconnected
            | FormKind::Died
            | FormKind::InBed
            | FormKind::QuitConfirm => return,
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
            FormKind::Delete(_)
            | FormKind::SignIn
            | FormKind::SignOut(_)
            | FormKind::Connecting
            | FormKind::CodeOfConduct
            | FormKind::Playing
            | FormKind::Paused
            | FormKind::Disconnected
            | FormKind::Died
            | FormKind::InBed
            | FormKind::QuitConfirm => None,
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
    fn act(
        &mut self,
        action: Action,
        renderer: Option<&mut Renderer>,
        clipboard: &dyn Clipboard,
    ) -> Option<Action> {
        let scale = self.scale?;
        match (&mut self.screen, action) {
            (Screen::Title, Action::Multiplayer) => {
                self.servers = Some(Servers::open(self.servers_path.clone(), scale, &self.fonts));
                self.screen = Screen::Servers;
            }
            (Screen::Title, Action::Quit) => return Some(Action::Quit),
            (Screen::Title, Action::Accounts) => self.open_accounts(scale),
            (Screen::Title, Action::Settings) => self.open_settings(scale),
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
                    let (name, ip) = (s.name.clone(), s.ip.clone());
                    self.start_join(&name, &ip);
                }
            }
            (
                Screen::Form(FormKind::Connecting | FormKind::Playing, ..),
                Action::Cancel | Action::Back,
            ) => self.leave(),
            (Screen::Form(FormKind::Paused, ..), Action::Confirm | Action::Back) => {
                self.enter_game();
            }
            (Screen::Form(FormKind::Paused, ..), Action::Cancel) => self.leave(),
            (Screen::Form(FormKind::CodeOfConduct, ..), Action::Confirm) => {
                if let Some(s) = &self.session {
                    s.answer_code_of_conduct(true);
                }
                self.connecting_form(NetStage::Joining);
            }
            (Screen::Form(FormKind::CodeOfConduct, ..), Action::Cancel | Action::Back) => {
                if let Some(s) = &self.session {
                    s.answer_code_of_conduct(false);
                }
                self.leave();
            }
            (Screen::Form(FormKind::Died, ..), Action::Confirm) => {
                if self
                    .death
                    .as_ref()
                    .is_some_and(|d| d.ready && !d.respawn_pressed)
                {
                    self.press_respawn();
                    self.death_form();
                }
            }
            // Title Screen: straight out in hardcore, else after asking.
            (Screen::Form(FormKind::Died, ..), Action::Cancel) => match &self.death {
                Some(d) if d.ready && d.hardcore => self.leave(),
                Some(d) if d.ready => self.session_form(
                    FormKind::QuitConfirm,
                    "Are you sure you want to quit?",
                    &[],
                    vec![
                        button("Title Screen", Action::Confirm),
                        button("Respawn", Action::Cancel),
                    ],
                ),
                _ => {}
            },
            // Leave Bed, and Escape (`onClose` sends the same): the screen stays until
            // the server wakes the player.
            (Screen::Form(FormKind::InBed, ..), Action::Confirm | Action::Back) => {
                if let Some(s) = &self.session {
                    s.leave_bed();
                }
            }
            // The death screen doesn't close on Escape.
            (Screen::Form(FormKind::Died, ..), Action::Back) => {}
            (Screen::Form(FormKind::QuitConfirm, ..), Action::Confirm) => self.leave(),
            (Screen::Form(FormKind::QuitConfirm, ..), Action::Cancel) => {
                self.press_respawn();
                self.death_form();
            }
            (Screen::Form(FormKind::QuitConfirm, ..), Action::Back) => self.death_form(),
            (Screen::Form(FormKind::Disconnected, ..), Action::Confirm | Action::Cancel) => {
                self.screen = Screen::Servers;
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
            (Screen::Accounts, Action::Back) => {
                self.accounts_screen = None;
                self.screen = Screen::Title;
            }
            (Screen::Accounts, Action::AddAccount) => {
                self.accounts.sign_in(false);
                self.open_sign_in();
            }
            (Screen::Accounts, Action::UseAccount) => {
                let selected = self
                    .accounts_screen
                    .as_ref()
                    .and_then(AccountsScreen::selected);
                if let Some(i) = selected.filter(|&i| i > 0)
                    && let Some(id) = self.accounts.list().get(i).map(|a| a.id.clone())
                {
                    self.accounts.use_account(&id);
                    // The account moves to the top: the selection follows it.
                    if let Some(a) = &mut self.accounts_screen {
                        a.select(Some(0));
                    }
                    self.sync_accounts();
                }
            }
            (Screen::Accounts, Action::SignOut) => {
                if let Some(i) = self
                    .accounts_screen
                    .as_ref()
                    .and_then(AccountsScreen::selected)
                {
                    self.open_form(FormKind::SignOut(i), scale);
                }
            }
            (Screen::Form(FormKind::SignIn, ..), Action::UseCode) => {
                self.accounts.sign_in(true);
                self.sync_accounts();
            }
            (Screen::Form(FormKind::SignIn, ..), Action::OpenPage) => {
                self.accounts.open_page_again()
            }
            (Screen::Form(FormKind::SignIn, ..), Action::CopyCode) => {
                if let Some(Stage::Device(c)) = &self.stage_shown {
                    clipboard.set(&c.user_code);
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
                        self.settings.settings.last_server = ip.clone();
                        self.settings_changed = Some(Instant::now());
                        if let Some(s) = &mut self.servers {
                            let e = s.direct(&ip);
                            let (name, ip) = (e.name.clone(), e.ip.clone());
                            self.start_join(&name, &ip);
                            return None;
                        }
                    }
                    FormKind::Delete(i) => {
                        if let Some(s) = &mut self.servers {
                            s.delete(i);
                            s.sync_rows(&self.fonts);
                        }
                    }
                    FormKind::SignOut(i) => {
                        if let Some(id) = self.accounts.list().get(i).map(|a| a.id.clone()) {
                            self.accounts.sign_out(&id);
                        }
                        if let Some(a) = &mut self.accounts_screen {
                            a.select(None);
                        }
                    }
                    // Handled above.
                    FormKind::Connecting
                    | FormKind::CodeOfConduct
                    | FormKind::Playing
                    | FormKind::Paused
                    | FormKind::Disconnected
                    | FormKind::Died
                    | FormKind::InBed
                    | FormKind::QuitConfirm => {}
                    // Try Again, the same way as before.
                    FormKind::SignIn => {
                        let device =
                            matches!(self.stage_shown, Some(Stage::Failed { device: true, .. }));
                        self.accounts.sign_in(device);
                        self.open_sign_in();
                        return None;
                    }
                }
                self.screen = kind.parent();
                self.sync_accounts();
            }
            (Screen::Form(kind, form, _), Action::Cancel | Action::Back) => {
                let kind = *kind;
                if kind == FormKind::Direct {
                    self.settings.settings.last_server = form.field(0).value().to_owned();
                    self.settings_changed = Some(Instant::now());
                }
                if kind == FormKind::SignIn {
                    self.accounts.cancel_sign_in();
                    self.stage_shown = None;
                }
                self.screen = kind.parent();
                self.sync_accounts();
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
        let escape = scancode == SCANCODE_ESCAPE && down && !repeat;
        match &self.screen {
            // Escape pauses (`pauseGame`) and closes the Game Menu again.
            Screen::Game if escape => {
                self.pause();
                return None;
            }
            Screen::Game => {
                self.game.key(scancode, down, self.session.as_ref());
                return None;
            }
            Screen::Form(FormKind::Paused, ..) if escape => {
                return self.act(Action::Back, None, clipboard);
            }
            _ => {}
        }
        let action = match &mut self.screen {
            // Handled above.
            Screen::Game => None,
            Screen::Settings => {
                let screen = self.settings_screen.as_mut()?;
                let e = match scancode {
                    SCANCODE_ESCAPE if down && !repeat => Some(SettingsEvent::Done),
                    SCANCODE_DOWN => screen.key(Key::Next, down),
                    SCANCODE_UP => screen.key(Key::Previous, down),
                    SCANCODE_LEFT | SCANCODE_RIGHT if down => {
                        screen.adjust(if scancode == SCANCODE_LEFT { -1 } else { 1 })
                    }
                    SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE if !repeat => {
                        screen.key(Key::Activate, down)
                    }
                    _ => screen.key(nav?, down),
                };
                self.settings_event(e?);
                return None;
            }
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
                self.title.as_ref()?.action(i)
            }
            Screen::Accounts => {
                let screen = self.accounts_screen.as_mut()?;
                match scancode {
                    SCANCODE_ESCAPE if down && !repeat => Some(Action::Back),
                    SCANCODE_DELETE if down && !repeat && screen.selected().is_some() => {
                        Some(Action::SignOut)
                    }
                    SCANCODE_UP | SCANCODE_DOWN if down => {
                        screen
                            .list
                            .move_selection(if scancode == SCANCODE_UP { -1 } else { 1 });
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
                        } else if enter && down && screen.selected().is_some_and(|i| i > 0) {
                            Some(Action::UseAccount)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
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
        self.act(action?, None, clipboard)
    }

    /// Typed text, for the focused field.
    pub fn text_input(&mut self, text: &str) {
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.edit(&self.fonts, |f| f.insert(text));
        }
        self.validate_form();
    }

    /// The wheel turned `notches` (positive: up); `raw` is SDL's `(x, y)` as it came.
    pub fn wheel(&mut self, notches: f32, raw: (f32, f32)) {
        match (&self.screen, &mut self.servers, &mut self.accounts_screen) {
            (Screen::Game, ..) => self.game.wheel(raw.0, raw.1, self.session.as_ref()),
            (Screen::Servers, Some(s), _) => s.screen.wheel(notches),
            (Screen::Accounts, _, Some(a)) => a.list.wheel(notches),
            (Screen::Settings, ..) => {
                if let Some(s) = &mut self.settings_screen {
                    s.wheel(notches);
                }
            }
            _ => {}
        }
    }

    /// Another mouse button (SDL's numbering) went down or up: in the world, the right
    /// one uses items.
    pub fn other_mouse_button(&mut self, button: u8, down: bool) {
        if matches!(self.screen, Screen::Game) {
            self.game.mouse_button(button, down, self.session.as_ref());
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
        clipboard: &dyn Clipboard,
    ) -> Option<Action> {
        self.mouse = Some((x, y));
        let scale = self.layout(pixel_size);
        let (ux, uy) = scale.from_window(x, y, window_size, pixel_size);
        let time = self.started.elapsed().as_secs_f64();
        let action = match &mut self.screen {
            Screen::Game => {
                self.game.mouse_button(1, down, self.session.as_ref());
                None
            }
            Screen::Settings => {
                let screen = self.settings_screen.as_mut()?;
                let e = if down {
                    screen.mouse_down(ux, uy)
                } else {
                    screen.mouse_up(ux, uy)
                };
                self.settings_event(e?);
                return None;
            }
            Screen::Title => {
                let title = self.title.as_ref()?;
                let hit = title.hit(ux, uy);
                if down {
                    self.title_states.mouse_down(hit);
                    None
                } else {
                    let i = self.title_states.mouse_up(hit)?;
                    title.action(i)
                }
            }
            Screen::Accounts => {
                let screen = self.accounts_screen.as_mut()?;
                let hit = screen.hit_button(ux, uy);
                if down {
                    screen.states.mouse_down(hit);
                    let double = hit.is_none() && screen.list.click_row(ux, uy, time);
                    // A double click on another account switches to it.
                    (double && screen.selected().is_some_and(|i| i > 0))
                        .then_some(Action::UseAccount)
                } else {
                    let i = screen.states.mouse_up(hit)?;
                    Some(screen.buttons[i].action)
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
        self.act(action?, Some(renderer), clipboard)
    }

    /// Takes newly loaded faces, frees the ones no account needs any more, and uploads
    /// the waiting ones; a replaced texture is destroyed.
    fn upload_faces(&mut self, renderer: &mut Renderer) {
        let free = |f: FaceTexture, renderer: &mut Renderer| {
            if let FaceTexture::Ready(t) = f {
                renderer.destroy_texture(t);
            }
        };
        for (skin, face) in self.accounts.take_faces() {
            let next = face.map_or(FaceTexture::None, FaceTexture::Pending);
            match self.faces.iter_mut().find(|(k, _)| *k == skin) {
                Some((_, f)) => free(std::mem::replace(f, next), renderer),
                None => self.faces.push((skin, next)),
            }
        }
        let keys = self.accounts.face_keys();
        for (_, f) in self.faces.extract_if(.., |(k, _)| !keys.contains(k)) {
            free(f, renderer);
        }
        for (_, f) in &mut self.faces {
            let FaceTexture::Pending(face) = f else {
                continue;
            };
            let desc = TextureDesc {
                mip_levels: face.levels.len() as u32,
                ..TextureDesc::rgba8(face.size, face.size)
            };
            let levels: Vec<&[u8]> = face.levels.iter().map(Vec::as_slice).collect();
            match renderer.create_texture(desc, &levels, Arrival::NextFrame) {
                Ok(t) => *f = FaceTexture::Ready(t),
                // No room this frame: the rest wait too.
                Err(e) if e.try_again() => break,
                Err(e) => {
                    eprintln!("[ruthenium] no player face: upload: {e}");
                    *f = FaceTexture::None;
                }
            }
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
                        Err(e) if e.try_again() => {}
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
                        Err(e) if e.try_again() => {}
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
        self.poll_session();
        self.sync_world(renderer);
        if matches!(self.screen, Screen::Game) && self.game.should_pause() {
            self.pause();
        }
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
        self.sync_accounts();
        self.upload_faces(renderer);
        self.apply_motion();
        let dragging = self
            .settings_screen
            .as_ref()
            .is_some_and(SettingsScreen::dragging);
        if !dragging
            && self
                .settings_changed
                .is_some_and(|t| t.elapsed() >= SAVE_AFTER)
        {
            self.save_settings();
        }
        let face_of = |skin: Option<&String>| {
            self.faces
                .iter()
                .find(|(k, _)| k.as_ref() == skin)
                .and_then(|(_, f)| match f {
                    FaceTexture::Ready(t) if renderer.texture_ready(*t) => Some(t.index()),
                    _ => None,
                })
        };
        let list = self.accounts.list();
        let face = face_of(list.first().and_then(|a| a.skin.as_ref()));
        let row_faces: Vec<Option<u32>> = list.iter().map(|a| face_of(a.skin.as_ref())).collect();
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
        let mut settings_ev = None;
        match &mut self.screen {
            Screen::Game => {
                let drawn = renderer.world_ready();
                let debug = self.settings.settings.frame_stats;
                // Over a drawn world the readout only shows with the statistics: its
                // text is not put together for nothing every frame.
                let session = self.session.as_ref().filter(|_| !drawn || debug);
                let view = session.and_then(Session::view);
                let world = session.and_then(crate::ingame::scene_line);
                let terrain = session
                    .and_then(|_| renderer.world_stats())
                    .map(|s| crate::ingame::terrain_lines(&s));
                crate::ingame::draw_hud(
                    crate::ingame::HudText {
                        view: view.as_ref(),
                        world: world.as_deref(),
                        terrain: terrain.as_deref(),
                        server: &self.server_name,
                        drawn,
                        debug,
                        // The HUD has its own once the world is drawn.
                        crosshair: self.crosshair && !(drawn && renderer.hud_stats().is_some()),
                    },
                    &scale,
                    &self.fonts,
                    atlas,
                    &mut self.shapes,
                    &mut self.text,
                );
            }
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
                    face,
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
            Screen::Settings => {
                if let Some(s) = &mut self.settings_screen {
                    settings_ev = s.pointer(pointer);
                    s.update(dt);
                    s.draw(&mut self.shapes, &mut self.text, atlas);
                }
            }
            Screen::Accounts => {
                if let Some(a) = &mut self.accounts_screen {
                    a.set_faces(row_faces);
                    a.pointer(pointer);
                    a.update(dt);
                    a.draw(&mut self.shapes, &mut self.text, atlas);
                }
            }
            Screen::Form(_, form, _) => {
                let hit = pointer.and_then(|(x, y)| form.hit(x, y));
                form.states.hover(hit);
                form.update(dt);
                form.draw(&mut self.shapes, &mut self.text, atlas);
            }
        }
        if let Some(e) = settings_ev {
            self.settings_event(e);
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
    /// Development: joins `address` straight away (`--join`).
    pub fn join_now(&mut self, address: &str, pixel_size: (u32, u32)) {
        self.layout(pixel_size);
        self.start_join(address, address);
    }

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

    /// Development: opens the Accounts screen, or one of its forms without signing in
    /// (for screenshots): "accounts", "sign-in", "code", "sign-in-failed", "sign-out".
    pub fn open_accounts_named(&mut self, name: &str, pixel_size: (u32, u32)) {
        let scale = self.layout(pixel_size);
        self.open_accounts(scale);
        self.preview_stage = match name {
            "sign-in" => Some(Stage::Browser {
                url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize".into(),
            }),
            "code" => Some(Stage::Device(ruthenium_auth::DeviceCode {
                user_code: "R7KX4QPM".into(),
                verification_uri: "https://www.microsoft.com/link".into(),
                message: String::new(),
                expires_in: 900,
            })),
            "sign-in-failed" => Some(Stage::Failed {
                error: ruthenium_auth::AuthError::NotApproved,
                device: false,
            }),
            "sign-out" => {
                self.open_form(FormKind::SignOut(0), scale);
                None
            }
            _ => None,
        };
        if self.preview_stage.is_some() {
            self.open_sign_in();
        }
    }

    /// Development: opens the Settings screen (for screenshots), with setting `focus`
    /// focused.
    pub fn open_settings_named(&mut self, pixel_size: (u32, u32), focus: Option<usize>) {
        let scale = self.layout(pixel_size);
        self.open_settings(scale);
        if let Some(s) = &mut self.settings_screen {
            s.focus(focus);
        }
    }

    /// Development: selects account `i` in the Accounts list.
    pub fn select_account(&mut self, i: usize) {
        if let Some(a) = &mut self.accounts_screen {
            a.select(Some(i));
        }
    }

    /// Development: selects server `i` in the list.
    pub fn select_server(&mut self, i: usize) {
        if let Some(s) = &mut self.servers {
            s.screen.select(Some(i));
        }
    }
}

/// How many threads mesh the world unless told otherwise: half of what the machine has
/// beyond the main thread and the game thread, four at most. The game thread's ticks
/// and the frames come first.
pub fn default_mesh_workers() -> usize {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    (threads.saturating_sub(2) / 2).clamp(1, 4)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use ruthenium_net::server_list::ServerList;

    use super::*;
    use crate::account::tests::accounts_in;

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

    /// Lets the accounts thread and the session work until `done` holds (or fails after
    /// a few seconds); the session runs a game frame each time round, as in the client.
    fn until(m: &mut Menu, done: impl Fn(&Menu) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            m.sync_accounts();
            m.game_frame(&|| None);
            m.poll_session();
            if done(m) {
                return;
            }
            assert!(
                std::time::Instant::now() < end,
                "timed out: {:?}",
                m.accounts
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ruthenium-menu-{name}-{}", std::process::id()))
    }

    fn names(m: &Menu) -> Vec<String> {
        m.accounts.list().iter().map(|a| a.name.clone()).collect()
    }

    fn form_kind(m: &Menu) -> Option<(FormKind, Vec<String>)> {
        match &m.screen {
            Screen::Form(k, f, _) => Some((*k, f.spec.message.clone())),
            _ => None,
        }
    }

    #[test]
    fn joining_shows_why_it_ends() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.start_join("Nowhere", "127.0.0.1:1");
        assert_eq!(
            form_kind(&m).map(|f| f.0),
            Some(FormKind::Disconnected),
            "no joiner"
        );
        m.act(Action::Confirm, None, &clip);
        assert!(matches!(m.screen, Screen::Servers));
        m.set_joiner(Joiner::new(None, None));
        // A port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        m.start_join("Local", &format!("127.0.0.1:{port}"));
        if cfg!(feature = "dev-offline") {
            // Development builds try, and say why it failed.
            assert_eq!(form_kind(&m).map(|f| f.0), Some(FormKind::Connecting));
            until(&mut m, |m| m.session.is_none());
            let (kind, message) = form_kind(&m).unwrap();
            assert_eq!(kind, FormKind::Disconnected);
            assert!(message.join(" ").contains("refused"), "{message:?}");
        } else {
            // Release builds need a signed-in account.
            let (kind, message) = form_kind(&m).unwrap();
            assert_eq!(kind, FormKind::Disconnected);
            assert!(message.join(" ").contains("Sign in"), "{message:?}");
        }
    }

    #[test]
    fn privileges_can_turn_multiplayer_off() {
        let dir = temp("privileges");
        let (accounts, _) = accounts_in(&dir, &["Child", "Alex"], None);
        let mut m = Menu::new(None, accounts, SettingsFile::transient());
        m.layout((1600, 900));
        until(&mut m, |m| {
            matches!(m.accounts.status(), AccountStatus::SignedIn { .. })
        });
        assert!(!m.title_states.enabled(0));
        // A new layout (window resize) keeps it off.
        m.layout((1280, 720));
        assert!(!m.title_states.enabled(0));
        m.accounts.use_account("id-Alex");
        until(
            &mut m,
            |m| matches!(m.accounts.status(), AccountStatus::SignedIn { name, .. } if name == "Alex"),
        );
        assert!(m.title_states.enabled(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accounts_sign_in_switch_and_sign_out() {
        let dir = temp("accounts");
        let (accounts, pages) = accounts_in(&dir, &["Alex"], Some(Ok("Notch")));
        let clip = FakeClipboard::default();
        let mut m = Menu::new(None, accounts, SettingsFile::transient());
        m.layout((1600, 900));
        // The account chip is the last control on the title: Tab reaches it.
        let chip = m.title.as_ref().unwrap().controls() - 1;
        for _ in 0..=chip {
            press(&mut m, SCANCODE_TAB, false, false, &clip);
        }
        assert_eq!(m.title_states.focused(), Some(chip));
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Accounts));

        // Add Account signs in in the browser, then comes back to the list.
        m.act(Action::AddAccount, None, &clip);
        assert!(matches!(m.screen, Screen::Form(FormKind::SignIn, ..)));
        until(&mut m, |m| matches!(m.screen, Screen::Accounts));
        assert_eq!(
            pages.lock().unwrap().as_slice(),
            ["https://login.example/authorize"]
        );
        assert_eq!(names(&m), ["Notch", "Alex"]);

        // Down twice selects Alex; Enter switches to it, and the selection follows.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert_eq!(names(&m), ["Alex", "Notch"]);
        assert_eq!(m.accounts_screen.as_ref().unwrap().selected(), Some(0));

        // Delete asks first; confirming signs Alex out and Notch takes over.
        press(&mut m, SCANCODE_DELETE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Form(FormKind::SignOut(0), ..)));
        m.act(Action::Confirm, None, &clip);
        assert!(matches!(m.screen, Screen::Accounts));
        until(
            &mut m,
            |m| matches!(m.accounts.status(), AccountStatus::SignedIn { name, .. } if name == "Notch"),
        );
        assert_eq!(names(&m), ["Notch"]);
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Title));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_in_with_a_code() {
        let dir = temp("code");
        let (accounts, _) = accounts_in(&dir, &[], None);
        let clip = FakeClipboard::default();
        let mut m = Menu::new(None, accounts, SettingsFile::transient());
        m.layout((1600, 900));
        m.act(Action::Accounts, None, &clip);
        m.act(Action::AddAccount, None, &clip);
        until(&mut m, |m| {
            matches!(m.stage_shown, Some(Stage::Browser { .. }))
        });
        m.act(Action::UseCode, None, &clip);
        until(&mut m, |m| matches!(m.stage_shown, Some(Stage::Device(_))));
        m.act(Action::CopyCode, None, &clip);
        assert_eq!(clip.get().as_deref(), Some("ABCD-1234"));
        // Escape gives the sign-in up.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Accounts));
        assert!(m.accounts.stage().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_sign_in_says_why_and_can_try_again() {
        let dir = temp("failed");
        let (accounts, _) =
            accounts_in(&dir, &[], Some(Err(ruthenium_auth::AuthError::NotApproved)));
        let clip = FakeClipboard::default();
        let mut m = Menu::new(None, accounts, SettingsFile::transient());
        m.layout((1600, 900));
        m.act(Action::Accounts, None, &clip);
        m.act(Action::AddAccount, None, &clip);
        until(&mut m, |m| {
            matches!(m.stage_shown, Some(Stage::Failed { .. }))
        });
        let Screen::Form(FormKind::SignIn, form, _) = &m.screen else {
            panic!("not the sign-in form");
        };
        assert_eq!(form.spec.title, "Couldn't sign in");
        assert!(form.spec.message[0].contains("approved"));
        // Try Again starts over (the fake now waits), Back gives up.
        m.act(Action::Confirm, None, &clip);
        until(&mut m, |m| {
            matches!(m.stage_shown, Some(Stage::Browser { .. }))
        });
        m.act(Action::Cancel, None, &clip);
        assert!(matches!(m.screen, Screen::Accounts));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_from_the_keyboard_apply_and_save() {
        let dir = temp("settings");
        let (accounts, _) = accounts_in(&dir, &[], None);
        let clip = FakeClipboard::default();
        let path = SettingsFile::path_in(&dir);
        let mut m = Menu::new(None, accounts, SettingsFile::load(path.clone()));
        let before = m.layout((1600, 900));
        // Down twice reaches Settings on the title.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Settings));
        // VSync off with Left, then the frame rate one step down from Unlimited.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_LEFT, false, false, &clip);
        assert!(!m.settings().vsync);
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_LEFT, false, false, &clip);
        assert_eq!(m.settings().max_fps, Some(crate::settings::FPS_MAX));
        // Menu Size: the next layout is larger.
        for _ in 0..3 {
            press(&mut m, SCANCODE_DOWN, false, false, &clip);
        }
        press(&mut m, SCANCODE_RIGHT, false, false, &clip);
        let after = m.layout((1600, 900));
        assert!(after.px > before.px, "{} -> {}", before.px, after.px);
        // F3 from anywhere shows on the screen too.
        m.toggle_frame_stats();
        let s = m.settings_screen.as_ref().unwrap();
        assert_eq!(s.value(crate::settings::FRAME_STATS), Some(1));
        assert!(!path.exists(), "saved only after a pause or on leaving");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Title));
        let saved = SettingsFile::load(path).settings;
        assert_eq!(&saved, m.settings());
        assert!(saved.frame_stats && !saved.vsync);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_death_screen_waits_then_respawns_or_asks_before_quitting() {
        let clip = FakeClipboard::default();
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.death = Some(DeathForm {
            title: "You Died!",
            lines: vec!["Steve fell from a high place".into(), "Score: 0".into()],
            hardcore: false,
            ready: false,
            respawn_pressed: false,
        });
        m.death_form();
        let buttons = |m: &Menu| match &m.screen {
            Screen::Form(FormKind::Died, f, _) => (f.button_enabled(0), f.button_enabled(1)),
            _ => panic!("not the death screen"),
        };
        // Nothing works for the first 20 ticks, and Escape never closes it.
        assert_eq!(buttons(&m), (false, false));
        m.act(Action::Confirm, None, &clip);
        assert!(!m.death.as_ref().unwrap().respawn_pressed);
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert_eq!(buttons(&m), (false, false));
        m.death.as_mut().unwrap().ready = true;
        m.death_form();
        assert_eq!(buttons(&m), (true, true));
        // Title Screen asks first; going back shows the death screen again.
        m.act(Action::Cancel, None, &clip);
        assert!(matches!(m.screen, Screen::Form(FormKind::QuitConfirm, ..)));
        m.act(Action::Back, None, &clip);
        assert_eq!(buttons(&m), (true, true));
        // Respawn works once.
        m.act(Action::Confirm, None, &clip);
        assert!(m.death.as_ref().unwrap().respawn_pressed);
        assert_eq!(buttons(&m), (false, true));
        // Quitting from the confirmation leaves.
        m.act(Action::Cancel, None, &clip);
        m.act(Action::Confirm, None, &clip);
        assert!(matches!(m.screen, Screen::Servers) && m.death.is_none());
    }

    #[test]
    fn device_page_addresses_read_short() {
        assert_eq!(
            short_url("https://www.microsoft.com/link"),
            "microsoft.com/link"
        );
        assert_eq!(short_url("http://example.com/"), "example.com");
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

        let mut m = Menu::new(
            Some(path.clone()),
            Accounts::disabled(),
            SettingsFile::transient(),
        );
        m.layout((1600, 900));
        assert_eq!(m.act(Action::Multiplayer, None, &clip), None);
        assert!(matches!(m.screen, Screen::Servers));
        // Down selects the first server; Enter would join it.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        assert_eq!(m.servers.as_ref().unwrap().screen.selected(), Some(0));

        // Add Server: the name field has focus and takes text; V is just a letter.
        m.act(Action::AddServer, None, &clip);
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
        m.act(Action::EditServer, None, &clip);
        m.act(Action::Cycle, None, &clip);
        m.act(Action::Confirm, None, &clip);
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
        m.act(Action::Confirm, None, &clip);
        let saved = ServerList::load(&path);
        assert_eq!(saved.servers.len(), 1);
        assert_eq!(saved.servers[0].name, "Vanilla");

        // Direct Connect keeps the address as a hidden server and joins it (here it
        // can't: no joiner); Escape goes back to the list, and from there to the title.
        m.act(Action::DirectConnect, None, &clip);
        m.text_input("direct.example");
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert_eq!(ServerList::load(&path).hidden[0].ip, "direct.example");
        assert_eq!(m.settings().last_server, "direct.example");
        assert!(matches!(m.screen, Screen::Form(FormKind::Disconnected, ..)));
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Servers));
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Title));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
