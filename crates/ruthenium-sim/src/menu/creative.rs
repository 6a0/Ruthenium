//! The creative inventory (`CreativeModeInventoryScreen`): its tabs (`tabs`), the item
//! picker's menu (`ItemPickerMenu`: five rows of the tab's stacks over the hotbar, or in
//! the inventory tab the player's own slots moved about and the destroy slot), scrolling,
//! the search box (`edit_box`) and its trees (`search`), and its clicks. Its clicks
//! don't go through `handleContainerInput` and `ServerboundContainerClickPacket` as
//! other screens' do: `slotClicked` changes the player's inventory menu itself, and the
//! server hears of each slot that changed through `ServerboundSetCreativeModeSlotPacket`
//! (the screen's `CreativeInventoryListener` on the inventory menu's
//! `broadcastChanges`, `handleCreativeModeItemAdd` and `handleCreativeModeItemDrop`).
//! Saved hotbars are in `hotbars`.
//!
//! What vanilla keeps in statics lives in [`Creative`] for the session (the tabs'
//! contents, rebuilt for each new configuration as `needsUpdate` sees a new registry
//! access), but the selected tab, which lasts for the client's run as vanilla's does.
//!
//! Not here: the player drawn in the inventory tab, the effects beside the window
//! (`EffectsInInventory`), the tooltip's lines past the name (the tabs' names in blue,
//! the `#` search's tags), placing the search box's caret with the mouse, and bundles'
//! own clicks (`ItemSlotMouseAction`), as on the other screens.

pub mod edit_box;
pub mod hotbars;
pub mod search;
pub mod tabs;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ruthenium_protocol::item::ItemStack;
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::play;
use ruthenium_protocol::text::{Lang, StyledText};

use super::screen::{Binds, Clicker, Mouse, Screen};
use super::{Container, Input, Kind, Menu, Menus, Rule, SLOT_OUTSIDE, Slot, Who};
use crate::inventory::Inventory;
use crate::item::{ItemTags, Items, Stack};
use edit_box::{EditBox, KeyPress};
use tabs::{Row, Tab, TabType, Tabs};

/// `imageWidth`, `imageHeight`.
pub const IMAGE_WIDTH: i32 = 195;
pub const IMAGE_HEIGHT: i32 = 136;
/// `CONTAINER`'s size: five rows of nine.
pub const CONTAINER_SLOTS: usize = 45;
/// Where the destroy slot is among the inventory tab's slots.
pub const DESTROY_SLOT: usize = 46;
/// The inventory menu's slots (`InventoryMenu`).
const INVENTORY_SLOTS: usize = 46;

/// The tab last selected (`selectedTab`, a static): by key, for the client's run.
static SELECTED: Mutex<Option<String>> = Mutex::new(None);

fn selected_key() -> String {
    SELECTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| "minecraft:building_blocks".into())
}

fn set_selected_key(key: &str) {
    *SELECTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.to_owned());
}

/// What the tabs were last built for (`CACHED_PARAMETERS`): the configuration (a new
/// one is a new registry access), the enabled features and the permission.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltFor {
    pub configuration: u64,
    pub features: Vec<String>,
    pub permissions: bool,
}

/// The creative search's trees (`SessionSearchTrees`' `creativeNameSearch` and
/// `creativeTagSearch`) over the search tab's stacks.
#[derive(Debug, Clone, Default)]
pub struct Trees {
    pub items: Vec<Stack>,
    names: search::FullTextTree,
    tags: search::IdTree,
}

impl Trees {
    /// `updateCreativeTooltips` and `updateCreativeTags`: each stack's lines in the
    /// language, its item's identifier, and its item's tags.
    pub fn build(items: &[tabs::CreativeItem], lang: &dyn Lang, item_tags: &ItemTags) -> Trees {
        let names: Vec<Vec<Vec<u16>>> = items
            .iter()
            .map(|i| {
                i.lines
                    .iter()
                    .filter_map(|l| search::search_line(&StyledText::from_nbt_in(l, lang).plain()))
                    .collect()
            })
            .collect();
        let keys: Vec<&str> = items.iter().map(|i| item_key(i.stack.item)).collect();
        let tag_lists: Vec<Vec<&str>> = items
            .iter()
            .map(|i| item_tags.tags_of(i.stack.item))
            .collect();
        Trees {
            items: items.iter().map(|i| i.stack.clone()).collect(),
            names: search::FullTextTree::new(&names, keys.iter().map(|k| vec![*k])),
            tags: search::IdTree::new(tag_lists),
        }
    }
}

fn item_key(item: i32) -> &'static str {
    u32::try_from(item)
        .ok()
        .and_then(ruthenium_data::v26_3::registries::Item::from_id)
        .map_or("minecraft:air", |it| it.name())
}

/// What a creative screen event sent and did besides the menus.
#[derive(Debug, Default)]
pub struct Sent {
    pub packets: Vec<Vec<u8>>,
    /// Stacks the player dropped (each swings the hand).
    pub dropped: usize,
    /// Text the search box put on the clipboard.
    pub clipboard: Option<String>,
}

/// The session-wide part: the tabs, the search trees, the player's inventory menu's
/// `lastSlots` and drop throttle, and the screen when it is open.
#[derive(Debug, Default)]
pub struct Creative {
    pub tabs: Arc<Tabs>,
    pub built: Option<BuiltFor>,
    /// The search trees, made the first time a search needs them after the tabs, the
    /// language or the item tags changed (vanilla makes them on another thread then).
    pub trees: Option<Arc<Trees>>,
    /// `AbstractContainerMenu.lastSlots` of the player's inventory menu: what its slot
    /// listeners last heard.
    pub last_slots: Vec<Stack>,
    /// `LocalPlayer.dropSpamThrottler` (`TickThrottler(20, 1280)`).
    pub drop_throttle: i32,
    pub screen: Option<CreativeScreen>,
    /// The selected tab's key, once this state chose one (else the client's).
    selected_key: Option<String>,
}

