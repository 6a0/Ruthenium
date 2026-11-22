//! The player's settings: what the Settings screen shows, kept in
//! `config/client.toml` in Ruthenium's data folder.
//!
//! Reading is forgiving: a missing file, key or section, or a value of the wrong type or
//! out of range, takes the default (a broken file is reported once). Saving writes the
//! whole file, with comments, to a new file beside it, then moves the previous one to
//! `client.toml_old` and the new one into place, as `servers.dat` is saved.

use std::path::{Path, PathBuf};

use ruthenium_sim::input::MouseOptions;
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
        }
    }
}

/// The Settings screen's rows, in order (counted over every section).
pub const VSYNC: usize = 0;
pub const MAX_FPS: usize = 1;
pub const FULLSCREEN: usize = 2;
pub const FRAME_STATS: usize = 3;
pub const MENU_SIZE: usize = 4;
pub const REDUCE_MOTION: usize = 5;

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
        let (display, interface, multiplayer, controls) = (
            section("display"),
            section("interface"),
            section("multiplayer"),
            section("controls"),
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
        Ok(Settings {
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

    /// What the Settings screen shows, with these values.
    pub fn sections(&self) -> Vec<SectionSpec> {
        let setting = |i: usize, label: &str, detail: &str, control| SettingSpec {
            label: label.into(),
            detail: detail.into(),
            control,
            value: self.get(i),
        };
        let rates = (1..FPS_STEPS as u32)
            .map(|k| format!("{} FPS", k * FPS_STEP))
            .chain(std::iter::once("Unlimited".to_owned()))
            .collect();
        let sizes = MenuSize::ALL.iter().map(|m| m.label().to_owned()).collect();
        vec![
            SectionSpec {
                title: "Display".into(),
                settings: vec![
                    setting(
                        VSYNC,
                        "VSync",
                        "Waits for the display, so frames never tear",
                        ControlSpec::Switch,
                    ),
                    setting(
                        MAX_FPS,
                        "Max Frame Rate",
                        "The most frames drawn each second",
                        ControlSpec::Slider(rates),
                    ),
                    setting(
                        FULLSCREEN,
                        "Fullscreen",
                        "Fills the screen; F11 switches it too",
                        ControlSpec::Switch,
                    ),
                    setting(
                        FRAME_STATS,
                        "Frame Stats",
                        "Frame rate and frame times in a corner; F3 too",
                        ControlSpec::Switch,
                    ),
                ],
            },
            SectionSpec {
                title: "Menus".into(),
                settings: vec![
                    setting(
                        MENU_SIZE,
                        "Menu Size",
                        "How large these menus are",
                        ControlSpec::Choice(sizes),
                    ),
                    setting(
                        REDUCE_MOTION,
                        "Reduce Motion",
                        "Buttons and switches fade instead of moving",
                        ControlSpec::Switch,
                    ),
                ],
            },
        ]
    }

    /// Row `i`'s value as the screen counts it.
    pub fn get(&self, i: usize) -> usize {
        match i {
            VSYNC => self.vsync.into(),
            MAX_FPS => self
                .max_fps
                .map_or(FPS_STEPS - 1, |f| (f / FPS_STEP) as usize - 1),
            FULLSCREEN => self.fullscreen.into(),
            FRAME_STATS => self.frame_stats.into(),
            MENU_SIZE => MenuSize::ALL
                .iter()
                .position(|&m| m == self.menu_size)
                .unwrap_or(1),
            REDUCE_MOTION => self.reduce_motion.into(),
            _ => 0,
        }
    }

    /// Sets row `i` from the screen's `value`.
    pub fn set(&mut self, i: usize, value: usize) {
        match i {
            VSYNC => self.vsync = value != 0,
            MAX_FPS => {
                self.max_fps = (value + 1 < FPS_STEPS).then(|| (value as u32 + 1) * FPS_STEP);
            }
            FULLSCREEN => self.fullscreen = value != 0,
            FRAME_STATS => self.frame_stats = value != 0,
            MENU_SIZE => {
                if let Some(&m) = MenuSize::ALL.get(value) {
                    self.menu_size = m;
                }
            }
            REDUCE_MOTION => self.reduce_motion = value != 0,
            _ => {}
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
             invert_mouse_y = {}\n",
            self.vsync,
            self.fullscreen,
            self.frame_stats,
            self.menu_size.key(),
            self.reduce_motion,
            self.mouse.sensitivity,
            self.mouse.invert_x,
            self.mouse.invert_y,
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
        let rows: usize = s.sections().iter().map(|x| x.settings.len()).sum();
        assert_eq!(rows, REDUCE_MOTION + 1);
        assert_eq!(s.get(MAX_FPS), FPS_STEPS - 1, "unlimited is the last step");
        s.set(MAX_FPS, 0);
        assert_eq!(s.max_fps, Some(10));
        s.set(MAX_FPS, 11);
        assert_eq!(s.max_fps, Some(120));
        assert_eq!(s.get(MAX_FPS), 11);
        s.set(MAX_FPS, FPS_STEPS - 1);
        assert_eq!(s.max_fps, None);
        s.set(MENU_SIZE, 3);
        assert_eq!(s.menu_size, MenuSize::Larger);
        for i in [VSYNC, FULLSCREEN, FRAME_STATS, REDUCE_MOTION] {
            let before = s.get(i);
            s.set(i, 1 - before);
            assert_eq!(s.get(i), 1 - before);
        }
        let ControlSpec::Slider(steps) = &s.sections()[0].settings[MAX_FPS].control else {
            panic!("not a slider");
        };
        assert_eq!((steps[0].as_str(), steps.len()), ("10 FPS", FPS_STEPS));
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
