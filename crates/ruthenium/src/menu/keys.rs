//! The Controls page's key bindings and the keys and buttons they hear: a row asks for
//! a key, the next key or mouse button pressed is bound (Escape unbinds, as in vanilla's
//! `KeyBindsScreen`), rows and the whole table reset; the bindings reach the world's
//! input and the HUD's text at once. Also what the keyboard holds whatever shows, for
//! `KeyMapping.setAll` when the world comes back.

use std::sync::Arc;
use std::time::Instant;

use ruthenium_protocol::text::{BoundKeys, Lang};
use ruthenium_sim::input::{HeldKeys, Key};
use ruthenium_sim::keybinds::{Action, KeyBinds};
use ruthenium_ui::modern::SettingsEvent;

use super::{Menu, SCANCODE_ESCAPE, Screen};
use crate::settings::{KeyNaming, Row, rows_of};

/// What the keyboard's layout calls a key, by scancode.
pub type SystemNames = Box<dyn Fn(i32) -> Option<String>>;

/// See the module docs.
pub(super) struct KeysState {
    /// Keyboard keys down now, whatever shows (key events, all of them).
    physical: HeldKeys,
    /// The binding awaiting a key, and its row.
    capture: Option<(Action, usize)>,
    /// A mouse button whose press was bound: its release goes nowhere.
    swallow_up: Option<u8>,
    system: SystemNames,
    /// The bindings as text shows them, rebuilt when they change.
    bound: Arc<BoundKeys>,
    /// The rows on show are named in the game's language.
    named_in_lang: bool,
}

impl KeysState {
    pub(super) fn new(binds: &KeyBinds) -> KeysState {
        let system: SystemNames = Box::new(|_| None);
        KeysState {
            physical: HeldKeys::default(),
            capture: None,
            swallow_up: None,
            bound: Arc::new(binds.bound_keys(&system)),
            system,
            named_in_lang: false,
        }
    }

    /// The keyboard keys down now.
    pub(super) fn physical(&self) -> Vec<Key> {
        self.physical.keyboard().collect()
    }

    pub(super) fn bound(&self) -> Arc<BoundKeys> {
        self.bound.clone()
    }
}

impl Menu {
    /// Has keys named as the keyboard's layout names them (SDL), for the Controls page
    /// and the HUD's text.
    pub fn set_key_names(&mut self, system: SystemNames) {
        self.keys.system = system;
        self.keys_changed();
    }

    /// The bindings as the HUD's text shows them (a new one each time they change).
    pub fn bound_keys(&self) -> Arc<BoundKeys> {
        self.keys.bound()
    }

    /// The bindings in use.
    pub fn binds(&self) -> &KeyBinds {
        &self.settings.settings.keys
    }

    /// Whether the world shows with no screen over it.
    pub fn in_world(&self) -> bool {
        matches!(self.screen, Screen::Game)
    }

    /// The bindings the world's input has.
    #[cfg(test)]
    fn game_binds(&self) -> &KeyBinds {
        self.game.binds()
    }

    /// Development: the focused key binding's row awaits a key (screenshots).
    pub fn capture_focused(&mut self) {
        let Some(i) = self.settings_screen.as_ref().and_then(|s| s.focused()) else {
            return;
        };
        self.key_row_event(SettingsEvent::Pressed { setting: i });
    }

    /// Once the game's language has loaded, the rows on show take its names.
    pub(super) fn name_keys_once_lang_loads(&mut self) {
        if self.keys.named_in_lang || self.settings_screen.is_none() {
            return;
        }
        let loaded = self.joiner.as_mut().and_then(|j| j.data_if_ready());
        if loaded.is_some_and(|d| d.lang.contains_key("key.forward")) {
            self.keys.named_in_lang = true;
            self.show_keys();
        }
    }

    /// Whether a row awaits a key: the next key or button is its.
    pub fn capturing(&self) -> bool {
        self.keys.capture.is_some()
    }

    /// A key event, whatever shows: what the keyboard holds is kept, and a row awaiting
    /// a key takes a press. True if it did (the event goes no further).
    pub(super) fn key_event(&mut self, scancode: u32, down: bool, repeat: bool) -> bool {
        self.keys.physical.set(Key::scancode(scancode), down);
        if self.keys.capture.is_none() {
            return false;
        }
        if down && !repeat {
            let key = if scancode == SCANCODE_ESCAPE {
                Key::UNBOUND
            } else {
                Key::scancode(scancode)
            };
            self.bind_captured(key);
        }
        true
    }

    /// A mouse button event (SDL's numbering): a row awaiting a key takes a press, and its
    /// release goes nowhere. True if it went nowhere else.
    pub(super) fn button_event(&mut self, button: u8, down: bool) -> bool {
        if !down && self.keys.swallow_up == Some(button) {
            self.keys.swallow_up = None;
            return true;
        }
        if !down || self.keys.capture.is_none() {
            return false;
        }
        self.keys.swallow_up = Some(button);
        self.bind_captured(Key::button(button));
        if let Some(s) = &mut self.settings_screen {
            s.cancel();
        }
        true
    }