impl Creative {
    /// `selectedTab = tab`: for this state and the client's run.
    pub fn set_selected(&mut self, key: &str) {
        self.selected_key = Some(key.to_owned());
        set_selected_key(key);
    }

    /// A new local player: a new inventory menu that has heard of nothing yet.
    pub fn new_player(&mut self) {
        self.last_slots = vec![Stack::empty(); INVENTORY_SLOTS];
        self.drop_throttle = 0;
    }

    /// `dropSpamThrottler.tick`, once a tick of the player's.
    pub fn tick(&mut self) {
        if self.drop_throttle > 0 {
            self.drop_throttle -= 1;
        }
    }

    /// `canDropItems`.
    pub fn can_drop_items(&self) -> bool {
        self.drop_throttle < 1280
    }

    /// The selected tab's place in the registry's order.
    pub fn selected(&self) -> usize {
        let key = self.selected_key.clone().unwrap_or_else(selected_key);
        self.tabs
            .tabs
            .iter()
            .position(|t| t.key == key)
            .unwrap_or_else(|| self.tabs.default_tab())
    }

    pub fn selected_tab(&self) -> Option<&Tab> {
        self.tabs.tabs.get(self.selected())
    }

    pub fn selected_type(&self) -> TabType {
        self.selected_tab().map_or(TabType::Category, |t| t.kind)
    }

    /// `CreativeModeTabs.tryRebuildTabContents`: whether they were built anew.
    pub fn try_rebuild(&mut self, p: &tabs::Parameters<'_>, configuration: u64) -> bool {
        let want = BuiltFor {
            configuration,
            features: p.features.to_vec(),
            permissions: p.has_permissions,
        };
        if self.built.as_ref() == Some(&want) {
            return false;
        }
        self.tabs = Arc::new(Tabs::build(p));
        self.built = Some(want);
        self.trees = None;
        true
    }

    /// The language or the item tags changed (`rebuildAfterLanguageChange`,
    /// `handleUpdateTags`): the trees are made again when next needed.
    pub fn forget_trees(&mut self) {
        self.trees = None;
    }
}

/// What a creative screen event runs with.
pub struct Ctx<'a> {
    pub creative: &'a mut Creative,
    pub menus: &'a mut Menus,
    pub inv: &'a mut Inventory,
    pub who: Who<'a>,
    /// `hasInfiniteMaterials`.
    pub infinite: bool,
    /// The server's enabled features (`isFeatureEnabled`).
    pub features: &'a [String],
    /// The synced registries' keys by network id (saved hotbars).
    pub registries: &'a HashMap<String, Vec<String>>,
    pub lang: &'a dyn Lang,
    pub item_tags: &'a ItemTags,
    pub sent: Sent,
}

impl std::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("infinite", &self.infinite)
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl Ctx<'_> {
    fn items(&self) -> &Items {
        self.who.items
    }

    fn feature_enabled(&self, item: i32) -> bool {
        let needs = usize::try_from(item)
            .ok()
            .and_then(|i| self.who.items.creative.item_features.get(i));
        needs.is_none_or(|n| n.iter().all(|f| self.features.iter().any(|e| e == f)))
    }

    /// `MultiPlayerGameMode.handleCreativeModeItemAdd`.
    pub fn item_add(&mut self, stack: &Stack, slot: i32) {
        if self.infinite && self.feature_enabled(stack.item) {
            if super::log_enabled() {
                eprintln!(
                    "[menu] sent creative slot {slot}: {}",
                    super::describe(stack)
                );
            }
            self.sent
                .packets
                .push(play::set_creative_mode_slot(slot as i16, &network(stack)));
        }
    }

    /// `handleCreativeModeItemDrop` (with the creative screen up, no other is), after
    /// `Player.drop`'s swing.
    fn item_drop(&mut self, stack: &Stack) {
        if !stack.is_empty() {
            self.sent.dropped += 1;
        }
        self.creative_drop(stack);
    }

    /// `handleCreativeModeItemDrop` alone.
    fn creative_drop(&mut self, stack: &Stack) {
        if stack.is_empty() {
            return;
        }
        if self.infinite && self.feature_enabled(stack.item) {
            if super::log_enabled() {
                eprintln!("[menu] sent creative drop: {}", super::describe(stack));
            }
            self.sent
                .packets
                .push(play::set_creative_mode_slot(-1, &network(stack)));
            self.creative.drop_throttle += 20;
        }
    }

    /// `AbstractContainerMenu.broadcastChanges` of the player's inventory menu: each
    /// slot unlike what its listeners last heard is noted, and sent with the creative
    /// screen's listener on (`CreativeInventoryListener`).
    pub fn broadcast(&mut self, listener: bool) {
        if self.creative.last_slots.len() != INVENTORY_SLOTS {
            self.creative.last_slots = vec![Stack::empty(); INVENTORY_SLOTS];
        }
        for i in 0..INVENTORY_SLOTS {
            let current = self.menus.inventory.item(self.inv, i).clone();
            if !self.creative.last_slots[i].matches(&current) {
                let copy = current.copy();
                self.creative.last_slots[i] = copy.clone();
                if listener {
                    self.item_add(&copy, i as i32);
                }
            }
        }
    }

    /// `CustomCreativeSlot.mayPickup`: an item of an enabled feature, not locked (the
    /// empty hotbars' paper); an empty slot.
    fn creative_may_pickup(&self, stack: &Stack) -> bool {
        stack.is_empty() || (self.feature_enabled(stack.item) && !locked(stack))
    }
}

