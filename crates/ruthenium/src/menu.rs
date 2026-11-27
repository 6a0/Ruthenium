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
    AccountView, AccountsScreen, Action, BedScreen, ButtonStates, Ease, Field, FormButton,
    FormScreen, FormSpec, Key, MenuTransition, PauseScreen, Presence, SettingsEvent,
    SettingsScreen, ShapeList, TitleScreen, UiScale, WEIGHTS,
};
use ruthenium_ui::scale::GuiScale;
use ruthenium_ui::text::{
    ATLAS_SIZE, FaceId, Fonts, GlyphAtlas, MAX_ATLAS_SIZE, Rendered, TextList, TextStyle,
};

use ruthenium_ui::face::Face;

use ruthenium_net::connection::Stage as NetStage;
use ruthenium_sim::session::{Session, SessionEvent, Transfer};

use crate::account::{AccountStatus, Accounts, Stage, failure_lines};
use crate::ingame::InGame;
use crate::join::Joiner;
use crate::route::RoutePlayer;
use crate::servers::{Selected, Servers, valid_address};
use crate::settings::{GAME, Group, RUTHENIUM, Row, Settings, SettingsFile, all_groups, rows_of};
use chat::{ChatHistory, ChatMethod, ChatOutcome, ChatScreen, Drafts, ExitReason};

mod chat;
mod container;
mod keys;
mod packs;
pub use packs::{prompt_lines, toast_lines};

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
const SCANCODE_PAGE_UP: u32 = 75;
const SCANCODE_DELETE: u32 = 76;
const SCANCODE_END: u32 = 77;
const SCANCODE_PAGE_DOWN: u32 = 78;
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

/// Glyphs on their way into the atlas: rendered on a thread, put in a copy of the atlas,
/// uploaded, and swapped in once the GPU has them.
#[derive(Debug, Default)]
enum AtlasWork {
    #[default]
    Idle,
    Rendering(JoinHandle<Rendered>),
    Uploading(GlyphAtlas, Option<TextureId>),
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
    /// The server's resource packs, to accept or decline (`PackConfirmScreen`).
    PackPrompt,
    /// Configured, the level loading, and Disconnect.
    Playing,
    /// Why the session ended.
    Disconnected,
    /// The death screen: Respawn (Spectate World in hardcore) and Title Screen, working
    /// 20 ticks after it opened.
    Died,
    /// The death screen's Title Screen, confirmed (`DeathScreen.TitleConfirmScreen`).
    QuitConfirm,
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
    /// The Game Menu over the world.
    Paused,
    /// The chat screen over the world.
    Chat(Box<ChatScreen>),
    /// A container screen over the world (the session runs it).
    Container,
    /// Respawn was pressed and the server answered: the level loads again under the
    /// death screen's veil as it goes, with nothing to click, until the player is in
    /// the world (vanilla's level loading screen, a split second).
    Respawning,
}

/// Which settings the Settings screen shows, and from where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsOf {
    /// All of them, from the title screen.
    Title,
    /// Ruthenium's, and the game's own, from the Game Menu.
    Ruthenium,
    Game,
}

impl SettingsOf {
    fn groups(self) -> Vec<Group> {
        match self {
            SettingsOf::Title => all_groups(),
            SettingsOf::Ruthenium => RUTHENIUM.to_vec(),
            SettingsOf::Game => GAME.to_vec(),
        }
    }
}

/// Development: what of the Game Menu to show for a screenshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShownMenu {
    /// The menu, so far come (0 to 1).
    Coming(f32),
    RutheniumOptions,
    /// MC Options, on this page.
    GameOptions(usize),
}

impl ShownMenu {
    pub fn parse(s: &str) -> Option<ShownMenu> {
        match s {
            "ruthenium" => Some(ShownMenu::RutheniumOptions),
            "mc" => Some(ShownMenu::GameOptions(0)),
            _ if s.starts_with("mc:") => {
                let page: usize = s["mc:".len()..].parse().ok()?;
                (page < GAME.len()).then_some(ShownMenu::GameOptions(page))
            }
            _ => {
                let share: f32 = s.parse().ok()?;
                (0.0..=1.0)
                    .contains(&share)
                    .then_some(ShownMenu::Coming(share))
            }
        }
    }
}

/// See the module docs.
pub struct Menu {
    fonts: Fonts,
    atlas: Atlas,
    /// Glyphs of text drawn since the atlas was made, on their way in.
    atlas_work: AtlasWork,
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
    /// Buttons pressed since the client last asked (each one clicks).
    clicks: u32,
    /// The Settings screen, while it shows.
    settings_screen: Option<SettingsScreen>,
    settings_of: SettingsOf,
    /// The Game Menu, while it or its options show and until it has gone; how it comes
    /// and goes.
    pause: Option<PauseScreen>,
    transition: MenuTransition,
    /// Development: shown as soon as the world is.
    shown_menu: Option<ShownMenu>,
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
    /// The server the session is with, as the list names it, and its address.
    server_name: String,
    server_address: String,
    /// The code of conduct showing: its hash, and whether to stop asking about it.
    conduct: Option<(i32, bool)>,
    /// The player left the session (Cancel, Disconnect): its end needs no screen.
    leaving: bool,
    /// The session followed a transfer (its failed login says so).
    transferred: bool,
    /// The death screen while the player is dead: its texts and buttons as shown.
    death: Option<DeathForm>,
    /// The session's bed screen is up (from [`SessionEvent::InBed`] until the player is
    /// out of bed, dead, respawned or gone): the bed screen shows whenever no other does.
    bed: bool,
    /// The bed screen's Leave Bed, once it has shown.
    bed_button: Option<BedScreen>,
    /// Input in the world.
    game: InGame,
    /// A container screen's input.
    container: container::ContainerInput,
    /// The session the terrain's meshes are of ended or was replaced: the renderer
    /// forgets them in the next frame.
    forget_world: bool,
    /// Meshing threads to start with the world's assets.
    mesh_workers: usize,
    /// Whether the crosshair is drawn over the world, and the HUD at all.
    crosshair: bool,
    hud: bool,
    /// The route played once the session is in the world (`--route`).
    route: Option<RoutePlayer>,
    /// The key bindings' rows and what the keyboard holds.
    keys: keys::KeysState,
    /// The messages sent, for the chat screen's Up and Down.
    chat_history: ChatHistory,
    /// The chat's draft, and a chat screen put away while a level loads.
    chat_drafts: Drafts,
    /// The fallback fonts' revision the text was shaped with: text is shaped again when a
    /// font arrives (Unifont, read meanwhile).
    fonts_revision: u32,
    /// The server's resource packs: the prompt, the toast, the reloads.
    packs: packs::PackState,
}

impl std::fmt::Debug for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Menu").finish_non_exhaustive()
    }
}

fn glyph_atlas() -> GlyphAtlas {
    let fonts = Fonts::new(&WEIGHTS);
    let mut atlas = GlyphAtlas::growing(ATLAS_SIZE, MAX_ATLAS_SIZE);
    // Every weight's glyphs in one batch, so they are generated in parallel.
    let glyphs: Vec<(FaceId, u16)> = WEIGHTS
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
                .map(|g| g.key())
        })
        .collect();
    if !atlas.add_many(&fonts, &glyphs) {
        eprintln!("[ruthenium] the glyph atlas is full; some menu text is missing");
    }
    atlas
}

/// Finds the system's fonts and reads which characters each has, off the main thread and
/// beside the atlas, for the first text that needs them.
fn warm_fallback() {
    let warmed = ruthenium_ui::text::fallback::warm();
    if warmed.families > 0 {
        eprintln!(
            "[ruthenium] menu fallback fonts: {} families' characters ({} KiB) read in {:.0} ms",
            warmed.families,
            warmed.cmap_bytes / 1024,
            warmed.ms
        );
    }
}

/// Reads the game's GNU Unifont, the menus' last fallback font, on a thread of its own:
/// from the game imported in `game`; without one (or with the fallback off), none.
pub fn load_unifont(game: Option<PathBuf>) {
    use ruthenium_ui::text::fallback;
    let Some(game) = game.filter(|_| fallback::mode() != fallback::Mode::Off) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("menu-unifont".into())
        .spawn(move || {
            let started = Instant::now();
            let Ok(resources) = crate::game_files::vanilla(&game) else {
                return;
            };
            let id = ruthenium_assets::Id::minecraft("include/unifont").expect("a valid id");
            let font = ruthenium_assets::Font::load(&resources, &id);
            for w in &font.warnings {
                eprintln!("[ruthenium] menu fallback: {w}");
            }
            if !font.has('A') {
                eprintln!("[ruthenium] menu fallback: no Unifont in the game files");
                return;
            }
            eprintln!(
                "[ruthenium] menu fallback: Unifont ({:.1} MiB) read in {:.0} ms",
                font.unihex_bytes() as f64 / (1024.0 * 1024.0),
                started.elapsed().as_secs_f64() * 1e3
            );
            fallback::set_unifont(font);
        });
    if let Err(e) = spawned {
        eprintln!("[ruthenium] no thread for Unifont: {e}");
    }
}

