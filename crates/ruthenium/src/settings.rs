//! The player's settings: what the Settings screen shows, kept in
//! `config/client.toml` in Ruthenium's data folder.
//!
//! Reading is forgiving: a missing file, key or section, or a value of the wrong type or
//! out of range, takes the default (a broken file is reported once). Saving writes the
//! whole file, with comments, to a new file beside it, then moves the previous one to
//! `client.toml_old` and the new one into place, as `servers.dat` is saved.

use std::path::{Path, PathBuf};

use ruthenium_render::clouds::CloudMode;
use ruthenium_sim::input::MouseOptions;
use ruthenium_ui::hud::AttackIndicator;
use ruthenium_ui::modern::{ControlSpec, SectionSpec, SettingSpec};

/// Vanilla's frame-rate slider: 10 to 250 FPS in steps of 10, then Unlimited
/// (`Options.framerateLimit`, 10..260 where 260 means unlimited).
pub const FPS_STEP: u32 = 10;
pub const FPS_MAX: u32 = 250;

/// How large the out-of-game menus are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuSize {
    Small,
    #[default]
    Default,
    Large,
    Larger,
}

impl MenuSize {
    pub const ALL: [MenuSize; 4] = [
        MenuSize::Small,
        MenuSize::Default,
        MenuSize::Large,
        MenuSize::Larger,
    ];

    /// The menus' size against the default.
    pub fn factor(self) -> f32 {
        match self {
            MenuSize::Small => 0.9,
            MenuSize::Default => 1.0,
            MenuSize::Large => 1.15,
            MenuSize::Larger => 1.3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MenuSize::Small => "Small",
            MenuSize::Default => "Default",
            MenuSize::Large => "Large",
            MenuSize::Larger => "Larger",
        }
    }

    fn key(self) -> &'static str {
        match self {
            MenuSize::Small => "small",
            MenuSize::Default => "default",
            MenuSize::Large => "large",
            MenuSize::Larger => "larger",
        }
    }
}

/// Vanilla's field of view slider, in degrees.
pub const FOV_MIN: u32 = 30;
pub const FOV_MAX: u32 = 110;
const FOV_NORMAL: u32 = 70;
/// The largest GUI Scale offered (0: as large as fits; a scale that does not fit is not
/// taken).
const GUI_SCALE_MAX: u32 = 8;
/// Steps of the sensitivity slider over 0 to 1: one for each percent vanilla shows.
const SENSITIVITY_STEPS: usize = 200;

/// The game's own options, as vanilla has them, as far as there are any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOptions {
    /// From top to bottom, in degrees.
    pub fov: u32,
    pub gui_scale: u32,
    pub view_bobbing: bool,
    pub entity_shadows: bool,
    pub vignette: bool,
    pub attack_indicator: AttackIndicator,
    /// How far the view tilts when hurt, and how far the field of view changes, in
    /// percent.
    pub damage_tilt: u32,
    pub fov_effects: u32,
    /// Clouds: off, flat or with sides; and how far out, in chunks.
    pub clouds: CloudMode,
    pub cloud_distance: u32,
}

impl Default for GameOptions {
    fn default() -> GameOptions {
        GameOptions {
            fov: FOV_NORMAL,
            gui_scale: 0,
            view_bobbing: true,
            entity_shadows: true,
            vignette: true,
            attack_indicator: AttackIndicator::Crosshair,
            damage_tilt: 100,
            fov_effects: 100,
            clouds: CloudMode::Fancy,
            cloud_distance: CLOUD_DISTANCE_MAX,
        }
    }
}

/// Vanilla's Cloud Distance slider, in chunks.
const CLOUD_DISTANCE_MIN: u32 = 2;
const CLOUD_DISTANCE_MAX: u32 = 128;

/// The Clouds option's choices: its key in the file, its label.
const CLOUDS: [(CloudMode, &str, &str); 3] = [
    (CloudMode::Off, "off", "Off"),
    (CloudMode::Fast, "fast", "Fast"),
    (CloudMode::Fancy, "fancy", "Fancy"),
];

/// The attack indicator's places: its key in the file, its label.
const INDICATORS: [(AttackIndicator, &str, &str); 3] = [
    (AttackIndicator::Off, "off", "Off"),
    (AttackIndicator::Crosshair, "crosshair", "Crosshair"),
    (AttackIndicator::Hotbar, "hotbar", "Hotbar"),
];

