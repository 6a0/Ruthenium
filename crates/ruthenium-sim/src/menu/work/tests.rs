//! The workstations against vanilla's own menus (`golden`, from WorkGolden.java): random
//! menus, stacks and actions drawn from the same generator on both sides, each case's
//! record compared by its hash.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as ComponentKind, Item};
use ruthenium_prepare::exporters::ItemClassData;
use ruthenium_protocol::item::{Component, ComponentPatch, HolderSet, ItemStack, TrimPart};
use ruthenium_protocol::packets::configuration::RegistryEntry;
use ruthenium_protocol::recipe::{SlotDisplay, StonecutterEntry};
use ruthenium_protocol::trade::{ItemCost, MerchantOffer};

use super::golden;
use super::{KindState, PlayerFacts, Station, WorkData};
use crate::inventory::Inventory;
use crate::item::{Items, RecipeSets, Stack};
use crate::menu::{Input, Kind, Menu, Who, quickcraft_mask};

struct NoRegistries;
impl ruthenium_protocol::hashed::SyncedRegistries for NoRegistries {
    fn key(&self, _: &str, _: i32) -> Option<&str> {
        None
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as i32
    }
}

fn id(name: &str) -> i32 {
    Item::from_name(name).unwrap().id() as i32
}

/// What the cases share: the items, the registries and the recipe sets.
pub(super) struct World {
    pub(super) data: Arc<WorkData>,
    pub(super) recipes: RecipeSets,
    /// Each enchantment's `max_level` and `exclusive_set` ids, by registry id.
    max_level: Vec<i32>,
    exclusive: Vec<Vec<i32>>,
}

pub(super) fn world() -> World {
    let mut classes = Vec::new();
    let mut reports = HashMap::new();
    for &(item, class, json) in golden::POOL {
        let name = Item::from_id(item as u32).unwrap().name();
        classes.push(ItemClassData {
            id: item as u32,
            key: name.to_owned(),
            classes: class.iter().map(|s| (*s).to_owned()).collect(),
            flags: 0,
            block: None,
            fields: Vec::new(),
        });
        reports.insert(name.to_owned(), serde_json::from_str(json).unwrap());
    }
    let mut items = Items::new(&classes, &reports);
    let files: Vec<(String, Vec<u8>)> = golden::ENCHANTMENTS
        .iter()
        .map(|(n, j)| {
            let file = format!("{}.json", n.trim_start_matches("minecraft:"));
            (file, j.as_bytes().to_vec())
        })
        .collect();
    let vanilla = crate::enchantment::Enchantments::from_json_files(files);
    let entries: Vec<RegistryEntry> = golden::ENCHANTMENTS
        .iter()
        .map(|(n, _)| RegistryEntry {
            id: (*n).to_owned(),
            data: None,
        })
        .collect();
    items.enchantments = crate::enchantment::Enchantments::from_registry(&entries, &vanilla);
    items.banner_patterns = golden::PATTERNS.iter().map(|n| (*n).into()).collect();
    let mut tags = HashMap::new();
    for (registry, list) in [
        ("minecraft:item", golden::ITEM_TAGS),
        ("minecraft:enchantment", golden::ENCHANTMENT_TAGS),
        ("minecraft:banner_pattern", golden::PATTERN_TAGS),
    ] {
        let t: HashMap<String, Vec<i32>> = list
            .iter()
            .map(|(n, ids)| ((*n).to_owned(), ids.to_vec()))
            .collect();
        tags.insert(registry.to_owned(), t);
    }
    // The tool's stonecutter list.
    let entry = |inputs: &[&str], out: &str, count: i32| StonecutterEntry {
        input: HolderSet::Ids(inputs.iter().map(|n| id(n)).collect()),
        display: SlotDisplay::ItemStack(ItemStack {
            item: id(out),
            count,
            patch: ComponentPatch::default(),
        }),
    };
    let outs = [
        "minecraft:stone_bricks",
        "minecraft:stone_slab",
        "minecraft:stone_stairs",
        "minecraft:chiseled_stone_bricks",
        "minecraft:stone_brick_slab",
    ];
    let mut stonecutter: Vec<StonecutterEntry> = (0..15)
        .map(|i| entry(&["minecraft:stone"], outs[i % 5], 1 + i as i32 / 5))
        .collect();
    for _ in 0..3 {
        stonecutter.push(entry(
            &["minecraft:cobblestone"],
            "minecraft:cobblestone_slab",
            2,
        ));
    }
    stonecutter.push(entry(
        &["minecraft:stone", "minecraft:cobblestone"],
        "minecraft:stone_button",
        1,
    ));
    let banner_patterns = golden::PATTERNS
        .iter()
        .map(|n| super::BannerPattern {
            id: (*n).to_owned(),
            asset: (*n).to_owned(),
            translation_key: String::new(),
        })
        .collect();
    let mut recipes = RecipeSets::new();
    recipes.set(
        "minecraft:smithing_template",
        [
            id("minecraft:netherite_upgrade_smithing_template"),
            id("minecraft:sentry_armor_trim_smithing_template"),
        ],
    );
    recipes.set(
        "minecraft:smithing_base",
        [
            id("minecraft:diamond_sword"),
            id("minecraft:iron_sword"),
            id("minecraft:diamond_chestplate"),
            id("minecraft:iron_helmet"),
        ],
    );
    recipes.set(
        "minecraft:smithing_addition",
        [
            id("minecraft:netherite_ingot"),
            id("minecraft:diamond"),
            id("minecraft:iron_ingot"),
            id("minecraft:redstone"),
        ],
    );
    let ench_tags = &tags["minecraft:enchantment"];
    let mut max_level = Vec::new();
    let mut exclusive = Vec::new();
    for (_, j) in golden::ENCHANTMENTS {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        max_level.push(v["max_level"].as_i64().unwrap() as i32);
        let ex = match &v["exclusive_set"] {
            serde_json::Value::String(s) if s.starts_with('#') => {
                ench_tags.get(&s[1..]).cloned().unwrap_or_default()
            }
            serde_json::Value::Array(a) => a
                .iter()
                .filter_map(|n| {
                    golden::ENCHANTMENTS
                        .iter()
                        .position(|(m, _)| Some(*m) == n.as_str())
                        .map(|p| p as i32)
                })
                .collect(),
            _ => Vec::new(),
        };
        exclusive.push(ex);
    }
    World {
        data: Arc::new(WorkData {
            items: Arc::new(items),
            tags,
            stonecutter,
            banner_patterns,
            language: Arc::default(),
        }),
        recipes,
        max_level,
        exclusive,
    }
}