fn network(stack: &Stack) -> ItemStack {
    if stack.is_empty() {
        return ItemStack::EMPTY;
    }
    ItemStack {
        item: stack.item,
        count: stack.count,
        patch: (*stack.patch).clone(),
    }
}

/// The stack can't be taken from the creative inventory (`CREATIVE_SLOT_LOCK`).
pub(crate) fn locked(stack: &Stack) -> bool {
    let lock = ruthenium_data::v26_3::registries::DataComponentType::CreativeSlotLock.id() as i32;
    stack.patch.added.iter().any(|(k, _)| *k == lock)
}

/// The screen's own state (`CreativeModeInventoryScreen`'s fields).
#[derive(Debug)]
pub struct CreativeScreen {
    /// `ItemPickerMenu`: `CONTAINER`'s five rows (its own stacks) over the hotbar.
    pub picker: Menu,
    /// `menu.items`: the tab's stacks, scrolled through.
    pub items: Vec<Stack>,
    pub scroll_offs: f32,
    pub scrolling: bool,
    pub search: EditBox,
    ignore_text_input: bool,
    /// `hasClickedOutside`, as the last press or release found it.
    has_clicked_outside: bool,
    /// The item tags a `#` search names (`visibleTags`).
    pub visible_tags: Vec<String>,
    /// The inventory tab's slots (`SlotWrapper`s, then the destroy slot): the inventory
    /// menu laid out as that tab lays it out, kept in step with it, while that tab is
    /// selected.
    pub layout: Option<Menu>,
    /// The Operator Items Tab option (`displayOperatorCreativeTab`).
    pub operator_tab: bool,
}

/// `ItemPickerMenu`'s slots: `CONTAINER`'s five rows, then the hotbar.
fn picker() -> Menu {
    let mut m = Menu::new(Kind::Creative, 0);
    m.slots.clear();
    for y in 0..5 {
        for x in 0..9 {
            m.slots.push(Slot {
                container: Container::Own,
                index: (y * 9 + x) as usize,
                x: 9 + x * 18,
                y: 18 + y * 18,
                rule: Rule::Creative,
            });
        }
    }
    // `addInventoryHotbarSlots(inventory, 9, 112)`.
    for x in 0..9 {
        m.slots.push(Slot {
            container: Container::Inventory,
            index: x as usize,
            x: 9 + x * 18,
            y: 112,
            rule: Rule::Plain,
        });
    }
    m.own = vec![Stack::empty(); CONTAINER_SLOTS];
    m.with_hashes()
}

/// Where `selectTab` puts the inventory menu's slot `i` in the inventory tab.
fn inventory_tab_place(i: usize) -> (i32, i32) {
    match i {
        5..9 => {
            let pos = (i - 5) as i32;
            (54 + pos / 2 * 54, 6 + pos % 2 * 27)
        }
        0..5 => (-2000, -2000),
        45 => (35, 20),
        _ => {
            let pos = (i - 9) as i32;
            let (col, row) = (pos % 9, pos / 9);
            (9 + col * 18, if i >= 36 { 112 } else { 54 + row * 18 })
        }
    }
}

/// The inventory tab's layout of the inventory menu, and the destroy slot.
fn inventory_layout(inv_menu: &Menu) -> Menu {
    let mut m = inv_menu.clone();
    for (i, s) in m.slots.iter_mut().enumerate() {
        (s.x, s.y) = inventory_tab_place(i);
    }
    // `new Slot(CONTAINER, 0, 173, 112)`: always empty; a place of its own past the
    // crafting result's.
    m.own.push(Stack::empty());
    m.slots.push(Slot {
        container: Container::Own,
        index: 1,
        x: 173,
        y: 112,
        rule: Rule::Creative,
    });
    m.with_hashes()
}

/// `getTabX`, `getTabY`: a tab's corner, from the window's.
pub fn tab_place(tab: &Tab) -> (i32, i32) {
    let x = if tab.aligned_right {
        IMAGE_WIDTH - 27 * (7 - tab.column) + 1
    } else {
        27 * tab.column
    };
    let y = if tab.row == Row::Top {
        -32
    } else {
        IMAGE_HEIGHT
    };
    (x, y)
}

/// `checkTabClicked`: within the tab's 26 by 32, edges in.
pub fn tab_clicked(tab: &Tab, xm: f64, ym: f64) -> bool {
    let (x, y) = tab_place(tab);
    let (x, y) = (f64::from(x), f64::from(y));
    xm >= x && xm <= x + 26.0 && ym >= y && ym <= y + 32.0
}

/// `Mth.clamp(f, 0, 1)`: NaN stays NaN.
fn clamp(v: f32) -> f32 {
    if v < 0.0 { 0.0 } else { v.min(1.0) }
}

/// The empty saved hotbar's paper (`selectTab`): locked, named "Press %s+%s to save
/// your current hotbar" with the keys bound to save and to that hotbar.
fn hotbar_placeholder(items: &Items, index: usize) -> Stack {
    let key = |name: String| {
        let mut c = Compound::new();
        c.put("keybind", Tag::String(name));
        Tag::Compound(c)
    };
    let name = crate::item_name::translation(
        "inventory.hotbarInfo",
        vec![
            key("key.saveToolbarActivator".into()),
            key(format!("key.hotbar.{}", index + 1)),
        ],
    );
    use ruthenium_data::v26_3::registries::DataComponentType as K;
    let mut patch = Vec::new();
    ruthenium_protocol::wire::write_varint(&mut patch, 1);
    ruthenium_protocol::wire::write_varint(
        &mut patch,
        ruthenium_data::v26_3::registries::Item::Paper as i32,
    );
    ruthenium_protocol::wire::write_varint(&mut patch, 2);
    ruthenium_protocol::wire::write_varint(&mut patch, 0);
    ruthenium_protocol::wire::write_varint(&mut patch, K::CreativeSlotLock.id() as i32);
    ruthenium_protocol::wire::write_varint(&mut patch, K::ItemName.id() as i32);
    if ruthenium_protocol::nbt::write_network(&mut patch, &name).is_err() {
        return Stack::empty();
    }
    let mut r = ruthenium_protocol::wire::Reader::new(&patch);
    ItemStack::read_optional(&mut r).map_or_else(|_| Stack::empty(), |s| items.stack(&s))
}

