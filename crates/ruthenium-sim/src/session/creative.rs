//! The creative inventory's side of the session (`CreativeModeInventoryScreen` as the
//! game thread runs it): opening it where vanilla opens the inventory screen in creative
//! (the inventory key, and the inventory screen's tick turning into it), its events,
//! its tick (tabs rebuilt for new parameters, the inventory screen again for a player
//! out of creative), what the packets for the inventory menu do while it is up
//! (`handleContainerSetSlot` hearing the listener, `handleSetCursorItem` left out), and
//! the saved hotbars' keys (`Minecraft.handleKeybinds`).

use std::collections::HashMap;
use std::sync::Arc;

use super::containers::ScreenInput;
use super::{Game, SessionEvent};
use crate::inventory::Hand;
use crate::item::Items;
use crate::menu::creative::{self, CreativeKey, CreativeScreen, Ctx, tabs};
use crate::menu::screen::{Mouse, Screen};
use crate::menu::{Kind, Who};

/// What the window tells the creative inventory each frame.
#[derive(Debug, Clone, Default)]
pub(super) struct CreativeInput {
    /// The Save Hotbar and Load Hotbar activators held (`isDown`).
    pub(super) save_down: bool,
    pub(super) load_down: bool,
    /// The language the search reads the tooltips in, when it changed.
    pub(super) language: Option<Arc<HashMap<String, String>>>,
    /// The Operator Items Tab option, when it changed.
    pub(super) operator_tab: Option<bool>,
}

/// The session's creative inventory state.
#[derive(Debug, Default)]
pub(super) struct CreativeSession {
    pub(super) state: creative::Creative,
    /// Counts configurations: a new one is a new registry access (`needsUpdate`).
    pub(super) configuration: u64,
    pub(super) language: Arc<HashMap<String, String>>,
    pub(super) operator_tab: bool,
    pub(super) save_down: bool,
    pub(super) load_down: bool,
    /// `player.containerMenu` is the creative picker (set by the screen, which leaves
    /// it there when it gives way to the inventory screen; `closeContainer` ends it).
    pub(super) picker_open: bool,
}

static NO_ITEMS: std::sync::LazyLock<Items> = std::sync::LazyLock::new(Items::default);

impl Game {
    pub(super) fn take_creative_input(&mut self, input: &CreativeInput) {
        let c = &mut self.creative;
        c.save_down = input.save_down;
        c.load_down = input.load_down;
        if let Some(l) = &input.language
            && !Arc::ptr_eq(l, &c.language)
        {
            c.language = l.clone();
            c.state.forget_trees();
        }
        if let Some(op) = input.operator_tab {
            c.operator_tab = op;
        }
    }

    /// Whether the creative screen is up.
    pub(super) fn creative_open(&self) -> bool {
        self.creative.state.screen.is_some()
    }

