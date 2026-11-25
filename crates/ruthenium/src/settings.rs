//! The player's settings: what the Settings screen shows, kept in
//! `config/client.toml` in Ruthenium's data folder.
//!
//! Reading is forgiving: a missing file, key or section, or a value of the wrong type or
//! out of range, takes the default (a broken file is reported once). Saving writes the
//! whole file, with comments, to a new file beside it, then moves the previous one to
//! `client.toml_old` and the new one into place, as `servers.dat` is saved.

use std::path::{Path, PathBuf};

use ruthenium_audio::{SoundSource, Volumes};
use ruthenium_protocol::text::Lang;
use ruthenium_render::clouds::CloudMode;
use ruthenium_render::weather;
use ruthenium_sim::input::MouseOptions;
use ruthenium_sim::keybinds::{self, Action, Category, Key, KeyBinds};
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
    /// How far from the camera rain and snow fall, in blocks.
    pub weather_radius: u32,
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
            weather_radius: weather::MAX_RADIUS,
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
    /// The key bindings, and whether sneak and sprint toggle instead of being held
    /// (vanilla's `toggleCrouch`, `toggleSprint`). Imported from vanilla's `options.txt`
    /// when the file has no bindings.
    pub keys: KeyBinds,
    pub toggle_sneak: bool,
    pub toggle_sprint: bool,
    /// The volume sliders. Imported from vanilla's `options.txt` when the file has none.
    pub sound: SoundSettings,
}

/// Vanilla's volume sliders (`soundCategory_*`): any value from 0 to 1, kept exactly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundSettings {
    pub volumes: [f64; SoundSource::COUNT],
}

impl Default for SoundSettings {
    fn default() -> SoundSettings {
        SoundSettings {
            volumes: [1.0; SoundSource::COUNT],
        }
    }
}

impl SoundSettings {
    /// As the engine takes them (vanilla's `floatValue`).
    pub fn volumes(&self) -> Volumes {
        Volumes(self.volumes.map(|v| v as f32))
    }

    fn parse(t: Option<&toml::Table>) -> SoundSettings {
        let mut s = SoundSettings::default();
        for source in SoundSource::ALL {
            let v = match t.and_then(|t| t.get(source.name())) {
                Some(toml::Value::Float(v)) => *v,
                Some(toml::Value::Integer(n)) => *n as f64,
                _ => continue,
            };
            if (0.0..=1.0).contains(&v) {
                s.volumes[source.index()] = v;
            }
        }
        s
    }

    fn toml_section(&self) -> String {
        let mut s =
            "\n[sound]\n# Each category's volume, 0 to 1, as vanilla's soundCategory_ options.\n"
                .to_owned();
        for source in SoundSource::ALL {
            s.push_str(&format!(
                "{} = {:?}\n",
                source.name(),
                self.volumes[source.index()]
            ));
        }
        s
    }
}

/// A volume slider's label and what it says it does.
fn volume_label(s: SoundSource) -> (&'static str, &'static str) {
    match s {
        SoundSource::Master => ("Master Volume", "Every sound"),
        SoundSource::Music => ("Music", "The game's music"),
        SoundSource::Records => ("Jukebox & Note Blocks", "Records and note blocks"),
        SoundSource::Weather => ("Weather", "Rain and thunder"),
        SoundSource::Blocks => ("Blocks", "Blocks placed, broken and used"),
        SoundSource::Hostile => ("Hostile Creatures", "Monsters"),
        SoundSource::Neutral => ("Friendly Creatures", "Animals and villagers"),
        SoundSource::Players => ("Players", "Steps, hits and eating"),
        SoundSource::Ambient => ("Ambient", "Caves and surroundings"),
        SoundSource::Voice => ("Voice", "The narrator"),
        SoundSource::Ui => ("Interface", "Menu clicks"),
    }
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
            keys: KeyBinds::DEFAULT,
            toggle_sneak: false,
            toggle_sprint: false,
            sound: SoundSettings::default(),
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
    ToggleSneak,
    ToggleSprint,
    DamageTilt,
    FovEffects,
    Clouds,
    CloudDistance,
    WeatherRadius,
    /// A key binding.
    Key(Action),
    /// The heading over a category of key bindings.
    KeyHeading(Category),
    /// Reset Keys: every binding back to its default.
    ResetKeys,
    /// A category's volume.
    Volume(SoundSource),
}