impl CreativeScreen {
    /// The constructor (`player.containerMenu = this.menu` is the session's): the
    /// picker, empty until `init`.
    pub fn new(operator_tab: bool) -> CreativeScreen {
        let mut search = EditBox::new(50);
        search.visible = false;
        CreativeScreen {
            picker: picker(),
            items: Vec::new(),
            scroll_offs: 0.0,
            scrolling: false,
            search,
            ignore_text_input: false,
            has_clicked_outside: false,
            visible_tags: Vec::new(),
            layout: None,
            operator_tab,
        }
    }

    /// The screen's menu (`this.menu`, with its slots as the selected tab has them).
    pub fn menu(&self) -> &Menu {
        self.layout.as_ref().unwrap_or(&self.picker)
    }

    /// `calculateRowCount`: the rows past the five shown.
    fn row_count(&self) -> i32 {
        let n = self.items.len() as i32;
        -(-n).div_euclid(9) - 5
    }

    /// `getRowIndexForScroll`.
    fn row_for_scroll(&self, scroll: f32) -> i32 {
        ((f64::from(scroll * self.row_count() as f32) + 0.5) as i32).max(0)
    }

    /// `getScrollForRowIndex`.
    fn scroll_for_row(&self, row: i32) -> f32 {
        clamp(row as f32 / self.row_count() as f32)
    }

    /// `subtractInputFromScroll`.
    fn subtract_from_scroll(&self, scroll: f32, input: f64) -> f32 {
        clamp(scroll - (input / f64::from(self.row_count())) as f32)
    }

    /// The menu's `canScroll`: more than five rows.
    fn menu_can_scroll(&self) -> bool {
        self.items.len() > CONTAINER_SLOTS
    }

    /// `scrollTo`: the rows from the scroll's into `CONTAINER`.
    pub fn scroll_to(&mut self, scroll: f32) {
        let row = self.row_for_scroll(scroll);
        for y in 0..5 {
            for x in 0..9 {
                let at = x + (y + row) * 9;
                let stack = usize::try_from(at)
                    .ok()
                    .and_then(|i| self.items.get(i))
                    .cloned()
                    .unwrap_or_default();
                self.picker.own[(x + y * 9) as usize] = stack;
            }
        }
    }

    /// The menus as the screen holds them, kept in step: the picker's cursor is the
    /// inventory menu's (`ItemPickerMenu.getCarried`), and the inventory tab's slots
    /// show the inventory menu's.
    pub fn sync(&mut self, inv_menu: &Menu) {
        self.picker.carried = inv_menu.carried.clone();
        if let Some(l) = &mut self.layout {
            l.carried = inv_menu.carried.clone();
            l.own[0] = inv_menu.own[0].clone();
            l.grid = inv_menu.grid.clone();
            l.quickcraft_status = inv_menu.quickcraft_status;
            l.quickcraft_type = inv_menu.quickcraft_type;
            l.quickcraft_slots = inv_menu.quickcraft_slots.clone();
        }
    }

    /// Whether a slot of the screen's menu is `CONTAINER`'s (`isCreativeSlot`): the
    /// rows, or the inventory tab's destroy slot.
    fn is_creative_slot(&self, i: usize) -> bool {
        self.menu()
            .slots
            .get(i)
            .is_some_and(|s| s.rule == Rule::Creative)
    }

    fn is_destroy_slot(&self, i: usize) -> bool {
        self.layout.is_some() && i == DESTROY_SLOT
    }
}

/// `selectTab`.
pub fn select_tab(s: &mut CreativeScreen, screen: &mut Screen, c: &mut Ctx<'_>, tab: usize) {
    let tabs = c.creative.tabs.clone();
    let Some(t) = tabs.tabs.get(tab) else {
        return;
    };
    let old = c.creative.selected_type();
    let same = c.creative.selected() == tab;
    c.creative.set_selected(&t.key);
    screen.clear_quick_craft();
    s.items.clear();
    match t.kind {
        TabType::Hotbar => {
            for i in 0..hotbars::SIZE {
                let loaded = hotbars::with(|m| {
                    let h = m.get(i);
                    (!h.is_empty()).then(|| h.load(c.registries))
                });
                match loaded {
                    None => {
                        for j in 0..9 {
                            s.items.push(if j == i {
                                hotbar_placeholder(c.items(), i)
                            } else {
                                Stack::empty()
                            });
                        }
                    }
                    Some(stacks) => {
                        let items = c.items();
                        s.items.extend(stacks.iter().map(|st| items.stack(st)));
                    }
                }
            }
        }
        TabType::Category => s.items.extend(t.display.iter().map(|i| i.stack.clone())),
        _ => {}
    }
    if t.kind == TabType::Inventory {
        s.layout = Some(inventory_layout(&c.menus.inventory));
    } else if old == TabType::Inventory {
        s.layout = None;
    }
    if t.kind == TabType::Search {
        s.search.visible = true;
        s.search.set_can_lose_focus(false);
        s.search.set_focused(true);
        if !same {
            s.search.set_value("");
        }
        refresh_search_results(s, c);
    } else {
        s.search.visible = false;
        s.search.set_can_lose_focus(true);
        s.search.set_focused(false);
        s.search.set_value("");
    }
    s.scroll_offs = 0.0;
    s.scroll_to(0.0);
    s.sync(&c.menus.inventory);
}