/// The glyph atlas and the logo, side by side.
fn prepare() -> Prepared {
    let started = Instant::now();
    // (Not waited for: the menus' text does not need it to show.)
    if let Err(e) = std::thread::Builder::new()
        .name("menu-fallback".into())
        .spawn(warm_fallback)
    {
        eprintln!("[ruthenium] no thread for the fallback fonts: {e}");
    }
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

/// The code of conduct's choice button (vanilla's "Do not notify again for this Code of
/// Conduct" box).
fn conduct_label(remember: bool) -> String {
    format!("Don't Ask Again: {}", if remember { "On" } else { "Off" })
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
        let keys = keys::KeysState::new(&settings.settings.keys);
        let mut game = InGame::default();
        game.set_binds(&settings.settings.keys);
        game.set_toggles(
            settings.settings.toggle_sneak,
            settings.settings.toggle_sprint,
        );
        Menu {
            fonts: Fonts::new(&WEIGHTS),
            atlas: Atlas::Building(handle),
            atlas_work: AtlasWork::Idle,
            scale: None,
            title: None,
            title_states: ButtonStates::new(0),
            screen: Screen::Title,
            servers: None,
            servers_path,
            settings,
            settings_changed: None,
            clicks: 0,
            settings_screen: None,
            settings_of: SettingsOf::Title,
            pause: None,
            transition: MenuTransition::default(),
            shown_menu: None,
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
            server_address: String::new(),
            conduct: None,
            leaving: false,
            transferred: false,
            death: None,
            bed: false,
            bed_button: None,
            game,
            container: container::ContainerInput::default(),
            forget_world: false,
            mesh_workers: default_mesh_workers(),
            crosshair: true,
            hud: true,
            route: None,
            keys,
            chat_history: ChatHistory::default(),
            chat_drafts: Drafts::default(),
            fonts_revision: ruthenium_ui::text::fallback::revision(),
            packs: packs::PackState::default(),
        }
    }

    /// Keeps the commands sent in `path` (`command_history.txt`), and has those it keeps.
    pub fn set_command_history(&mut self, path: Option<PathBuf>) {
        self.chat_history = ChatHistory::load(path);
    }

    /// Opens the chat screen (`openChatScreen`), with the draft if `method` brings it
    /// back: keys let go, the mouse freed.
    fn open_chat(&mut self, method: ChatMethod) {
        self.game.release(self.session.as_ref());
        let chat = self.chat_drafts.open(method, &self.chat_history);
        self.screen = Screen::Chat(Box::new(chat));
    }

    /// The chat screen, if it is open, is closing for `reason` (`removed`): the draft
    /// is kept or dropped.
    fn chat_removed(&mut self, reason: ExitReason) {
        if let Screen::Chat(chat) = &mut self.screen {
            let save = self.settings.settings.save_chat_drafts;
            self.chat_drafts.removed(chat, reason, save);
        }
    }

    /// The session's bed screen opened (`Gui.tick`: no screen, the player asleep): the bed
    /// screen over the world. Over another screen (one opened in a frame the game thread
    /// had not taken yet) it waits until that one closes to the world, as vanilla's next
    /// tick would open it then.
    fn in_bed(&mut self) {
        self.bed = true;
        if matches!(self.screen, Screen::Game) {
            self.game.release(self.session.as_ref());
            let chat = self.chat_drafts.open_in_bed(&self.chat_history);
            self.screen = Screen::Chat(Box::new(chat));
            self.bed_button = None;
        }
    }

    /// The player is out of bed (`Gui.tick`, `InBedChatScreen.onPlayerWokeUp`): text
    /// typed in the bed screen goes on in a plain chat screen (the draft goes); else it
    /// closes, the draft kept as it was (and a chat screen put away comes back).
    fn woke_up(&mut self) {
        self.bed = false;
        let Screen::Chat(chat) = &self.screen else {
            return;
        };
        if !chat.in_bed() {
            return;
        }
        match chat.woke_up() {
            Some(text) => {
                self.chat_removed(ExitReason::Done);
                let chat = ChatScreen::new(&text, &self.chat_history);
                self.screen = Screen::Chat(Box::new(chat));
            }
            None => {
                self.chat_removed(ExitReason::Interrupted);
                self.enter_game();
            }
        }
    }

    /// The level loading screen, until the player is in the world
    /// (`startWaitingForNewLevel`); an open chat screen is put away meanwhile.
    fn level_loading(&mut self) {
        eprintln!("[menu] the level loading form");
        self.preserve_chat();
        let title = self.server_name.clone();
        self.session_form(
            FormKind::Playing,
            &title,
            &["Loading the world..."],
            vec![button("Disconnect", Action::Cancel)],
        );
    }

    /// A level starts loading (`startWaitingForNewLevel`): an open chat screen is put
    /// away, to come back once the player is in the world again.
    fn preserve_chat(&mut self) {
        self.chat_removed(ExitReason::Interrupted);
        if let Screen::Chat(_) = self.screen
            && let Screen::Chat(chat) = std::mem::replace(&mut self.screen, Screen::Game)
        {
            self.chat_drafts.preserve(chat);
        }
    }

    /// A key on the chat screen (down or repeated).
    fn chat_key(&mut self, scancode: u32, shift: bool, shortcut: bool, clipboard: &dyn Clipboard) {
        let Screen::Chat(chat) = &mut self.screen else {
            return;
        };
        let bed = chat.in_bed();
        match chat.key(scancode, shift, shortcut, &self.chat_history, clipboard) {
            ChatOutcome::Stay => {}
            // The bed screen's Escape (`onClose`) asks to leave the bed; it stays.
            ChatOutcome::Close if bed => {
                if let Some(s) = &self.session {
                    s.leave_bed();
                }
            }
            ChatOutcome::Close => {
                self.chat_removed(ExitReason::Intentional);
                self.enter_game();
            }
            ChatOutcome::Send(text) => {
                if !bed {
                    self.chat_removed(ExitReason::Done);
                }
                // `handleChatInput`: kept as sent, unless nothing is left of it.
                let message = ruthenium_sim::chat::normalize(&text);
                if !message.is_empty() {
                    self.chat_history.add(&message);
                    if let Some(s) = &self.session {
                        s.send_chat(&message);
                    }
                }
                match &mut self.screen {
                    Screen::Chat(chat) if bed => chat.sent_in_bed(),
                    _ => self.enter_game(),
                }
            }
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

    /// Development: shows the Game Menu as soon as the world is there.
    pub fn show_menu(&mut self, menu: ShownMenu) {
        self.shown_menu = Some(menu);
    }

    /// Whether the player list shows without its key held (it does not unless said
    /// otherwise).
    pub fn set_player_list(&mut self, on: bool) {
        self.game.set_player_list(on);
    }

    /// How this frame's HUD is to be drawn: over the world, and under a screen that is
    /// open over it, unless it is switched off.
    pub fn hud_params(&self) -> Option<ruthenium_render::HudParams> {
        let game = &self.settings.settings.game;
        (self.hud && self.session.is_some()).then(|| ruthenium_render::HudParams {
            gui_scale: game.gui_scale,
            vignette: game.vignette,
            options: ruthenium_ui::hud::HudOptions {
                crosshair: self.crosshair,
                attack_indicator: game.attack_indicator,
            },
            millis: self.started.elapsed().as_millis() as i64,
            now: Instant::now(),
        })
    }

    /// The world to draw this frame, if the session is in one.
    pub fn world_view(&self) -> Option<crate::ingame::WorldView> {
        // (Not with the assets of a server whose session is over.)
        if self.packs.stale {
            return None;
        }
        let fov = self.settings.settings.game.fov as f32;
        crate::ingame::world_view(self.session.as_ref()?, Instant::now(), fov)
    }

    /// Hands the world's drawing assets to the renderer once they are built (they
    /// start building when the first session starts) and the skins it asks for, and has
    /// it forget the meshes of a session that is over.
    fn sync_world(&mut self, renderer: &mut Renderer) {
        if std::mem::take(&mut self.forget_world) {
            renderer.clear_world();
        }
        // The world's assets (vanilla's for the first world, then whatever a server's
        // packs make them) come with the reloads.
        self.sync_reloads(renderer);
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
        // (A chat screen is interrupted: its draft is kept.)
        self.chat_removed(ExitReason::Interrupted);
        // (Options that were open over the world.)
        if matches!(self.screen, Screen::Settings) {
            self.settings_screen = None;
            self.save_settings();
        }
        let message = lines
            .iter()
            .flat_map(|l| l.split('\n').map(str::to_owned))
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

    /// The server's code of conduct arrived: one accepted before and unchanged since is
    /// accepted without asking (`ClientConfigurationPacketListenerImpl.handleCodeOfConduct`);
    /// any other is shown.
    fn code_of_conduct_arrived(&mut self, text: &str) {
        let hash = crate::servers::java_hash(text);
        if self.code_of_conduct() == hash {
            if let Some(s) = &self.session {
                s.answer_code_of_conduct(true);
            }
            return;
        }
        self.conduct = Some((hash, false));
        self.session_form(
            FormKind::CodeOfConduct,
            "Code of Conduct",
            &[text],
            vec![
                button(&conduct_label(false), Action::Cycle),
                button("Accept", Action::Confirm),
                button("Decline", Action::Cancel),
            ],
        );
    }

    /// The code of conduct accepted for the server joined (`servers.dat`'s hash; 0 for
    /// none, and for a server not in the list).
    fn code_of_conduct(&self) -> i32 {
        let (name, ip) = (&self.server_name, &self.server_address);
        let mut list = match (&self.servers, &self.servers_path) {
            (Some(s), _) => s.list.clone(),
            (None, Some(p)) => ruthenium_net::server_list::ServerList::load(p),
            (None, None) => return 0,
        };
        crate::servers::entry_named(&mut list, name, ip).map_or(0, |e| e.accepted_code_of_conduct)
    }

    /// Keeps the answer to a code of conduct with the server joined, as
    /// `CodeOfConductScreen.onResult` does: its hash if it was accepted with "Don't Ask
    /// Again" on, else nothing.
    fn answered_code_of_conduct(&mut self, accepted: bool) {
        let Some((hash, remember)) = self.conduct.take() else {
            return;
        };
        let value = if accepted && remember { hash } else { 0 };
        let (name, ip) = (self.server_name.clone(), self.server_address.clone());
        if let Some(s) = &mut self.servers {
            if let Some(e) = crate::servers::entry_named(&mut s.list, &name, &ip) {
                e.accepted_code_of_conduct = value;
                s.save();
            }
        } else if let Some(p) = &self.servers_path {
            let mut list = ruthenium_net::server_list::ServerList::load(p);
            if let Some(e) = crate::servers::entry_named(&mut list, &name, &ip) {
                e.accepted_code_of_conduct = value;
                if let Err(e) = list.save(p) {
                    eprintln!("[ruthenium] couldn't save {}: {e}", p.display());
                }
            }
        }
    }

    /// Starts joining `name` at `address`.
    fn start_join(&mut self, name: &str, address: &str) {
        self.begin_session(name, address, None);
    }

    /// The server transferred the player (`handleTransfer`): the session ended, and the
    /// next one goes where the server said, from the same server list entry (vanilla's
    /// `ServerData` goes on, so its name, Server Resource Packs setting and accepted code
    /// of conduct stay the entry's), under "Transferring to new server...".
    fn start_transfer(&mut self, t: Transfer) {
        eprintln!("[ruthenium] {} transfers to {}", self.server_name, t.to);
        // As the end of a session (`Minecraft.disconnect`).
        self.bed = false;
        self.death = None;
        self.pause = None;
        let (name, address) = (self.server_name.clone(), self.server_address.clone());
        self.begin_session(&name, &address, Some(t));
    }

    /// Starts a session for the server list entry `name` at `address`: a join, or a
    /// transfer's.
    fn begin_session(&mut self, name: &str, address: &str, transfer: Option<Transfer>) {
        self.server_name = name.to_owned();
        self.server_address = address.to_owned();
        self.leaving = false;
        self.transferred = transfer.is_some();
        // A session still closing is dropped (and so disconnected) here; the resources
        // go back to vanilla's (its own request for that goes with it).
        if self.session.take().is_some() {
            self.packs_session_over();
        }
        self.forget_world = true;
        self.packs.session_number += 1;
        self.packs.prompt = None;
        let pack_status = self.entry_pack_status(name, address);
        let transferred = self.transferred;
        let started = match (&mut self.joiner, transfer) {
            (Some(j), None) => j.join(address, &self.accounts, pack_status),
            (Some(j), Some(t)) => j.join_transfer(t, &self.accounts, pack_status),
            (None, _) => Err("Joining servers isn't possible here".into()),
        };
        match started {
            Ok(s) => {
                if !transferred {
                    eprintln!("[ruthenium] joining {name} ({address})");
                }
                self.session = Some(s);
                self.connecting_form(if transferred {
                    NetStage::Transferring
                } else {
                    NetStage::Connecting
                });
            }
            Err(e) => {
                eprintln!("[ruthenium] can't join {name}: {e}");
                self.session_form(
                    FormKind::Disconnected,
                    if transferred {
                        "Connection failed while transferring to the server"
                    } else {
                        "Couldn't connect to the server"
                    },
                    &[&e],
                    vec![button("Back to Server List", Action::Confirm)],
                );
            }
        }
    }

    /// Into the world, no screen open (`setScreen(null)`, which grabs the mouse); or the
    /// chat screen put away while a level loaded, back (`restoreChatScreen`), unless the
    /// player is dead; or else, while the player is in bed, the bed screen (as the next
    /// `Gui.tick` opens it).
    fn enter_game(&mut self) {
        let alive = self.death.is_none();
        let mut chat = if alive {
            self.chat_drafts.restore(&self.chat_history)
        } else {
            None
        };
        if chat.is_none() && alive && self.bed {
            chat = Some(Box::new(self.chat_drafts.open_in_bed(&self.chat_history)));
            self.bed_button = None;
        }
        if chat.is_some() {
            self.game.release(self.session.as_ref());
        } else {
            self.game
                .enter(self.session.as_ref(), self.keys.physical().into_iter());
        }
        if matches!(self.screen, Screen::Settings) {
            self.settings_screen = None;
            self.save_settings();
        }
        match (&self.screen, &mut self.pause) {
            // The Game Menu goes as it came.
            (Screen::Paused, Some(p)) => {
                p.states.cancel();
                p.states.hover(None);
                p.states.set_focus(None, false);
            }
            _ => self.pause = None,
        }
        self.screen = chat.map_or(Screen::Game, Screen::Chat);
        // A bed screen put away while a level loaded, back with the player out of bed:
        // it goes as on waking (`Gui.tick`).
        if !self.bed && matches!(&self.screen, Screen::Chat(c) if c.in_bed()) {
            self.woke_up();
        }
    }

    /// Whether the Game Menu or its options are open.
    fn game_menu_open(&self) -> bool {
        match self.screen {
            Screen::Paused => true,
            Screen::Settings => self.settings_of != SettingsOf::Title,
            _ => false,
        }
    }

    /// Whether the death screen (or its quit confirmation) is over the world: the world
    /// behind it is veiled as behind the Game Menu.
    fn death_veiled(&self) -> bool {
        self.death.is_some()
            && matches!(
                self.screen,
                Screen::Form(FormKind::Died | FormKind::QuitConfirm, ..)
            )
    }

    /// The Game Menu (`pauseGame`): keys let go, the mouse freed.
    fn pause(&mut self) {
        self.game.release(self.session.as_ref());
        if let Some(s) = &self.session {
            s.pause();
        }
        let Some(scale) = self.scale else { return };
        self.pause = Some(PauseScreen::new(scale, &self.fonts));
        self.screen = Screen::Paused;
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

    /// The server respawned the player (`handleRespawn`): the level loading screen until
    /// the player is back in the world. After the death screen that is the world under
    /// the veil as it goes, with nothing to click (the connecting form, titled with the
    /// server's name, flashed for a split second there); else (another dimension) the
    /// form.
    fn respawned(&mut self) {
        // (`LocalPlayer.respawn` ends toggled keys, before the screen.)
        self.game.respawned(self.session.as_ref());
        let died = self.death.take().is_some();
        self.bed = false;
        if died {
            self.preserve_chat();
            self.screen = Screen::Respawning;
            eprintln!("[menu] respawned: the world under the veil until it loads");
        } else {
            self.level_loading();
        }
    }

    /// The death screen's Respawn (`LocalPlayer.respawn`).
    fn press_respawn(&mut self) {
        if let Some(s) = &self.session {
            s.respawn();
        }
        self.game.respawned(self.session.as_ref());
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
        self.container_game_frame();
        if let Some(s) = &self.session {
            if let Some(r) = &mut self.route {
                if r.running() && !in_world {
                    r.fail("a screen opened over the world while the route played".into());
                }
                r.step(s, s.view().as_ref(), terrain);
            }
            s.set_screen_open(!in_world);
            s.set_fov_effects(self.settings.settings.game.fov_effects as f32 / 100.0);
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
            self.keys_focus_lost();
        }
        self.game.set_focus(on, self.session.as_ref());
    }

    /// Leaves the session (Cancel, Disconnect, declining the code of conduct).
    fn leave(&mut self) {
        if let Some(s) = &self.session {
            s.disconnect();
        }
        // Drop the server's language, sounds, toast, and reloads right away instead of
        // waiting for the connection thread's Closed event.
        self.packs_session_over();
        self.death = None;
        self.bed = false;
        self.leaving = true;
        self.pause = None;
        self.transition.end();
        self.screen = Screen::Servers;
    }

    /// Takes in the session's news and shows it.
    fn poll_session(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let events: Vec<SessionEvent> = session.poll().collect();
        // Coalesce PackPrompt bursts in a single frame so rapid pushes cannot freeze UI
        // text shaping or repeatedly churn FormScreen layout.
        let last_prompt = events
            .iter()
            .rposition(|e| matches!(e, SessionEvent::PackPrompt(_)));
        let ours = self.game_menu_open()
            || matches!(
                self.screen,
                Screen::Form(
                    FormKind::Connecting | FormKind::CodeOfConduct | FormKind::Playing,
                    ..
                )
            );
        for (idx, e) in events.into_iter().enumerate() {
            match e {
                SessionEvent::Stage(s) if ours && !self.leaving => self.connecting_form(s),
                SessionEvent::CodeOfConduct(text) if !self.leaving => {
                    self.code_of_conduct_arrived(&text);
                }
                SessionEvent::Playing if !self.leaving => self.level_loading(),
                SessionEvent::Loaded if !self.leaving => self.enter_game(),
                SessionEvent::Died {
                    cause,
                    hardcore,
                    score,
                } if !self.leaving => {
                    if let Some(r) = &mut self.route {
                        r.fail("the player died".into());
                    }
                    let lang = self.joiner.as_mut().map(|j| j.server_lang());
                    let empty = std::collections::HashMap::new();
                    let lang = lang.as_deref().unwrap_or(&empty);
                    let mut lines = Vec::new();
                    if let Some(c) = &cause {
                        lines.push(crate::join::plain_text(c, lang));
                    }
                    lines.push(format!("Score: {score}"));
                    self.game.release(self.session.as_ref());
                    // (It takes the bed screen's place.)
                    self.bed = false;
                    self.death = Some(DeathForm {
                        title: if hardcore { "Game Over!" } else { "You Died!" },
                        lines,
                        hardcore,
                        ready: false,
                        respawn_pressed: false,
                    });
                    self.death_form();
                }
                SessionEvent::PackPrompt(p) if !self.leaving => {
                    if last_prompt == Some(idx) {
                        self.pack_prompt_arrived(p);
                    }
                }
                SessionEvent::PackStatus(s) => self.pack_status_changed(s),
                SessionEvent::PackToast(t) => {
                    self.packs.toast = match t {
                        ruthenium_sim::session::PackToast::Hide => None,
                        t => Some((t, Instant::now())),
                    };
                }
                // (Also once the session is over: the client goes back to vanilla's.)
                SessionEvent::ReloadResources(r) => self.reload_asked(r),
                SessionEvent::ContainerOpened if !self.leaving => self.container_opened(),
                SessionEvent::ContainerClosed => self.container_closed(),
                SessionEvent::InBed if !self.leaving => self.in_bed(),
                SessionEvent::WokeUp => self.woke_up(),
                SessionEvent::RespawnReady => {
                    if let Some(d) = &mut self.death {
                        d.ready = true;
                    }
                    if matches!(self.screen, Screen::Form(FormKind::Died, ..)) {
                        self.death_form();
                    }
                }
                SessionEvent::Respawned if !self.leaving => self.respawned(),
                SessionEvent::Reconfiguring if !self.leaving => {
                    self.connecting_form(NetStage::Joining);
                }
                // (The session's Closed follows: the next session replaces it.)
                SessionEvent::Transfer(t) if !self.leaving => {
                    self.start_transfer(t);
                    return;
                }
                SessionEvent::Closed(c) => {
                    self.session = None;
                    self.bed = false;
                    self.death = None;
                    self.forget_world = true;
                    // (Before its reason is put in words: in vanilla's language.)
                    self.packs_session_over();
                    if self.leaving || crate::join::left_on_purpose(&c) {
                        self.leaving = false;
                        // Exit any session-bound screen (including PackPrompt) so closing
                        // via Local::Closed can never strand the player on an orphaned form.
                        let session_bound = self.game_menu_open()
                            || matches!(
                                self.screen,
                                Screen::Game
                                    | Screen::Chat(_)
                                    | Screen::Container
                                    | Screen::Paused
                                    | Screen::Form(
                                        FormKind::Connecting
                                            | FormKind::Playing
                                            | FormKind::CodeOfConduct
                                            | FormKind::PackPrompt
                                            | FormKind::Died
                                            | FormKind::QuitConfirm,
                                        ..
                                    )
                            );
                        if session_bound {
                            self.settings_screen = None;
                            self.screen = Screen::Servers;
                        }
                        if let Some(r) = &mut self.route {
                            r.fail("the session was left".into());
                        }
                    } else {
                        let lang = self.joiner.as_mut().map(|j| j.server_lang());
                        let empty = std::collections::HashMap::new();
                        let (title, reason) = crate::join::closed_text(
                            &c,
                            lang.as_deref().unwrap_or(&empty),
                            self.transferred,
                        );
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
        if matches!(self.screen, Screen::Container) {
            self.container_moved();
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
        if let Some(p) = &mut self.pause {
            p.states.cancel();
        }
        if let Some(b) = &mut self.bed_button {
            b.states.cancel();
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.states.cancel();
        }
    }

    /// How many buttons were pressed since the last call: each plays the click.
    pub fn take_clicks(&mut self) -> u32 {
        std::mem::take(&mut self.clicks)
    }

    /// The settings as they are now (the client applies them).
    pub fn settings(&self) -> &Settings {
        &self.settings.settings
    }

    /// Changes a setting from outside the settings screen, which shows it if it has it.
    fn set_setting(&mut self, row: Row, value: usize) {
        let before = self.settings.settings.clone();
        self.settings.settings.set(row, value);
        if self.settings.settings != before {
            self.settings_changed = Some(Instant::now());
        }
        if let Some(s) = &mut self.settings_screen {
            let rows = rows_of(&self.settings_of.groups());
            if let Some(i) = rows.iter().position(|r| *r == row) {
                s.set_value(i, self.settings.settings.get(row));
            }
        }
    }

    /// F3: shows or hides the frame stats.
    pub fn toggle_frame_stats(&mut self) {
        self.set_setting(
            Row::FrameStats,
            usize::from(!self.settings.settings.frame_stats),
        );
    }

    /// F11: fullscreen on or off.
    pub fn toggle_fullscreen(&mut self) {
        self.set_fullscreen(!self.settings.settings.fullscreen);
    }

    /// The window went in or out of fullscreen (F11, or the window manager).
    pub fn set_fullscreen(&mut self, on: bool) {
        self.set_setting(Row::Fullscreen, usize::from(on));
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
                if let Some(row) = rows_of(&self.settings_of.groups()).get(setting) {
                    self.settings.settings.set(*row, value);
                    self.settings_changed = Some(Instant::now());
                    let s = &self.settings.settings;
                    self.game.set_toggles(s.toggle_sneak, s.toggle_sprint);
                }
            }
            SettingsEvent::Pressed { .. } | SettingsEvent::Reset { .. } => self.key_row_event(e),
            SettingsEvent::Done => {
                self.settings_screen = None;
                self.screen = if self.settings_of == SettingsOf::Title {
                    Screen::Title
                } else {
                    self.transition.change_screen();
                    Screen::Paused
                };
                self.save_settings();
            }
        }
    }

    /// Opens the Settings screen.
    fn open_settings(&mut self, scale: UiScale, of: SettingsOf) {
        let groups = of.groups();
        let sections =
            self.with_key_naming(|m, names| m.settings.settings.sections_named(&groups, names));
        let fonts = &self.fonts;
        if of != SettingsOf::Title {
            self.transition.change_screen();
        }
        self.settings_of = of;
        let mut screen = match of {
            SettingsOf::Title => SettingsScreen::paged(scale, fonts, "Settings", sections),
            SettingsOf::Ruthenium => {
                SettingsScreen::new(scale, fonts, "Ruthenium Options", sections)
            }
            SettingsOf::Game => SettingsScreen::paged(scale, fonts, "MC Options", sections),
        };
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
        if let Some(p) = &mut self.pause {
            p.states.set_reduce_motion(still);
        }
        if let Some(b) = &mut self.bed_button {
            b.states.set_reduce_motion(still);
        }
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.states.set_reduce_motion(still);
        }
    }

    /// Whether a text field has focus (the client turns text input on for it).
    pub fn wants_text_input(&self) -> bool {
        matches!(&self.screen, Screen::Form(_, form, _) if form.focused_field().is_some())
            || matches!(self.screen, Screen::Chat(_))
    }

    /// Lays every live screen out for the window, if its size changed.
    fn layout(&mut self, pixel_size: (u32, u32)) -> UiScale {
        let factor = self.settings.settings.menu_size.factor();
        let scale = UiScale::sized(pixel_size.0, pixel_size.1, factor);
        let revision = ruthenium_ui::text::fallback::revision();
        if self.scale == Some(scale) && self.fonts_revision == revision {
            return scale;
        }
        self.scale = Some(scale);
        self.fonts_revision = revision;
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
        if let Some(p) = &mut self.pause {
            p.layout(scale, &self.fonts);
        }
        if let Some(b) = &mut self.bed_button {
            b.layout(scale, &self.fonts, None);
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
            | FormKind::PackPrompt
            | FormKind::Playing
            | FormKind::Disconnected
            | FormKind::Died
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
            | FormKind::PackPrompt
            | FormKind::Playing
            | FormKind::Disconnected
            | FormKind::Died
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
            (Screen::Title, Action::Settings) => self.open_settings(scale, SettingsOf::Title),
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
                // A LAN game joins under its message as its name, as vanilla's does.
                let target = match self.servers.as_ref().and_then(Servers::selection) {
                    Some(Selected::Saved(_, s)) => Some((s.name.clone(), s.ip.clone())),
                    Some(Selected::Lan(l)) => Some((l.motd.clone(), l.address.clone())),
                    Some(Selected::Scanning) | None => None,
                };
                if let Some((name, ip)) = target {
                    self.start_join(&name, &ip);
                }
            }
            (
                Screen::Form(FormKind::Connecting | FormKind::Playing, ..),
                Action::Cancel | Action::Back,
            ) => self.leave(),
            (Screen::Paused, Action::Back) => self.enter_game(),
            (Screen::Paused, Action::Settings) => self.open_settings(scale, SettingsOf::Ruthenium),
            (Screen::Paused, Action::GameSettings) => self.open_settings(scale, SettingsOf::Game),
            (Screen::Paused, Action::Cancel) => self.leave(),
            (Screen::Form(FormKind::CodeOfConduct, ..), Action::Confirm) => {
                if let Some(s) = &self.session {
                    s.answer_code_of_conduct(true);
                }
                self.answered_code_of_conduct(true);
                self.connecting_form(NetStage::Joining);
            }
            (Screen::Form(FormKind::CodeOfConduct, ..), Action::Cancel | Action::Back) => {
                if let Some(s) = &self.session {
                    s.answer_code_of_conduct(false);
                }
                self.answered_code_of_conduct(false);
                self.leave();
            }
            (Screen::Form(FormKind::PackPrompt, ..), Action::Confirm) => {
                self.answer_pack_prompt(true);
            }
            (Screen::Form(FormKind::PackPrompt, ..), Action::Cancel | Action::Back) => {
                self.answer_pack_prompt(false);
            }
            (Screen::Form(FormKind::CodeOfConduct, form, _), Action::Cycle) => {
                if let Some((_, remember)) = &mut self.conduct {
                    *remember = !*remember;
                    form.set_button_label(CYCLE, &self.fonts, &conduct_label(*remember));
                }
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
            // The bed screen's Leave Bed: it stays until the server wakes the player.
            (Screen::Chat(chat), Action::Confirm) if chat.in_bed() => {
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
                    | FormKind::PackPrompt
                    | FormKind::Playing
                    | FormKind::Disconnected
                    | FormKind::Died
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
        if self.key_event(scancode, down, repeat) {
            return None;
        }
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
                // `handleKeybinds`: the Chat key opens the chat, the Command key with a
                // slash typed.
                if down && self.route.is_none() {
                    let key = ruthenium_sim::input::Key::scancode(scancode);
                    let actions: Vec<_> = self.binds().on(key).collect();
                    if actions.contains(&ruthenium_sim::keybinds::Action::Chat) {
                        self.open_chat(ChatMethod::Message);
                    } else if actions.contains(&ruthenium_sim::keybinds::Action::Command) {
                        self.open_chat(ChatMethod::Command);
                    }
                }
                return None;
            }
            Screen::Chat(_) => {
                if down {
                    self.chat_key(scancode, shift, shortcut, clipboard);
                }
                return None;
            }
            Screen::Container => {
                self.container_key(scancode, down);
                return None;
            }
            Screen::Paused if escape => return self.act(Action::Back, None, clipboard),
            _ => {}
        }
        let action = match &mut self.screen {
            // Handled above; nothing to press while respawning.
            Screen::Game | Screen::Chat(_) | Screen::Container | Screen::Respawning => None,
            Screen::Paused => {
                let screen = self.pause.as_mut()?;
                let key = match scancode {
                    SCANCODE_DOWN | SCANCODE_RIGHT => Some(Key::Next),
                    SCANCODE_UP | SCANCODE_LEFT => Some(Key::Previous),
                    SCANCODE_RETURN | SCANCODE_KP_ENTER | SCANCODE_SPACE if !repeat => {
                        Some(Key::Activate)
                    }
                    _ => nav,
                }?;
                let i = screen.states.key(key, down)?;
                Some(screen.buttons[i].action)
            }
            Screen::Settings => {
                let screen = self.settings_screen.as_mut()?;
                let e = match scancode {
                    SCANCODE_ESCAPE if down && !repeat => Some(SettingsEvent::Done),
                    SCANCODE_DOWN => screen.key(Key::Next, down),
                    SCANCODE_UP => screen.key(Key::Previous, down),
                    SCANCODE_PAGE_UP | SCANCODE_PAGE_DOWN if down => {
                        screen.turn(if scancode == SCANCODE_PAGE_UP { -1 } else { 1 });
                        None
                    }
                    SCANCODE_LEFT | SCANCODE_RIGHT if down => {
                        screen.adjust(if scancode == SCANCODE_LEFT { -1 } else { 1 })
                    }
                    SCANCODE_DELETE | SCANCODE_BACKSPACE if down => screen.reset_focused(),
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
        if let Screen::Chat(chat) = &mut self.screen {
            chat.type_text(text);
        }
        self.validate_form();
    }

    /// The wheel turned `notches` (positive: up); `raw` is SDL's `(x, y)` as it came.
    pub fn wheel(&mut self, notches: f32, raw: (f32, f32)) {
        // A form's message that is longer than it shows (the pack prompt) scrolls.
        if let Screen::Form(_, form, _) = &mut self.screen {
            form.wheel(notches);
        }
        // (Shift held: one line a notch.)
        let held = self.keys.physical();
        if let Screen::Chat(chat) = &mut self.screen {
            let shift = [225, 229]
                .into_iter()
                .any(|k| held.contains(&ruthenium_sim::input::Key::scancode(k)));
            chat.wheel(notches, shift);
        }
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
        if self.button_event(button, down) {
            return;
        }
        if matches!(self.screen, Screen::Game) {
            self.game.mouse_button(button, down, self.session.as_ref());
        }
        if matches!(self.screen, Screen::Container) {
            self.container_button(button, down);
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
        if self.button_event(1, down) {
            return None;
        }
        let scale = self.layout(pixel_size);
        let (ux, uy) = scale.from_window(x, y, window_size, pixel_size);
        let time = self.started.elapsed().as_secs_f64();
        let action = match &mut self.screen {
            Screen::Game => {
                self.game.mouse_button(1, down, self.session.as_ref());
                None
            }
            // The bed screen's Leave Bed. (Clicks in the edit box and on the chat's links:
            // not yet.)
            Screen::Chat(chat) if chat.in_bed() => {
                let b = self.bed_button.as_mut()?;
                let hit = b.hit(ux, uy);
                if down {
                    b.states.mouse_down(hit);
                    None
                } else {
                    b.states.mouse_up(hit).map(|_| b.button.action)
                }
            }
            Screen::Chat(_) | Screen::Respawning => None,
            Screen::Container => {
                self.container_button(1, down);
                None
            }
            Screen::Paused => {
                let screen = self.pause.as_mut()?;
                let hit = screen.hit(ux, uy);
                if down {
                    screen.states.mouse_down(hit);
                    None
                } else {
                    let i = screen.states.mouse_up(hit)?;
                    Some(screen.buttons[i].action)
                }
            }
            Screen::Settings => {
                let screen = self.settings_screen.as_mut()?;
                let e = if down {
                    screen.mouse_down(ux, uy)
                } else {
                    screen.mouse_up(ux, uy)
                };
                let e = e?;
                if !down {
                    self.clicks += 1;
                }
                self.settings_event(e);
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
        let action = action?;
        if !down {
            self.clicks += 1;
        }
        self.act(action, Some(renderer), clipboard)
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
        self.advance_glyphs(renderer);
        icon
    }

    /// Moves glyphs on their way into the atlas along; swaps the grown atlas in once the
    /// GPU has its texture.
    fn advance_glyphs(&mut self, renderer: &mut Renderer) {
        let Atlas::Ready { atlas, texture, .. } = &mut self.atlas else {
            return;
        };
        self.atlas_work = match std::mem::take(&mut self.atlas_work) {
            AtlasWork::Rendering(h) if h.is_finished() => match h.join() {
                Ok(rendered) => {
                    let mut next = atlas.clone();
                    next.insert(rendered);
                    AtlasWork::Uploading(next, None)
                }
                Err(_) => {
                    eprintln!("[ruthenium] the glyph thread panicked; some text is left out");
                    AtlasWork::Idle
                }
            },
            AtlasWork::Uploading(next, None) => {
                let desc = TextureDesc::rgba8(next.size, next.size);
                match renderer.create_texture(desc, &[&next.pixels], Arrival::Background) {
                    Ok(t) => AtlasWork::Uploading(next, Some(t)),
                    Err(e) if e.try_again() => AtlasWork::Uploading(next, None),
                    Err(e) => {
                        eprintln!("[ruthenium] glyph atlas upload: {e}");
                        AtlasWork::Idle
                    }
                }
            }
            AtlasWork::Uploading(next, Some(t)) if renderer.texture_ready(t) => {
                renderer.destroy_texture(std::mem::replace(texture, t));
                *atlas = next;
                AtlasWork::Idle
            }
            other => other,
        };
    }

    /// Starts rendering the glyphs text drew without (the atlas didn't have them yet).
    fn take_in_glyphs(&mut self, missing: Vec<(FaceId, u16)>) {
        if missing.is_empty()
            || !matches!(self.atlas_work, AtlasWork::Idle)
            || !matches!(self.atlas, Atlas::Ready { .. })
        {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("menu-glyphs".into())
            .spawn(move || Rendered::new(&Fonts::new(&WEIGHTS), &missing));
        match spawned {
            Ok(h) => self.atlas_work = AtlasWork::Rendering(h),
            Err(e) => eprintln!("[ruthenium] no thread for new glyphs: {e}"),
        }
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
        self.check_pack_prompt();
        self.expire_pack_toast();
        // The chat screen, as the HUD draws it.
        let chat = match &mut self.screen {
            Screen::Chat(chat) => {
                if let Some(shown) = renderer.chat_shown() {
                    chat.shown(shown);
                }
                Some(chat.input(now))
            }
            _ => None,
        };
        renderer.set_chat_input(chat);
        self.container_frame(renderer, pixel_size, window_size);
        self.name_keys_once_lang_loads();
        self.sync_world(renderer);
        if matches!(self.screen, Screen::Game) && self.game.should_pause() {
            self.pause();
        }
        if matches!(self.screen, Screen::Game)
            && renderer.world_ready()
            && let Some(shown) = self.shown_menu
        {
            self.pause();
            match shown {
                ShownMenu::Coming(_) => {}
                ShownMenu::RutheniumOptions => self.open_settings(scale, SettingsOf::Ruthenium),
                ShownMenu::GameOptions(page) => {
                    self.open_settings(scale, SettingsOf::Game);
                    if let Some(s) = &mut self.settings_screen {
                        s.turn_to(page);
                    }
                }
            }
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
        if self.session.is_none() {
            self.transition.end();
        }
        let options = matches!(self.screen, Screen::Settings);
        self.transition
            .step(self.game_menu_open() || self.death_veiled(), options, dt);
        if let Some(ShownMenu::Coming(share)) = self.shown_menu {
            self.transition = MenuTransition::at(share);
        }
        if !self.transition.shows() && !self.game_menu_open() {
            self.pause = None;
        }
        let mut settings_ev = None;
        // The bed screen's Leave Bed, over the chat, where vanilla's is at the HUD's scale.
        let bed = matches!(&self.screen, Screen::Chat(c) if c.in_bed());
        if bed {
            let b = self
                .bed_button
                .get_or_insert_with(|| BedScreen::new(scale, &self.fonts));
            let gui = self.settings.settings.game.gui_scale;
            b.place(Some(GuiScale::new(pixel_size.0, pixel_size.1, gui, false)));
            let hit = pointer.and_then(|(x, y)| b.hit(x, y));
            b.states.hover(hit);
            b.states.update(dt);
        }
        match &mut self.screen {
            // The Game Menu on its way out: nothing of it is there to be clicked.
            Screen::Game if self.transition.menu_shows() && self.pause.is_some() => {
                if let Some(p) = &mut self.pause {
                    p.states.update(dt);
                    p.draw(&mut self.shapes, &mut self.text, atlas, logo);
                }
            }
            // Only the veil, going.
            Screen::Respawning => {}
            Screen::Paused => {
                if let Some(p) = &mut self.pause {
                    let hit = pointer.and_then(|(x, y)| p.hit(x, y));
                    p.states.hover(hit);
                    p.states.update(dt);
                    p.draw(&mut self.shapes, &mut self.text, atlas, logo);
                }
            }
            Screen::Game | Screen::Chat(_) | Screen::Container => {
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
                if let Some(b) = self.bed_button.as_ref().filter(|_| bed) {
                    b.draw(&mut self.shapes, &mut self.text, atlas);
                }
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
        packs::draw_pack_toast(
            self.toast_text(),
            &scale,
            &self.fonts,
            atlas,
            &mut self.shapes,
            &mut self.text,
        );
        if let Some(e) = settings_ev {
            self.settings_event(e);
        }
        let missing = self.text.take_missing();
        self.take_in_glyphs(missing);
        if self.appear.value < 1.0 {
            self.shapes.fade_all(self.appear.value);
            self.text.fade_all(self.appear.value);
        }
        let veil = (self.transition.shows() && renderer.world_ready()).then(|| {
            let look = self
                .transition
                .look(scale.px, self.settings.settings.reduce_motion);
            // Only the Game Menu and its options come and go (the death screen's buttons
            // are a form's, which comes on its own).
            let menu = self.game_menu_open()
                || matches!(self.screen, Screen::Game)
                    && self.transition.menu_shows()
                    && self.pause.is_some();
            ruthenium_render::Veil {
                saturation: look.saturation,
                brightness: look.brightness,
                blur: look.blur,
                menu_blur: if menu { look.menu_blur } else { 0.0 },
                menu_opacity: if menu { look.menu_opacity } else { 1.0 },
            }
        });
        GuiFrame {
            shapes: self.shapes.shapes(),
            glyphs: self.text.quads(),
            font_atlas: texture,
            veil,
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

    /// Development: opens a form over the server list (for screenshots): "add", "edit"
    /// (the selected server), "direct" or "delete".
    pub fn open_form_named(&mut self, name: &str, pixel_size: (u32, u32)) {
        let scale = self.layout(pixel_size);
        let kind = match name {
            "add" => FormKind::Add,
            "edit" => FormKind::Edit(
                self.servers
                    .as_ref()
                    .and_then(Servers::selected)
                    .map_or(0, |(i, _)| i),
            ),
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
        self.open_settings(scale, SettingsOf::Title);
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
    use crate::account::tests::{accounts_held_in, accounts_in};
    use crate::servers::java_hash;

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

    /// `name` inside a fresh temporary folder, deleted with the guard even when the
    /// test fails.
    fn temp(name: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(name);
        (tmp, dir)
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

    /// Text is shaped again once a fallback font arrives (Unifont, read at start-up).
    #[test]
    fn the_menus_are_laid_out_again_when_a_fallback_font_comes() {
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.title = None;
        m.layout((1600, 900));
        assert!(m.title.is_none(), "nothing changed: not laid out again");
        // (A font of one character no other test draws: the process keeps it.)
        let font = ruthenium_assets::Font::of_unihex(&format!("01E5D0:{}\n", "F".repeat(64)));
        ruthenium_ui::text::fallback::set_unifont(font.unwrap());
        m.layout((1600, 900));
        assert!(m.title.is_some());
        m.title = None;
        m.layout((1600, 900));
        assert!(m.title.is_none());
    }

    #[test]
    fn the_chat_key_opens_the_chat_screen_and_enter_sends() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        // T: empty, typed into, the world's keys let go.
        press(&mut m, 23, false, false, &clip);
        assert!(matches!(&m.screen, Screen::Chat(c) if c.text().is_empty()));
        assert!(m.wants_text_input() && !m.wants_mouse_grab() && !m.in_world());
        m.text_input("  hello   there ");
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Game));
        // Sent as made tidy, and kept for Up.
        assert_eq!(m.chat_history.get(0), Some("hello there"));
        // /: a slash typed already; Escape closes without sending.
        press(&mut m, 56, false, false, &clip);
        assert!(matches!(&m.screen, Screen::Chat(c) if c.text() == "/"));
        m.text_input("say x");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Game));
        assert_eq!(m.chat_history.len(), 1);
        // Nothing left once tidied: nothing kept.
        press(&mut m, 23, false, false, &clip);
        m.text_input("   ");
        press(&mut m, SCANCODE_KP_ENTER, false, false, &clip);
        assert_eq!(m.chat_history.len(), 1);
        // Up brings back what was sent.
        press(&mut m, 23, false, false, &clip);
        press(&mut m, SCANCODE_UP, false, false, &clip);
        assert!(matches!(&m.screen, Screen::Chat(c) if c.text() == "hello there"));
    }

    /// What the chat screen shows, and whether as a draft.
    fn chat_shows(m: &Menu) -> Option<(String, bool)> {
        match &m.screen {
            Screen::Chat(c) => Some((c.text().to_owned(), c.is_draft())),
            _ => None,
        }
    }

    fn dead() -> DeathForm {
        DeathForm {
            title: "You Died!",
            lines: Vec::new(),
            hardcore: false,
            ready: false,
            respawn_pressed: false,
        }
    }

    #[test]
    fn chat_drafts_outlive_other_screens_and_escape_only_with_the_option() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        let some = |s: &str, draft| Some((s.to_owned(), draft));
        // Typed, then the death screen took its place: the Chat key brings it back.
        press(&mut m, 23, false, false, &clip);
        m.text_input("before dying");
        m.death = Some(dead());
        m.death_form();
        assert!(matches!(m.screen, Screen::Form(FormKind::Died, ..)));
        m.death = None;
        m.enter_game();
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("before dying", true));
        // Escape with Save Unsent Chats off (the default): gone.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("", false));
        // With it on: kept.
        m.set_setting(Row::SaveChatDrafts, 1);
        m.text_input("kept");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("kept", true));
        // Backspace clears it; what is sent is no draft.
        press(&mut m, SCANCODE_BACKSPACE, false, false, &clip);
        assert_eq!(chat_shows(&m), some("", false));
        m.text_input("x");
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("", false));
        // A message's draft, and the Command key: a slash, and the slash is the draft
        // once Escape closes it.
        m.text_input("a message");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        press(&mut m, 56, false, false, &clip);
        assert_eq!(chat_shows(&m), some("/", false));
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("/", true));
        // Leaving the server (its screen in the chat's place) keeps it for the next.
        press(&mut m, SCANCODE_BACKSPACE, false, false, &clip);
        m.text_input("/say on the next server");
        m.session_form(FormKind::Disconnected, "Disconnected", &[], Vec::new());
        m.screen = Screen::Game;
        press(&mut m, 56, false, false, &clip);
        assert_eq!(chat_shows(&m), some("/say on the next server", true));
    }

    #[test]
    fn a_chat_screen_open_when_a_level_loads_comes_back_after() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        press(&mut m, 23, false, false, &clip);
        m.text_input("through the portal");
        m.level_loading();
        assert!(matches!(m.screen, Screen::Form(FormKind::Playing, ..)));
        // W held while it loads: not walked on while the chat is back.
        m.key(26, true, false, false, false, &clip);
        m.enter_game();
        assert_eq!(
            chat_shows(&m),
            Some(("through the portal".to_owned(), false))
        );
        assert!(m.wants_text_input() && !m.wants_mouse_grab());
        assert!(!m.game.holds(ruthenium_sim::input::Key::Keyboard(26)));
        m.key(26, false, false, false, false, &clip);
        // Once: closed, the world.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Game));
        // While the player is dead it waits (the death screen comes first).
        press(&mut m, 23, false, false, &clip);
        m.text_input("later");
        m.level_loading();
        m.death = Some(dead());
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
        m.death = None;
        m.enter_game();
        assert_eq!(chat_shows(&m), Some(("later".to_owned(), false)));
        // Another screen in the loading's place leaves no chat to come back.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        m.session_form(FormKind::Playing, "Server", &[], Vec::new());
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
    }

    /// What the bed screen shows, and whether as a draft (none: it isn't open).
    fn bed_shows(m: &Menu) -> Option<(String, bool)> {
        match &m.screen {
            Screen::Chat(c) if c.in_bed() => Some((c.text().to_owned(), c.is_draft())),
            _ => None,
        }
    }

    #[test]
    fn the_bed_screen_is_a_chat_screen_that_stays_until_the_player_wakes() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        let some = |s: &str, draft| Some((s.to_owned(), draft));
        // Into bed with a draft: it shows, as one.
        press(&mut m, 23, false, false, &clip);
        m.text_input("good night");
        m.death = Some(dead());
        m.death_form();
        m.death = None;
        m.enter_game();
        m.in_bed();
        assert_eq!(bed_shows(&m), some("good night", true));
        assert!(m.wants_text_input() && !m.wants_mouse_grab() && !m.in_world());
        // Enter sends it and empties the box; the screen stays (and isn't removed, so
        // the draft stays until it is).
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert_eq!(bed_shows(&m), some("", false));
        assert_eq!(m.chat_history.get(0), Some("good night"));
        assert!(m.chat_drafts.latest().is_some());
        // Escape asks to leave the bed: it stays too.
        m.text_input("zz");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert_eq!(bed_shows(&m), some("zz", false));
        // Up and Down go on from where the screen opened (`historyPos` isn't moved on
        // by what is sent): Up has nothing before, Down the text typed, then Up the
        // message sent.
        press(&mut m, SCANCODE_UP, false, false, &clip);
        assert_eq!(bed_shows(&m), some("zz", false));
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        assert_eq!(bed_shows(&m), some("", false));
        press(&mut m, SCANCODE_UP, false, false, &clip);
        assert_eq!(bed_shows(&m), some("good night", false));
        // Awake with text typed: the plain chat screen with it, no draft.
        m.woke_up();
        assert_eq!(chat_shows(&m), some("good night", false));
        assert_eq!(bed_shows(&m), None);
        assert_eq!(m.chat_drafts.latest(), None);
        assert!(m.wants_text_input() && !m.wants_mouse_grab());
        // That one closes on Enter.
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert!(matches!(m.screen, Screen::Game));
        // Awake with nothing typed: the world.
        m.in_bed();
        assert_eq!(bed_shows(&m), some("", false));
        m.woke_up();
        assert!(matches!(m.screen, Screen::Game));
        // Awake with a draft as it was brought back: closed, the draft kept.
        press(&mut m, 23, false, false, &clip);
        m.text_input("for later");
        m.session_form(FormKind::Playing, "Server", &[], Vec::new());
        m.screen = Screen::Game;
        m.in_bed();
        assert_eq!(bed_shows(&m), some("for later", true));
        m.woke_up();
        assert!(matches!(m.screen, Screen::Game));
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("for later", true));
        // Edited, then awake: the plain chat screen; Escape there (option off) drops it.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        m.in_bed();
        assert_eq!(bed_shows(&m), some("", false));
        m.text_input("x");
        press(&mut m, SCANCODE_BACKSPACE, false, false, &clip);
        m.text_input(" ");
        m.woke_up();
        assert_eq!(chat_shows(&m), some(" ", false));
    }

    #[test]
    fn the_bed_screen_waits_for_other_screens_and_goes_for_the_death_screen() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        let some = |s: &str, draft| Some((s.to_owned(), draft));
        // The chat opened just before: it stays open (and isn't the bed screen).
        press(&mut m, 23, false, false, &clip);
        m.text_input("hi");
        m.in_bed();
        assert_eq!(chat_shows(&m), some("hi", false));
        assert_eq!(bed_shows(&m), None);
        // Closed while in bed: the bed screen, with what it left (Escape, the option
        // off: nothing).
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert_eq!(bed_shows(&m), some("", false));
        // Sent from the chat: the bed screen, empty.
        m.woke_up();
        press(&mut m, 23, false, false, &clip);
        m.in_bed();
        m.text_input("sent");
        press(&mut m, SCANCODE_RETURN, false, false, &clip);
        assert_eq!(bed_shows(&m), some("", false));
        // Awake while another screen shows: it is left alone, and the world after it.
        m.woke_up();
        m.pause();
        m.in_bed();
        assert!(matches!(m.screen, Screen::Paused));
        m.woke_up();
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
        // The Game Menu closed while in bed: the bed screen.
        m.pause();
        m.in_bed();
        m.enter_game();
        assert_eq!(bed_shows(&m), some("", false));
        // The death screen takes its place, keeping what was typed as the draft; while dead
        // nothing comes back, and after the respawn the world, not the bed screen.
        m.text_input("last words");
        m.death = Some(dead());
        m.death_form();
        assert!(matches!(m.screen, Screen::Form(FormKind::Died, ..)));
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
        m.bed = false;
        m.death = None;
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
        press(&mut m, 23, false, false, &clip);
        assert_eq!(chat_shows(&m), some("last words", true));
        // Leaving the server: no bed screen in the next.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        m.in_bed();
        m.leave();
        m.screen = Screen::Game;
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
    }

    #[test]
    fn a_bed_screen_put_away_by_a_level_load_comes_back_or_goes_as_on_waking() {
        let clip = FakeClipboard(RefCell::new(None));
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        let some = |s: &str, draft| Some((s.to_owned(), draft));
        m.in_bed();
        m.text_input("still here");
        m.level_loading();
        assert!(matches!(m.screen, Screen::Form(FormKind::Playing, ..)));
        // Still in bed once loaded: the bed screen as it was.
        m.enter_game();
        assert_eq!(bed_shows(&m), some("still here", false));
        // Out of bed by then (a respawn): as on waking, the plain chat screen.
        m.level_loading();
        m.bed = false;
        m.enter_game();
        assert_eq!(chat_shows(&m), some("still here", false));
        assert_eq!(bed_shows(&m), None);
        // With nothing typed: the world.
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        m.in_bed();
        m.level_loading();
        m.bed = false;
        m.enter_game();
        assert!(matches!(m.screen, Screen::Game));
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

    /// A server transfers the player in configuration, to a server that doesn't accept
    /// transfers: the next session goes from the same entry, says it is transferring, and
    /// its failed login says it was a transfer's.
    #[cfg(feature = "dev-offline")]
    #[test]
    fn a_transfer_is_followed_from_the_same_entry() {
        use ruthenium_data::v26_3::packets::{configuration as cfg_ids, login as login_ids};
        use ruthenium_net::transport::Transport;
        use ruthenium_protocol::wire::{self, Reader};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        struct Peer(TcpStream, Transport);
        impl Peer {
            fn accept(l: &TcpListener) -> Peer {
                let (s, _) = l.accept().unwrap();
                s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                Peer(s, Transport::new())
            }
            fn recv(&mut self) -> Option<(u32, Vec<u8>)> {
                loop {
                    if let Some(p) = self.1.next_packet().unwrap() {
                        let mut r = Reader::new(&p);
                        return Some((r.varint().unwrap() as u32, r.rest().to_vec()));
                    }
                    let mut buf = [0u8; 4096];
                    match self.0.read(&mut buf).unwrap_or(0) {
                        0 => return None,
                        n => self.1.receive(&buf[..n]),
                    }
                }
            }
            fn send(&mut self, id: u32, body: impl FnOnce(&mut Vec<u8>)) {
                let bytes = self.1.encode(&wire::packet(id, body)).unwrap();
                self.0.write_all(&bytes).unwrap();
            }
            /// The handshake's intent, after the hello.
            fn opening(&mut self) -> i32 {
                let (_, body) = self.recv().unwrap();
                let mut r = Reader::new(&body);
                r.varint().unwrap();
                r.string(255).unwrap();
                r.u16().unwrap();
                let intent = r.varint().unwrap();
                assert_eq!(self.recv().unwrap().0, login_ids::Serverbound::Hello.id());
                intent
            }
            fn until_closed(&mut self) {
                while self.recv().is_some() {}
            }
        }

        let a = TcpListener::bind("127.0.0.1:0").unwrap();
        let b = TcpListener::bind("127.0.0.1:0").unwrap();
        let (port_a, port_b) = (
            a.local_addr().unwrap().port(),
            b.local_addr().unwrap().port(),
        );
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let servers = std::thread::spawn(move || {
            let mut s = Peer::accept(&a);
            assert_eq!(s.opening(), 2, "LOGIN");
            s.send(login_ids::Clientbound::LoginFinished.id(), |o| {
                wire::write_uuid(o, 1);
                wire::write_string(o, "Developer");
                wire::write_varint(o, 0);
                wire::write_uuid(o, 2);
            });
            for _ in 0..3 {
                s.recv().unwrap();
            }
            s.send(cfg_ids::Clientbound::Transfer.id(), |o| {
                wire::write_string(o, "127.0.0.1");
                wire::write_varint(o, i32::from(port_b));
            });
            s.until_closed();
            let mut s = Peer::accept(&b);
            assert_eq!(s.opening(), 3, "TRANSFER");
            wait.recv().unwrap();
            // What a vanilla server with accepts-transfers=false answers.
            s.send(login_ids::Clientbound::LoginDisconnect.id(), |o| {
                wire::write_string(
                    o,
                    r#"{"translate":"multiplayer.disconnect.transfers_disabled"}"#,
                );
            });
            s.until_closed();
        });
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        m.set_joiner(Joiner::new(None, None));
        m.start_join("Lobby", &format!("127.0.0.1:{port_a}"));
        until(&mut m, |m| {
            form_kind(m).is_some_and(|(k, message)| {
                k == FormKind::Connecting && message.join(" ").contains("Transferring")
            })
        });
        // Still the entry joined.
        assert_eq!(
            (m.server_name.as_str(), m.server_address.clone()),
            ("Lobby", format!("127.0.0.1:{port_a}"))
        );
        go.send(()).unwrap();
        until(&mut m, |m| m.session.is_none());
        let Screen::Form(FormKind::Disconnected, form, _) = &m.screen else {
            panic!("{:?}", form_kind(&m));
        };
        assert_eq!(
            form.spec.title,
            "Connection failed while transferring to the server"
        );
        assert!(
            form.spec.message.join(" ").contains("transfers_disabled"),
            "{:?}",
            form.spec.message
        );
        servers.join().unwrap();
        // The form says so from the start.
        m.start_transfer(Transfer {
            to: ruthenium_net::ServerAddress::new("127.0.0.1", 1),
            state: Default::default(),
        });
        let (kind, message) = form_kind(&m).unwrap();
        assert_eq!(kind, FormKind::Connecting);
        assert_eq!(message, ["Transferring to new server..."]);
        // A join after it is a join again.
        m.start_join("Nowhere", "127.0.0.1:1");
        assert!(!m.transferred);
        assert_eq!(form_kind(&m).unwrap().1, ["Connecting to the server..."]);
    }

    #[test]
    fn privileges_can_turn_multiplayer_off() {
        let (_tmp, dir) = temp("privileges");
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
    }

    #[test]
    fn accounts_sign_in_switch_and_sign_out() {
        let (_tmp, dir) = temp("accounts");
        let (accounts, pages, answer) = accounts_held_in(&dir, &["Alex"], Some(Ok("Notch")));
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

        // Add Account shows the sign-in form and opens the browser's page. Once the
        // player has signed in there (the answer was held back until now: it can come
        // before the menu first looks, and then the form never shows), it comes back to
        // the list.
        m.act(Action::AddAccount, None, &clip);
        assert!(matches!(m.screen, Screen::Form(FormKind::SignIn, ..)));
        until(&mut m, |m| {
            matches!(m.stage_shown, Some(Stage::Browser { .. }))
        });
        assert!(matches!(m.screen, Screen::Form(FormKind::SignIn, ..)));
        assert_eq!(
            pages.lock().unwrap().as_slice(),
            ["https://login.example/authorize"]
        );
        answer.store(true, std::sync::atomic::Ordering::Relaxed);
        until(&mut m, |m| matches!(m.screen, Screen::Accounts));
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
    }

    #[test]
    fn signing_in_with_a_code() {
        let (_tmp, dir) = temp("code");
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
    }

    #[test]
    fn a_failed_sign_in_says_why_and_can_try_again() {
        let (_tmp, dir) = temp("failed");
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
    }

    #[test]
    fn settings_from_the_keyboard_apply_and_save() {
        let (_tmp, dir) = temp("settings");
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
        // Ruthenium's own first; the next page is Video.
        press(&mut m, SCANCODE_PAGE_DOWN, false, false, &clip);
        assert_eq!(m.settings_screen.as_ref().unwrap().page(), 1);
        // Menu Size, its third row: the next layout is larger.
        for _ in 0..3 {
            press(&mut m, SCANCODE_DOWN, false, false, &clip);
        }
        press(&mut m, SCANCODE_RIGHT, false, false, &clip);
        // VSync off with Left, then the frame rate one step down from Unlimited.
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_LEFT, false, false, &clip);
        assert!(!m.settings().vsync);
        press(&mut m, SCANCODE_DOWN, false, false, &clip);
        press(&mut m, SCANCODE_LEFT, false, false, &clip);
        assert_eq!(m.settings().max_fps, Some(crate::settings::FPS_MAX));
        let after = m.layout((1600, 900));
        assert!(after.px > before.px, "{} -> {}", before.px, after.px);
        // F3 from anywhere shows on the screen too.
        m.toggle_frame_stats();
        let s = m.settings_screen.as_ref().unwrap();
        assert_eq!(s.value(0), Some(1));
        assert!(!path.exists(), "saved only after a pause or on leaving");
        press(&mut m, SCANCODE_ESCAPE, false, false, &clip);
        assert!(matches!(m.screen, Screen::Title));
        let saved = SettingsFile::load(path).settings;
        assert_eq!(&saved, m.settings());
        assert!(saved.frame_stats && !saved.vsync);
    }

    #[test]
    fn the_game_menu_opens_its_options_and_goes_as_it_came() {
        let (_tmp, dir) = temp("game-menu");
        let (accounts, _) = accounts_in(&dir, &["Alex"], None);
        let clip = FakeClipboard::default();
        let mut m = Menu::new(None, accounts, SettingsFile::transient());
        m.layout((1600, 900));
        m.screen = Screen::Game;
        let key = |m: &mut Menu, code| press(m, code, false, false, &clip);
        // Escape opens it and closes it; it is kept until it has gone.
        key(&mut m, SCANCODE_ESCAPE);
        assert!(matches!(m.screen, Screen::Paused) && m.game_menu_open());
        assert!(!m.wants_mouse_grab());
        key(&mut m, SCANCODE_DOWN);
        key(&mut m, SCANCODE_ESCAPE);
        assert!(matches!(m.screen, Screen::Game) && !m.game_menu_open());
        assert!(m.wants_mouse_grab());
        let pause = m.pause.as_ref().unwrap();
        assert_eq!(pause.states.focused(), None);
        // Back to Game, by the keyboard: the first key shows where the focus starts.
        key(&mut m, SCANCODE_ESCAPE);
        key(&mut m, SCANCODE_DOWN);
        key(&mut m, SCANCODE_RETURN);
        assert!(matches!(m.screen, Screen::Game));
        // The two options beside each other, and back to the menu from both.
        for (moves, of, name) in [
            (2, SettingsOf::Ruthenium, "Overlay"),
            (3, SettingsOf::Game, "Video"),
        ] {
            key(&mut m, SCANCODE_ESCAPE);
            for _ in 0..moves {
                key(&mut m, SCANCODE_RIGHT);
            }
            m.transition = MenuTransition::at(1.0);
            key(&mut m, SCANCODE_RETURN);
            assert!(matches!(m.screen, Screen::Settings) && m.game_menu_open());
            assert_eq!(m.settings_of, of);
            let screen = m.settings_screen.as_ref().unwrap();
            assert_eq!(screen.sections[0].title, name);
            // They come as the menu did.
            assert!(m.transition.shows() && !m.transition.menu_shows());
            key(&mut m, SCANCODE_ESCAPE);
            assert!(matches!(m.screen, Screen::Paused) && m.settings_screen.is_none());
            key(&mut m, SCANCODE_ESCAPE);
        }
        // What is changed there is what is kept: vignette off, in the game's own.
        m.screen = Screen::Paused;
        m.act(Action::GameSettings, None, &clip);
        let row = |row| rows_of(&GAME).iter().position(|r| *r == row).unwrap();
        m.settings_event(SettingsEvent::Changed {
            setting: row(Row::Vignette),
            value: 0,
        });
        assert!(!m.settings().game.vignette && !m.settings().reduce_motion);
        // F3's row is not one of these; F11's is.
        m.toggle_frame_stats();
        assert!(m.settings().frame_stats);
        m.toggle_fullscreen();
        let screen = m.settings_screen.as_ref().unwrap();
        assert_eq!(screen.value(row(Row::Fullscreen)), Some(1));
        assert_eq!(
            screen.value(0),
            Some(40),
            "the field of view's is as it was"
        );
        // The world takes the player back from the options: nothing of them stays.
        m.enter_game();
        assert!(m.settings_screen.is_none() && m.pause.is_none());
        // Disconnect.
        key(&mut m, SCANCODE_ESCAPE);
        m.transition = MenuTransition::at(1.0);
        m.act(Action::Cancel, None, &clip);
        assert!(m.leaving && matches!(m.screen, Screen::Servers));
        assert!(m.pause.is_none() && !m.transition.shows());
    }

    #[test]
    fn every_option_s_words_fit_at_every_menu_size() {
        let settings = Settings::default();
        let fonts = Fonts::new(&ruthenium_ui::modern::WEIGHTS);
        for of in [SettingsOf::Title, SettingsOf::Ruthenium, SettingsOf::Game] {
            for factor in [0.9, 1.0, 1.15, 1.3] {
                let scale = UiScale::sized(1600, 900, factor);
                let sections = settings.sections(&of.groups());
                let screen = SettingsScreen::paged(scale, &fonts, "x", sections);
                assert!(screen.cut_off().is_empty(), "{of:?} {:?}", screen.cut_off());
            }
        }
    }

    #[test]
    fn what_of_the_menu_to_show_is_read() {
        assert_eq!(ShownMenu::parse("0.5"), Some(ShownMenu::Coming(0.5)));
        assert_eq!(ShownMenu::parse("mc"), Some(ShownMenu::GameOptions(0)));
        assert_eq!(ShownMenu::parse("mc:4"), Some(ShownMenu::GameOptions(4)));
        assert_eq!(
            ShownMenu::parse("ruthenium"),
            Some(ShownMenu::RutheniumOptions)
        );
        for bad in ["", "1.5", "-1", "x", "NaN", "mc:", "mc:9"] {
            assert_eq!(ShownMenu::parse(bad), None, "{bad}");
        }
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

    /// The world behind the death screen is veiled as behind the Game Menu; once the
    /// server respawns the player, no form shows until the world is back.
    #[test]
    fn the_death_screen_veils_the_world_and_respawns_without_a_form() {
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.layout((1600, 900));
        assert!(!m.death_veiled());
        m.death = Some(DeathForm {
            title: "You Died!",
            lines: vec!["Score: 0".into()],
            hardcore: false,
            ready: true,
            respawn_pressed: true,
        });
        m.death_form();
        assert!(m.death_veiled());
        m.act(Action::Cancel, None, &FakeClipboard::default());
        assert!(matches!(m.screen, Screen::Form(FormKind::QuitConfirm, ..)));
        assert!(m.death_veiled());
        m.death_form();
        m.respawned();
        assert!(matches!(m.screen, Screen::Respawning));
        assert!(m.death.is_none() && !m.death_veiled());
        // A respawn into another dimension, alive: the level loading form.
        m.respawned();
        assert!(matches!(m.screen, Screen::Form(FormKind::Playing, ..)));
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
    fn lan_games_join_under_their_message() {
        let clip = FakeClipboard::default();
        let (_tmp, dir) = temp("lan");
        let path = dir.join("servers.dat");
        let mut list = ServerList::default();
        list.add(ServerEntry::new("Saved", "127.0.0.1:1"), false);
        list.save(&path).unwrap();
        let mut m = Menu::new(
            Some(path.clone()),
            Accounts::disabled(),
            SettingsFile::transient(),
        );
        m.layout((1600, 900));
        m.open_servers((1600, 900));
        let s = m.servers.as_mut().unwrap();
        s.found_on_lan(ruthenium_net::lan::LanServer {
            motd: "§aA ᴡᴏʀʟᴅ".into(),
            address: "192.168.1.5:25565".into(),
        });
        s.sync_rows(&m.fonts);
        // The saved server, the scanning line, the game.
        s.screen.select(Some(1));
        assert_eq!(s.selection(), Some(Selected::Scanning));
        m.act(Action::Join, None, &clip);
        assert!(
            matches!(m.screen, Screen::Servers),
            "the line joins nothing"
        );
        let s = m.servers.as_mut().unwrap();
        s.screen.select(Some(2));
        assert!(matches!(s.selection(), Some(Selected::Lan(_))));
        // Edit and Delete are for saved servers only.
        m.act(Action::EditServer, None, &clip);
        m.act(Action::DeleteServer, None, &clip);
        assert!(matches!(m.screen, Screen::Servers));
        m.act(Action::Join, None, &clip);
        assert_eq!(
            (m.server_name.as_str(), m.server_address.as_str()),
            ("§aA ᴡᴏʀʟᴅ", "192.168.1.5:25565")
        );
    }

    #[test]
    fn codes_of_conduct_accepted_for_good_are_not_asked_again() {
        let clip = FakeClipboard::default();
        let (_tmp, dir) = temp("conduct");
        let path = dir.join("servers.dat");
        let mut list = ServerList::default();
        list.add(ServerEntry::new("Local", "127.0.0.1:25631"), false);
        list.add(ServerEntry::new("Other", "127.0.0.1:25631"), false);
        list.save(&path).unwrap();
        let mut m = Menu::new(
            Some(path.clone()),
            Accounts::disabled(),
            SettingsFile::transient(),
        );
        m.layout((1600, 900));
        m.open_servers((1600, 900));
        // No joiner here: the join ends at once, but the server joined is known.
        m.start_join("Local", "127.0.0.1:25631");
        let text = "Be kind. ʙᴇ ᴋɪɴᴅ. 日本語";
        let shown = |m: &Menu| matches!(m.screen, Screen::Form(FormKind::CodeOfConduct, ..));
        // Accepted once: asked again next time.
        m.code_of_conduct_arrived(text);
        assert!(shown(&m));
        m.act(Action::Confirm, None, &clip);
        assert_eq!(
            ServerList::load(&path).servers[0].accepted_code_of_conduct,
            0
        );
        m.code_of_conduct_arrived(text);
        assert!(shown(&m));
        // Accepted with "Don't Ask Again": kept as Java's hash of the text, by name and
        // address, and not asked again while it doesn't change.
        m.act(Action::Cycle, None, &clip);
        m.act(Action::Confirm, None, &clip);
        let saved = ServerList::load(&path);
        assert_eq!(saved.servers[0].accepted_code_of_conduct, java_hash(text));
        assert_eq!(saved.servers[1].accepted_code_of_conduct, 0);
        m.screen = Screen::Servers;
        m.code_of_conduct_arrived(text);
        assert!(!shown(&m));
        m.code_of_conduct_arrived("Be kinder.");
        assert!(shown(&m));
        // Declining forgets it.
        m.act(Action::Cycle, None, &clip);
        m.act(Action::Cancel, None, &clip);
        assert_eq!(
            ServerList::load(&path).servers[0].accepted_code_of_conduct,
            0
        );
    }

    #[test]
    fn add_edit_and_delete_servers_from_the_keyboard() {
        let (_tmp, dir) = temp("servers");
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
    }
}
