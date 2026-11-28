//! The workstations' menus as the client runs them: the anvil, the smithing table, the
//! stonecutter, the loom, the cartography table, the crafter and the merchant. Each
//! one's slots (in vanilla's order and numbering), rules, `quickMoveStack`, data slots
//! and buttons live in its own module; this one holds what they share: the kinds and
//! slot rules [`Menu`] dispatches on, what the stacks say of themselves
//! ([`ItemFacts`]), the session's registries and tags ([`WorkData`]), and the state a
//! menu and its screen keep ([`State`]).
//!
//! Vanilla's client runs much of these menus itself: an anvil works out its result and
//! cost, a merchant its sale, a loom its banner, while the others clear their results;
//! the server's slot packets then put the true ones in. Each container's `setItem` and
//! `setChanged` do here what they do there ([`container_set`], [`set_changed`]), in
//! the same order, so what a click changes (and so what it tells the server) is what
//! vanilla's client would have changed.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_data::v26_3::registries::DataComponentType as ComponentKind;
use ruthenium_protocol::item::{Component, ComponentPatch, HolderSet, ItemStack};

use super::{Container, Inventory, Menu, Outcome, Rule, Slot, Who};
use crate::item::{Items, Stack};

pub mod anvil;
pub mod cartography;
pub mod crafter;
pub mod edit_box;
pub mod loom;
pub mod merchant;
pub mod screen;
pub mod smithing;
pub mod stonecutter;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;

/// The workstations' menu types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Station {
    /// `AnvilMenu`.
    Anvil,
    /// `SmithingMenu`.
    Smithing,
    /// `StonecutterMenu`.
    Stonecutter,
    /// `LoomMenu`.
    Loom,
    /// `CartographyTableMenu`.
    Cartography,
    /// `CrafterMenu`.
    Crafter,
    /// `MerchantMenu` (villagers and wandering traders).
    Merchant,
}

impl Station {
    /// The station of a `minecraft:menu` name, if it is one.
    pub fn from_menu_name(name: &str) -> Option<Station> {
        Some(match name.trim_start_matches("minecraft:") {
            "anvil" => Station::Anvil,
            "smithing" => Station::Smithing,
            "stonecutter" => Station::Stonecutter,
            "loom" => Station::Loom,
            "cartography_table" => Station::Cartography,
            "crafter_3x3" => Station::Crafter,
            "merchant" => Station::Merchant,
            _ => return None,
        })
    }

    /// `imageWidth`, `imageHeight`.
    pub fn image_size(self) -> (i32, i32) {
        match self {
            Station::Merchant => (276, 166),
            _ => (176, 166),
        }
    }
}

/// What the workstations' own slots allow (the `Slot` subclasses their menus add).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotRule {
    /// `ItemCombinerMenu`'s result slot (an anvil's, a smithing table's).
    CombinerResult,
    /// A smithing table's inputs (`RecipePropertySet` tests).
    Template,
    Base,
    Addition,
    /// The stonecutter's result.
    CutterResult,
    /// The loom's banner, dye and pattern slots, and its result.
    Banner,
    Dye,
    Pattern,
    LoomResult,
    /// The cartography table's map, its paper, map or glass pane, and its result.
    Map,
    MapAddition,
    MapResult,
    /// `CrafterSlot`: off when its data slot says so.
    CrafterGrid,
    /// `NonInteractiveResultSlot`: the crafter's result.
    CrafterResult,
    /// `MerchantResultSlot`.
    TradeResult,
}

/// A set of registry entries as an item's defaults or its patch give it: a tag, ids, or
/// names (a dynamic registry's, before its ids are known).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holders {
    /// A tag's full identifier (`minecraft:repairs_iron_armor`).
    Tag(String),
    Ids(Vec<i32>),
    Names(Vec<String>),
}

fn full_id(s: &str) -> String {
    if s.contains(':') {
        s.to_owned()
    } else {
        format!("minecraft:{s}")
    }
}

impl Holders {
    /// A holder set's JSON form (`HolderSetCodec`): `"#tag"`, an id, or a list of ids.
    pub fn from_json(v: &serde_json::Value) -> Option<Holders> {
        match v {
            serde_json::Value::String(s) => Some(match s.strip_prefix('#') {
                Some(tag) => Holders::Tag(full_id(tag)),
                None => Holders::Names(vec![full_id(s)]),
            }),
            serde_json::Value::Array(a) => Some(Holders::Names(
                a.iter()
                    .filter_map(|n| Some(full_id(n.as_str()?)))
                    .collect(),
            )),
            _ => None,
        }
    }

