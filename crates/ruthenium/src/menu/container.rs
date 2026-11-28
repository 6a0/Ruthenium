//! The container screens (the inventory, chests, furnaces...) as the window sees them:
//! the game thread runs them ([`ruthenium_sim::session::ScreenInput`]); here their
//! mouse and keys are handed over with the bindings on them, in GUI units, and the
//! renderer is told what to draw. They open and close as the session says, and Escape
//! or the inventory key closes them at once here too (the game thread does the same with
//! the event), so the keys held come back the same frame (`setScreen(null)`'s
//! `KeyMapping.setAll`).

use std::time::Instant;

use ruthenium_sim::input::Key;
use ruthenium_sim::keybinds::Action;
use ruthenium_sim::menu::screen::Binds;
use ruthenium_sim::session::{At, Pointer, ScreenInput};
use ruthenium_ui::GuiScale;

use super::chat::ExitReason;
use super::{
    Menu, SCANCODE_A, SCANCODE_BACKSPACE, SCANCODE_C, SCANCODE_DELETE, SCANCODE_END,
    SCANCODE_ESCAPE, SCANCODE_HOME, SCANCODE_LEFT, SCANCODE_RIGHT, SCANCODE_V, SCANCODE_X, Screen,
};
use ruthenium_sim::menu::work::edit_box::EditKey;
use ruthenium_sim::menu::work::{KindState, State};

/// SDL's scancodes of the shift and control keys, left and right.
const SHIFT: [u32; 2] = [225, 229];
const CTRL: [u32; 2] = [224, 228];
/// SDL's scancodes of Page Up and Page Down (`SDLK_PAGEUP`, `SDLK_PAGEDOWN`).
const PAGE_UP: u32 = 75;
const PAGE_DOWN: u32 = 78;

/// The container screen's input between frames.
#[derive(Debug, Default)]
pub(super) struct ContainerInput {
    /// The window's pixel and window sizes, as the last frame had them.
    sizes: Option<((u32, u32), (u32, u32))>,
    /// The button held (`MouseHandler.activeButton`), and whether the pointer moved
    /// this frame.
    held: Option<u8>,
    moved: bool,
    /// What the creative search box copied, for the clipboard at the next key.
    pub(super) clipboard: Option<String>,
}

impl Menu {
    /// The session opened a container screen: it takes the place of the chat or the
    /// Game Menu (`setScreen`), and the keys are let go.
    pub(super) fn container_opened(&mut self) {
        if matches!(self.screen, Screen::Chat(_)) {
            self.chat_removed(ExitReason::Interrupted);
        }
        if matches!(self.screen, Screen::Settings) {
            self.settings_screen = None;
            self.save_settings();
        }
        self.pause = None;
        self.game.release(self.session.as_ref());
        self.container.held = None;
        self.screen = Screen::Container;
    }

    /// The container screen closed: back to the world, with the keys held now.
    pub(super) fn container_closed(&mut self) {
        if matches!(self.screen, Screen::Container) {
            self.enter_game();
        }
    }

    fn shift_held(&self) -> bool {
        let held = self.keys.physical();
        SHIFT.iter().any(|k| held.contains(&Key::scancode(*k)))
    }

    fn ctrl_held(&self) -> bool {
        let held = self.keys.physical();
        CTRL.iter().any(|k| held.contains(&Key::scancode(*k)))
    }

    /// The bindings a screen reads on `key`.
    fn screen_binds(&self, key: Key) -> Binds {
        let mut b = Binds::default();
        for a in self.binds().on(key) {
            match a {
                Action::Inventory => b.inventory = true,
                Action::SwapOffhand => b.swap_offhand = true,
                Action::PickItem => b.pick = true,
                Action::Drop => b.drop = true,
                a => {
                    if let Some(slot) = a.hotbar_slot() {
                        b.hotbar[usize::from(slot)] = true;
                    }
                }
            }
        }
        b
    }

    /// Where the pointer is in GUI units, and the GUI's size.
    fn gui_pointer(&self) -> Option<Pointer> {
        let ((pw, ph), (ww, wh)) = self.container.sizes?;
        let gui = GuiScale::new(pw, ph, self.settings.settings.game.gui_scale, false);
        let (x, y) = self.mouse.unwrap_or((-1.0, -1.0));
        Some(Pointer {
            x: f64::from(x) * f64::from(gui.width) / f64::from(ww.max(1)),
            y: f64::from(y) * f64::from(gui.height) / f64::from(wh.max(1)),
            width: gui.width as i32,
            height: gui.height as i32,
        })
    }

