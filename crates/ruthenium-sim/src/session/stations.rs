//! The parts of the beacon's, the enchanting table's and the lectern's screens that are
//! not slots (`BeaconScreen`, `EnchantmentScreen`, `LecternScreen` and its
//! `BookViewScreen`): their buttons, what pressing them sends, and the state the
//! screens keep (the beacon's chosen effects, the lectern's page). The menus' slots are
//! [`crate::menu::stations`]; drawing reads [`StationView`].
//!
//! Each screen's listener on its menu (`ContainerListener`: the beacon's `dataChanged`,
//! the lectern's `slotChanged` and `dataChanged`) runs as the screen's next event or
//! frame finds the menu changed ([`Game::station_sync`]), before anything reads what
//! it sets. Not here: keyboard focus moving between buttons (Tab, the arrows) and
//! pressing the focused one, click events in a book's text.

use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as K, MobEffect};
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::play;

use super::Game;
use crate::item::Stack;
use crate::menu::{Kind, stations};

/// `BeaconBlockEntity.BEACON_EFFECTS`: the effects of each tier, by registry id.
pub const BEACON_EFFECTS: [&[MobEffect]; 4] = [
    &[MobEffect::Speed, MobEffect::Haste],
    &[MobEffect::Resistance, MobEffect::JumpBoost],
    &[MobEffect::Strength],
    &[MobEffect::Regeneration],
];

/// What a screen's button is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// A beacon's effect (`BeaconPowerButton`): a primary one of the first three tiers,
    /// or a secondary; `upgrade` is the one that makes the primary level II
    /// (`BeaconUpgradePowerButton`).
    BeaconEffect {
        effect: i32,
        primary: bool,
        tier: i32,
        upgrade: bool,
    },
    BeaconConfirm,
    BeaconCancel,
    /// A book's `PageButton`s, and the lectern's Done and Take Book.
    PageForward,
    PageBack,
    Done,
    TakeBook,
}

/// A button as the screen has it: where (GUI units), and whether it shows, can be
/// pressed, is chosen (a beacon's) or has the focus (after a press).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    pub kind: ButtonKind,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub visible: bool,
    pub active: bool,
    pub selected: bool,
    pub focused: bool,
}

impl Button {
    fn new(kind: ButtonKind, x: i32, y: i32, width: i32, height: i32) -> Button {
        Button {
            kind,
            x,
            y,
            width,
            height,
            visible: true,
            active: true,
            selected: false,
            focused: false,
        }
    }

    /// `isMouseOver`: shown, pressable and under the pointer.
    fn under(&self, x: f64, y: f64) -> bool {
        self.visible && self.active && self.contains(x, y)
    }

    /// `areCoordinatesInRectangle`.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && y >= f64::from(self.y)
            && x < f64::from(self.x + self.width)
            && y < f64::from(self.y + self.height)
    }
}

/// An offer of an enchanting table as its tooltip names it (`enchantClue`,
/// `levelClue`): the enchantment's name, whether it is a curse, its level and its
/// highest.
#[derive(Debug, Clone, PartialEq)]
pub struct EnchantClue {
    pub name: Option<Arc<Tag>>,
    pub curse: bool,
    pub level: i32,
    pub max_level: i32,
}

/// What the drawing needs of these screens.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StationView {
    pub buttons: Vec<Button>,
    /// A lectern's book: its pages and the one shown (`currentPage`).
    pub pages: Arc<[Arc<Tag>]>,
    pub page: i32,
    /// The player's experience level and `hasInfiniteMaterials`, which colour an
    /// enchanting table's offers.
    pub experience_level: i32,
    pub infinite_materials: bool,
    /// Each offer's enchantment, if the registry has it.
    pub clues: [Option<EnchantClue>; 3],
}

/// The screens' state between events.
#[derive(Debug, Default)]
pub(super) struct StationState {
    buttons: Vec<Button>,
    /// The beacon's chosen effects, by registry id.
    primary: Option<i32>,
    secondary: Option<i32>,
    /// What the listener last saw of the menu: its data, and a lectern's book.
    data_seen: Vec<i32>,
    book_seen: Stack,
    pages: Arc<[Arc<Tag>]>,
    page: i32,
}