impl World {
    fn compatible(&self, a: i32, b: i32) -> bool {
        a != b
            && !self.exclusive[a as usize].contains(&b)
            && !self.exclusive[b as usize].contains(&a)
    }
}

const NAMES: [&str; 7] = [
    "Foo",
    "Bar",
    "item.minecraft.diamond_sword",
    "  ",
    "item.minecraft.stone",
    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
    "a\u{a7}b",
];
const KINDS: [&str; 7] = [
    "anvil",
    "smithing",
    "stonecutter",
    "loom",
    "cartography",
    "crafter",
    "merchant",
];
const STATIONS: [Station; 7] = [
    Station::Anvil,
    Station::Smithing,
    Station::Stonecutter,
    Station::Loom,
    Station::Cartography,
    Station::Crafter,
    Station::Merchant,
];
const OWN: [i32; 7] = [3, 4, 2, 4, 3, 9, 3];

fn other(kind: ComponentKind, data: Vec<u8>) -> (i32, Component) {
    let k = kind.id() as i32;
    (k, Component::Other { kind: k, data })
}

/// Each kind's favourite items (pool indices), drawn for its own slots half the time.
const FAVOURITES: [&[usize]; 7] = [
    &[1, 2, 5, 3, 4, 19, 24],
    &[17, 18, 1, 19, 20, 4, 21],
    &[0, 22],
    &[6, 7, 8, 9, 10],
    &[11, 12, 13, 14],
    &[0, 23, 22],
    &[15, 4, 3, 16, 13],
];

/// The tool's `ownStack`.
fn own_stack(r: &mut Lcg, w: &World, kind: usize) -> Stack {
    if r.next(2) == 0 {
        let f = FAVOURITES[kind];
        let item = golden::POOL[f[r.next(f.len() as i32) as usize]].0;
        return stack_of(r, w, item);
    }
    random_stack(r, w)
}