    pub fn from_network(set: &HolderSet) -> Holders {
        match set {
            HolderSet::Tag(t) => Holders::Tag(full_id(t)),
            HolderSet::Ids(ids) => Holders::Ids(ids.clone()),
        }
    }

    /// The entries in order (`ImmutableList.copyOf` of the set): a tag's as the server
    /// listed them, names by `names` (the registry's ids in order).
    pub fn entries(&self, tags: Option<&HashMap<String, Vec<i32>>>, names: &[String]) -> Vec<i32> {
        match self {
            Holders::Tag(t) => tags
                .and_then(|tags| tags.get(t))
                .cloned()
                .unwrap_or_default(),
            Holders::Ids(ids) => ids.clone(),
            Holders::Names(n) => n
                .iter()
                .filter_map(|n| names.iter().position(|x| x == n))
                .map(|i| i as i32)
                .collect(),
        }
    }

    /// `HolderSet.contains` of the entry `id` (named `name`).
    pub fn contains(&self, id: i32, name: &str, tags: Option<&HashMap<String, Vec<i32>>>) -> bool {
        match self {
            Holders::Tag(t) => tags
                .and_then(|tags| tags.get(t))
                .is_some_and(|ids| ids.contains(&id)),
            Holders::Ids(ids) => ids.contains(&id),
            Holders::Names(n) => n.iter().any(|n| n == name),
        }
    }
}

/// What the workstations read of a stack's components, with the item's defaults under
/// its patch.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemFacts {
    /// `repair_cost` (0 without it).
    pub repair_cost: i32,
    /// `repairable`: the items that repair it.
    pub repairable: Option<Holders>,
    /// It has the `enchantments` component (every item has it by default).
    pub has_enchantments: bool,
    /// `stored_enchantments`, if it has the component (an enchanted book).
    pub stored_enchantments: Option<Vec<(i32, i32)>>,
    /// `dye`: a `DyeColor` id.
    pub dye: Option<u8>,
    /// `provides_banner_patterns`.
    pub provides_banner_patterns: Option<Holders>,
    /// It has a `map_id`.
    pub map_id: bool,
}

impl ItemFacts {
    /// An item's defaults (its report's `components`).
    pub fn from_defaults(v: &serde_json::Value) -> ItemFacts {
        ItemFacts {
            repair_cost: v
                .get("minecraft:repair_cost")
                .and_then(serde_json::Value::as_i64)
                .map_or(0, |n| n as i32),
            repairable: v
                .pointer("/minecraft:repairable/items")
                .and_then(Holders::from_json)
                .map(item_holders),
            has_enchantments: v.get("minecraft:enchantments").is_some(),
            stored_enchantments: v.get("minecraft:stored_enchantments").map(|_| Vec::new()),
            dye: v
                .get("minecraft:dye")
                .and_then(|d| d.as_str())
                .and_then(|d| crate::item::DYE_NAMES.iter().position(|n| *n == d))
                .map(|i| i as u8),
            provides_banner_patterns: v
                .get("minecraft:provides_banner_patterns")
                .and_then(Holders::from_json),
            map_id: v.get("minecraft:map_id").is_some(),
        }
    }

    /// Whether a patch touches anything kept here.
    pub fn touched_by(patch: &ComponentPatch) -> bool {
        const KINDS: [ComponentKind; 7] = [
            ComponentKind::RepairCost,
            ComponentKind::Repairable,
            ComponentKind::Enchantments,
            ComponentKind::StoredEnchantments,
            ComponentKind::Dye,
            ComponentKind::ProvidesBannerPatterns,
            ComponentKind::MapId,
        ];
        let ids = KINDS.map(|k| k.id() as i32);
        patch.added.iter().any(|(k, _)| ids.contains(k))
            || patch.removed.iter().any(|k| ids.contains(k))
    }