    fn pointer_at(&self) -> At {
        self.gui_pointer()
            .map_or(At::Outside, |p| At::Gui(p.x, p.y))
    }

    /// Whether the creative screen is up (as the last view has it), and its search tab.
    fn creative_shown(&self) -> Option<bool> {
        let v = self.session.as_ref()?.container_view()?;
        let c = v.creative.as_ref()?;
        Some(c.search.is_some())
    }

    /// A key over the creative screen (down, a repeat or up): with its bindings and what
    /// its search box reads; Escape closes it, and the inventory key outside the search
    /// tab (whose box takes every other key).
    fn creative_key(
        &mut self,
        scancode: u32,
        down: bool,
        shift: bool,
        shortcut: bool,
        clipboard: &dyn super::Clipboard,
        search: bool,
    ) {
        let Some(s) = &self.session else {
            return;
        };
        if !down {
            s.screen_input(ScreenInput::KeyReleased);
            return;
        }
        let key = Key::scancode(scancode);
        let mut binds = self.screen_binds(key);
        binds.escape = scancode == SCANCODE_ESCAPE;
        let chat = self.binds().on(key).any(|a| a == Action::Chat);
        let paste = shortcut && scancode == ruthenium_sim::menu::creative::edit_box::SCANCODE_V;
        let k = ruthenium_sim::menu::creative::CreativeKey {
            binds,
            chat,
            key: ruthenium_sim::menu::creative::edit_box::KeyPress {
                scancode,
                shift,
                shortcut,
                clipboard: paste.then(|| clipboard.get()).flatten(),
            },
            ctrl: self.ctrl_held(),
        };
        let at = self.pointer_at();
        s.screen_input(ScreenInput::CreativeKey { key: k, at });
        if binds.escape || (binds.inventory && !search) {
            self.container_closed();
        }
    }

    /// The wheel turned over the screen (`mouseScrolled`; SDL's vertical amount): the
    /// creative screen and the workstations' lists scroll.
    pub(super) fn container_wheel(&mut self, y: f32) {
        let at = self.pointer_at();
        if let Some(s) = &self.session {
            s.screen_input(ScreenInput::Scroll {
                y: f64::from(y),
                at,
            });
        }
    }

    /// Typed text over the screen (`charTyped`): the creative screen's search box, an
    /// anvil's name box.
    pub(super) fn container_text(&mut self, text: &str) {
        if let Some(s) = &self.session {
            s.screen_input(ScreenInput::Text(text.to_owned()));
        }
    }

    /// Whether the screen takes typed text (the creative one, an anvil).
    pub(super) fn container_wants_text(&self) -> bool {
        matches!(self.screen, Screen::Container)
            && (self.creative_shown().is_some() || self.anvil_box().is_some())
    }

    /// A key went down or repeated over the screen (`keyPressed`); Escape and the
    /// inventory key close it. An anvil's name box reads the key first: what it takes, or
    /// any key while it takes input, goes no further (`AnvilScreen.keyPressed`).
    pub(super) fn container_key(
        &mut self,
        scancode: u32,
        down: bool,
        shift: bool,
        shortcut: bool,
        clipboard: &dyn super::Clipboard,
    ) {
        if let Some(text) = self.container.clipboard.take() {
            clipboard.set(&text);
        }
        if let Some(search) = self.creative_shown() {
            self.creative_key(scancode, down, shift, shortcut, clipboard, search);
            return;
        }
        if !down {
            return;
        }
        let mut binds = self.screen_binds(Key::scancode(scancode));
        binds.escape = scancode == SCANCODE_ESCAPE;
        (binds.page_up, binds.page_down) = (scancode == PAGE_UP, scancode == PAGE_DOWN);
        let (ctrl, at) = (self.ctrl_held(), self.pointer_at());
        // A lectern's book screen isn't a container screen: the inventory key does
        // nothing there.
        let book = self
            .session
            .as_ref()
            .and_then(|s| s.container_view())
            .is_some_and(|v| v.kind == ruthenium_sim::menu::Kind::Lectern);
        let mut swallowed = false;
        if let Some(name) = self.anvil_box() {
            let key = edit_key(scancode, shortcut);
            // (Copying and cutting take the selection the screen shows.)
            if matches!(key, EditKey::Copy | EditKey::Cut) {
                clipboard.set(&name.highlighted());
            }
            let paste = if key == EditKey::Paste {
                clipboard.get().unwrap_or_default()
            } else {
                String::new()
            };
            swallowed = key != EditKey::Other || name.editable;
            if let Some(s) = &self.session {
                s.screen_input(ScreenInput::EditKey {
                    key,
                    shift,
                    ctrl: shortcut,
                    paste,
                });
            }
        }
        if let Some(s) = &self.session {
            s.screen_input(ScreenInput::Key { binds, ctrl, at });
        }
        if binds.escape || (binds.inventory && !swallowed && !book) {
            self.container_closed();
        }
    }