/// `refreshSearchResults`: the search tab's stacks, or the tree's for the text
/// (`#`: the tag tree, else the name tree), from the top.
pub fn refresh_search_results(s: &mut CreativeScreen, c: &mut Ctx<'_>) {
    s.items.clear();
    s.visible_tags.clear();
    let term = s.search.value();
    let tabs = c.creative.tabs.clone();
    if term.is_empty() {
        if let Some(t) = tabs.tabs.get(c.creative.selected()) {
            s.items.extend(t.display.iter().map(|i| i.stack.clone()));
        }
    } else {
        let trees = match &c.creative.trees {
            Some(t) => t.clone(),
            None => {
                let search = tabs.search_tab().and_then(|i| tabs.tabs.get(i));
                let items = search.map_or(&[][..], |t| t.display.as_slice());
                let t = Arc::new(Trees::build(items, c.lang, c.item_tags));
                c.creative.trees = Some(t.clone());
                t
            }
        };
        let found = if let Some(tag) = term.strip_prefix('#') {
            update_visible_tags(s, c.item_tags, tag);
            trees.tags.search(&search::lower(tag))
        } else {
            trees.names.search(&search::lower(&term))
        };
        s.items.extend(
            found
                .into_iter()
                .filter_map(|i| trees.items.get(i).cloned()),
        );
    }
    s.scroll_offs = 0.0;
    s.scroll_to(0.0);
}

/// `updateVisibleTags`: the item tags whose names hold the text (by namespace and path
/// around a colon).
fn update_visible_tags(s: &mut CreativeScreen, item_tags: &ItemTags, term: &str) {
    let matcher = |id: &str| {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        match term.split_once(':') {
            None => path.contains(term),
            Some((n, p)) => ns.contains(n.trim()) && path.contains(p.trim()),
        }
    };
    s.visible_tags = item_tags
        .names()
        .filter(|t| matcher(t))
        .map(str::to_owned)
        .collect();
}

/// `refreshCurrentTabContents`: the rebuilt tab's stacks, on the same row.
fn refresh_current_tab(s: &mut CreativeScreen, c: &mut Ctx<'_>) {
    let old_row = s.row_for_scroll(s.scroll_offs);
    s.items.clear();
    let tabs = c.creative.tabs.clone();
    let selected = c.creative.selected();
    if c.creative.selected_type() == TabType::Search {
        refresh_search_results(s, c);
    } else if let Some(t) = tabs.tabs.get(selected) {
        s.items.extend(t.display.iter().map(|i| i.stack.clone()));
    }
    s.scroll_offs = s.scroll_for_row(old_row);
    s.scroll_to(s.scroll_offs);
}

/// `init` (and the constructor's `tryRebuildTabContents`): the selected tab chosen
/// again (anew from the default, as `init` does), its listener on the inventory menu
/// (which hears every slot unlike what it last heard).
pub fn init(s: &mut CreativeScreen, screen: &mut Screen, c: &mut Ctx<'_>) {
    let tab = c.creative.selected();
    let default = c.creative.tabs.default_tab();
    if let Some(t) = c.creative.tabs.tabs.get(default) {
        let key = t.key.clone();
        c.creative.set_selected(&key);
    }
    select_tab(s, screen, c, tab);
    c.broadcast(true);
    if !c.creative.selected_tab().is_some_and(Tab::should_display) {
        select_tab(s, screen, c, default);
    }
}

/// `tryRefreshInvalidatedTabs`: tabs rebuilt for new parameters show again; a category
/// left empty gives way to the default tab.
pub fn refresh_invalidated(
    s: &mut CreativeScreen,
    screen: &mut Screen,
    c: &mut Ctx<'_>,
    rebuilt: bool,
) {
    if !rebuilt {
        return;
    }
    let tab = c.creative.selected_tab().cloned();
    match tab {
        Some(t) if t.kind == TabType::Category && t.display.is_empty() => {
            let d = c.creative.tabs.default_tab();
            select_tab(s, screen, c, d);
        }
        Some(_) => refresh_current_tab(s, c),
        None => {}
    }
}

