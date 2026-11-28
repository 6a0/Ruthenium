//! The menus' side of the session: the container screens the game thread runs
//! (`MenuScreens.create`, `InventoryScreen`, `LocalPlayer.closeContainer` and
//! `clientSideCloseContainer`), the packets that open, fill and close menus, and the
//! inventory key (`Minecraft.handleKeybinds`).
//!
//! The screen's mouse and key events come with the frame's input and run where the
//! frame's events do (after the packets, before the ticks); a drag runs after the
//! ticks, with the frame's pointer (`handleAccumulatedMovement`). Each click is sent
//! as it is made (`MultiPlayerGameMode.handleContainerInput`).
//!
//! A menu type the client doesn't run (an anvil, an enchanting table, a horse's
//! inventory...) opens as any other does (`Gui.setScreen`: the keys and the mouse let
//! go), with a placeholder screen ([`Kind::Placeholder`]) that keeps the menu's
//! container id and slots, so the server's packets for it land, clicks nothing, and
//! closes on Escape or the inventory key as a container screen does. Closing such a
//! menu at once instead left a held use key using the block again a few ticks later,
//! open after open, which anticheats see.

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::play;
use std::collections::HashMap;

use super::{Configured, Game, SessionEvent, lock};
use crate::inventory::{Hand, Inventory};
use crate::item::{Items, Stack};
use crate::menu::screen::{self, Binds, Clicker, Mouse, Screen};
use crate::menu::{Input, Kind, Menu, Menus, Placeholder, Rule, Who};

/// `MouseHandler.DOUBLE_CLICK_THRESHOLD_MS`.
const DOUBLE_CLICK: Duration = Duration::from_millis(250);

impl Configured {
    /// The synced registries' keys by network id, as hashing stacks reads them.
    pub(super) fn keys(&self) -> HashMap<String, Vec<String>> {
        self.registries
            .iter()
            .map(|(r, entries)| (r.clone(), entries.iter().map(|e| e.id.clone()).collect()))
            .collect()
    }
}

/// Where a screen event happens: in GUI units, or over a slot or outside the window
/// (for scripts, which know slots and not places).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum At {
    Gui(f64, f64),
    Slot(usize),
    Outside,
    /// The middle of a screen's button ([`super::stations`]), of a beacon's effect
    /// (primary or secondary), or of an enchanting table's offer (0 to 2).
    Button(super::ButtonKind),
    Effect {
        effect: i32,
        primary: bool,
    },
    Offer(usize),
}

/// A container screen's event, from the window or a script.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenInput {
    /// A mouse button went down (SDL's numbering: 1 left, 2 middle, 3 right) or up, with
    /// the bindings on it and whether shift was held.
    Press {
        button: u8,
        at: At,
        shift: bool,
        binds: Binds,
        when: Instant,
    },
    Release {
        button: u8,
        at: At,
        shift: bool,
        binds: Binds,
    },
    /// The pointer moved with a button held (once a frame).
    Drag { at: At },
    /// A key went down (or repeated), with its bindings, ctrl, and the pointer.
    Key { binds: Binds, ctrl: bool, at: At },
    /// A click as the screen's `slotClicked` would make it, without the screen's
    /// logic (scripts).
    Click {
        slot: i32,
        button: i32,
        input: Input,
    },
    /// A key over the creative screen, with what its search box reads of it.
    CreativeKey {
        key: crate::menu::creative::CreativeKey,
        at: At,
    },
    /// A key went up (the creative screen's `keyReleased`).
    KeyReleased,
    /// Typed text (the creative screen's `charTyped`).
    Text(String),
    /// The wheel over the screen: SDL's vertical amount (`mouseScrolled`).
    Scroll { y: f64, at: At },
    /// The creative screen's tab of this place among those shown is selected (scripts).
    Tab(usize),
}

/// The frame's pointer over a screen: where (GUI units), the GUI's size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pointer {
    pub x: f64,
    pub y: f64,
    pub width: i32,
    pub height: i32,
}