    /// The patch over the defaults.
    pub fn apply(&mut self, patch: &ComponentPatch) {
        for &kind in &patch.removed {
            match u32::try_from(kind).ok().and_then(ComponentKind::from_id) {
                Some(ComponentKind::RepairCost) => self.repair_cost = 0,
                Some(ComponentKind::Repairable) => self.repairable = None,
                Some(ComponentKind::Enchantments) => self.has_enchantments = false,
                Some(ComponentKind::StoredEnchantments) => self.stored_enchantments = None,
                Some(ComponentKind::Dye) => self.dye = None,
                Some(ComponentKind::ProvidesBannerPatterns) => self.provides_banner_patterns = None,
                Some(ComponentKind::MapId) => self.map_id = false,
                _ => {}
            }
        }
        for (kind, c) in &patch.added {
            let data = match c {
                Component::Other { data, .. } => Some(&data[..]),
                _ => None,
            };
            let mut r = data.map(ruthenium_protocol::wire::Reader::new);
            match u32::try_from(*kind).ok().and_then(ComponentKind::from_id) {
                Some(ComponentKind::RepairCost) => {
                    if let Some(n) = r.as_mut().and_then(|r| r.varint().ok()) {
                        self.repair_cost = n;
                    }
                }
                Some(ComponentKind::Repairable) => {
                    self.repairable = r
                        .as_mut()
                        .and_then(|r| HolderSet::read(r).ok())
                        .map(|s| Holders::from_network(&s));
                }
                Some(ComponentKind::Enchantments) => self.has_enchantments = true,
                Some(ComponentKind::StoredEnchantments) => {
                    if let Component::StoredEnchantments(e) = c {
                        self.stored_enchantments = Some(e.clone());
                    }
                }
                Some(ComponentKind::Dye) => {
                    self.dye = r
                        .as_mut()
                        .and_then(|r| r.varint().ok())
                        .and_then(|d| u8::try_from(d).ok());
                }
                Some(ComponentKind::ProvidesBannerPatterns) => {
                    self.provides_banner_patterns = r
                        .as_mut()
                        .and_then(|r| HolderSet::read(r).ok())
                        .map(|s| Holders::from_network(&s));
                }
                Some(ComponentKind::MapId) => self.map_id = true,
                _ => {}
            }
        }
    }
}

/// Item names as item ids (the item registry is built in).
fn item_holders(h: Holders) -> Holders {
    match h {
        Holders::Names(n) => Holders::Ids(
            n.iter()
                .filter_map(|n| ruthenium_data::v26_3::registries::Item::from_name(n))
                .map(|i| i.id() as i32)
                .collect(),
        ),
        h => h,
    }
}

/// One entry of the `minecraft:banner_pattern` registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerPattern {
    pub id: String,
    /// `asset_id` (its picture).
    pub asset: String,
    /// `translation_key`.
    pub translation_key: String,
}

/// What the session knows that the workstations read: the items, the synced registries'
/// tags, the stonecutter's recipes, the banner patterns, and the language names are
/// read in (`getString` of a stack's name).
#[derive(Debug, Clone, Default)]
pub struct WorkData {
    pub items: Arc<Items>,
    /// Each registry's tags, by full name, as entry ids in the server's order.
    pub tags: HashMap<String, HashMap<String, Vec<i32>>>,
    pub stonecutter: Vec<ruthenium_protocol::recipe::StonecutterEntry>,
    pub banner_patterns: Vec<BannerPattern>,
    pub language: Arc<HashMap<String, String>>,
}

impl WorkData {
    pub fn tags(&self, registry: &str) -> Option<&HashMap<String, Vec<i32>>> {
        self.tags.get(registry)
    }

    /// `ItemStack.is(tag)` for an item tag.
    pub fn item_is(&self, stack: &Stack, tag: &str) -> bool {
        !stack.is_empty()
            && self
                .tags("minecraft:item")
                .and_then(|t| t.get(tag))
                .is_some_and(|ids| ids.contains(&stack.item))
    }

    /// `HolderSet.contains` of a stack's item.
    pub fn item_in(&self, set: &Holders, stack: &Stack) -> bool {
        set.contains(stack.item, stack.c.name, self.tags("minecraft:item"))
    }

    /// `getHoverName().getString()`.
    pub fn hover_name(&self, stack: &Stack) -> String {
        let name = crate::item_name::hover_name(stack);
        let mut out = String::new();
        component_string(&name, &self.language, &mut out, 0);
        out
    }

    /// The registry's names in id order.
    pub fn banner_pattern_names(&self) -> Vec<String> {
        self.banner_patterns.iter().map(|p| p.id.clone()).collect()
    }

