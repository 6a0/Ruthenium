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
use super::{Menu, SCANCODE_ESCAPE, Screen};

/// SDL's scancodes of the shift and control keys, left and right.
const SHIFT: [u32; 2] = [225, 229];
const CTRL: [u32; 2] = [224, 228];

/// The container screen's input between frames.
#[derive(Debug, Default)]
pub(super) struct ContainerInput {
    /// The window's pixel and window sizes, as the last frame had them.
    sizes: Option<((u32, u32), (u32, u32))>,
    /// The button held (`MouseHandler.activeButton`), and whether the pointer moved
    /// this frame.
    held: Option<u8>,
    moved: bool,
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

    /// A key went down or repeated over the screen (`keyPressed`); Escape and the
    /// inventory key close it.
    pub(super) fn container_key(&mut self, scancode: u32, down: bool) {
        if !down {
            return;
        }
        let mut binds = self.screen_binds(Key::scancode(scancode));
        binds.escape = scancode == SCANCODE_ESCAPE;
        let (ctrl, at) = (self.ctrl_held(), self.pointer_at());
        if let Some(s) = &self.session {
            s.screen_input(ScreenInput::Key { binds, ctrl, at });
        }
        if binds.escape || binds.inventory {
            self.container_closed();
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

    /// Before the session's frame: where the pointer is, and whether it dragged.
    pub(super) fn container_game_frame(&mut self) {
        if !matches!(self.screen, Screen::Container) {
            return;
        }
        let drag = std::mem::take(&mut self.container.moved) && self.container.held.is_some();
        if let (Some(s), Some(p)) = (&self.session, self.gui_pointer()) {
            s.set_pointer(p, drag);
        }
    }
}