    /// Runs `f` with the creative screen and what it works on, then sends what it sent.
    fn with_creative<R>(
        &mut self,
        f: impl FnOnce(&mut CreativeScreen, Option<&mut Screen>, &mut Ctx<'_>) -> R,
    ) -> Option<R> {
        let mut s = self.creative.state.screen.take()?;
        let r = self.creative_ctx(|c, screen| f(&mut s, screen, c));
        self.creative.state.screen = Some(s);
        r
    }

    /// Runs `f` with what the creative inventory works on (and the container screen,
    /// if one is up), then sends what it sent.
    fn creative_ctx<R>(
        &mut self,
        f: impl FnOnce(&mut Ctx<'_>, Option<&mut Screen>) -> R,
    ) -> Option<R> {
        let w = self.world.as_mut()?;
        let items: &Items = self.physics.as_ref().map_or(&NO_ITEMS, |p| &p.items);
        let creative = w.player.creative;
        let infinite = w.player.abilities.instabuild;
        let p = &mut w.player;
        let mut ctx = Ctx {
            creative: &mut self.creative.state,
            menus: &mut p.menus,
            inv: &mut p.inventory,
            who: Who {
                items,
                recipes: &self.recipe_sets,
                creative,
            },
            infinite,
            features: &self.configured.features,
            registries: &self.containers.registries,
            lang: &*self.creative.language,
            item_tags: &self.item_tags,
            sent: creative::Sent::default(),
        };
        let r = f(&mut ctx, self.containers.screen.as_mut());
        let sent = std::mem::take(&mut ctx.sent);
        for packet in sent.packets {
            self.connection.send(packet);
        }
        for _ in 0..sent.dropped {
            p.swing(Hand::Main, crate::item::DEFAULT_SWING);
        }
        if let Some(text) = sent.clipboard {
            self.report(SessionEvent::Clipboard(text));
        }
        Some(r)
    }

    /// The tabs' parameters now (`ItemDisplayParameters`), and whether they were built
    /// anew (`tryRebuildTabContents`; the search trees go with them).
    fn rebuild_tabs(&mut self) -> bool {
        let Some(w) = &self.world else {
            return false;
        };
        let items: &Items = self.physics.as_ref().map_or(&NO_ITEMS, |p| &p.items);
        // `hasPermissions`: `canUseGameMasterBlocks` and the option.
        let p = &w.player;
        let permissions =
            p.abilities.instabuild && p.permission_level >= 2 && self.creative.operator_tab;
        let params = tabs::Parameters {
            data: &items.creative,
            items,
            registries: &self.configured.registries,
            tags: &self.configured.tags,
            features: &self.configured.features,
            has_permissions: permissions,
        };
        let configuration = self.creative.configuration;
        self.creative.state.try_rebuild(&params, configuration)
    }

    /// `new CreativeModeInventoryScreen` set as the screen, and its `init`.
    pub(super) fn open_creative(&mut self) {
        self.rebuild_tabs();
        self.show_screen(Kind::Creative, None);
        self.creative.picker_open = true;
        let mut s = CreativeScreen::new(self.creative.operator_tab);
        self.creative_ctx(|c, screen| {
            if let Some(screen) = screen {
                creative::init(&mut s, screen, c);
            }
        });
        self.creative.state.screen = Some(s);
    }

    /// The creative screen goes (`removed`: its listener off the inventory menu).
    pub(super) fn creative_removed(&mut self) {
        self.creative.state.screen = None;
    }

    /// One of the frame's events for the creative screen. Whether it closes.
    pub(super) fn creative_input(
        &mut self,
        e: &ScreenInput,
        place: (f64, f64),
        double: bool,
    ) -> bool {
        let (x, y) = place;
        self.with_creative(|s, screen, c| {
            let Some(screen) = screen else {
                return false;
            };
            match e {
                ScreenInput::Press {
                    button,
                    shift,
                    binds,
                    ..
                } => {
                    let m = Mouse {
                        button: *button,
                        x,
                        y,
                        shift: *shift,
                        binds: *binds,
                    };
                    creative::mouse_clicked(s, screen, c, m, double);
                    false
                }
                ScreenInput::Release {
                    button,
                    shift,
                    binds,
                    ..
                } => {
                    let m = Mouse {
                        button: *button,
                        x,
                        y,
                        shift: *shift,
                        binds: *binds,
                    };
                    creative::mouse_released(s, screen, c, m);
                    false
                }
                ScreenInput::Drag { .. } => {
                    creative::mouse_dragged(s, screen, c, x, y);
                    false
                }
                ScreenInput::Key { binds, ctrl, .. } => {
                    let k = CreativeKey {
                        binds: *binds,
                        ctrl: *ctrl,
                        ..CreativeKey::default()
                    };
                    creative::key_pressed(s, screen, c, &k, x, y)
                }
                ScreenInput::CreativeKey { key, .. } => {
                    creative::key_pressed(s, screen, c, key, x, y)
                }
                ScreenInput::KeyReleased => {
                    creative::key_released(s);
                    false
                }
                ScreenInput::Text(t) => {
                    creative::char_typed(s, c, t);
                    false
                }
                ScreenInput::Scroll { y: amount, .. } => {
                    creative::mouse_scrolled(s, c, *amount);
                    false
                }
                ScreenInput::Click {
                    slot,
                    button,
                    input,
                } => {
                    let at = usize::try_from(*slot).ok();
                    creative::slot_clicked(s, c, at, *slot, *button, *input);
                    false
                }
                ScreenInput::Tab(i) => {
                    let tabs = c.creative.tabs.clone();
                    if let Some((t, _)) = tabs.shown().nth(*i) {
                        creative::select_tab(s, screen, c, t);
                    }
                    false
                }
            }
        })
        .unwrap_or(false)
    }

    /// The frame's drawing for the creative screen: the slot under the pointer.
    pub(super) fn creative_frame(&mut self, x: f64, y: f64, drag: bool) {
        self.with_creative(|s, screen, c| {
            if let Some(screen) = screen {
                if drag {
                    creative::mouse_dragged(s, screen, c, x, y);
                }
                creative::frame(s, screen, c, x, y);
            }
        });
    }

    /// `containerTick`: the tabs rebuilt for new parameters show again, and a player
    /// without infinite materials gets the inventory screen.
    pub(super) fn creative_tick(&mut self) {
        let rebuilt = self.rebuild_tabs();
        self.with_creative(|s, screen, c| {
            if let Some(screen) = screen {
                creative::refresh_invalidated(s, screen, c, rebuilt);
            }
        });
        let infinite = self
            .world
            .as_ref()
            .is_some_and(|w| w.player.abilities.instabuild);
        if !infinite {
            // `setScreen(new InventoryScreen(player))`: this one is removed, no packet.
            eprintln!("[menu] out of creative: the inventory screen");
            self.show_screen(Kind::Inventory, None);
        }
    }

    /// `handleContainerSetSlot` with the creative screen up: the listener hears the
    /// inventory menu's changes (and sends them back).
    pub(super) fn creative_set_slot(&mut self) {
        if !self.creative_open() {
            return;
        }
        self.creative_ctx(|c, _| c.broadcast(true));
        if let Some(w) = &self.world
            && let Some(s) = &mut self.creative.state.screen
        {
            s.sync(&w.player.menus.inventory);
        }
    }

    /// `Minecraft.handleKeybinds`' hotbar keys with Save or Load Hotbar held, in
    /// creative with no screen: `handleHotbarLoadOrSave`.
    pub(super) fn creative_hotbar_key(&mut self, slot: usize) -> bool {
        let infinite = self
            .world
            .as_ref()
            .is_some_and(|w| w.player.abilities.instabuild);
        let (save, load) = (self.creative.save_down, self.creative.load_down);
        if !infinite || (!load && !save) {
            return false;
        }
        let message = self.creative_ctx(|c, _| creative::hotbar_load_or_save(c, slot, load, save));
        if let Some(Some(m)) = message {
            self.hud_text.set_overlay(m, false);
        }
        true
    }

    /// A new local player (a login, a respawn): its inventory menu heard nothing yet.
    pub(super) fn creative_new_player(&mut self) {
        self.creative.state.screen = None;
        self.creative.state.new_player();
        self.creative.picker_open = false;
    }

    /// The view of the creative screen, for drawing.
    pub(super) fn creative_view(&self) -> Option<Arc<creative::CreativeView>> {
        let s = self.creative.state.screen.as_ref()?;
        Some(Arc::new(creative::view(s, &self.creative.state)))
    }

    /// Where a script's slot is on the creative screen.
    pub(super) fn creative_slot_place(&self, i: usize) -> Option<(i32, i32)> {
        let s = self.creative.state.screen.as_ref()?;
        s.menu().slots.get(i).map(|sl| (sl.x, sl.y))
    }

    /// `closeContainer`'s part: the picker is no longer the open menu.
    pub(super) fn creative_closed(&mut self) {
        self.creative.picker_open = false;
    }
}