    /// Builds the session's banner patterns from the registry it was sent: an entry's
    /// own data, else vanilla's (a known pack's, sent without data: its asset is its id,
    /// its key `block.minecraft.banner.<path>`).
    pub fn banner_patterns_from(
        entries: &[ruthenium_protocol::packets::configuration::RegistryEntry],
    ) -> Vec<BannerPattern> {
        entries
            .iter()
            .map(|e| {
                let data = e.data.as_ref().and_then(|d| d.as_compound());
                let path = e.id.strip_prefix("minecraft:").unwrap_or(&e.id);
                BannerPattern {
                    id: e.id.clone(),
                    asset: data
                        .and_then(|d| d.string("asset_id"))
                        .unwrap_or(&e.id)
                        .to_owned(),
                    translation_key: data
                        .and_then(|d| d.string("translation_key"))
                        .map_or_else(|| format!("block.minecraft.banner.{path}"), str::to_owned),
                }
            })
            .collect()
    }
}

/// `Component.getString` of a component's NBT form: its text, translations filled in,
/// then its siblings; nothing read as formatting (a `§` stays).
fn component_string(
    t: &ruthenium_protocol::nbt::Tag,
    lang: &HashMap<String, String>,
    out: &mut String,
    depth: u32,
) {
    use ruthenium_protocol::nbt::Tag;
    if depth > 16 || out.len() > 1 << 16 {
        return;
    }
    match t {
        Tag::String(s) => out.push_str(s),
        Tag::List(items) => {
            for i in items {
                component_string(i, lang, out, depth + 1);
            }
        }
        Tag::Compound(c) => {
            if let Some(s) = c.string("text") {
                out.push_str(s);
            } else if let Some(key) = c.string("translate") {
                let format = lang
                    .get(key)
                    .map(String::as_str)
                    .or_else(|| c.string("fallback"))
                    .unwrap_or(key);
                let args = c.list("with");
                translate(format, args, lang, out, depth);
            }
            for e in c.list("extra") {
                component_string(e, lang, out, depth + 1);
            }
        }
        Tag::Int(n) => out.push_str(&n.to_string()),
        _ => {}
    }
}

/// `TranslatableContents.decompose`: `%s`, `%1$s` and `%%` in `format`.
fn translate(
    format: &str,
    args: &[ruthenium_protocol::nbt::Tag],
    lang: &HashMap<String, String>,
    out: &mut String,
    depth: u32,
) {
    let mut next = 0;
    let mut rest = format;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        if let Some(t) = tail.strip_prefix('%') {
            out.push('%');
            rest = t;
        } else if let Some(t) = tail.strip_prefix('s') {
            if let Some(a) = args.get(next) {
                component_string(a, lang, out, depth + 1);
            }
            next += 1;
            rest = t;
        } else if let Some(dollar) = tail
            .find("$s")
            .filter(|&d| tail[..d].chars().all(|c| c.is_ascii_digit()) && d > 0)
        {
            let i: usize = tail[..dollar].parse().unwrap_or(1);
            if let Some(a) = args.get(i.saturating_sub(1)) {
                component_string(a, lang, out, depth + 1);
            }
            rest = &tail[dollar + 2..];
        } else {
            out.push('%');
            rest = tail;
        }
    }
    out.push_str(rest);
}

/// What of the player the stations read between clicks: `hasInfiniteMaterials` and
/// `experienceLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerFacts {
    pub creative: bool,
    pub xp_level: i32,
}

/// A workstation menu's own state, and its screen's.
#[derive(Debug, Clone)]
pub struct State {
    pub data: Arc<WorkData>,
    pub player: PlayerFacts,
    /// Packets the menu sent while a click ran (an anvil's rename), which go out before
    /// the click's own.
    pub outbox: Vec<Vec<u8>>,
    pub container_id: i32,
    pub kind: KindState,
}

/// Equal states: the same kind state (the registries and the outbox aside).
impl PartialEq for State {
    fn eq(&self, other: &State) -> bool {
        self.kind == other.kind && self.player == other.player
    }
}

/// Each station's state.
#[derive(Debug, Clone, PartialEq)]
pub enum KindState {
    Anvil(Box<anvil::Anvil>),
    Smithing(smithing::Smithing),
    Stonecutter(stonecutter::Stonecutter),
    Loom(Box<loom::Loom>),
    Cartography,
    Crafter,
    Merchant(Box<merchant::Merchant>),
}