/// The tool's `randomStack`.
fn random_stack(r: &mut Lcg, w: &World) -> Stack {
    let (item, _, _) = golden::POOL[r.next(golden::POOL.len() as i32) as usize];
    stack_of(r, w, item)
}

fn stack_of(r: &mut Lcg, w: &World, item: i32) -> Stack {
    let items = &w.data.items;
    let d = items.defaults(item);
    let count = 1 + r.next(d.max_stack_size());
    let mut added = Vec::new();
    let damageable = d.damageable();
    if damageable && r.next(2) == 0 {
        let v = 1 + r.next(d.max_damage.unwrap() - 1);
        added.push((ComponentKind::Damage.id() as i32, Component::Damage(v)));
    }
    if item == id("minecraft:enchanted_book") {
        let e = golden::ENCHS[r.next(golden::ENCHS.len() as i32) as usize];
        let level = 1 + r.next(w.max_level[e as usize]);
        added.push((
            ComponentKind::StoredEnchantments.id() as i32,
            Component::StoredEnchantments(vec![(e, level)]),
        ));
    } else if damageable && r.next(3) == 0 {
        let a = golden::ENCHS[r.next(golden::ENCHS.len() as i32) as usize];
        let la = 1 + r.next(w.max_level[a as usize]);
        let b = golden::ENCHS[r.next(golden::ENCHS.len() as i32) as usize];
        let lb = 1 + r.next(w.max_level[b as usize]);
        let mut e = vec![(a, la)];
        if w.compatible(a, b) {
            e.push((b, lb));
        }
        added.push((
            ComponentKind::Enchantments.id() as i32,
            Component::Enchantments(e),
        ));
    }
    if r.next(6) == 0 {
        let name = NAMES[r.next(NAMES.len() as i32) as usize];
        added.push(other(ComponentKind::CustomName, super::literal_text(name)));
    }
    if r.next(6) == 0 {
        let cost = 1 + r.next(40);
        added.push(other(ComponentKind::RepairCost, super::varint_bytes(cost)));
    }
    if item == id("minecraft:white_banner") && r.next(2) == 0 {
        let n = r.next(8);
        let layers: Vec<(TrimPart, i32)> = (0..n)
            .map(|_| {
                let p = r.next(golden::PATTERNS.len() as i32);
                (TrimPart::Id(p), r.next(16))
            })
            .collect();
        if !layers.is_empty() {
            added.push((
                ComponentKind::BannerPatterns.id() as i32,
                Component::BannerPatterns(layers),
            ));
        }
    }
    if item == id("minecraft:filled_map") {
        added.push(other(ComponentKind::MapId, super::varint_bytes(r.next(5))));
        if r.next(3) == 0 {
            added.push(other(
                ComponentKind::MapPostProcessing,
                super::varint_bytes(r.next(2)),
            ));
        }
    }
    items.stack(&ItemStack {
        item,
        count,
        patch: ComponentPatch {
            added,
            ..ComponentPatch::default()
        },
    })
}

fn varint(data: &[u8]) -> i32 {
    ruthenium_protocol::wire::Reader::new(data)
        .varint()
        .unwrap_or(-1)
}

/// A component's value as the tool writes it.
fn value(c: &Component) -> String {
    match c {
        Component::Damage(n) => n.to_string(),
        Component::Enchantments(e) | Component::StoredEnchantments(e) => {
            let mut e = e.clone();
            e.sort();
            e.iter().map(|(i, l)| format!("{i}x{l}+")).collect()
        }
        Component::BannerPatterns(l) => l
            .iter()
            .map(|(p, colour)| match p {
                TrimPart::Id(id) => format!("{id}/{colour}+"),
                TrimPart::Inline { .. } => format!("?/{colour}+"),
            })
            .collect(),
        Component::Other { kind, data } => {
            match u32::try_from(*kind).ok().and_then(ComponentKind::from_id) {
                Some(ComponentKind::CustomName) => {
                    match ruthenium_protocol::nbt::read_network(data) {
                        Ok((Some(ruthenium_protocol::nbt::Tag::String(s)), _)) => format!("s{s}"),
                        _ => "s?".into(),
                    }
                }
                Some(
                    ComponentKind::RepairCost
                    | ComponentKind::MapId
                    | ComponentKind::MapPostProcessing,
                ) => varint(data).to_string(),
                _ => "?".into(),
            }
        }
        _ => "?".into(),
    }
}