/// The Controls page: the mouse, Sneak and Sprint (hold or toggle), Reset Keys, then
/// every key binding under its heading in the order vanilla lists them.
const CONTROLS: [Row; 6 + Category::ALL.len() + keybinds::COUNT] = {
    let mut rows = [Row::Sensitivity; 6 + Category::ALL.len() + keybinds::COUNT];
    rows[1] = Row::InvertX;
    rows[2] = Row::InvertY;
    rows[3] = Row::ToggleSneak;
    rows[4] = Row::ToggleSprint;
    rows[5] = Row::ResetKeys;
    let (mut n, mut i) = (6, 0);
    let mut last = None;
    while i < keybinds::COUNT {
        let a = Action::SORTED[i];
        let c = a.category();
        let new = match last {
            Some(l) => l as u8 != c as u8,
            None => true,
        };
        if new {
            rows[n] = Row::KeyHeading(c);
            n += 1;
            last = Some(c);
        }
        rows[n] = Row::Key(a);
        n += 1;
        i += 1;
    }
    rows
};

/// What the Controls page names keys and bindings with: the game's language where it
/// is loaded (else Ruthenium's own words), and what the keyboard's layout calls a key
/// (by scancode).
#[derive(Clone, Copy)]
#[allow(missing_debug_implementations)]
pub struct KeyNaming<'a> {
    pub lang: Option<&'a dyn Lang>,
    pub system: &'a dyn Fn(i32) -> Option<String>,
}