impl State {
    pub fn new(station: Station, container_id: i32) -> State {
        State {
            data: Arc::default(),
            player: PlayerFacts::default(),
            outbox: Vec::new(),
            container_id,
            kind: match station {
                Station::Anvil => KindState::Anvil(Box::default()),
                Station::Smithing => KindState::Smithing(smithing::Smithing::default()),
                Station::Stonecutter => KindState::Stonecutter(stonecutter::Stonecutter::default()),
                Station::Loom => KindState::Loom(Box::default()),
                Station::Cartography => KindState::Cartography,
                Station::Crafter => KindState::Crafter,
                Station::Merchant => KindState::Merchant(Box::default()),
            },
        }
    }
}

pub(super) fn slot(index: usize, x: i32, y: i32, rule: Rule) -> Slot {
    Slot {
        container: Container::Own,
        index,
        x,
        y,
        rule,
    }
}

/// A station's slots, its own container's size and its data slots
/// (`addSlot`, `addStandardInventorySlots` and `addDataSlot` in its constructor).
pub(super) fn layout(station: Station, slots: &mut Vec<Slot>) -> (usize, usize) {
    let st = |r: SlotRule| Rule::Station(r);
    match station {
        // `AnvilMenu.createInputSlotDefinitions`, then the cost.
        Station::Anvil => {
            slots.push(slot(0, 27, 47, Rule::Plain));
            slots.push(slot(1, 76, 47, Rule::Plain));
            slots.push(slot(2, 134, 47, st(SlotRule::CombinerResult)));
            super::standard_inventory(slots, 8, 84);
            (3, 1)
        }
        // `SmithingMenu.createInputSlotDefinitions`, then `hasRecipeError`.
        Station::Smithing => {
            slots.push(slot(0, 8, 48, st(SlotRule::Template)));
            slots.push(slot(1, 26, 48, st(SlotRule::Base)));
            slots.push(slot(2, 44, 48, st(SlotRule::Addition)));
            slots.push(slot(3, 98, 48, st(SlotRule::CombinerResult)));
            super::standard_inventory(slots, 8, 84);
            (4, 1)
        }
        Station::Stonecutter => {
            slots.push(slot(0, 20, 33, Rule::Plain));
            slots.push(slot(1, 143, 33, st(SlotRule::CutterResult)));
            super::standard_inventory(slots, 8, 84);
            (2, 1)
        }
        Station::Loom => {
            slots.push(slot(0, 13, 26, st(SlotRule::Banner)));
            slots.push(slot(1, 33, 26, st(SlotRule::Dye)));
            slots.push(slot(2, 23, 45, st(SlotRule::Pattern)));
            slots.push(slot(3, 143, 57, st(SlotRule::LoomResult)));
            super::standard_inventory(slots, 8, 84);
            (4, 1)
        }
        Station::Cartography => {
            slots.push(slot(0, 15, 15, st(SlotRule::Map)));
            slots.push(slot(1, 15, 52, st(SlotRule::MapAddition)));
            slots.push(slot(2, 145, 39, st(SlotRule::MapResult)));
            super::standard_inventory(slots, 8, 84);
            (3, 0)
        }
        // The grid, the inventory, then the result (slot 45); ten data slots.
        Station::Crafter => {
            for y in 0..3 {
                for x in 0..3 {
                    let i = (x + y * 3) as usize;
                    slots.push(slot(i, 26 + x * 18, 17 + y * 18, st(SlotRule::CrafterGrid)));
                }
            }
            super::standard_inventory(slots, 8, 84);
            slots.push(slot(9, 134, 35, st(SlotRule::CrafterResult)));
            (10, 10)
        }
        Station::Merchant => {
            slots.push(slot(0, 136, 37, Rule::Plain));
            slots.push(slot(1, 162, 37, Rule::Plain));
            slots.push(slot(2, 220, 37, st(SlotRule::TradeResult)));
            super::standard_inventory(slots, 108, 84);
            (3, 0)
        }
    }
}

impl Menu {
    /// The station's state (a workstation's menu always has one).
    pub fn work(&self) -> Option<&State> {
        self.work.as_deref()
    }