    /// The window lost focus: what the keyboard held is let go, and a row awaiting a
    /// key stops.
    pub(super) fn keys_focus_lost(&mut self) {
        self.keys.physical.release_all();
        self.stop_capture();
    }

    fn stop_capture(&mut self) {
        self.keys.capture = None;
        if let Some(s) = &mut self.settings_screen {
            s.set_capturing(None);
        }
    }

    fn bind_captured(&mut self, key: Key) {
        let Some((action, _)) = self.keys.capture.take() else {
            return;
        };
        self.settings.settings.keys.set(action, key);
        self.stop_capture();
        self.keys_changed();
    }

    /// A key binding row, Reset Keys or a row's Reset.
    pub(super) fn key_row_event(&mut self, e: SettingsEvent) {
        let rows = rows_of(&self.settings_of.groups());
        match e {
            SettingsEvent::Pressed { setting } => match rows.get(setting) {
                Some(&Row::Key(a)) => {
                    self.keys.capture = Some((a, setting));
                    if let Some(s) = &mut self.settings_screen {
                        s.set_capturing(Some(setting));
                    }
                }
                Some(Row::ResetKeys) => {
                    self.settings.settings.keys.reset_all();
                    self.keys_changed();
                }
                _ => {}
            },
            SettingsEvent::Reset { setting } => {
                if let Some(&Row::Key(a)) = rows.get(setting) {
                    self.settings.settings.keys.reset(a);
                    self.keys_changed();
                }
            }
            SettingsEvent::Changed { .. } | SettingsEvent::Done => {}
        }
    }