    /// The open anvil's name box, as last drawn.
    fn anvil_box(&self) -> Option<ruthenium_sim::menu::work::edit_box::EditBox> {
        let view = self.session.as_ref()?.container_view()?;
        let w: &State = view.work.as_deref()?;
        match &w.kind {
            KindState::Anvil(a) => Some(a.name.clone()),
            _ => None,
        }
    }

    /// A mouse button (SDL's numbering) went down or up over the screen.
    pub(super) fn container_button(&mut self, button: u8, down: bool) {
        let binds = self.screen_binds(Key::button(button));
        let (shift, at) = (self.shift_held(), self.pointer_at());
        self.container.held = down.then_some(button);
        let Some(s) = &self.session else {
            return;
        };
        s.screen_input(if down {
            ScreenInput::Press {
                button,
                at,
                shift,
                binds,
                when: Instant::now(),
            }
        } else {
            ScreenInput::Release {
                button,
                at,
                shift,
                binds,
            }
        });
    }

    /// The pointer moved over the screen.
    pub(super) fn container_moved(&mut self) {
        self.container.moved = true;
    }

    /// The frame's part: the renderer draws the session's screen at the pointer.
    pub(super) fn container_frame(
        &mut self,
        renderer: &mut ruthenium_render::Renderer,
        pixel_size: (u32, u32),
        window_size: (u32, u32),
    ) {
        self.container.sizes = Some((pixel_size, window_size));
        let view = matches!(self.screen, Screen::Container)
            .then(|| self.session.as_ref()?.container_view())
            .flatten();
        let pointer = self.gui_pointer();
        renderer.set_container_screen(view.zip(pointer).map(|(v, p)| (v, (p.x, p.y))));
    }

    /// Before the session's frame: where the pointer is, and whether it dragged. (And
    /// the language stacks' names are read in, for an anvil's name box.)
    pub(super) fn container_game_frame(&mut self) {
        if let (Some(s), Some(j)) = (&self.session, &mut self.joiner) {
            s.set_language(j.server_lang());
        }
        if !matches!(self.screen, Screen::Container) {
            return;
        }
        let drag = std::mem::take(&mut self.container.moved) && self.container.held.is_some();
        if let (Some(s), Some(p)) = (&self.session, self.gui_pointer()) {
            s.set_pointer(p, drag);
        }
    }
}

/// The edit box's key for a scancode (\`EditBox.keyPressed\`'s keys; \`shortcut\` is the
/// platform's control or command).
fn edit_key(scancode: u32, shortcut: bool) -> EditKey {
    match scancode {
        SCANCODE_BACKSPACE => EditKey::Backspace,
        SCANCODE_DELETE => EditKey::Delete,
        SCANCODE_HOME => EditKey::Home,
        SCANCODE_END => EditKey::End,
        SCANCODE_RIGHT => EditKey::Right,
        SCANCODE_LEFT => EditKey::Left,
        SCANCODE_A if shortcut => EditKey::SelectAll,
        SCANCODE_C if shortcut => EditKey::Copy,
        SCANCODE_X if shortcut => EditKey::Cut,
        SCANCODE_V if shortcut => EditKey::Paste,
        _ => EditKey::Other,
    }
}