impl KeyNaming<'_> {
    /// No language, no layout.
    pub const PLAIN: KeyNaming<'static> = KeyNaming {
        lang: None,
        system: &|_| None,
    };

    fn action(&self, a: Action) -> String {
        self.lang
            .and_then(|l| l.get(a.name()))
            .unwrap_or(a.label())
            .to_owned()
    }

    fn category(&self, c: Category) -> String {
        self.lang
            .and_then(|l| l.get(&c.lang_key()))
            .unwrap_or(c.label())
            .to_owned()
    }

    fn key(&self, key: Key) -> String {
        let system = match key {
            Key::Keyboard(v) => (self.system)(v),
            Key::Mouse(_) => None,
        };
        keybinds::key_display(key, system.as_deref(), self.lang)
    }
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
            Row::WeatherRadius,
            Row::Vignette,
        ],
    ),
    ("Controls", &CONTROLS),
    ("Online", &[]),
    ("Resources", &[]),
    ("Accessibility", &[Row::DamageTilt, Row::FovEffects]),
    (
        "Sound",
        &[
            Row::Volume(SoundSource::Master),
            Row::Volume(SoundSource::Music),
            Row::Volume(SoundSource::Records),
            Row::Volume(SoundSource::Weather),
            Row::Volume(SoundSource::Blocks),
            Row::Volume(SoundSource::Hostile),
            Row::Volume(SoundSource::Neutral),
            Row::Volume(SoundSource::Players),
            Row::Volume(SoundSource::Ambient),
            Row::Volume(SoundSource::Voice),
            Row::Volume(SoundSource::Ui),
        ],
    ),
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
        let sound = SoundSettings::parse(section("sound"));
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
            weather_radius: number(
                "weather_radius",
                weather::MIN_RADIUS..=weather::MAX_RADIUS,
                g.weather_radius,
            ),
        };
        let toggle_sneak = flag(controls, "toggle_sneak", d.toggle_sneak);
        let toggle_sprint = flag(controls, "toggle_sprint", d.toggle_sprint);
        let mut keys = d.keys;
        if let Some(t) = section("keys") {
            // (`key.forward = ...` without quotes is a dotted key: tables in tables.)
            let mut named = Vec::new();
            flatten(t, "", &mut named);
            for (action, key) in named {
                if let (Some(a), keybinds::Parsed::Key(k)) =
                    (Action::by_name(&action), Key::parse(key))
                {
                    keys.set(a, k);
                }
            }
        }
        Ok(Settings {
            game,
            keys,
            toggle_sneak,
            toggle_sprint,
            vsync: flag(display, "vsync", d.vsync),
            max_fps,
            fullscreen: flag(display, "fullscreen", d.fullscreen),
            frame_stats: flag(display, "frame_stats", d.frame_stats),
            menu_size,
            reduce_motion: flag(interface, "reduce_motion", d.reduce_motion),
            last_server,
            mouse,
            sound,
        })
    }

    /// Whether `text` has the volumes (else they're imported from vanilla).
    pub fn has_sound(text: &str) -> bool {
        text.parse::<toml::Table>()
            .is_ok_and(|t| t.get("sound").and_then(toml::Value::as_table).is_some())
    }

    /// Whether `text` sets the mouse's sensitivity (else it's imported from vanilla).
    pub fn has_mouse(text: &str) -> bool {
        text.parse::<toml::Table>().is_ok_and(|t| {
            t.get("controls")
                .and_then(toml::Value::as_table)
                .is_some_and(|c| c.contains_key("mouse_sensitivity"))
        })
    }

    /// Whether `text` has the key bindings (else they're imported from vanilla).
    pub fn has_keys(text: &str) -> bool {
        text.parse::<toml::Table>()
            .is_ok_and(|t| t.get("keys").is_some_and(toml::Value::is_table))
    }

    /// What a screen of these groups shows, with these values (keys named in
    /// Ruthenium's own words).
    #[allow(dead_code)]
    pub fn sections(&self, groups: &[Group]) -> Vec<SectionSpec> {
        self.sections_named(groups, &KeyNaming::PLAIN)
    }

    /// The same, with keys and bindings named by `names`.
    pub fn sections_named(&self, groups: &[Group], names: &KeyNaming<'_>) -> Vec<SectionSpec> {
        groups
            .iter()
            .map(|(title, rows)| SectionSpec {
                title: (*title).into(),
                settings: rows
                    .iter()
                    .map(|&r| self.key_spec(r, names).unwrap_or_else(|| self.spec(r)))
                    .collect(),
            })
            .collect()
    }

    /// A key binding's row, a heading or Reset Keys as it shows now.
    pub fn key_spec(&self, row: Row, names: &KeyNaming<'_>) -> Option<SettingSpec> {
        let keys = &self.keys;
        let (label, detail, control) = match row {
            Row::Key(a) => {
                let others: Vec<String> = keys.conflicts(a).map(|o| names.action(o)).collect();
                let detail = if others.is_empty() {
                    String::new()
                } else {
                    format!("Also used by {}", others.join(", "))
                };
                let control = ControlSpec::Key {
                    key: names.key(keys.get(a)),
                    conflict: !others.is_empty(),
                    reset: !keys.is_default(a),
                };
                (names.action(a), detail, control)
            }
            Row::KeyHeading(c) => (names.category(c), String::new(), ControlSpec::Heading),
            Row::ResetKeys => {
                let changed = keys.changed();
                let detail = match changed {
                    0 => "Click one, then press a key or button; Esc unbinds".to_owned(),
                    1 => "1 key differs from the default".to_owned(),
                    n => format!("{n} keys differ from the defaults"),
                };
                let control = ControlSpec::Button {
                    label: "Reset Keys".into(),
                    enabled: changed > 0,
                };
                ("Key Binds".to_owned(), detail, control)
            }
            _ => return None,
        };
        Some(SettingSpec {
            label,
            detail,
            control,
            value: 0,
        })
    }

    /// One row as its screen starts with it.
    fn spec(&self, row: Row) -> SettingSpec {
        use ControlSpec::{Choice, Slider, Switch};
        if let Some(spec) = self.key_spec(row, &KeyNaming::PLAIN) {
            return spec;
        }
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
            Row::ToggleSneak | Row::ToggleSprint => (
                if row == Row::ToggleSneak {
                    "Sneak"
                } else {
                    "Sprint"
                },
                "Hold its key, or press it once to start and again to stop",
                Choice(vec!["Hold".into(), "Toggle".into()]),
            ),
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
            Row::WeatherRadius => (
                "Weather Radius",
                "How far around you rain and snow fall",
                Slider(
                    (weather::MIN_RADIUS..=weather::MAX_RADIUS)
                        .map(|b| format!("{b} blocks"))
                        .collect(),
                ),
            ),
            Row::FovEffects => (
                "FOV Effects",
                "How far speed and fluids change the view",
                Slider(percents(100)),
            ),
            // Above.
            Row::Key(_) | Row::KeyHeading(_) | Row::ResetKeys => ("", "", Switch),
            Row::Volume(s) => {
                let (label, detail) = volume_label(s);
                // `percentValueOrOffLabel`.
                let steps = std::iter::once("Off".to_owned())
                    .chain((1..=100).map(|p| format!("{p}%")))
                    .collect();
                (label, detail, Slider(steps))
            }
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
            Row::ToggleSneak => self.toggle_sneak.into(),
            Row::ToggleSprint => self.toggle_sprint.into(),
            Row::DamageTilt => g.damage_tilt as usize,
            Row::FovEffects => g.fov_effects as usize,
            Row::Clouds => CLOUDS.iter().position(|c| c.0 == g.clouds).unwrap_or(2),
            Row::CloudDistance => g
                .cloud_distance
                .clamp(CLOUD_DISTANCE_MIN, CLOUD_DISTANCE_MAX)
                .abs_diff(CLOUD_DISTANCE_MIN) as usize,
            Row::WeatherRadius => g
                .weather_radius
                .clamp(weather::MIN_RADIUS, weather::MAX_RADIUS)
                .abs_diff(weather::MIN_RADIUS) as usize,
            // Not values: they act (see `SettingsEvent::Pressed`).
            Row::Key(_) | Row::KeyHeading(_) | Row::ResetKeys => 0,
            // (Vanilla's own may lie between two steps: the nearest shows.)
            Row::Volume(s) => (self.sound.volumes[s.index()] * 100.0).round() as usize,
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
            Row::ToggleSneak => self.toggle_sneak = value != 0,
            Row::ToggleSprint => self.toggle_sprint = value != 0,
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
            Row::WeatherRadius => {
                g.weather_radius = (weather::MIN_RADIUS + value as u32).min(weather::MAX_RADIUS);
            }
            Row::Key(_) | Row::KeyHeading(_) | Row::ResetKeys => {}
            Row::Volume(s) => self.sound.volumes[s.index()] = f64::from(percent) / 100.0,
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
             # Whether Sneak and Sprint toggle instead of being held.\n\
             toggle_sneak = {}\n\
             toggle_sprint = {}\n\
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
             cloud_distance = {}\n\
             # {} to {} blocks.\n\
             weather_radius = {}\n\
             \n\
             [keys]\n\
             # Each of vanilla's key bindings by its name, and its key as options.txt names\n\
             # them (\"key.keyboard.w\", \"key.mouse.left\", \"key.mouse.4\", \"key.keyboard.unknown\"\n\
             # for none). Escape always opens the menu, whatever is bound. An empty [keys]\n\
             # puts every key back to its default; without [keys] they are taken from\n\
             # vanilla's options.txt again.\n\
             {}",
            self.vsync,
            self.fullscreen,
            self.frame_stats,
            self.menu_size.key(),
            self.reduce_motion,
            self.mouse.sensitivity,
            self.mouse.invert_x,
            self.mouse.invert_y,
            self.toggle_sneak,
            self.toggle_sprint,
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
            weather::MIN_RADIUS,
            weather::MAX_RADIUS,
            self.game.weather_radius,
            self.keys_toml(),
        ) + &self.sound.toml_section()
    }

    /// The `[keys]` lines: every binding, in `options.txt`'s order.
    fn keys_toml(&self) -> String {
        Action::ALL
            .iter()
            .map(|&a| {
                let name = toml::Value::String(a.name().to_owned());
                let key = toml::Value::String(self.keys.get(a).name());
                format!("{name} = {key}\n")
            })
            .collect()
    }
}