/// See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Present in step with the display (FIFO); off: as soon as a frame is ready.
    pub vsync: bool,
    /// The frame limiter's rate (`None`: unlimited), a multiple of [`FPS_STEP`] up to
    /// [`FPS_MAX`].
    pub max_fps: Option<u32>,
    /// Borderless fullscreen on the window's display.
    pub fullscreen: bool,
    /// The frame-stats overlay (F3).
    pub frame_stats: bool,
    pub menu_size: MenuSize,
    /// No sliding, growing or gliding in the menus; fades stay.
    pub reduce_motion: bool,
    /// The last address typed into Direct Connect (vanilla's `lastServer`).
    pub last_server: String,
    /// The mouse's sensitivity (0 to 1, exactly as vanilla's `mouseSensitivity`) and
    /// inversion. Imported from vanilla's `options.txt` when the file has none.
    pub mouse: MouseOptions,
    pub game: GameOptions,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            vsync: true,
            max_fps: None,
            fullscreen: false,
            frame_stats: false,
            menu_size: MenuSize::Default,
            reduce_motion: false,
            last_server: String::new(),
            mouse: MouseOptions::DEFAULT,
            game: GameOptions::default(),
        }
    }
}

/// What can be set on a settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    FrameStats,
    ReduceMotion,
    Fov,
    GuiScale,
    MenuSize,
    Vsync,
    MaxFps,
    Fullscreen,
    AttackIndicator,
    ViewBobbing,
    EntityShadows,
    Vignette,
    Sensitivity,
    InvertX,
    InvertY,
    DamageTilt,
    FovEffects,
    Clouds,
    CloudDistance,
}

/// A titled group of rows.
pub type Group = (&'static str, &'static [Row]);

/// What is Ruthenium's own.
pub const RUTHENIUM: [Group; 2] = [
    ("Overlay", &[Row::FrameStats]),
    ("Menus", &[Row::ReduceMotion]),
];

/// What vanilla has too, under vanilla's headings. Those without any are to come.
pub const GAME: [Group; 9] = [
    (
        "Video",
        &[
            Row::Fov,
            Row::GuiScale,
            Row::MenuSize,
            Row::Vsync,
            Row::MaxFps,
            Row::Fullscreen,
            Row::AttackIndicator,
            Row::ViewBobbing,
            Row::EntityShadows,
            Row::Clouds,
            Row::CloudDistance,
            Row::Vignette,
        ],
    ),
    ("Controls", &[Row::Sensitivity, Row::InvertX, Row::InvertY]),
    ("Online", &[]),
    ("Resources", &[]),
    ("Accessibility", &[Row::DamageTilt, Row::FovEffects]),
    ("Sound", &[]),
    ("Chat", &[]),
    ("Skin", &[]),
    ("Language", &[]),
];

/// All of it, for the title screen's Settings.
pub fn all_groups() -> Vec<Group> {
    let own: Group = ("Ruthenium", &[Row::FrameStats, Row::ReduceMotion]);
    std::iter::once(own).chain(GAME).collect()
}

/// The rows of some groups, in the order a screen counts them.
pub fn rows_of(groups: &[Group]) -> Vec<Row> {
    groups.iter().flat_map(|g| g.1).copied().collect()
}

/// The frame-rate slider's steps: 10, 20, ... 250, then Unlimited.
const FPS_STEPS: usize = (FPS_MAX / FPS_STEP) as usize + 1;

/// The longest Direct Connect address kept (vanilla's address field limit).
const MAX_SERVER: usize = ruthenium_net::server_list::MAX_ADDRESS;