impl Game {
    fn station_kind(&self) -> Option<Kind> {
        self.containers.screen.as_ref().map(|s| s.kind)
    }

    /// The screen of `kind` opened (`init`): its buttons, as vanilla makes them.
    pub(super) fn station_opened(&mut self, kind: Kind) {
        let menu_data = self
            .world
            .as_ref()
            .map(|w| vec![0; w.player.menus.container().data.len()])
            .unwrap_or_default();
        self.containers.station = StationState {
            data_seen: menu_data,
            ..StationState::default()
        };
        self.station_layout(kind);
    }

    /// The buttons' places for the screen's size (`init` again on a resize keeps the
    /// state).
    pub(super) fn station_layout(&mut self, kind: Kind) {
        let Some(s) = &self.containers.screen else {
            return;
        };
        let (left, top) = (s.left, s.top);
        let width = self.containers.pointer.width;
        // (`init` again on a resize makes them anew: until the next tick the beacon's
        // are as new ones are.)
        let mut b = Vec::new();
        match kind {
            Kind::Beacon => {
                for (tier, effects) in BEACON_EFFECTS.iter().enumerate().take(3) {
                    let n = effects.len() as i32;
                    let total = n * 22 + (n - 1) * 2;
                    for (c, e) in effects.iter().enumerate() {
                        let mut button = Button::new(
                            ButtonKind::BeaconEffect {
                                effect: e.id() as i32,
                                primary: true,
                                tier: tier as i32,
                                upgrade: false,
                            },
                            left + 76 + c as i32 * 24 - total / 2,
                            top + 22 + tier as i32 * 25,
                            22,
                            22,
                        );
                        button.active = false;
                        b.push(button);
                    }
                }
                let n = BEACON_EFFECTS[3].len() as i32 + 1;
                let total = n * 22 + (n - 1) * 2;
                for (c, e) in BEACON_EFFECTS[3].iter().enumerate() {
                    let mut button = Button::new(
                        ButtonKind::BeaconEffect {
                            effect: e.id() as i32,
                            primary: false,
                            tier: 3,
                            upgrade: false,
                        },
                        left + 167 + c as i32 * 24 - total / 2,
                        top + 47,
                        22,
                        22,
                    );
                    button.active = false;
                    b.push(button);
                }
                let mut upgrade = Button::new(
                    ButtonKind::BeaconEffect {
                        effect: BEACON_EFFECTS[0][0].id() as i32,
                        primary: false,
                        tier: 3,
                        upgrade: true,
                    },
                    left + 167 + (n - 1) * 24 - total / 2,
                    top + 47,
                    22,
                    22,
                );
                upgrade.visible = false;
                b.push(upgrade);
                b.push(Button::new(
                    ButtonKind::BeaconConfirm,
                    left + 164,
                    top + 107,
                    22,
                    22,
                ));
                b.push(Button::new(
                    ButtonKind::BeaconCancel,
                    left + 190,
                    top + 107,
                    22,
                    22,
                ));
            }
            Kind::Lectern => {
                // `LecternScreen.createMenuControls`, then the page buttons.
                let (book_left, book_top) = ((width - 192) / 2, 2);
                let y = book_top + 192 + 2;
                let may_build = self
                    .world
                    .as_ref()
                    .is_some_and(|w| w.player.abilities.may_build);
                if may_build {
                    b.push(Button::new(ButtonKind::Done, width / 2 - 98 - 2, y, 98, 20));
                    b.push(Button::new(ButtonKind::TakeBook, width / 2 + 2, y, 98, 20));
                } else {
                    b.push(Button::new(ButtonKind::Done, (width - 200) / 2, y, 200, 20));
                }
                b.push(Button::new(
                    ButtonKind::PageForward,
                    book_left + 116,
                    book_top + 157,
                    23,
                    13,
                ));
                b.push(Button::new(
                    ButtonKind::PageBack,
                    book_left + 43,
                    book_top + 157,
                    23,
                    13,
                ));
            }
            _ => {}
        }
        self.containers.station.buttons = b;
        if kind == Kind::Lectern {
            self.update_page_buttons();
        }
    }