fn stack(s: &Stack) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let mut out = format!("{}*{}", s.item, s.count);
    let mut added: Vec<(i32, String)> = s.patch.added.iter().map(|(k, c)| (*k, value(c))).collect();
    added.sort_by_key(|(k, _)| *k);
    if !added.is_empty() {
        out.push('{');
        for (k, v) in added {
            out += &format!("{k}:{v},");
        }
        out.push('}');
    }
    let mut removed = s.patch.removed.clone();
    removed.sort();
    if !removed.is_empty() {
        out.push('!');
        for k in removed {
            out += &format!("{k},");
        }
    }
    out
}

fn contents(m: &Menu, inv: &Inventory) -> String {
    (0..m.slots.len())
        .filter(|&i| !m.item(inv, i).is_empty())
        .map(|i| format!("{i}={};", stack(m.item(inv, i))))
        .collect()
}

/// The menu's outbox as the tool's events (an anvil's renames).
fn outbox(m: &mut Menu, events: &mut Vec<String>) {
    for p in m.take_outbox() {
        let mut r = ruthenium_protocol::wire::Reader::new(&p[1..]);
        events.push(format!("rename {}", r.string(32767).unwrap_or("?")));
    }
}

fn record(m: &mut Menu, inv: &Inventory, events: &mut Vec<String>) -> String {
    outbox(m, events);
    let mut b = format!("{}|c={}|d=", contents(m, inv), stack(&m.carried));
    for d in &m.data {
        b += &format!("{d},");
    }
    b += "|x=";
    match m.work.as_ref().map(|w| &w.kind) {
        Some(KindState::Anvil(a)) => {
            b += &a.name.value();
            if a.name.editable {
                b += " e";
            }
        }
        Some(KindState::Stonecutter(s)) => {
            for i in &s.recipes {
                b += &format!("{i},");
            }
        }
        Some(KindState::Loom(l)) => {
            for p in &l.patterns {
                b += &format!("{p},");
            }
        }
        Some(KindState::Merchant(t)) => {
            b += &format!("{}/{}:", t.xp, t.future_xp);
            for o in &t.offers {
                b += &format!("{},", o.uses);
            }
        }
        _ => {}
    }
    b += "|e=";
    b += &events.join(";");
    events.clear();
    b
}

/// The click as the tool writes it: its fields, then each stack's item, count and component
/// types (added and removed, sorted: vanilla's hashes go in an `IdentityHashMap`'s order).
fn click(body: &[u8]) -> String {
    let mut r = ruthenium_protocol::wire::Reader::new(body);
    let container = r.varint().unwrap();
    let state = r.varint().unwrap();
    let slot = r.u16().unwrap() as i16;
    let button = r.u8().unwrap() as i8;
    let input = r.varint().unwrap();
    let stack = |r: &mut ruthenium_protocol::wire::Reader<'_>| {
        if !r.bool().unwrap() {
            return "-".to_owned();
        }
        let item = r.varint().unwrap();
        let count = r.varint().unwrap();
        let mut added: Vec<i32> = (0..r.varint().unwrap())
            .map(|_| {
                let k = r.varint().unwrap();
                r.i32().unwrap();
                k
            })
            .collect();
        let mut removed: Vec<i32> = (0..r.varint().unwrap())
            .map(|_| r.varint().unwrap())
            .collect();
        added.sort();
        removed.sort();
        let list = |v: &[i32]| {
            let s: Vec<String> = v.iter().map(i32::to_string).collect();
            format!("[{}]", s.join(", "))
        };
        format!("{item}*{count}+{}-{}", list(&added), list(&removed))
    };
    let mut out = format!("{container},{state},{slot},{button},{input}|");
    for _ in 0..r.varint().unwrap() {
        let k = r.u16().unwrap() as i16;
        out += &format!("{k}={};", stack(&mut r));
    }
    out += &format!("|c={}", stack(&mut r));
    out
}