impl Settings {
    /// Reads `text` (a `client.toml`); anything missing or unusable takes its default.
    pub fn parse(text: &str) -> Result<Settings, String> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let d = Settings::default();
        let section = |name: &str| table.get(name).and_then(toml::Value::as_table);
        let (display, interface, multiplayer, controls, game) = (
            section("display"),
            section("interface"),
            section("multiplayer"),
            section("controls"),
            section("game"),
        );
        fn get<'a>(t: Option<&'a toml::Table>, key: &str) -> Option<&'a toml::Value> {
            t.and_then(|t| t.get(key))
        }
        let flag = |t, key, default| {
            get(t, key)
                .and_then(toml::Value::as_bool)
                .unwrap_or(default)
        };
        let max_fps = match get(display, "max_fps") {
            Some(toml::Value::String(s)) if s == "unlimited" => None,
            Some(toml::Value::Integer(n)) => u32::try_from(*n)
                .ok()
                .map(|n| (n / FPS_STEP * FPS_STEP).clamp(FPS_STEP, FPS_MAX)),
            _ => d.max_fps,
        };
        let menu_size = get(interface, "menu_size")
            .and_then(toml::Value::as_str)
            .and_then(|s| MenuSize::ALL.into_iter().find(|m| m.key() == s))
            .unwrap_or(d.menu_size);
        let last_server = get(multiplayer, "last_server")
            .and_then(toml::Value::as_str)
            .filter(|s| s.encode_utf16().count() <= MAX_SERVER)
            .map_or(d.last_server, str::to_owned);
        let sensitivity = match get(controls, "mouse_sensitivity") {
            Some(toml::Value::Float(v)) => Some(*v),
            Some(toml::Value::Integer(n)) => Some(*n as f64),
            _ => None,
        }
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(d.mouse.sensitivity);
        let mouse = MouseOptions {
            sensitivity,
            invert_x: flag(controls, "invert_mouse_x", d.mouse.invert_x),
            invert_y: flag(controls, "invert_mouse_y", d.mouse.invert_y),
        };
        let number = |key, range: std::ops::RangeInclusive<u32>, default| {
            get(game, key)
                .and_then(toml::Value::as_integer)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| range.contains(n))
                .unwrap_or(default)
        };
        let g = d.game;
        let game = GameOptions {
            fov: number("fov", FOV_MIN..=FOV_MAX, g.fov),
            gui_scale: number("gui_scale", 0..=GUI_SCALE_MAX, g.gui_scale),
            view_bobbing: flag(game, "view_bobbing", g.view_bobbing),
            entity_shadows: flag(game, "entity_shadows", g.entity_shadows),
            vignette: flag(game, "vignette", g.vignette),
            attack_indicator: get(game, "attack_indicator")
                .and_then(toml::Value::as_str)
                .and_then(|s| INDICATORS.into_iter().find(|i| i.1 == s))
                .map_or(g.attack_indicator, |i| i.0),
            damage_tilt: number("damage_tilt", 0..=100, g.damage_tilt),
            fov_effects: number("fov_effects", 0..=100, g.fov_effects),
            clouds: get(game, "clouds")
                .and_then(toml::Value::as_str)
                .and_then(|s| CLOUDS.into_iter().find(|c| c.1 == s))
                .map_or(g.clouds, |c| c.0),
            cloud_distance: number(
                "cloud_distance",
                CLOUD_DISTANCE_MIN..=CLOUD_DISTANCE_MAX,
                g.cloud_distance,
            ),
        };
        Ok(Settings {
            game,
            vsync: flag(display, "vsync", d.vsync),
            max_fps,
            fullscreen: flag(display, "fullscreen", d.fullscreen),
            frame_stats: flag(display, "frame_stats", d.frame_stats),
            menu_size,
            reduce_motion: flag(interface, "reduce_motion", d.reduce_motion),
            last_server,
            mouse,
        })
    }

    /// Whether `text` sets the mouse's sensitivity (else it's imported from vanilla).
    pub fn has_mouse(text: &str) -> bool {
        text.parse::<toml::Table>().is_ok_and(|t| {
            t.get("controls")
                .and_then(toml::Value::as_table)
                .is_some_and(|c| c.contains_key("mouse_sensitivity"))
        })
    }

    /// What a screen of these groups shows, with these values.
    pub fn sections(&self, groups: &[Group]) -> Vec<SectionSpec> {
        groups
            .iter()
            .map(|(title, rows)| SectionSpec {
                title: (*title).into(),
                settings: rows.iter().map(|&r| self.spec(r)).collect(),
            })
            .collect()
    }

    /// One row as its screen starts with it.
    fn spec(&self, row: Row) -> SettingSpec {
        use ControlSpec::{Choice, Slider, Switch};
        let percents = |last: u32| (0..=last).map(|p| format!("{p}%")).collect();
        let (label, detail, control) = match row {
            Row::FrameStats => (
                "Frame Stats",
                "Frame rate and frame times in a corner; F3 too",
                Switch,
            ),
            Row::ReduceMotion => (
                "Reduce Motion",
                "Buttons and switches fade instead of moving",
                Switch,
            ),
            Row::Fov => (
                "FOV",
                "How much of the world is in view",
                Slider(
                    (FOV_MIN..=FOV_MAX)
                        .map(|f| match f {
                            FOV_NORMAL => "Normal".to_owned(),
                            FOV_MAX => "Quake Pro".to_owned(),
                            f => f.to_string(),
                        })
                        .collect(),
                ),
            ),
            Row::GuiScale => (
                "GUI Scale",
                "The HUD's size, as far as it fits",
                Slider(
                    std::iter::once("Auto".to_owned())
                        .chain((1..=GUI_SCALE_MAX).map(|s| s.to_string()))
                        .collect(),
                ),
            ),
            Row::MenuSize => (
                "Menu Size",
                "How large these menus are",
                Choice(MenuSize::ALL.iter().map(|m| m.label().to_owned()).collect()),
            ),
            Row::Vsync => (
                "VSync",
                "Waits for the display, so frames never tear",
                Switch,
            ),
            Row::MaxFps => (
                "Max Frame Rate",
                "The most frames drawn each second",
                Slider(
                    (1..FPS_STEPS as u32)
                        .map(|k| format!("{} FPS", k * FPS_STEP))
                        .chain(std::iter::once("Unlimited".to_owned()))
                        .collect(),
                ),
            ),
            Row::Fullscreen => (
                "Fullscreen",
                "Fills the screen; F11 switches it too",
                Switch,
            ),
            Row::AttackIndicator => (
                "Attack Indicator",
                "Where an attack's charge shows",
                Choice(INDICATORS.iter().map(|i| i.2.to_owned()).collect()),
            ),
            Row::ViewBobbing => ("View Bobbing", "The view sways with your steps", Switch),
            Row::EntityShadows => ("Entity Shadows", "Shadows under players", Switch),
            Row::Vignette => ("Vignette", "The corners darken in the dark", Switch),
            Row::Sensitivity => (
                "Sensitivity",
                "How far the mouse turns the view",
                Slider(percents(SENSITIVITY_STEPS as u32)),
            ),
            Row::InvertX => ("Invert Mouse X", "Left turns right", Switch),
            Row::InvertY => ("Invert Mouse Y", "Up looks down", Switch),
            Row::DamageTilt => (
                "Damage Tilt",
                "How far the view tilts when you are hurt",
                Slider(percents(100)),
            ),
            Row::Clouds => (
                "Clouds",
                "Flat, or with sides and depth",
                Choice(CLOUDS.iter().map(|c| c.2.to_owned()).collect()),
            ),
            Row::CloudDistance => (
                "Cloud Distance",
                "How far out clouds are drawn",
                Slider(
                    (CLOUD_DISTANCE_MIN..=CLOUD_DISTANCE_MAX)
                        .map(|c| format!("{c} chunks"))
                        .collect(),
                ),
            ),
            Row::FovEffects => (
                "FOV Effects",
                "How far speed and fluids change the view",
                Slider(percents(100)),
            ),
        };
        SettingSpec {
            label: label.into(),
            detail: detail.into(),
            control,
            value: self.get(row),
        }
    }

    /// A row's value as its screen counts it.
    pub fn get(&self, row: Row) -> usize {
        let g = &self.game;
        match row {
            Row::FrameStats => self.frame_stats.into(),
            Row::ReduceMotion => self.reduce_motion.into(),
            Row::Fov => g.fov.clamp(FOV_MIN, FOV_MAX).abs_diff(FOV_MIN) as usize,
            Row::GuiScale => g.gui_scale as usize,
            Row::MenuSize => MenuSize::ALL
                .iter()
                .position(|&m| m == self.menu_size)
                .unwrap_or(1),
            Row::Vsync => self.vsync.into(),
            Row::MaxFps => self
                .max_fps
                .map_or(FPS_STEPS - 1, |f| (f / FPS_STEP) as usize - 1),
            Row::Fullscreen => self.fullscreen.into(),
            Row::AttackIndicator => INDICATORS
                .iter()
                .position(|p| p.0 == g.attack_indicator)
                .unwrap_or(0),
            Row::ViewBobbing => g.view_bobbing.into(),
            Row::EntityShadows => g.entity_shadows.into(),
            Row::Vignette => g.vignette.into(),
            // (Vanilla's own may lie between two steps: the nearest shows.)
            Row::Sensitivity => {
                (self.mouse.sensitivity * SENSITIVITY_STEPS as f64).round() as usize
            }
            Row::InvertX => self.mouse.invert_x.into(),
            Row::InvertY => self.mouse.invert_y.into(),
            Row::DamageTilt => g.damage_tilt as usize,
            Row::FovEffects => g.fov_effects as usize,
            Row::Clouds => CLOUDS.iter().position(|c| c.0 == g.clouds).unwrap_or(2),
            Row::CloudDistance => g
                .cloud_distance
                .clamp(CLOUD_DISTANCE_MIN, CLOUD_DISTANCE_MAX)
                .abs_diff(CLOUD_DISTANCE_MIN) as usize,
        }
    }

    /// Sets a row from its screen's `value`.
    pub fn set(&mut self, row: Row, value: usize) {
        let g = &mut self.game;
        let percent = (value as u32).min(100);
        match row {
            Row::FrameStats => self.frame_stats = value != 0,
            Row::ReduceMotion => self.reduce_motion = value != 0,
            Row::Fov => g.fov = (FOV_MIN + value as u32).min(FOV_MAX),
            Row::GuiScale => g.gui_scale = (value as u32).min(GUI_SCALE_MAX),
            Row::MenuSize => {
                if let Some(&m) = MenuSize::ALL.get(value) {
                    self.menu_size = m;
                }
            }
            Row::Vsync => self.vsync = value != 0,
            Row::MaxFps => {
                self.max_fps = (value + 1 < FPS_STEPS).then(|| (value as u32 + 1) * FPS_STEP);
            }
            Row::Fullscreen => self.fullscreen = value != 0,
            Row::AttackIndicator => {
                if let Some(p) = INDICATORS.get(value) {
                    g.attack_indicator = p.0;
                }
            }
            Row::ViewBobbing => g.view_bobbing = value != 0,
            Row::EntityShadows => g.entity_shadows = value != 0,
            Row::Vignette => g.vignette = value != 0,
            Row::Sensitivity => {
                let step = value.min(SENSITIVITY_STEPS);
                self.mouse.sensitivity = step as f64 / SENSITIVITY_STEPS as f64;
            }
            Row::InvertX => self.mouse.invert_x = value != 0,
            Row::InvertY => self.mouse.invert_y = value != 0,
            Row::DamageTilt => g.damage_tilt = percent,
            Row::FovEffects => g.fov_effects = percent,
            Row::Clouds => {
                if let Some(c) = CLOUDS.get(value) {
                    g.clouds = c.0;
                }
            }
            Row::CloudDistance => {
                g.cloud_distance = (CLOUD_DISTANCE_MIN + value as u32).min(CLOUD_DISTANCE_MAX);
            }
        }
    }

    /// The file's text.
    pub fn to_toml(&self) -> String {
        let max_fps = self
            .max_fps
            .map_or_else(|| "\"unlimited\"".to_owned(), |n| n.to_string());
        let last_server = toml::Value::String(self.last_server.clone()).to_string();
        format!(
            "# Ruthenium's settings, written by its Settings screen. Values that can't be\n\
             # read fall back to their defaults.\n\
             \n\
             [display]\n\
             vsync = {}\n\
             # {FPS_STEP} to {FPS_MAX} in steps of {FPS_STEP}, or \"unlimited\".\n\
             max_fps = {max_fps}\n\
             fullscreen = {}\n\
             frame_stats = {}\n\
             \n\
             [interface]\n\
             # \"small\", \"default\", \"large\" or \"larger\".\n\
             menu_size = \"{}\"\n\
             reduce_motion = {}\n\
             \n\
             [multiplayer]\n\
             last_server = {last_server}\n\
             \n\
             [controls]\n\
             # Vanilla's mouseSensitivity (0 to 1; 0.5 is vanilla's 100%), exactly: the\n\
             # camera turns as vanilla's would at the same value.\n\
             mouse_sensitivity = {:?}\n\
             invert_mouse_x = {}\n\
             invert_mouse_y = {}\n\
             \n\
             [game]\n\
             # {FOV_MIN} to {FOV_MAX} degrees.\n\
             fov = {}\n\
             # 1 to {GUI_SCALE_MAX}, or 0: as large as fits.\n\
             gui_scale = {}\n\
             view_bobbing = {}\n\
             entity_shadows = {}\n\
             vignette = {}\n\
             # \"off\", \"crosshair\" or \"hotbar\".\n\
             attack_indicator = \"{}\"\n\
             # Percent.\n\
             damage_tilt = {}\n\
             fov_effects = {}\n\
             # \"off\", \"fast\" or \"fancy\"; {CLOUD_DISTANCE_MIN} to {CLOUD_DISTANCE_MAX} chunks.\n\
             clouds = \"{}\"\n\
             cloud_distance = {}\n",
            self.vsync,
            self.fullscreen,
            self.frame_stats,
            self.menu_size.key(),
            self.reduce_motion,
            self.mouse.sensitivity,
            self.mouse.invert_x,
            self.mouse.invert_y,
            self.game.fov,
            self.game.gui_scale,
            self.game.view_bobbing,
            self.game.entity_shadows,
            self.game.vignette,
            INDICATORS[self.get(Row::AttackIndicator)].1,
            self.game.damage_tilt,
            self.game.fov_effects,
            CLOUDS[self.get(Row::Clouds)].1,
            self.game.cloud_distance,
        )
    }
}