/// `slotClicked`: `slot` is the screen's menu's (none for a click outside or a drag's
/// start and end).
pub fn slot_clicked(
    s: &mut CreativeScreen,
    c: &mut Ctx<'_>,
    slot: Option<usize>,
    slot_id: i32,
    button: i32,
    input: Input,
) {
    if slot.is_some_and(|i| s.is_creative_slot(i)) {
        s.search.move_cursor_to_end(false);
        s.search.set_highlight_pos(0);
    }
    let quick_key = input == Input::QuickMove;
    let input = if slot_id == SLOT_OUTSIDE && input == Input::Pickup {
        Input::Throw
    } else {
        input
    };
    if input == Input::Throw && !c.creative.can_drop_items() {
        return;
    }
    let tab = c.creative.selected_type();
    let carried = |c: &Ctx<'_>| c.menus.inventory.carried.clone();
    if slot.is_none() && tab != TabType::Inventory && input != Input::QuickCraft {
        if !carried(c).is_empty() && s.has_clicked_outside {
            if !c.creative.can_drop_items() {
                return;
            }
            if button == 0 {
                let all = std::mem::take(&mut c.menus.inventory.carried);
                c.item_drop(&all);
            }
            if button == 1 {
                let one = c.menus.inventory.carried.split(1);
                c.item_drop(&one);
            }
        }
        s.sync(&c.menus.inventory);
        return;
    }
    if let Some(i) = slot {
        let may = if s.is_creative_slot(i) {
            c.creative_may_pickup(s.menu().item(c.inv, i))
        } else {
            s.menu().may_pickup(c.inv, i, &c.who)
        };
        if !may {
            return;
        }
    }
    let destroy = slot.is_some_and(|i| s.is_destroy_slot(i));
    if destroy && quick_key {
        // Every slot of the inventory menu emptied and sent, without its listener.
        for i in 0..INVENTORY_SLOTS {
            c.menus.inventory.set(c.inv, i, Stack::empty());
            c.item_add(&Stack::empty(), i as i32);
        }
    } else if tab == TabType::Inventory {
        if destroy {
            c.menus.inventory.carried = Stack::empty();
        } else if let (Input::Throw, Some(i)) = (input, slot)
            && !c.menus.inventory.item(c.inv, i).is_empty()
        {
            let max = c.menus.inventory.item(c.inv, i).max_stack_size();
            let to_drop = c
                .menus
                .inventory
                .remove(c.inv, i, if button == 0 { 1 } else { max });
            let after = c.menus.inventory.item(c.inv, i).clone();
            c.item_drop(&to_drop);
            c.item_add(&after, i as i32);
        } else if input == Input::Throw && slot_id == SLOT_OUTSIDE && !carried(c).is_empty() {
            let all = std::mem::take(&mut c.menus.inventory.carried);
            c.item_drop(&all);
        } else {
            let at = slot.map_or(slot_id, |i| i as i32);
            let out = c.menus.inventory.clicked(c.inv, &c.who, at, button, input);
            menu_drops(c, input, &out);
            c.broadcast(true);
        }
    } else if let Some(i) = slot.filter(|&i| input != Input::QuickCraft && s.is_creative_slot(i)) {
        let clicked = s.menu().item(c.inv, i).clone();
        match input {
            Input::Swap => {
                if !clicked.is_empty() {
                    let b = usize::try_from(button).unwrap_or(0);
                    c.inv
                        .set(b, clicked.copy_with_count(clicked.max_stack_size()));
                    c.broadcast(true);
                }
                s.sync(&c.menus.inventory);
                return;
            }
            Input::Clone => {
                if c.menus.inventory.carried.is_empty() && !clicked.is_empty() {
                    c.menus.inventory.carried = clicked.copy_with_count(clicked.max_stack_size());
                }
                s.sync(&c.menus.inventory);
                return;
            }
            Input::Throw => {
                if !clicked.is_empty() {
                    let n = if button == 0 {
                        1
                    } else {
                        clicked.max_stack_size()
                    };
                    let to_drop = clicked.copy_with_count(n);
                    c.item_drop(&to_drop);
                }
                return;
            }
            _ => {}
        }
        let carried = &mut c.menus.inventory.carried;
        if !carried.is_empty() && !clicked.is_empty() && carried.same_item_same_components(&clicked)
        {
            if button == 0 {
                if quick_key {
                    carried.count = carried.max_stack_size();
                } else if carried.count < carried.max_stack_size() {
                    carried.grow(1);
                }
            } else {
                carried.shrink(1);
            }
        } else if !clicked.is_empty() && carried.is_empty() {
            let n = if quick_key {
                clicked.max_stack_size()
            } else {
                clicked.count
            };
            *carried = clicked.copy_with_count(n);
        } else if button == 0 {
            *carried = Stack::empty();
        } else if !carried.is_empty() {
            carried.shrink(1);
        }
    } else {
        // The picker's own `clicked` (its cursor the inventory menu's), on the hotbar
        // or a drag.
        let old = slot.map(|i| s.picker.item(c.inv, i).clone());
        s.picker.carried = c.menus.inventory.carried.clone();
        let at = slot.map_or(slot_id, |i| i as i32);
        let out = s.picker.clicked(c.inv, &c.who, at, button, input);
        c.menus.inventory.carried = s.picker.carried.clone();
        menu_drops(c, input, &out);
        if super::quickcraft_header(button) == 2 {
            for k in 0..9 {
                let stack = s.picker.item(c.inv, CONTAINER_SLOTS + k).clone();
                c.item_add(&stack, 36 + k as i32);
            }
        } else if let Some(i) = slot
            && s.picker.slots[i].index < 9
            && tab != TabType::Inventory
        {
            if let Some(old) = old
                && input == Input::Throw
                && !old.is_empty()
                && !c.menus.inventory.carried.is_empty()
            {
                // `oldItemStack.shrink`: the slot's own stack, where it is.
                let n = if button == 0 { 1 } else { old.count };
                let to_drop = old.copy_with_count(n);
                let mut here = s.picker.item(c.inv, i).clone();
                if here.same_object(&old) {
                    here.shrink(n);
                    s.picker.set(c.inv, i, here);
                }
                c.item_drop(&to_drop);
            }
            c.broadcast(true);
        }
    }
    s.sync(&c.menus.inventory);
}

/// What a menu's `clicked` dropped: each swings the hand, and what a throw took
/// (`doClick`'s THROW) goes to `handleCreativeModeItemDrop` too.
fn menu_drops(c: &mut Ctx<'_>, input: Input, out: &super::Outcome) {
    c.sent.dropped += out.dropped.len();
    if input == Input::Throw {
        for s in &out.dropped {
            c.creative_drop(s);
        }
    }
}

/// What the generic screen clicks through.
struct Clicks<'s, 'c, 'a> {
    s: &'s mut CreativeScreen,
    c: &'c mut Ctx<'a>,
}