    /// The menu's listener (`ContainerListener`), for what changed since it last
    /// looked: a beacon's effects follow its data (`BeaconScreen`'s `dataChanged`); a
    /// lectern's book and page follow its slot and data.
    pub(super) fn station_sync(&mut self) {
        let Some(kind) = self.station_kind() else {
            return;
        };
        let Some(w) = &self.world else {
            return;
        };
        let m = w.player.menus.container();
        let st = &mut self.containers.station;
        match kind {
            Kind::Beacon => {
                if m.data != st.data_seen {
                    st.data_seen = m.data.clone();
                    // `getPrimaryEffect`, `getSecondaryEffect`: id + 1, 0 for none.
                    let effect = |v: i32| (v > 0).then_some(v - 1);
                    st.primary = m.data.get(1).copied().and_then(effect);
                    st.secondary = m.data.get(2).copied().and_then(effect);
                }
            }
            Kind::Lectern => {
                let book = m.item(&w.player.inventory, 0).clone();
                let page = m.data.first().copied().unwrap_or(0);
                let mut changed = false;
                if !book.matches(&st.book_seen) {
                    st.book_seen = book.copy();
                    // `bookChanged`: `setBookAccess`.
                    st.pages = book_pages(&book).into();
                    st.page = st.page.clamp(0, st.pages.len() as i32);
                    changed = true;
                }
                if st.data_seen.first().copied() != Some(page) {
                    st.data_seen = vec![page];
                    // `pageChanged`: `setPage` (`Mth.clamp` as Java's, min first).
                    st.page = page.max(0).min(st.pages.len() as i32 - 1);
                    changed = true;
                }
                if changed {
                    self.update_page_buttons();
                }
            }
            _ => {}
        }
    }

    /// `updateButtonVisibility`.
    fn update_page_buttons(&mut self) {
        let st = &mut self.containers.station;
        let (page, pages) = (st.page, st.pages.len() as i32);
        for b in &mut st.buttons {
            match b.kind {
                ButtonKind::PageForward => b.visible = page < pages - 1,
                ButtonKind::PageBack => b.visible = page > 0,
                _ => {}
            }
        }
    }

    /// `BeaconScreen.updateButtons` (each tick, `containerTick`, and after a press).
    pub(super) fn station_tick(&mut self) {
        self.station_sync();
        if self.station_kind() == Some(Kind::Beacon) {
            self.update_beacon_buttons();
        }
    }

    fn update_beacon_buttons(&mut self) {
        let Some(w) = &self.world else {
            return;
        };
        let m = w.player.menus.container();
        let levels = m.data.first().copied().unwrap_or(0);
        let payment = !m.item(&w.player.inventory, 0).is_empty();
        let st = &mut self.containers.station;
        let (primary, secondary) = (st.primary, st.secondary);
        for b in &mut st.buttons {
            match &mut b.kind {
                ButtonKind::BeaconEffect {
                    effect,
                    primary: is_primary,
                    tier,
                    upgrade,
                } => {
                    if *upgrade {
                        match primary {
                            Some(p) => {
                                b.visible = true;
                                *effect = p;
                            }
                            None => {
                                b.visible = false;
                                continue;
                            }
                        }
                    }
                    b.active = *tier < levels;
                    b.selected = Some(*effect) == if *is_primary { primary } else { secondary };
                }
                ButtonKind::BeaconConfirm => b.active = payment && primary.is_some(),
                _ => {}
            }
        }
    }