    pub fn work_mut(&mut self) -> Option<&mut State> {
        self.work.as_deref_mut()
    }

    /// The packets a click had the menu send before it (an anvil's rename).
    pub fn take_outbox(&mut self) -> Vec<Vec<u8>> {
        self.work
            .as_mut()
            .map(|w| std::mem::take(&mut w.outbox))
            .unwrap_or_default()
    }

    /// The station this menu is, if it is one.
    pub fn station(&self) -> Option<Station> {
        match self.kind {
            super::Kind::Station(s) => Some(s),
            _ => None,
        }
    }
}

/// `Container.setItem` and `Slot.setChanged` of a station's own container slot
/// `index`, written by `Slot.set` (or a slot method built on it): what the container
/// does about it.
pub(super) fn container_set(m: &mut Menu, inv: &mut Inventory, index: usize) {
    let Some(station) = m.station() else {
        return;
    };
    match station {
        Station::Anvil => anvil::changed(m, index),
        Station::Smithing => smithing::changed(m, index),
        Station::Stonecutter => stonecutter::changed(m, index),
        Station::Loom => loom::changed(m, index),
        Station::Cartography => cartography::changed(m),
        Station::Crafter => {}
        Station::Merchant => merchant::changed(m, index),
    }
    let _ = inv;
}

/// `Slot.setChanged` alone (the end of a pickup click, a quick move that left some
/// behind, `Slot.onTake`'s default).
pub(super) fn set_changed(m: &mut Menu, inv: &mut Inventory, i: usize) {
    if m.slots[i].container == Container::Own {
        let index = m.slots[i].index;
        container_set(m, inv, index);
    }
}

/// `Slot.mayPlace` of a station's slot.
pub(super) fn may_place(m: &Menu, i: usize, rule: SlotRule, stack: &Stack, who: &Who) -> bool {
    let data = m.work.as_ref().map(|w| &*w.data);
    match rule {
        SlotRule::CombinerResult
        | SlotRule::CutterResult
        | SlotRule::LoomResult
        | SlotRule::MapResult
        | SlotRule::CrafterResult
        | SlotRule::TradeResult => false,
        SlotRule::Template => who.recipes.contains("minecraft:smithing_template", stack),
        SlotRule::Base => who.recipes.contains("minecraft:smithing_base", stack),
        SlotRule::Addition => who.recipes.contains("minecraft:smithing_addition", stack),
        SlotRule::Banner => loom::is_banner(stack),
        SlotRule::Dye => data.is_some_and(|d| loom::is_dye(d, stack)),
        SlotRule::Pattern => data.is_some_and(|d| loom::is_pattern(d, stack)),
        SlotRule::Map => stack.c.work.map_id,
        SlotRule::MapAddition => cartography::is_addition(stack),
        SlotRule::CrafterGrid => !crafter::is_disabled(m, m.slots[i].index as i32),
    }
}

/// `Slot.mayPickup` of a station's slot.
pub(super) fn may_pickup(m: &Menu, inv: &Inventory, rule: SlotRule, who: &Who) -> bool {
    match (rule, m.station()) {
        (SlotRule::CombinerResult, Some(Station::Anvil)) => anvil::may_pickup(m, who),
        (SlotRule::CrafterResult, _) => false,
        (SlotRule::TradeResult, _) => merchant::may_pickup(m),
        _ => {
            let _ = inv;
            true
        }
    }
}

/// `Slot.remove` of a station's slot whose container doesn't split: a
/// `ResultContainer` takes it all, a merchant's result too, and the crafter's result
/// gives nothing. `None`: the container splits it as any other.
pub(super) fn remove_whole(rule: SlotRule) -> Option<bool> {
    match rule {
        SlotRule::CombinerResult
        | SlotRule::CutterResult
        | SlotRule::MapResult
        | SlotRule::TradeResult => Some(true),
        SlotRule::CrafterResult => Some(false),
        _ => None,
    }
}