impl Clicker for Clicks<'_, '_, '_> {
    fn menu(&self) -> &Menu {
        self.s.menu()
    }

    fn inventory(&self) -> &Inventory {
        self.c.inv
    }

    fn who(&self) -> Who<'_> {
        self.c.who
    }

    fn click(&mut self, slot: i32, button: i32, input: Input) {
        let at = usize::try_from(slot).ok();
        slot_clicked(self.s, self.c, at, slot, button, input);
    }

    fn outside_except(&self) -> Option<(i32, i32)> {
        let tabs = &self.c.creative.tabs;
        tabs.tabs.get(self.c.creative.selected()).map(tab_place)
    }

    fn clicked_outside(&mut self, outside: bool) {
        self.s.has_clicked_outside = outside;
    }
}

/// `canScroll`: the tab scrolls and has more than five rows.
fn can_scroll(s: &CreativeScreen, c: &Ctx<'_>) -> bool {
    c.creative.selected_tab().is_some_and(|t| t.can_scroll) && s.menu_can_scroll()
}

/// `insideScrollbar` (the pointer in GUI units).
fn inside_scrollbar(screen: &Screen, c: &Ctx<'_>, x: f64, y: f64) -> bool {
    let (xs, ys) = (f64::from(screen.left + 175), f64::from(screen.top + 18));
    x >= xs
        && y >= ys
        && x < xs + 14.0
        && y < ys + 112.0
        && c.creative.selected_tab().is_some_and(|t| t.can_scroll)
}

/// Whether a press lands in the search box (`EditBox.mouseClicked`: shown, and the
/// pointer over its 80 by 9).
fn over_search_box(s: &CreativeScreen, screen: &Screen, x: f64, y: f64) -> bool {
    let (bx, by) = (f64::from(screen.left + 82), f64::from(screen.top + 6));
    s.search.visible && x >= bx && y >= by && x < bx + 80.0 && y < by + 9.0
}

/// `mouseClicked`.
pub fn mouse_clicked(
    s: &mut CreativeScreen,
    screen: &mut Screen,
    c: &mut Ctx<'_>,
    m: Mouse,
    double: bool,
) {
    if m.button == 1 {
        let (xm, ym) = (m.x - f64::from(screen.left), m.y - f64::from(screen.top));
        let tabs = c.creative.tabs.clone();
        if tabs.shown().any(|(_, t)| tab_clicked(t, xm, ym)) {
            return;
        }
        if c.creative.selected_type() != TabType::Inventory && inside_scrollbar(screen, c, m.x, m.y)
        {
            s.scrolling = can_scroll(s, c);
            return;
        }
    }
    // The search box, a widget, takes a press over it first (placing the caret by the
    // pointer is left out).
    if over_search_box(s, screen, m.x, m.y) {
        return;
    }
    screen.mouse_clicked(&mut Clicks { s, c }, m, double);
}

/// `mouseReleased`: a release over a tab selects it.
pub fn mouse_released(s: &mut CreativeScreen, screen: &mut Screen, c: &mut Ctx<'_>, m: Mouse) {
    if m.button == 1 {
        let (xm, ym) = (m.x - f64::from(screen.left), m.y - f64::from(screen.top));
        s.scrolling = false;
        let tabs = c.creative.tabs.clone();
        if let Some((i, _)) = tabs.shown().find(|(_, t)| tab_clicked(t, xm, ym)) {
            select_tab(s, screen, c, i);
            return;
        }
    }
    screen.mouse_released(&mut Clicks { s, c }, m);
}

/// `mouseDragged`: the scroller follows the pointer while held.
pub fn mouse_dragged(s: &mut CreativeScreen, screen: &mut Screen, c: &mut Ctx<'_>, x: f64, y: f64) {
    if s.scrolling {
        let ys = (screen.top + 18) as f32;
        let ye = ys + 112.0;
        s.scroll_offs = clamp((y as f32 - ys - 7.5) / (ye - ys - 15.0));
        s.scroll_to(s.scroll_offs);
        return;
    }
    screen.mouse_dragged(&mut Clicks { s, c }, x, y);
}

/// `mouseScrolled`.
pub fn mouse_scrolled(s: &mut CreativeScreen, c: &mut Ctx<'_>, scroll_y: f64) {
    if !can_scroll(s, c) {
        return;
    }
    s.scroll_offs = s.subtract_from_scroll(s.scroll_offs, scroll_y);
    s.scroll_to(s.scroll_offs);
}

/// The frame: the slot under the pointer, a drag's slots kept to those that take it.
pub fn frame(s: &mut CreativeScreen, screen: &mut Screen, c: &mut Ctx<'_>, x: f64, y: f64) {
    s.sync(&c.menus.inventory);
    screen.frame(&Clicks { s, c }, x, y);
}

/// A key over the creative screen: its bindings, the key, the modifiers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreativeKey {
    pub binds: Binds,
    /// The chat key is on it (`keyChat`).
    pub chat: bool,
    pub key: KeyPress,
    pub ctrl: bool,
}

/// `getNumericKeyValue`: the number row and the keypad's digits.
fn is_digit(scancode: u32) -> bool {
    matches!(scancode, 30..=39 | 89..=98)
}