    /// A press on the screen, before its slots see it: a button under the pointer
    /// takes it (`ContainerEventHandler.mouseClicked`), and an enchanting table's offer
    /// (`EnchantmentScreen.mouseClicked`). Whether it was taken.
    pub(super) fn station_press(&mut self, button: u8, x: f64, y: f64) -> bool {
        let Some(kind) = self.station_kind() else {
            return false;
        };
        self.station_sync();
        if kind == Kind::Enchantment {
            return self.enchantment_press(x, y);
        }
        let st = &mut self.containers.station;
        let Some(i) = st.buttons.iter().position(|b| b.under(x, y)) else {
            return false;
        };
        // Only the left button presses (`isValidClickButton`); a page button keeps no
        // focus.
        if button != 1 {
            return true;
        }
        let pressed = st.buttons[i];
        if !matches!(pressed.kind, ButtonKind::PageForward | ButtonKind::PageBack) {
            for (k, b) in st.buttons.iter_mut().enumerate() {
                b.focused = k == i;
            }
        }
        self.press_button(pressed);
        true
    }

    /// `onPress` of a screen's button.
    fn press_button(&mut self, b: Button) {
        let id = self
            .world
            .as_ref()
            .map_or(0, |w| w.player.menus.container().id);
        match b.kind {
            ButtonKind::BeaconEffect {
                effect, primary, ..
            } => {
                if !b.selected {
                    let st = &mut self.containers.station;
                    if primary {
                        st.primary = Some(effect);
                        if st.secondary != Some(effect) {
                            st.secondary = None;
                        }
                    } else {
                        st.secondary = Some(effect);
                    }
                    self.update_beacon_buttons();
                }
            }
            ButtonKind::BeaconConfirm => {
                let st = &self.containers.station;
                eprintln!("[menu] sent set_beacon {:?} {:?}", st.primary, st.secondary);
                self.connection
                    .send(play::set_beacon(st.primary, st.secondary));
                self.close_container();
            }
            ButtonKind::BeaconCancel | ButtonKind::Done => self.close_container(),
            // `LecternScreen`: the server turns the page.
            ButtonKind::PageBack => self.send_button(id, 1),
            ButtonKind::PageForward => self.send_button(id, 2),
            ButtonKind::TakeBook => self.send_button(id, 3),
        }
    }

    /// `MultiPlayerGameMode.handleInventoryButtonClick`.
    fn send_button(&mut self, container_id: i32, button: i32) {
        eprintln!("[menu] sent button {button} in container {container_id}");
        self.connection
            .send(play::container_button_click(container_id, button));
    }

    /// `EnchantmentScreen.mouseClicked`: an offer under the pointer that
    /// `clickMenuButton` allows is sent (any mouse button).
    fn enchantment_press(&mut self, x: f64, y: f64) -> bool {
        let Some(s) = &self.containers.screen else {
            return false;
        };
        let Some(w) = &self.world else {
            return false;
        };
        let (left, top) = (f64::from(s.left), f64::from(s.top));
        let p = &w.player;
        let m = p.menus.container();
        for i in 0..3 {
            let (xx, yy) = (x - (left + 60.0), y - (top + 14.0 + 19.0 * i as f64));
            if !(xx >= 0.0 && yy >= 0.0 && xx < 108.0 && yy < 19.0) {
                continue;
            }
            // `EnchantmentMenu.clickMenuButton` on the client: the checks alone.
            let (item, lapis) = (m.item(&p.inventory, 0), m.item(&p.inventory, 1));
            let cost = m.data[stations::ENCHANT_COSTS + i];
            let n = i as i32 + 1;
            let infinite = p.creative;
            if (lapis.is_empty() || lapis.count < n) && !infinite {
                continue;
            }
            if cost <= 0
                || item.is_empty()
                || (p.experience_level < n || p.experience_level < cost) && !infinite
            {
                continue;
            }
            let id = m.id;
            self.send_button(id, i as i32);
            return true;
        }
        false
    }