/// A container screen as drawn: the menu's slots and stacks, the cursor's stack and
/// the screen's state.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerView {
    /// A new number for each screen opened.
    pub serial: u64,
    pub kind: Kind,
    /// The server's title (`None`: the inventory's, "Crafting").
    pub title: Option<Arc<Tag>>,
    pub image_width: i32,
    pub image_height: i32,
    /// Each slot: where, its rule, and its stack.
    pub slots: Vec<(i32, i32, Rule)>,
    pub stacks: Vec<Stack>,
    pub carried: Stack,
    pub screen: screen::View,
    /// The menu's data slots (a furnace's progress).
    pub data: Vec<i32>,
    /// Each slot's cooldown as of this tick: ticks left and of how many.
    pub cooldowns: Vec<crate::hud::Cooldown>,
    /// The creative inventory's tabs, scroller and search box.
    pub creative: Option<Arc<crate::menu::creative::CreativeView>>,
    /// A beacon's, an enchanting table's or a lectern's buttons and state.
    pub station: Option<super::StationView>,
}

/// The session's container screen state.
#[derive(Debug, Default)]
pub(super) struct Containers {
    pub(super) screen: Option<Screen>,
    title: Option<Arc<Tag>>,
    serial: u64,
    pub(super) pointer: Pointer,
    last_click: Option<(Instant, u64, u8)>,
    /// Inventory key presses not yet read (`keyInventory.clickCount`).
    pub(super) inventory_clicks: u32,
    /// The synced registries' keys, for hashing stacks (set as play begins).
    pub(super) registries: HashMap<String, Vec<String>>,
    pub(super) station: super::stations::StationState,
}

static NO_ITEMS: LazyLock<Items> = LazyLock::new(Items::default);

/// What a screen event clicks through.
struct Ctx<'a> {
    menus: &'a mut Menus,
    inv: &'a mut Inventory,
    who: Who<'a>,
    registries: &'a HashMap<String, Vec<String>>,
    sent: Vec<Vec<u8>>,
    dropped: usize,
}

impl Clicker for Ctx<'_> {
    fn menu(&self) -> &Menu {
        self.menus.container()
    }

    fn inventory(&self) -> &Inventory {
        self.inv
    }

    fn who(&self) -> Who<'_> {
        self.who
    }

    fn click(&mut self, slot: i32, button: i32, input: Input) {
        let id = self.menus.container().id;
        if let Some((packet, outcome)) = self.menus.container_mut().handle_container_input(
            self.inv,
            &self.who,
            id,
            slot,
            button,
            input,
            self.registries,
        ) {
            self.sent.push(packet);
            self.dropped += outcome.dropped.len();
        }
    }
}

/// `HasCustomInventoryScreen`: horses and their kin, nautiluses, chest boats.
fn has_custom_inventory(entity_type: i32) -> bool {
    use ruthenium_data::v26_3::registries::EntityType;
    let Some(t) = u32::try_from(entity_type)
        .ok()
        .and_then(EntityType::from_id)
    else {
        return false;
    };
    let name = t.name().trim_start_matches("minecraft:");
    matches!(
        name,
        "horse"
            | "donkey"
            | "mule"
            | "skeleton_horse"
            | "zombie_horse"
            | "llama"
            | "trader_llama"
            | "camel"
            | "camel_husk"
    ) || name.ends_with("nautilus")
        || name.ends_with("chest_boat")
        || name.ends_with("chest_raft")
}

/// The title of a mount's inventory screen, if `entity_type` is an `AbstractHorse` or an
/// `AbstractNautilus` (`getDisplayName`: its type's name; custom names aren't kept).
fn mount_title(entity_type: i32) -> Option<Tag> {
    use ruthenium_data::v26_3::registries::EntityType;
    let t = EntityType::from_id(u32::try_from(entity_type).ok()?)?;
    let name = t.name().trim_start_matches("minecraft:");
    let mount = matches!(
        name,
        "horse"
            | "donkey"
            | "mule"
            | "skeleton_horse"
            | "zombie_horse"
            | "llama"
            | "trader_llama"
            | "camel"
            | "camel_husk"
    ) || name.ends_with("nautilus");
    mount.then(|| {
        let mut c = ruthenium_protocol::nbt::Compound::new();
        c.put("translate", Tag::String(format!("entity.minecraft.{name}")));
        Tag::Compound(c)
    })
}