/// `keyPressed`: whether the screen closes.
pub fn key_pressed(
    s: &mut CreativeScreen,
    screen: &mut Screen,
    c: &mut Ctx<'_>,
    k: &CreativeKey,
    x: f64,
    y: f64,
) -> bool {
    s.ignore_text_input = false;
    if c.creative.selected_type() != TabType::Search {
        if k.chat {
            s.ignore_text_input = true;
            if let Some(search) = c.creative.tabs.search_tab() {
                select_tab(s, screen, c, search);
            }
            return false;
        }
        return screen.key_pressed(&mut Clicks { s, c }, k.binds, k.ctrl, x, y);
    }
    // (The slot the last frame drew as hovered.)
    let hovered = screen.slot_at(s.menu(), x.floor(), y.floor());
    let quick_swap = !hovered.is_some_and(|h| s.is_creative_slot(h))
        || hovered.is_some_and(|h| !s.menu().item(c.inv, h).is_empty());
    if quick_swap && is_digit(k.key.scancode) {
        // `checkHotbarKeyPressed`.
        let empty_cursor = c.menus.inventory.carried.is_empty();
        if let Some(h) = hovered.filter(|_| empty_cursor) {
            let swap = if k.binds.swap_offhand {
                Some(40)
            } else {
                k.binds.hotbar.iter().position(|b| *b).map(|i| i as i32)
            };
            if let Some(b) = swap {
                slot_clicked(s, c, Some(h), h as i32, b, Input::Swap);
                s.ignore_text_input = true;
                return false;
            }
        }
    }
    let old = s.search.value();
    let (taken, copied) = s.search.key_pressed(&k.key);
    if copied.is_some() {
        c.sent.clipboard = copied;
    }
    if taken {
        if s.search.value() != old {
            refresh_search_results(s, c);
        }
        return false;
    }
    if s.search.captures_input() && !k.binds.escape {
        return false;
    }
    screen.key_pressed(&mut Clicks { s, c }, k.binds, k.ctrl, x, y)
}

/// `keyReleased`.
pub fn key_released(s: &mut CreativeScreen) {
    s.ignore_text_input = false;
}

/// `charTyped`, a code point at a time.
pub fn char_typed(s: &mut CreativeScreen, c: &mut Ctx<'_>, text: &str) {
    for ch in text.chars() {
        if s.ignore_text_input || c.creative.selected_type() != TabType::Search {
            return;
        }
        let old = s.search.value();
        if s.search.char_typed(ch) && s.search.value() != old {
            refresh_search_results(s, c);
        }
    }
}

/// `handleHotbarLoadOrSave`: X and a number loads that saved hotbar into the hotbar
/// (each slot sent), C and a number saves the hotbar; the message the save shows.
pub fn hotbar_load_or_save(c: &mut Ctx<'_>, index: usize, load: bool, save: bool) -> Option<Tag> {
    if load {
        let stacks = hotbars::with(|m| m.get(index).load(c.registries));
        for (i, st) in stacks.iter().enumerate().take(9) {
            let stack = c.items().stack(st);
            c.inv.set(i, stack.clone());
            c.item_add(&stack, 36 + i as i32);
        }
        c.broadcast(false);
        None
    } else if save {
        let stacks: Vec<ItemStack> = (0..9).map(|i| network(c.inv.get(i))).collect();
        hotbars::with(|m| {
            m.get(index).store_from(&stacks, c.registries);
            m.save();
        });
        let key = |name: String| {
            let mut t = Compound::new();
            t.put("keybind", Tag::String(name));
            Tag::Compound(t)
        };
        Some(crate::item_name::translation(
            "inventory.hotbarSaved",
            vec![
                key("key.loadToolbarActivator".into()),
                key(format!("key.hotbar.{}", index + 1)),
            ],
        ))
    } else {
        None
    }
}

/// What the screen draws (see [`crate::session::ContainerView`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CreativeView {
    /// The tabs shown, in order.
    pub tabs: Vec<TabView>,
    /// The selected tab's place among them.
    pub selected: Option<usize>,
    /// The window's picture (`textures/gui/container/creative_inventory/tab_*.png`).
    pub background: String,
    /// The selected tab's title, if it shows it.
    pub title: Option<Arc<Tag>>,
    /// The scroller's place (0 to 1) and whether it can move, if the tab scrolls.
    pub scroller: Option<(f32, bool)>,
    /// The search box, when shown: its text, caret and selection's end (UTF-16 units),
    /// whether it has the focus, and since when.
    pub search: Option<SearchBoxView>,
    /// The inventory tab is selected (its destroy slot is the menu's 47th).
    pub inventory_tab: bool,
}

/// A tab as drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct TabView {
    pub x: i32,
    pub y: i32,
    pub top: bool,
    pub column: i32,
    pub icon: Stack,
    pub title: Arc<Tag>,
}

/// The search box as drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchBoxView {
    pub text: String,
    pub cursor: usize,
    pub highlight: usize,
    pub focused: bool,
    pub focused_at: std::time::Instant,
}

/// The screen as drawn now.
pub fn view(s: &CreativeScreen, creative: &Creative) -> CreativeView {
    let selected = creative.selected();
    let tabs: Vec<(usize, &Tab)> = creative.tabs.shown().collect();
    let tab = creative.selected_tab();
    CreativeView {
        tabs: tabs
            .iter()
            .map(|(_, t)| {
                let (x, y) = tab_place(t);
                TabView {
                    x,
                    y,
                    top: t.row == Row::Top,
                    column: t.column,
                    icon: t.icon.clone(),
                    title: t.title.clone(),
                }
            })
            .collect(),
        selected: tabs.iter().position(|(i, _)| *i == selected),
        background: tab.map_or_else(String::new, |t| t.background.clone()),
        title: tab.filter(|t| t.show_title).map(|t| t.title.clone()),
        scroller: tab
            .filter(|t| t.can_scroll)
            .map(|t| (s.scroll_offs, t.can_scroll && s.menu_can_scroll())),
        search: s.search.visible.then(|| SearchBoxView {
            text: s.search.value(),
            cursor: s.search.cursor(),
            highlight: s.search.highlight(),
            focused: s.search.is_focused(),
            focused_at: s.search.focused_at,
        }),
        inventory_tab: s.layout.is_some(),
    }
}

#[cfg(test)]
mod search_golden;
#[cfg(test)]
mod tabs_golden;
#[cfg(test)]
mod tests;