/// What is imported with the key bindings from vanilla's `options.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VanillaKeys {
    pub keys: KeyBinds,
    pub toggle_sneak: bool,
    pub toggle_sprint: bool,
}

/// A table's string values by their dotted names (`a.b = "x"` and `"a.b" = "x"` alike).
fn flatten<'a>(table: &'a toml::Table, prefix: &str, out: &mut Vec<(String, &'a str)>) {
    for (k, v) in table {
        let name = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(s) => out.push((name, s)),
            toml::Value::Table(t) => flatten(t, &name, out),
            _ => {}
        }
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
    /// Whether the file set the mouse's sensitivity, and the key bindings.
    has_mouse: bool,
    has_keys: bool,
    /// Whether the file had the volumes.
    has_sound: bool,
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
            has_keys: true,
            has_sound: true,
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
        let text = std::fs::read_to_string(&path);
        let has_mouse = text.as_ref().is_ok_and(|t| Settings::has_mouse(t));
        let has_keys = text.as_ref().is_ok_and(|t| Settings::has_keys(t));
        let has_sound = text.as_ref().is_ok_and(|t| Settings::has_sound(t));
        SettingsFile {
            saved: settings.clone(),
            settings,
            path: Some(path),
            has_mouse,
            has_keys,
            has_sound,
        }
    }

    /// Takes the key bindings, and whether sneak and sprint toggle, from `import`
    /// (vanilla's) if the file had no bindings; they are saved with the next change, or
    /// on quitting.
    pub fn import_keys(&mut self, import: impl FnOnce() -> Option<(VanillaKeys, PathBuf)>) {
        if self.has_keys {
            return;
        }
        self.has_keys = true;
        match import() {
            Some((v, from)) => {
                eprintln!(
                    "[ruthenium] key bindings ({} not vanilla's defaults{}{}) from {}",
                    v.keys.changed(),
                    if v.toggle_sneak {
                        ", sneak toggles"
                    } else {
                        ""
                    },
                    if v.toggle_sprint {
                        ", sprint toggles"
                    } else {
                        ""
                    },
                    from.display()
                );
                self.settings.keys = v.keys;
                self.settings.toggle_sneak = v.toggle_sneak;
                self.settings.toggle_sprint = v.toggle_sprint;
            }
            None => eprintln!("[ruthenium] no key bindings in vanilla's options.txt; the defaults"),
        }
        // Written even when they are the defaults, so the import happens once.
        self.saved.keys.set(Action::Attack, Key::Mouse(i32::MIN));
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

    /// Takes the volumes from `import` (vanilla's) if the file didn't have them; they are
    /// saved with the next change, or on quitting.
    pub fn import_sound(&mut self, import: impl FnOnce() -> Option<(SoundSettings, PathBuf)>) {
        if self.has_sound {
            return;
        }
        self.has_sound = true;
        if let Some((sound, from)) = import() {
            eprintln!("[ruthenium] volumes from {}", from.display());
            self.settings.sound = sound;
        }
        // Written even when it's the default, so the import happens once.
        self.saved.sound.volumes[0] = f64::NAN;
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
                weather_radius: 5,
            },
            keys: {
                let mut k = KeyBinds::DEFAULT;
                k.set(Action::Attack, Key::Keyboard(13));
                k.set(Action::Use, Key::Mouse(5));
                k.set(Action::Jump, Key::UNBOUND);
                k.set(Action::Hotbar1, Key::Keyboard(300));
                k.set(Action::DebugOverlay, Key::Mouse(9));
                k
            },
            toggle_sneak: true,
            toggle_sprint: false,
            sound: SoundSettings {
                volumes: [
                    0.5,
                    0.0,
                    1.0,
                    0.25,
                    0.3333333333333333,
                    1.0,
                    0.9,
                    0.07,
                    1.0,
                    0.0,
                    0.6,
                ],
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
        // (Key bindings' rows have no values: they act.)
        let rows: Vec<Row> = rows_of(&all_groups())
            .into_iter()
            .filter(|r| !matches!(r, Row::Key(_) | Row::KeyHeading(_) | Row::ResetKeys))
            .collect();
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
        // Sliders that start above zero.
        s.set(Row::WeatherRadius, 2);
        s.set(Row::CloudDistance, 10);
        assert_eq!((s.game.weather_radius, s.game.cloud_distance), (5, 12));
        assert_eq!(
            (s.get(Row::WeatherRadius), s.get(Row::CloudDistance)),
            (2, 10)
        );
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
             damage_tilt = 101\nfov_effects = \"all\"\nclouds = true\ncloud_distance = 129\nweather_radius = 2\n",
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
                ControlSpec::Key { .. } | ControlSpec::Heading | ControlSpec::Button { .. } => 1,
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
    fn key_bindings_are_kept_by_their_names() {
        let s = Settings::parse(
            "[keys]\n\"key.forward\" = \"key.keyboard.i\"\nkey.attack = \"key.keyboard.j\"\n\
             key.hotbar.1 = \"key.mouse.4\"\n\"key.use\" = \"nonsense\"\n\
             \"key.mystery\" = \"key.keyboard.q\"\n\"key.jump\" = 5\n\
             \"key.sneak\" = \"key.keyboard.unknown\"\n",
        )
        .unwrap();
        let k = &s.keys;
        assert_eq!(k.get(Action::Forward), Key::Keyboard(12));
        assert_eq!(k.get(Action::Attack), Key::Keyboard(13), "a dotted name");
        assert_eq!(k.get(Action::Hotbar1), Key::Mouse(4));
        assert_eq!(k.get(Action::Sneak), Key::UNBOUND);
        // What can't be read keeps the default, and unknown names are dropped.
        assert!(k.is_default(Action::Use) && k.is_default(Action::Jump));
        assert_eq!(k.changed(), 4);
        assert!(!s.to_toml().contains("mystery"));
        // Every binding is written, in options.txt's order, and an empty table is the
        // defaults.
        let text = s.to_toml();
        let at = |name: &str| text.find(&format!("\"{name}\" = ")).unwrap();
        assert!(at("key.attack") < at("key.use") && at("key.use") < at("key.forward"));
        assert!(text.contains("\"key.hotbar.1\" = \"key.mouse.4\"\n"));
        assert_eq!(Settings::parse("[keys]\n").unwrap().keys, KeyBinds::DEFAULT);
        assert!(Settings::has_keys("[keys]\n"));
        assert!(!Settings::has_keys("[controls]\nmouse_sensitivity = 0.5\n"));
        assert!(!Settings::has_keys("keys = 5\n"));
        assert!(Settings::has_keys(&Settings::default().to_toml()));
    }

    #[test]
    fn the_controls_page_lists_every_binding_under_its_heading() {
        let controls = GAME.iter().find(|g| g.0 == "Controls").unwrap();
        let rows = controls.1.to_vec();
        let start = rows.iter().position(|&r| r == Row::ResetKeys).unwrap();
        assert_eq!(
            &rows[start - 5..start],
            [
                Row::Sensitivity,
                Row::InvertX,
                Row::InvertY,
                Row::ToggleSneak,
                Row::ToggleSprint
            ]
        );
        let keys: Vec<Action> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Key(a) => Some(*a),
                _ => None,
            })
            .collect();
        assert_eq!(keys, Action::SORTED);
        let headings: Vec<Category> = rows
            .iter()
            .filter_map(|r| match r {
                Row::KeyHeading(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(headings, Category::ALL);
        // Each heading is over its own.
        let mut under = None;
        for r in &rows[start + 1..] {
            match *r {
                Row::KeyHeading(c) => under = Some(c),
                Row::Key(a) => assert_eq!(Some(a.category()), under, "{a:?}"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn binding_rows_show_their_keys_conflicts_and_resets() {
        let mut s = Settings::default();
        let names = KeyNaming {
            lang: None,
            system: &|v| (v == 26).then(|| "W".to_owned()),
        };
        let spec = |s: &Settings, r| s.key_spec(r, &names).unwrap();
        let control = |s: &Settings, r| spec(s, r).control;
        assert_eq!(
            control(&s, Row::Key(Action::Forward)),
            ControlSpec::Key {
                key: "W".into(),
                conflict: false,
                reset: false
            }
        );
        assert_eq!(spec(&s, Row::Key(Action::Forward)).label, "Walk Forward");
        assert_eq!(
            spec(&s, Row::KeyHeading(Category::Misc)).label,
            "Miscellaneous"
        );
        assert_eq!(
            control(&s, Row::ResetKeys),
            ControlSpec::Button {
                label: "Reset Keys".into(),
                enabled: false
            }
        );
        s.keys.set(Action::Jump, Key::Keyboard(26));
        let jump = spec(&s, Row::Key(Action::Jump));
        assert_eq!(
            jump.control,
            ControlSpec::Key {
                key: "W".into(),
                conflict: true,
                reset: true
            }
        );
        assert_eq!(jump.detail, "Also used by Walk Forward");
        assert_eq!(
            spec(&s, Row::Key(Action::Forward)).detail,
            "Also used by Jump"
        );
        let reset = spec(&s, Row::ResetKeys);
        assert_eq!(reset.detail, "1 key differs from the default");
        assert!(matches!(
            reset.control,
            ControlSpec::Button { enabled: true, .. }
        ));
        s.keys.set(Action::Use, Key::Mouse(4));
        assert_eq!(
            control(&s, Row::Key(Action::Use)),
            ControlSpec::Key {
                key: "Button 4".into(),
                conflict: false,
                reset: true
            }
        );
        // The game's language names them, where it is loaded.
        let mut lang = std::collections::HashMap::new();
        lang.insert("key.use".to_owned(), "Use Item/Place Block".to_owned());
        lang.insert("key.mouse".to_owned(), "Button %1$s".to_owned());
        let with = KeyNaming {
            lang: Some(&lang),
            ..names
        };
        assert_eq!(
            s.key_spec(Row::Key(Action::Use), &with).unwrap().label,
            "Use Item/Place Block"
        );
        assert_eq!(s.key_spec(Row::Fov, &names), None);
    }

    #[test]
    fn key_bindings_are_imported_once_when_the_file_has_none() {
        let dir = std::env::temp_dir().join(format!("ruthenium-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = SettingsFile::path_in(&dir);
        let mut imported = KeyBinds::DEFAULT;
        imported.set(Action::Forward, Key::Keyboard(82));
        let vanilla = VanillaKeys {
            keys: imported.clone(),
            toggle_sneak: false,
            toggle_sprint: true,
        };
        let mut f = SettingsFile::load(path.clone());
        f.import_keys(|| Some((vanilla, PathBuf::from("options.txt"))));
        assert_eq!(f.settings.keys, imported);
        assert!(f.settings.toggle_sprint && !f.settings.toggle_sneak);
        // Saved at once, though nothing else changed.
        f.save();
        let again = SettingsFile::load(path.clone());
        assert_eq!(again.settings.keys, imported);
        assert!(again.settings.toggle_sprint);
        // The file has them now: no second import.
        let mut f = again;
        f.import_keys(|| panic!("imported twice"));
        // Nothing to import still writes them, so it is asked once.
        let _ = std::fs::remove_dir_all(&dir);
        let mut f2 = SettingsFile::load(path.clone());
        f2.import_keys(|| None);
        f2.save();
        assert!(std::fs::read_to_string(&path).unwrap().contains("[keys]"));
        let _ = std::fs::remove_dir_all(&dir);
        drop(f);
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