/// `Slot.onTake` of a station's slot.
pub(super) fn on_take(
    m: &mut Menu,
    inv: &mut Inventory,
    i: usize,
    rule: SlotRule,
    taken: &mut Stack,
    who: &Who,
    out: &mut Outcome,
) {
    match rule {
        SlotRule::CombinerResult => match m.station() {
            Some(Station::Anvil) => anvil::on_take(m, who),
            Some(Station::Smithing) => smithing::on_take(m, taken),
            _ => {}
        },
        SlotRule::CutterResult => stonecutter::on_take(m, inv, i, taken),
        SlotRule::LoomResult => loom::on_take(m, inv, i),
        SlotRule::MapResult => cartography::on_take(m, inv, i, taken),
        SlotRule::TradeResult => merchant::on_take(m, taken),
        SlotRule::CrafterResult => {}
        // (`Slot.onTake`: `setChanged`.)
        _ => set_changed(m, inv, i),
    }
    let _ = out;
}

/// `canTakeItemForPickAll` of a station's menu.
pub(super) fn can_take_for_pick_all(m: &Menu, i: usize) -> bool {
    !matches!(
        (m.station(), m.slots[i].rule),
        (Some(Station::Merchant), _)
            | (
                _,
                Rule::Station(SlotRule::CutterResult | SlotRule::MapResult)
            )
            | (
                Some(Station::Smithing),
                Rule::Station(SlotRule::CombinerResult)
            )
    )
}

/// `quickMoveStack` of a station's menu.
pub(super) fn quick_move_stack(
    m: &mut Menu,
    inv: &mut Inventory,
    i: usize,
    who: &Who,
    out: &mut Outcome,
) -> Stack {
    match m.station() {
        Some(Station::Anvil | Station::Smithing) => combiner_quick_move(m, inv, i, who, out),
        Some(Station::Stonecutter) => stonecutter::quick_move(m, inv, i, who, out),
        Some(Station::Loom) => loom::quick_move(m, inv, i, who, out),
        Some(Station::Cartography) => cartography::quick_move(m, inv, i, who, out),
        Some(Station::Crafter) => crafter::quick_move(m, inv, i, who, out),
        Some(Station::Merchant) => merchant::quick_move(m, inv, i, who, out),
        None => Stack::empty(),
    }
}

/// The tail most stations' `quickMoveStack` share: an emptied slot is set empty, else
/// told it changed; nothing moved ends it; else the slot's `onTake`.
pub(super) fn quick_move_tail(
    m: &mut Menu,
    inv: &mut Inventory,
    i: usize,
    stack: Stack,
    clicked: &Stack,
    who: &Who,
    out: &mut Outcome,
) -> Stack {
    if stack.is_empty() {
        m.set(inv, i, Stack::empty());
    } else {
        m.set(inv, i, stack.clone());
    }
    if stack.count == clicked.count {
        return Stack::empty();
    }
    let mut taken = stack;
    m.on_take_stack(inv, i, &mut taken, who, out);
    clicked.clone()
}

/// `ItemCombinerMenu.quickMoveStack` (the anvil's and the smithing table's).
fn combiner_quick_move(
    m: &mut Menu,
    inv: &mut Inventory,
    i: usize,
    who: &Who,
    out: &mut Outcome,
) -> Stack {
    if !m.has_item(inv, i) {
        return Stack::empty();
    }
    let mut stack = m.get(inv, i);
    let clicked = stack.copy();
    let result = if m.station() == Some(Station::Anvil) {
        2
    } else {
        3
    };
    let (inv_start, inv_end) = (result + 1, result + 28);
    let (row_start, row_end) = (inv_end, inv_end + 9);
    let moved = if i == result {
        m.move_item_stack_to(inv, &mut stack, inv_start, row_end, true, who)
    } else if i < result {
        m.move_item_stack_to(inv, &mut stack, inv_start, row_end, false, who)
    } else if can_move_into_inputs(m, inv, &stack, who) && (inv_start..row_end).contains(&i) {
        m.move_item_stack_to(inv, &mut stack, 0, result, false, who)
    } else if (inv_start..inv_end).contains(&i) {
        m.move_item_stack_to(inv, &mut stack, row_start, row_end, false, who)
    } else if (row_start..row_end).contains(&i) {
        m.move_item_stack_to(inv, &mut stack, inv_start, inv_end, false, who)
    } else {
        true
    };
    // (The stack moved in place: the slot holds what is left of it.)
    m.own_or_inv_put(inv, i, stack.clone());
    if !moved {
        return Stack::empty();
    }
    quick_move_tail(m, inv, i, stack, &clicked, who, out)
}