fn run_case(c: u64, w: &World) -> String {
    let mut r = Lcg(0x5EED_7000 + c);
    let kind = r.next(7) as usize;
    let creative = r.next(5) == 0;
    let xp = r.next(45);
    let mut m = Menu::new(Kind::Station(STATIONS[kind]), 1);
    {
        let state = m.work_mut().unwrap();
        state.data = w.data.clone();
        state.player = PlayerFacts {
            creative,
            xp_level: xp,
        };
    }
    let mut inv = Inventory::default();
    let items = w.data.items.clone();
    let who = Who {
        items: &items,
        recipes: &w.recipes,
        creative,
    };
    let mut events = Vec::new();
    let mut out = format!(
        "{}{} xp {xp}\n",
        KINDS[kind],
        if creative { " creative" } else { "" }
    );
    if STATIONS[kind] == Station::Merchant {
        const COSTS: [&str; 5] = [
            "minecraft:emerald",
            "minecraft:diamond",
            "minecraft:iron_ingot",
            "minecraft:book",
            "minecraft:paper",
        ];
        let n = 1 + r.next(10);
        let mut offers = Vec::new();
        for _ in 0..n {
            let a = id(COSTS[r.next(5) as usize]);
            let cost_a = ItemCost {
                item: a,
                count: 1 + r.next(16),
                components: Vec::new(),
            };
            let cost_b = (r.next(3) == 0).then(|| {
                let b = id(COSTS[r.next(5) as usize]);
                ItemCost {
                    item: b,
                    count: 1 + r.next(8),
                    components: Vec::new(),
                }
            });
            let result = random_stack(&mut r, w);
            let max_uses = 1 + r.next(12);
            let uses = r.next(max_uses + 1);
            let xp = r.next(10);
            let mult = [0.0f32, 0.05, 0.2][r.next(3) as usize];
            let demand = r.next(10) - 3;
            let special = -r.next(5);
            offers.push(MerchantOffer {
                cost_a,
                result: ItemStack {
                    item: result.item,
                    count: result.count,
                    patch: (*result.patch).clone(),
                },
                cost_b,
                out_of_stock: false,
                uses,
                max_uses,
                xp,
                special_price_diff: special,
                price_multiplier: mult,
                demand,
            });
        }
        let villager_xp = r.next(300);
        let level = 1 + r.next(5);
        super::merchant::set_offers(&mut m, &w.data, &offers, level, villager_xp, true, true);
        out += &format!("offers {}\n", offers.len());
    }
    let n = m.slots.len() as i32;
    let own = OWN[kind];
    for i in 0..n {
        let (of, under) = if i < own { (3, 2) } else { (4, 1) };
        if r.next(of) < under {
            let s = if i < own {
                own_stack(&mut r, w, kind)
            } else {
                random_stack(&mut r, w)
            };
            m.set(&mut inv, i as usize, s);
        }
    }
    inv.selected = r.next(9);
    if r.next(3) == 0 {
        m.carried = random_stack(&mut r, w);
    }
    let state_id = r.next(100);
    let copies: Vec<Stack> = (0..m.slots.len()).map(|i| m.item(&inv, i).copy()).collect();
    let carried = m.carried.clone();
    m.initialize(&mut inv, state_id, copies, carried);
    out += &format!("state {}\n", m.state_id);
    out += &record(&mut m, &inv, &mut events);
    out.push('\n');
    let pick = |r: &mut Lcg| {
        if r.next(2) == 0 {
            r.next(own)
        } else {
            r.next(n)
        }
    };
    for _ in 0..16 {
        let action = r.next(10);
        let mut packet = None;
        let mut drops = 0;
        let desc = if action == 7 {
            let at = r.next(own);
            let st = r.next(100);
            let s = if r.next(4) == 0 {
                Stack::empty()
            } else {
                own_stack(&mut r, w, kind)
            };
            m.set_item(&mut inv, at, st, s);
            format!("s{at}")
        } else if action == 8 {
            let id = r.next(10);
            let value = r.next(47) - 1;
            if (id as usize) < m.data.len() {
                m.set_data(id, value);
            }
            format!("d{id}={value}")
        } else if action == 9 {
            match STATIONS[kind] {
                Station::Stonecutter => {
                    let count = super::stonecutter::cutter(&m).unwrap().recipes.len() as i32;
                    let id = r.next(count + 3) - 1;
                    if super::stonecutter::click_menu_button(&mut m, id) {
                        events.push(format!("button {id}"));
                    }
                    format!("w{id}")
                }
                Station::Loom => {
                    let count = super::loom::loom(&m).unwrap().patterns.len() as i32;
                    let id = r.next(count + 3) - 1;
                    if super::loom::click_menu_button(&mut m, id) {
                        events.push(format!("button {id}"));
                    }
                    format!("w{id}")
                }
                Station::Merchant => {
                    let count = super::merchant::merchant(&m).unwrap().offers.len() as i32;
                    let id = r.next(count + 2) - 1;
                    super::merchant::select(&mut m, &mut inv, id, &who);
                    events.push(format!("trade {id}"));
                    format!("w{id}")
                }
                Station::Anvil => {
                    let which = r.next(NAMES.len() as i32 + 1) as usize;
                    let name = if which == NAMES.len() {
                        let s = m.item(&inv, 0).clone();
                        if s.is_empty() {
                            String::new()
                        } else {
                            w.data.hover_name(&s)
                        }
                    } else {
                        NAMES[which].to_owned()
                    };
                    let editable = matches!(
                        m.work.as_ref().map(|s| &s.kind),
                        Some(KindState::Anvil(a)) if a.name.editable
                    );
                    if editable {
                        if let Some(KindState::Anvil(a)) = m.work.as_mut().map(|s| &mut s.kind) {
                            a.name.set_value(&name);
                        }
                        super::anvil::on_name_changed(&mut m, &name);
                    }
                    format!("w{which}")
                }
                _ => "w".to_owned(),
            }
        } else {
            let input = r.next(7);
            let (slot, button) = match input {
                2 => {
                    let slot = pick(&mut r);
                    let b = r.next(10);
                    (slot, if b == 9 { 40 } else { b })
                }
                5 => {
                    let h = r.next(3);
                    let t = r.next(3);
                    let slot = if h == 1 { pick(&mut r) } else { -999 };
                    (slot, quickcraft_mask(h, t))
                }
                _ => {
                    let x = r.next(20);
                    let slot = if x == 0 {
                        -999
                    } else if x == 1 && input == 0 {
                        -1
                    } else {
                        pick(&mut r)
                    };
                    let button = match input {
                        0 | 1 => [0, 0, 1, 1, 2][r.next(5) as usize],
                        3 => r.next(3),
                        _ => r.next(2),
                    };
                    (slot, button)
                }
            };
            let i = Input::from_id(input).unwrap();
            if STATIONS[kind] == Station::Crafter
                && let Some((at, on)) = super::crafter::toggle(&m, &inv, slot, button, i)
            {
                super::crafter::set_slot_state(&mut m, at, on);
                events.push(format!("slot {at} {}", if on { "on" } else { "off" }));
            }
            let (p, outcome) = m
                .handle_container_input(&mut inv, &who, 1, slot, button, i, &NoRegistries)
                .unwrap();
            packet = Some(p);
            drops = outcome.dropped.len();
            format!("{input}:{slot}:{button}")
        };
        let hex = packet.map_or_else(|| "-".to_owned(), |p| click(&p[1..]));
        out += &format!(
            "{desc}|{}|dr={drops}|p={hex}\n",
            record(&mut m, &inv, &mut events)
        );
    }
    out
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Every case as vanilla's menus ran it.
#[test]
fn workstations_do_what_vanillas_menus_do() {
    let w = world();
    let mut failed = Vec::new();
    for (c, &want) in golden::CASES.iter().enumerate() {
        if fnv(&run_case(c as u64, &w)) != want {
            failed.push(c);
        }
    }
    if let Some(&first) = failed.first() {
        panic!(
            "{} of {} cases differ (first {first}):\n{}",
            failed.len(),
            golden::CASES.len(),
            run_case(first as u64, &w)
        );
    }
}

/// One case written out (`RUTHENIUM_WORK_CASE=n cargo test -p ruthenium-sim
/// print_a_work_case -- --nocapture`), to compare with the tool's.
#[test]
fn print_a_work_case() {
    if let Some(c) = std::env::var("RUTHENIUM_WORK_CASE")
        .ok()
        .and_then(|c| c.parse().ok())
    {
        print!("{}", run_case(c, &world()));
    }
}