/// The settings and the file they live in.
#[derive(Debug)]
pub struct SettingsFile {
    pub settings: Settings,
    /// `None`: nothing is read or written (benchmarks, screenshots, no data folder).
    path: Option<PathBuf>,
    /// What the file holds (or would hold): saving is skipped when nothing changed.
    saved: Settings,
    /// Whether the file set the mouse's sensitivity.
    has_mouse: bool,
}

impl SettingsFile {
    /// `client.toml` in `data_dir`'s `config` folder.
    pub fn path_in(data_dir: &Path) -> PathBuf {
        data_dir.join("config").join("client.toml")
    }

    /// Defaults, never saved.
    pub fn transient() -> SettingsFile {
        SettingsFile {
            settings: Settings::default(),
            path: None,
            saved: Settings::default(),
            has_mouse: true,
        }
    }

    /// Reads `path`; a missing file gives the defaults, a broken one too (with a warning).
    pub fn load(path: PathBuf) -> SettingsFile {
        let settings = match std::fs::read_to_string(&path) {
            Ok(text) => Settings::parse(&text).unwrap_or_else(|e| {
                eprintln!(
                    "[ruthenium] {}: {e}; using the default settings",
                    path.display()
                );
                Settings::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                eprintln!(
                    "[ruthenium] could not read {}: {e}; using the default settings",
                    path.display()
                );
                Settings::default()
            }
        };
        let has_mouse = std::fs::read_to_string(&path).is_ok_and(|t| Settings::has_mouse(&t));
        SettingsFile {
            saved: settings.clone(),
            settings,
            path: Some(path),
            has_mouse,
        }
    }

    /// Takes the mouse options from `import` (vanilla's) if the file didn't set them;
    /// they are saved with the next change, or on quitting.
    pub fn import_mouse(&mut self, import: impl FnOnce() -> Option<(MouseOptions, PathBuf)>) {
        if self.has_mouse {
            return;
        }
        self.has_mouse = true;
        match import() {
            Some((mouse, from)) => {
                eprintln!(
                    "[ruthenium] mouse sensitivity {:?}{}{} from {}",
                    mouse.sensitivity,
                    if mouse.invert_x { ", X inverted" } else { "" },
                    if mouse.invert_y { ", Y inverted" } else { "" },
                    from.display()
                );
                self.settings.mouse = mouse;
            }
            None => eprintln!(
                "[ruthenium] no vanilla options.txt found; mouse sensitivity at vanilla's default"
            ),
        }
        // Written even when it's the default, so the import happens once.
        self.saved.mouse.sensitivity = f64::NAN;
    }

    /// Writes the settings if they changed since they were read or last saved.
    pub fn save(&mut self) {
        if self.settings == self.saved {
            return;
        }
        let Some(path) = &self.path else {
            self.saved = self.settings.clone();
            return;
        };
        match write_atomically(path, &self.settings.to_toml()) {
            Ok(()) => self.saved = self.settings.clone(),
            Err(e) => eprintln!("[ruthenium] could not save {}: {e}", path.display()),
        }
    }
}

/// Writes `text` beside `path`, keeps the old file as `<name>_old`, then moves the new
/// one into place.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map_or_else(
        || "client.toml".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let new = path.with_file_name(format!("{name}_new"));
    let old = path.with_file_name(format!("{name}_old"));
    std::fs::write(&new, text)?;
    if path.exists() {
        std::fs::rename(path, &old)?;
    }
    if let Err(e) = std::fs::rename(&new, path) {
        let _ = std::fs::rename(&old, path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file() {
        let s = Settings {
            vsync: false,
            max_fps: Some(140),
            fullscreen: true,
            frame_stats: true,
            menu_size: MenuSize::Larger,
            reduce_motion: true,
            last_server: "play.example.com:25566 \"quoted\"".into(),
            mouse: MouseOptions {
                sensitivity: 0.27464788732394363,
                invert_x: false,
                invert_y: true,
            },
            game: GameOptions {
                fov: 95,
                gui_scale: 3,
                view_bobbing: false,
                entity_shadows: false,
                vignette: false,
                attack_indicator: AttackIndicator::Hotbar,
                damage_tilt: 35,
                fov_effects: 0,
                clouds: CloudMode::Fast,
                cloud_distance: 12,
            },
        };
        assert_eq!(Settings::parse(&s.to_toml()), Ok(s.clone()));
        assert!(Settings::has_mouse(&s.to_toml()));
        assert!(!Settings::has_mouse(
            "[display]
vsync = true
"
        ));
        // Every double comes back bit for bit, however it prints.
        for v in [0.0, 1.0, 1.0e-5, 0.1 + 0.2, 0.5000000000000001, 5e-324] {
            let mut t = s.clone();
            t.mouse.sensitivity = v;
            let back = Settings::parse(&t.to_toml()).unwrap().mouse.sensitivity;
            assert_eq!(back.to_bits(), v.to_bits(), "{v:?}");
        }
        let d = Settings::default();
        assert_eq!(Settings::parse(&d.to_toml()), Ok(d));
    }

    #[test]
    fn screen_values_map_to_settings_and_back() {
        let mut s = Settings::default();
        let rows = rows_of(&all_groups());
        // Every row keeps what it is set to, and each is a row of its own.
        for (i, &row) in rows.iter().enumerate() {
            let others: Vec<usize> = rows.iter().map(|&r| s.get(r)).collect();
            let new = usize::from(others[i] == 0);
            s.set(row, new);
            for (j, &other) in rows.iter().enumerate() {
                let want = if i == j { new } else { others[j] };
                assert_eq!(s.get(other), want, "{row:?} {other:?}");
            }
        }
        let mut s = Settings::default();
        assert_eq!(
            s.get(Row::MaxFps),
            FPS_STEPS - 1,
            "unlimited is the last step"
        );
        s.set(Row::MaxFps, 0);
        assert_eq!(s.max_fps, Some(10));
        s.set(Row::MaxFps, 11);
        assert_eq!((s.max_fps, s.get(Row::MaxFps)), (Some(120), 11));
        s.set(Row::MaxFps, FPS_STEPS - 1);
        assert_eq!(s.max_fps, None);
        s.set(Row::MenuSize, 3);
        assert_eq!(s.menu_size, MenuSize::Larger);
        assert_eq!(s.get(Row::Fov), 40);
        s.set(Row::Fov, 80);
        assert_eq!(s.game.fov, 110);
        s.set(Row::Fov, 999);
        s.set(Row::DamageTilt, 999);
        assert_eq!((s.game.fov, s.game.damage_tilt), (FOV_MAX, 100));
        // Vanilla's 100% is a half; one between two steps stays as it is until it is set.
        assert_eq!(s.get(Row::Sensitivity), 100);
        s.mouse.sensitivity = 0.274_647_887_323_943_63;
        assert_eq!(s.get(Row::Sensitivity), 55);
        s.set(Row::Sensitivity, 200);
        assert_eq!(s.mouse.sensitivity, 1.0);
        // Out of range in the file.
        let s = Settings::parse(
            "[game]\nfov = 111\ngui_scale = -1\nvignette = 3\nattack_indicator = \"x\"\n\
             damage_tilt = 101\nfov_effects = \"all\"\nclouds = true\ncloud_distance = 129\n",
        )
        .unwrap();
        assert_eq!(s.game, GameOptions::default());
    }

    #[test]
    fn sliders_have_a_label_for_every_step() {
        let s = Settings::default();
        let labels = |row| match s.spec(row).control {
            ControlSpec::Slider(l) => l,
            other => panic!("{other:?}"),
        };
        let fov = labels(Row::Fov);
        assert_eq!([&fov[0], &fov[40], &fov[80]], ["30", "Normal", "Quake Pro"]);
        let scale = labels(Row::GuiScale);
        assert_eq!((scale[0].as_str(), scale.len()), ("Auto", 9));
        let fps = labels(Row::MaxFps);
        assert_eq!((fps[0].as_str(), fps.len()), ("10 FPS", FPS_STEPS));
        assert_eq!(labels(Row::Sensitivity).len(), 201);
        assert_eq!(labels(Row::FovEffects)[100], "100%");
        // What is shown first is what is set.
        for row in rows_of(&all_groups()) {
            let spec = s.spec(row);
            let steps = match &spec.control {
                ControlSpec::Switch => 2,
                ControlSpec::Slider(l) | ControlSpec::Choice(l) => l.len(),
            };
            assert!(spec.value < steps && !spec.label.is_empty(), "{row:?}");
        }
    }

    #[test]
    fn what_vanilla_has_is_the_games_and_the_rest_ruthenium_s() {
        let (own, game) = (rows_of(&RUTHENIUM), rows_of(&GAME));
        assert_eq!(own, [Row::FrameStats, Row::ReduceMotion]);
        for row in [Row::MenuSize, Row::Vsync, Row::MaxFps, Row::Fullscreen] {
            assert!(game.contains(&row) && !own.contains(&row));
        }
        // The title screen has all of them, each once.
        let all = rows_of(&all_groups());
        assert_eq!(all.len(), own.len() + game.len());
        for (i, r) in all.iter().enumerate() {
            assert!(!all[..i].contains(r), "{r:?}");
            assert!(own.contains(r) || game.contains(r));
        }
        let titles: Vec<&str> = GAME.iter().map(|g| g.0).collect();
        let first = ["Video", "Controls", "Online", "Resources", "Accessibility"];
        assert_eq!(titles[..5], first);
    }

    #[test]
    fn unusable_values_take_their_defaults() {
        let s = Settings::parse(
            "[display]\nvsync = \"yes\"\nmax_fps = 1000\nfullscreen = true\n\
             [interface]\nmenu_size = \"huge\"\n[multiplayer]\nlast_server = 7\n",
        )
        .unwrap();
        assert!(s.vsync && s.fullscreen);
        assert_eq!(s.max_fps, Some(FPS_MAX));
        assert_eq!(s.menu_size, MenuSize::Default);
        assert_eq!(s.last_server, "");
        // Odd rates snap down to the slider's steps; tiny ones go up to the first.
        let rate = |t: &str| Settings::parse(t).unwrap().max_fps;
        assert_eq!(rate("[display]\nmax_fps = 145"), Some(140));
        assert_eq!(rate("[display]\nmax_fps = 3"), Some(FPS_STEP));
        assert_eq!(rate("[display]\nmax_fps = -5"), None);
        assert_eq!(rate(""), None);
        assert!(Settings::parse("[display\n").is_err());
    }

    #[test]
    fn saves_only_changes_and_keeps_the_old_file() {
        let dir = std::env::temp_dir().join(format!("ruthenium-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = SettingsFile::path_in(&dir);
        let mut f = SettingsFile::load(path.clone());
        assert_eq!(f.settings, Settings::default());
        f.save();
        assert!(!path.exists(), "nothing changed, nothing written");
        f.settings.vsync = false;
        f.save();
        assert!(!SettingsFile::load(path.clone()).settings.vsync);
        f.settings.max_fps = Some(60);
        f.save();
        let old = std::fs::read_to_string(path.with_file_name("client.toml_old")).unwrap();
        assert!(old.contains("max_fps = \"unlimited\""));
        assert_eq!(SettingsFile::load(path).settings.max_fps, Some(60));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