/// `canMoveIntoInputSlots`: anything for an anvil; for a smithing table, a stack one of
/// its empty inputs takes.
fn can_move_into_inputs(m: &Menu, inv: &Inventory, stack: &Stack, who: &Who) -> bool {
    if m.station() != Some(Station::Smithing) {
        return true;
    }
    let r = who.recipes;
    (r.contains("minecraft:smithing_template", stack) && !m.has_item(inv, 0))
        || (r.contains("minecraft:smithing_base", stack) && !m.has_item(inv, 1))
        || (r.contains("minecraft:smithing_addition", stack) && !m.has_item(inv, 2))
}

/// `Item.onCraftedBy` on the client: a map loses what the cartography table was to do
/// to it (`MapItem.onCraftedPostProcess` removes `map_post_processing`).
pub fn crafted_by(items: &Items, stack: &mut Stack) {
    if !stack.is_empty() && stack.c.is("MapItem") {
        *stack = remove_component(items, stack, ComponentKind::MapPostProcessing);
    }
}

/// `ItemStack.set`: the stack (a new object) with `kind` set to `value`; a value equal to
/// the item's default (`is_default`) leaves the patch without it.
pub fn set_component(
    items: &Items,
    stack: &Stack,
    kind: ComponentKind,
    value: Component,
    is_default: bool,
) -> Stack {
    let id = kind.id() as i32;
    let mut patch = (*stack.patch).clone();
    if is_default {
        patch.added.retain(|(k, _)| *k != id);
        patch.removed.retain(|k| *k != id);
    } else if let Some(at) = patch.added.iter_mut().find(|(k, _)| *k == id) {
        at.1 = value;
    } else {
        patch.removed.retain(|k| *k != id);
        patch.added.push((id, value));
    }
    rebuild(items, stack, patch)
}

/// `ItemStack.remove`: an item that has `kind` by default gets it removed in the patch;
/// otherwise the patch forgets it.
pub fn remove_component(items: &Items, stack: &Stack, kind: ComponentKind) -> Stack {
    let id = kind.id() as i32;
    let had = stack.patch.added.iter().any(|(k, _)| *k == id) || stack.patch.removed.contains(&id);
    let default = has_by_default(items, stack.item, kind);
    if !had && !default {
        return stack.clone();
    }
    let mut patch = (*stack.patch).clone();
    patch.added.retain(|(k, _)| *k != id);
    if default {
        if !patch.removed.contains(&id) {
            patch.removed.push(id);
        }
    } else {
        patch.removed.retain(|k| *k != id);
    }
    rebuild(items, stack, patch)
}

/// Whether the item has the component by default (the ones the stations remove).
fn has_by_default(items: &Items, item: i32, kind: ComponentKind) -> bool {
    let d = items.defaults(item);
    match kind {
        ComponentKind::CustomName => d.custom_name.is_some(),
        ComponentKind::Enchantments => d.work.has_enchantments,
        ComponentKind::StoredEnchantments => d.work.stored_enchantments.is_some(),
        _ => false,
    }
}

fn rebuild(items: &Items, stack: &Stack, patch: ComponentPatch) -> Stack {
    let mut s = items.stack(&ItemStack {
        item: stack.item,
        count: stack.count,
        patch,
    });
    // (A copy keeps the original's count and object identity rules: a new object.)
    s.count = stack.count;
    s
}

/// A text component's network bytes for a literal string (`Component.literal`: a
/// string tag).
pub fn literal_text(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = ruthenium_protocol::nbt::write_network(
        &mut out,
        &ruthenium_protocol::nbt::Tag::String(s.to_owned()),
    );
    out
}

/// A varint's bytes (a component written as one: `repair_cost`).
pub fn varint_bytes(v: i32) -> Vec<u8> {
    let mut out = Vec::new();
    ruthenium_protocol::wire::write_varint(&mut out, v);
    out
}

/// `StringUtil.filterText`: `§` and control characters out (by UTF-16 unit).
pub fn filter_text(s: &str) -> String {
    s.chars()
        .filter(|&c| c != '§' && c >= ' ' && c != '\u{7F}')
        .collect()
}

/// `StringUtil.isBlank`.
pub fn is_blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.chars().all(|c| c.is_whitespace() || is_space_char(c)))
}

/// `Character.isSpaceChar`: the Unicode space separators.
fn is_space_char(c: char) -> bool {
    matches!(
        c,
        '\u{20}' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}