impl Game {
    /// Runs `f` on the open screen with what it clicks through, then sends what it
    /// clicked and swings the hand for what it dropped.
    fn with_screen<R>(&mut self, f: impl FnOnce(&mut Screen, &mut Ctx<'_>) -> R) -> Option<R> {
        let screen = self.containers.screen.as_mut()?;
        let w = self.world.as_mut()?;
        let items: &Items = self.physics.as_ref().map_or(&NO_ITEMS, |p| &p.items);
        let creative = w.player.creative;
        let p = &mut w.player;
        let mut ctx = Ctx {
            menus: &mut p.menus,
            inv: &mut p.inventory,
            who: Who {
                items,
                recipes: &self.recipe_sets,
                creative,
            },
            registries: &self.containers.registries,
            sent: Vec::new(),
            dropped: 0,
        };
        let r = f(screen, &mut ctx);
        let (sent, dropped) = (std::mem::take(&mut ctx.sent), ctx.dropped);
        for packet in sent {
            self.connection.send(packet);
        }
        for _ in 0..dropped {
            p.swing(Hand::Main, crate::item::DEFAULT_SWING);
        }
        Some(r)
    }

    /// Where an event happens, in GUI units.
    fn place(&self, at: At) -> (f64, f64) {
        let s = self.containers.screen.as_ref();
        let menu = self.world.as_ref().map(|w| w.player.menus.container());
        match (at, s, menu) {
            (At::Gui(x, y), ..) => (x, y),
            (At::Slot(i), Some(s), _) if s.kind == Kind::Creative => {
                self.creative_slot_place(i).map_or((-1.0, -1.0), |(x, y)| {
                    (f64::from(s.left + x + 8), f64::from(s.top + y + 8))
                })
            }
            (At::Slot(i), Some(s), Some(m)) => m.slots.get(i).map_or((-1.0, -1.0), |slot| {
                (
                    f64::from(s.left + slot.x + 8),
                    f64::from(s.top + slot.y + 8),
                )
            }),
            (At::Outside, Some(s), _) => (f64::from(s.left - 20), f64::from(s.top - 20)),
            (At::Offer(i), Some(s), _) => (
                f64::from(s.left + 60 + 54),
                f64::from(s.top + 14 + 19 * i as i32 + 9),
            ),
            (At::Button(_) | At::Effect { .. }, ..) => self.station_place(at),
            _ => (-1.0, -1.0),
        }
    }

    /// One of the frame's screen events (`MouseHandler.onButton`, `KeyboardHandler`).
    pub(super) fn screen_input(&mut self, e: &ScreenInput) {
        let Some(s) = &self.containers.screen else {
            return;
        };
        // A lectern's book screen has buttons and keys of its own, no slots.
        if s.kind == Kind::Lectern {
            match *e {
                ScreenInput::Press { button, at, .. } => {
                    let (x, y) = self.place(at);
                    self.station_press(button, x, y);
                }
                ScreenInput::Key { binds, .. } => {
                    self.lectern_key(binds.escape, binds.page_up, binds.page_down);
                }
                _ => {}
            }
            return;
        }
        // A placeholder clicks nothing; Escape and the inventory key close it.
        if matches!(s.kind, Kind::Placeholder(_)) {
            if let ScreenInput::Key { binds, .. } = e
                && (binds.escape || binds.inventory)
            {
                self.close_container();
            }
            return;
        }
        let serial = self.containers.serial;
        if s.kind == Kind::Creative {
            let at = match e {
                ScreenInput::Press { at, .. }
                | ScreenInput::Release { at, .. }
                | ScreenInput::Drag { at }
                | ScreenInput::Key { at, .. }
                | ScreenInput::CreativeKey { at, .. }
                | ScreenInput::Scroll { at, .. } => *at,
                _ => At::Gui(self.containers.pointer.x, self.containers.pointer.y),
            };
            let place = self.place(at);
            let mut double = false;
            if let ScreenInput::Press { button, when, .. } = *e {
                double = self.containers.last_click.is_some_and(|(t, s, b)| {
                    s == serial && b == button && when.saturating_duration_since(t) < DOUBLE_CLICK
                });
                self.containers.last_click = Some((when, serial, button));
            }
            if self.creative_input(e, place, double) {
                self.close_container();
            }
            return;
        }
        match *e {
            ScreenInput::Press {
                button,
                at,
                shift,
                binds,
                when,
            } => {
                let (x, y) = self.place(at);
                let double = self.containers.last_click.is_some_and(|(t, s, b)| {
                    s == serial && b == button && when.saturating_duration_since(t) < DOUBLE_CLICK
                });
                let m = Mouse {
                    button,
                    x,
                    y,
                    shift,
                    binds,
                };
                // A button of the screen takes the press first.
                if !self.station_press(button, x, y) {
                    self.with_screen(|s, c| s.mouse_clicked(c, m, double));
                }
                self.containers.last_click = Some((when, serial, button));
            }
            ScreenInput::Release {
                button,
                at,
                shift,
                binds,
            } => {
                let (x, y) = self.place(at);
                let m = Mouse {
                    button,
                    x,
                    y,
                    shift,
                    binds,
                };
                self.with_screen(|s, c| s.mouse_released(c, m));
            }
            ScreenInput::Drag { at } => {
                let (x, y) = self.place(at);
                if crate::menu::log_enabled() {
                    eprintln!("[menu] drag at {x}, {y}");
                }
                self.with_screen(|s, c| s.mouse_dragged(c, x, y));
            }
            ScreenInput::Key { binds, ctrl, at } => {
                let (x, y) = self.place(at);
                if self.with_screen(|s, c| s.key_pressed(c, binds, ctrl, x, y)) == Some(true) {
                    // `onClose`.
                    self.close_container();
                }
            }
            ScreenInput::Click {
                slot,
                button,
                input,
            } => {
                self.with_screen(|_, c| c.click(slot, button, input));
            }
            // The creative screen's alone.
            ScreenInput::CreativeKey { .. }
            | ScreenInput::KeyReleased
            | ScreenInput::Text(_)
            | ScreenInput::Scroll { .. }
            | ScreenInput::Tab(_) => {}
        }
    }

    /// The frame's pointer: the screen follows the GUI's size.
    pub(super) fn set_pointer(&mut self, p: Pointer) {
        let resized = (p.width, p.height)
            != (
                self.containers.pointer.width,
                self.containers.pointer.height,
            );
        self.containers.pointer = p;
        if resized && let Some(s) = &mut self.containers.screen {
            s.resize(p.width, p.height);
            let kind = s.kind;
            self.station_layout(kind);
            let Some(s) = &self.containers.screen else {
                return;
            };
            if crate::menu::log_enabled() {
                eprintln!(
                    "[menu] screen at {}, {} on a GUI of {} by {}",
                    s.left, s.top, p.width, p.height
                );
            }
        }
    }

    /// After the ticks (`handleAccumulatedMovement` and the frame's drawing): a drag
    /// with the pointer that moved, the slot under it, and what is drawn.
    pub(super) fn container_frame(&mut self, drag: bool) {
        let Pointer { x, y, .. } = self.containers.pointer;
        if self.creative_open() {
            self.creative_frame(x, y, drag);
            let view = self.containers.screen.as_ref().and_then(|s| {
                let w = self.world.as_ref()?;
                let cs = self.creative.state.screen.as_ref()?;
                let p = &w.player;
                let m = cs.menu();
                let stacks: Vec<Stack> = (0..m.slots.len())
                    .map(|i| m.item(&p.inventory, i).clone())
                    .collect();
                let cooldowns = stacks
                    .iter()
                    .map(|st| {
                        if st.is_empty() {
                            crate::hud::Cooldown::default()
                        } else {
                            p.cooldowns.of(&st.cooldown_group())
                        }
                    })
                    .collect();
                Some(ContainerView {
                    serial: self.containers.serial,
                    kind: s.kind,
                    title: None,
                    image_width: s.image_width,
                    image_height: s.image_height,
                    slots: m.slots.iter().map(|sl| (sl.x, sl.y, sl.rule)).collect(),
                    stacks,
                    carried: p.menus.inventory.carried.clone(),
                    screen: s.view(m),
                    data: Vec::new(),
                    cooldowns,
                    creative: self.creative_view(),
                    station: None,
                })
            });
            *lock(&self.container_view) = view.map(Arc::new);
            return;
        }
        if drag {
            if crate::menu::log_enabled() {
                eprintln!("[menu] drag at {x}, {y}");
            }
            self.with_screen(|s, c| s.mouse_dragged(c, x, y));
        }
        self.with_screen(|s, c| s.frame(c, x, y));
        self.station_sync();
        let station = self.station_view();
        let view = self.containers.screen.as_ref().and_then(|s| {
            let w = self.world.as_ref()?;
            let p = &w.player;
            let m = p.menus.container();
            let stacks: Vec<Stack> = (0..m.slots.len())
                .map(|i| m.item(&p.inventory, i).clone())
                .collect();
            let cooldowns = stacks
                .iter()
                .map(|st| {
                    if st.is_empty() {
                        crate::hud::Cooldown::default()
                    } else {
                        p.cooldowns.of(&st.cooldown_group())
                    }
                })
                .collect();
            Some(ContainerView {
                serial: self.containers.serial,
                kind: s.kind,
                title: self.containers.title.clone(),
                image_width: s.image_width,
                image_height: s.image_height,
                // (A slot that isn't active is neither drawn nor pointed at.)
                slots: (m.slots.iter().enumerate())
                    .map(|(i, sl)| match m.slot_active(i) {
                        true => (sl.x, sl.y, sl.rule),
                        false => (crate::menu::HIDDEN, crate::menu::HIDDEN, sl.rule),
                    })
                    .collect(),
                stacks,
                carried: m.carried.clone(),
                screen: s.view(m),
                data: m.data.clone(),
                cooldowns,
                creative: None,
                station,
            })
        });
        *lock(&self.container_view) = view.map(Arc::new);
    }

    /// `KeyMapping.releaseAll`: the keys go up and their presses are forgotten.
    pub(super) fn release_all(&mut self) {
        self.release_keys();
        self.hotbar_clicks = [0; 9];
        self.use_keys.use_clicks = 0;
        self.use_keys.swap_clicks = 0;
        self.use_keys.attack_clicks = 0;
        self.containers.inventory_clicks = 0;
    }

    /// `gui.setScreen` of a container screen for the menu now open (`kind`'s).
    pub(super) fn show_screen(&mut self, kind: Kind, title: Option<Tag>) {
        self.replace_screen();
        let Pointer { width, height, .. } = self.containers.pointer;
        self.containers.screen = Some(Screen::new(kind, width, height));
        self.containers.title = title.map(Arc::new);
        self.containers.serial += 1;
        self.station_opened(kind);
        // It takes the place of the death screen or the bed screen.
        self.death_screen = None;
        self.bed_screen = false;
        self.release_all();
        eprintln!("[menu] {kind:?} screen open");
        self.report(SessionEvent::ContainerOpened);
    }

    /// The container screen goes (`Screen.removed`: its menu's `removed`).
    fn replace_screen(&mut self) -> bool {
        let Some(s) = self.containers.screen.take() else {
            return false;
        };
        if s.kind == Kind::Creative {
            self.creative_removed();
        }
        if s.kind == Kind::Inventory
            && let Some(w) = &mut self.world
        {
            w.player.menus.inventory.removed();
        }
        true
    }

    /// `ClientPacketListener.handleOpenScreen` (`MenuScreens.create`).
    pub(super) fn open_screen(&mut self, container_id: i32, menu_type: i32, title: Tag) {
        if self.world.is_none() {
            return;
        }
        eprintln!("[menu] open_screen container {container_id} type {menu_type}");
        match Kind::from_menu_type(menu_type) {
            Some(kind) => {
                let menu = self.new_menu(kind, container_id);
                if let Some(w) = &mut self.world {
                    w.player.menus.open = Some(menu);
                }
                self.show_screen(kind, Some(title));
            }
            None => {
                let name = u32::try_from(menu_type)
                    .ok()
                    .and_then(ruthenium_data::v26_3::registries::Menu::from_id)
                    .map_or("?", |m| m.name());
                eprintln!(
                    "[menu] no screen of its own for {name} (container {container_id}): a placeholder"
                );
                self.open_placeholder(container_id, Placeholder::for_menu(name), title);
            }
        }
    }

    /// A menu the client doesn't run opens as one it runs would, with a placeholder
    /// screen.
    fn open_placeholder(&mut self, container_id: i32, p: Placeholder, title: Tag) {
        let kind = Kind::Placeholder(p);
        if let Some(w) = &mut self.world {
            w.player.menus.open = Some(Menu::new(kind, container_id));
        }
        self.show_screen(kind, Some(title));
    }

    /// `handleMountScreenOpen`: a horse's or a nautilus's inventory, which the client
    /// doesn't run, opens with a placeholder screen; for anything else (or an entity
    /// the client doesn't know) nothing happens.
    pub(super) fn mount_screen_open(&mut self, container_id: i32, columns: i32, entity_id: i32) {
        let Some(e) = self.world.as_ref().and_then(|w| w.entities.get(entity_id)) else {
            eprintln!("[menu] mount screen for unknown entity {entity_id}: nothing opens");
            return;
        };
        let Some(title) = mount_title(e.entity_type) else {
            eprintln!("[menu] mount screen for entity {entity_id}, not a mount: nothing opens");
            return;
        };
        let name = u32::try_from(e.entity_type)
            .ok()
            .and_then(ruthenium_data::v26_3::registries::EntityType::from_id)
            .map_or("", |t| t.name());
        let state = crate::menu::mount::MountState {
            alive: !e.removed && e.health > 0.0,
            baby: e.baby,
            // `AbstractHorse.isTamed` is its flags' bit 2; a nautilus is a
            // `TamableAnimal`.
            tamed: if name.ends_with("nautilus") {
                e.tame
            } else {
                e.horse_flags & 2 != 0
            },
        };
        let tags = self
            .physics
            .as_ref()
            .map(|p| p.items.tags.clone())
            .unwrap_or_default();
        let entity_type = e.entity_type;
        let Some(mount) = crate::menu::mount::Mount::new(name, entity_type, columns, state, &tags)
        else {
            self.open_placeholder(container_id, Placeholder::mount(columns), title);
            return;
        };
        eprintln!(
            "[menu] entity {entity_id}'s inventory ({columns} columns, container {container_id}): {mount:?}"
        );
        let kind = Kind::Mount(mount);
        let menu = self.new_menu(kind, container_id);
        if let Some(w) = &mut self.world {
            w.player.menus.open = Some(menu);
        }
        self.show_screen(kind, Some(title));
    }

    /// `LocalPlayer.closeContainer`: the server is told, then the client closes it.
    pub(super) fn close_container(&mut self) {
        let Some(w) = &self.world else {
            return;
        };
        let id = w.player.menus.container().id;
        eprintln!("[menu] sent close container {id}");
        self.creative_closed();
        self.connection.send(play::container_close(id));
        self.client_side_close_container();
    }

    /// `clientSideCloseContainer`: the player's own menu is the one open again and the
    /// screen goes (`setScreen(null)`, which shows a dead player the death screen).
    pub(super) fn client_side_close_container(&mut self) {
        let Some(w) = &mut self.world else {
            return;
        };
        w.player.menus.open = None;
        if self.replace_screen() {
            eprintln!("[menu] screen closed");
            self.report(SessionEvent::ContainerClosed);
        }
        let Some(w) = &self.world else {
            return;
        };
        if w.player.is_dead_or_dying() && self.death_screen.is_none() {
            if w.player.show_death_screen {
                self.open_death_screen(None);
            } else {
                self.connection
                    .send(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
            }
        }
    }

    /// The level or the player is replaced (a respawn, a new login, a
    /// reconfiguration): another menu open is closed first if `tell` (`handleRespawn`),
    /// and any screen goes without a word.
    pub(super) fn drop_screen(&mut self, tell: bool) {
        // (The creative picker open counts: `containerMenu` isn't the inventory's.)
        if tell
            && (self.creative.picker_open
                || self
                    .world
                    .as_ref()
                    .is_some_and(|w| w.player.menus.has_container_open()))
        {
            self.close_container();
        }
        self.creative_new_player();
        if self.containers.screen.take().is_some() {
            self.report(SessionEvent::ContainerClosed);
        }
        *lock(&self.container_view) = None;
    }

    /// `Minecraft.handleKeybinds`' inventory key, one press at a time: riding what has
    /// its own inventory asks the server to open it; else the inventory screen opens
    /// (the creative one isn't run: nothing opens in creative). Whether a screen opened.
    pub(super) fn inventory_key(&mut self) -> bool {
        while self.containers.inventory_clicks > 0 {
            self.containers.inventory_clicks -= 1;
            let Some(w) = &self.world else {
                return false;
            };
            let p = &w.player;
            let custom = p
                .riding
                .as_ref()
                .and_then(|r| w.entities.get(r.vehicle))
                .is_some_and(|v| has_custom_inventory(v.entity_type));
            if custom {
                self.connection.send(play::player_command(
                    p.entity_id,
                    play::PLAYER_COMMAND_OPEN_INVENTORY,
                    0,
                ));
            } else if p.abilities.instabuild {
                // `InventoryScreen.init` gives way to the creative screen at once.
                self.open_creative();
                return true;
            } else {
                self.show_screen(Kind::Inventory, None);
                return true;
            }
        }
        false
    }

    /// `Gui.tick`'s `screen.tick()` for a container screen: it closes for a player no
    /// longer alive (`AbstractContainerScreen.tick`), and the inventory's for one with
    /// infinite materials (vanilla's turns into the creative one, not run here).
    pub(super) fn container_tick(&mut self) {
        let Some(s) = &self.containers.screen else {
            return;
        };
        let Some(w) = &self.world else {
            return;
        };
        let p = &w.player;
        // (A lectern's book screen isn't a container screen: it has no such tick.)
        if s.kind == Kind::Lectern {
            self.station_tick();
        } else if !p.is_alive() || p.health <= 0.0 {
            self.close_container();
        } else if s.kind == Kind::Inventory && p.abilities.instabuild {
            // `InventoryScreen.containerTick`: the creative screen takes its place.
            self.open_creative();
        } else if s.kind == Kind::Creative {
            self.creative_tick();
        } else {
            self.station_tick();
        }
    }

    pub(super) fn container_screen_open(&self) -> bool {
        self.containers.screen.is_some()
    }
}

impl Game {
    /// A menu of `kind` opening with container id `id`, with the session's items (a
    /// grindstone makes its result with them).
    fn new_menu(&self, kind: Kind, id: i32) -> Menu {
        let mut m = Menu::new(kind, id);
        m.items = crate::menu::stations::MenuItems(self.physics.as_ref().map(|p| p.items.clone()));
        m
    }

    /// The tags the menus read (a beacon's payment, a brewing stand's potions, who may
    /// wear a mount's armour, curses), from the server's tags: into the items, where the
    /// menus' rules find them.
    pub(super) fn apply_menu_tags(
        &mut self,
        tags: &[ruthenium_protocol::packets::configuration::RegistryTags],
    ) {
        let Some(physics) = &mut self.physics else {
            return;
        };
        let items = Arc::make_mut(&mut physics.items);
        let mine = Arc::make_mut(&mut items.tags);
        for t in tags {
            match t.registry.as_str() {
                "minecraft:item" => {
                    for (tag, ids) in &t.tags {
                        let names = ids.iter().filter_map(|&id| {
                            ruthenium_data::v26_3::registries::Item::from_id(
                                u32::try_from(id).ok()?,
                            )
                            .map(|i| i.name())
                        });
                        mine.items.set(tag, names);
                    }
                }
                "minecraft:entity_type" => {
                    for (tag, ids) in &t.tags {
                        mine.entity_types.set(tag, ids.iter().copied());
                    }
                }
                "minecraft:enchantment" => {
                    for (tag, ids) in &t.tags {
                        mine.enchantments.set(tag, ids.iter().copied());
                    }
                }
                _ => {}
            }
        }
    }
}