    /// How the Controls page names things now: with the game's language once it is
    /// loaded.
    pub(super) fn with_key_naming<T>(&mut self, f: impl FnOnce(&Menu, &KeyNaming<'_>) -> T) -> T {
        let data = self.joiner.as_mut().and_then(|j| j.data_if_ready());
        self.keys.named_in_lang = data
            .as_ref()
            .is_some_and(|d| d.lang.contains_key("key.forward"));
        let lang = data.as_ref().map(|d| &d.lang as &dyn Lang);
        let naming = KeyNaming {
            lang: lang.filter(|l| l.get("key.forward").is_some()),
            system: &*self.keys.system,
        };
        f(self, &naming)
    }

    /// The bindings changed: they are saved, and the world's input, the HUD's text and
    /// the rows showing them follow.
    fn keys_changed(&mut self) {
        self.settings_changed = Some(Instant::now());
        let binds = self.settings.settings.keys.clone();
        self.game.set_binds(&binds);
        self.keys.bound = Arc::new(binds.bound_keys(&*self.keys.system));
        self.show_keys();
    }

    /// The rows on show say what the bindings are now.
    fn show_keys(&mut self) {
        if self.settings_screen.is_none() {
            return;
        }
        let rows = rows_of(&self.settings_of.groups());
        let specs: Vec<_> = self.with_key_naming(|m, naming| {
            rows.iter()
                .enumerate()
                .filter_map(|(i, &r)| Some((i, m.settings.settings.key_spec(r, naming)?)))
                .collect()
        });
        if let Some(s) = &mut self.settings_screen {
            for (i, spec) in specs {
                s.set_spec(i, &spec, &self.fonts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ruthenium_ui::modern::{ControlSpec, SettingsEvent};

    use super::*;
    use crate::account::Accounts;
    use crate::settings::{GAME, SettingsFile};

    struct NoClipboard;

    impl super::super::Clipboard for NoClipboard {
        fn get(&self) -> Option<String> {
            None
        }

        fn set(&self, _: &str) {}
    }

    /// The Game Menu's MC Options, on the Controls page.
    fn controls() -> Menu {
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        let scale = m.layout((1600, 900));
        m.screen = Screen::Paused;
        m.open_settings(scale, super::super::SettingsOf::Game);
        m
    }

    fn row(r: Row) -> usize {
        rows_of(&GAME).iter().position(|&x| x == r).unwrap()
    }

    fn key(m: &mut Menu, code: u32, down: bool) {
        m.key(code, down, false, false, false, &NoClipboard);
    }

    fn shown(m: &Menu, r: Row) -> ControlSpec {
        let names = KeyNaming::PLAIN;
        let spec = m.settings.settings.key_spec(r, &names).unwrap();
        // What the screen shows is what the settings say.
        let s = m.settings_screen.as_ref().unwrap();
        assert_eq!(s.len(), rows_of(&GAME).len());
        spec.control
    }

    #[test]
    fn a_row_takes_the_next_key_and_escape_unbinds() {
        let mut m = controls();
        let jump = row(Row::Key(Action::Jump));
        m.settings_event(SettingsEvent::Pressed { setting: jump });
        assert!(m.capturing());
        assert_eq!(m.settings_screen.as_ref().unwrap().capturing(), Some(jump));
        // Tab and the arrows are keys like any other now, and the press binds (its
        // release goes nowhere).
        key(&mut m, 43, true);
        key(&mut m, 43, false);
        assert!(!m.capturing());
        assert_eq!(m.binds().get(Action::Jump), Key::Keyboard(43));
        assert_eq!(m.settings_screen.as_ref().unwrap().capturing(), None);
        assert!(m.settings_changed.is_some());
        // Tab is the player list's too: both rows say so.
        assert!(matches!(
            shown(&m, Row::Key(Action::Jump)),
            ControlSpec::Key {
                conflict: true,
                reset: true,
                ..
            }
        ));
        assert!(matches!(
            shown(&m, Row::Key(Action::PlayerList)),
            ControlSpec::Key {
                conflict: true,
                reset: false,
                ..
            }
        ));
        // Escape unbinds instead of leaving, and the screen stays.
        m.settings_event(SettingsEvent::Pressed { setting: jump });
        key(&mut m, SCANCODE_ESCAPE, true);
        assert_eq!(m.binds().get(Action::Jump), Key::UNBOUND);
        assert!(matches!(m.screen, Screen::Settings));
        // Not capturing, Escape leaves.
        key(&mut m, SCANCODE_ESCAPE, false);
        key(&mut m, SCANCODE_ESCAPE, true);
        assert!(matches!(m.screen, Screen::Paused));
    }

    #[test]
    fn mouse_buttons_bind_and_their_release_goes_nowhere() {
        let mut m = controls();
        let attack = row(Row::Key(Action::Attack));
        m.settings_event(SettingsEvent::Pressed { setting: attack });
        // A repeat does not bind; a side button does.
        m.key(30, true, true, false, false, &NoClipboard);
        assert!(m.capturing());
        assert!(m.button_event(4, true));
        assert_eq!(m.binds().get(Action::Attack), Key::Mouse(4));
        assert!(m.button_event(4, false), "the release is swallowed");
        assert!(!m.button_event(4, false), "once");
        // The left button binds too, as in vanilla.
        let use_row = row(Row::Key(Action::Use));
        m.settings_event(SettingsEvent::Pressed { setting: use_row });
        assert!(m.button_event(1, true));
        assert_eq!(m.binds().get(Action::Use), Key::Mouse(1));
        assert!(m.button_event(1, false));
        // Without a row awaiting one, buttons go on to the screen.
        assert!(!m.button_event(1, true));
        // The world's input has the new bindings at once.
        assert_eq!(m.game_binds(), m.binds());
    }

    #[test]
    fn rows_and_the_whole_table_reset() {
        let mut m = controls();
        m.settings
            .settings
            .keys
            .set(Action::Jump, Key::Keyboard(26));
        m.settings.settings.keys.set(Action::Drop, Key::Mouse(5));
        m.settings_event(SettingsEvent::Reset {
            setting: row(Row::Key(Action::Jump)),
        });
        assert!(m.binds().is_default(Action::Jump));
        assert_eq!(m.binds().changed(), 1);
        // Delete on a focused row resets it too.
        m.settings_screen
            .as_mut()
            .unwrap()
            .focus(Some(row(Row::Key(Action::Drop))));
        key(&mut m, 76, true);
        assert_eq!(m.binds().changed(), 0);
        m.settings
            .settings
            .keys
            .set(Action::Sneak, Key::Keyboard(29));
        m.settings.settings.keys.set(Action::Use, Key::UNBOUND);
        m.settings_event(SettingsEvent::Pressed {
            setting: row(Row::ResetKeys),
        });
        assert_eq!(m.binds(), &KeyBinds::DEFAULT);
        assert!(matches!(
            shown(&m, Row::ResetKeys),
            ControlSpec::Button { enabled: false, .. }
        ));
        assert_eq!(m.game_binds(), &KeyBinds::DEFAULT);
    }

    #[test]
    fn focus_loss_stops_a_row_and_lets_go_of_keys() {
        let mut m = controls();
        key(&mut m, 26, true);
        key(&mut m, 4, true);
        key(&mut m, 4, false);
        assert_eq!(m.keys.physical(), [Key::Keyboard(26)]);
        m.settings_event(SettingsEvent::Pressed {
            setting: row(Row::Key(Action::Jump)),
        });
        m.focus(false);
        assert!(!m.capturing() && m.keys.physical().is_empty());
        assert!(m.binds().is_default(Action::Jump));
    }

    #[test]
    fn text_names_the_bound_keys() {
        let mut m = controls();
        let before = m.bound_keys();
        m.settings_event(SettingsEvent::Pressed {
            setting: row(Row::Key(Action::Sneak)),
        });
        key(&mut m, 29, true);
        let after = m.bound_keys();
        assert!(!Arc::ptr_eq(&before, &after), "a new table for the HUD");
        assert_eq!(after.get("key.sneak").unwrap().name, "key.keyboard.z");
        m.set_key_names(Box::new(|v| (v == 29).then(|| "Y".to_owned())));
        assert_eq!(m.bound_keys().get("key.sneak").unwrap().system, Some("Y"));
    }
}