    /// A key on a lectern's screen (`BookViewScreen.keyPressed`): Escape closes it, Page
    /// Up and Page Down press the page buttons whether they show or not. Whether the
    /// screen used it.
    pub(super) fn lectern_key(&mut self, escape: bool, page_up: bool, page_down: bool) -> bool {
        self.station_sync();
        let id = self
            .world
            .as_ref()
            .map_or(0, |w| w.player.menus.container().id);
        if escape {
            self.close_container();
        } else if page_up {
            self.send_button(id, 1);
        } else if page_down {
            self.send_button(id, 2);
        } else {
            return false;
        }
        true
    }

    /// What the drawing needs (with the frame's view).
    pub(super) fn station_view(&self) -> Option<StationView> {
        let kind = self.station_kind()?;
        if !matches!(kind, Kind::Beacon | Kind::Enchantment | Kind::Lectern) {
            return None;
        }
        let w = self.world.as_ref()?;
        let p = &w.player;
        let st = &self.containers.station;
        let mut clues = [None, None, None];
        if kind == Kind::Enchantment {
            let m = p.menus.container();
            let items = self.physics.as_ref().map(|ph| &ph.items);
            for (i, c) in clues.iter_mut().enumerate() {
                let id = m.data[stations::ENCHANT_CLUES + i];
                let Some(e) = items.and_then(|it| it.enchantments.get(id)) else {
                    continue;
                };
                *c = Some(EnchantClue {
                    name: e.description.clone(),
                    curse: items
                        .is_some_and(|it| it.tags.enchantments.contains("minecraft:curse", id)),
                    level: m.data[stations::ENCHANT_LEVELS + i],
                    max_level: e.max_level,
                });
            }
        }
        Some(StationView {
            buttons: st.buttons.clone(),
            pages: st.pages.clone(),
            page: st.page,
            experience_level: p.experience_level,
            infinite_materials: p.creative,
            clues,
        })
    }
}

/// `BookViewScreen.BookAccess.fromItem`: a written book's pages (unfiltered), else a
/// book and quill's as plain text; none for anything else.
pub fn book_pages(book: &Stack) -> Vec<Arc<Tag>> {
    let raw = |kind: K| {
        book.patch.added.iter().find_map(|(k, c)| match c {
            ruthenium_protocol::item::Component::Other { data, .. } if *k == kind.id() as i32 => {
                Some(data.as_slice())
            }
            _ => None,
        })
    };
    if let Some(data) = raw(K::WrittenBookContent) {
        return written_pages(data).unwrap_or_default();
    }
    if let Some(data) = raw(K::WritableBookContent) {
        return writable_pages(data).unwrap_or_default();
    }
    Vec::new()
}

/// `Filterable`: the raw value, then the filtered one if there is one.
fn skip_filtered<T>(
    r: &mut ruthenium_protocol::wire::Reader<'_>,
    f: impl Fn(&mut ruthenium_protocol::wire::Reader<'_>) -> Option<T>,
) -> Option<T> {
    let raw = f(r)?;
    if r.bool().ok()? {
        f(r)?;
    }
    Some(raw)
}

/// `WrittenBookContent.STREAM_CODEC`'s pages.
fn written_pages(data: &[u8]) -> Option<Vec<Arc<Tag>>> {
    let mut r = ruthenium_protocol::wire::Reader::new(data);
    skip_filtered(&mut r, |r| r.string(32).ok().map(drop))?;
    r.string(32767).ok()?;
    r.varint().ok()?;
    let n = r.count(4096).ok()?;
    let mut pages = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let page = skip_filtered(&mut r, |r| r.nbt().ok())?;
        pages.push(Arc::new(page.unwrap_or_else(|| Tag::String(String::new()))));
    }
    Some(pages)
}

/// `WritableBookContent.STREAM_CODEC`'s pages, each `Component.literal`.
fn writable_pages(data: &[u8]) -> Option<Vec<Arc<Tag>>> {
    let mut r = ruthenium_protocol::wire::Reader::new(data);
    let n = r.count(100).ok()?;
    let mut pages = Vec::with_capacity(n);
    for _ in 0..n {
        let text = skip_filtered(&mut r, |r| r.string(1024).ok().map(str::to_owned))?;
        pages.push(Arc::new(Tag::String(text)));
    }
    Some(pages)
}
