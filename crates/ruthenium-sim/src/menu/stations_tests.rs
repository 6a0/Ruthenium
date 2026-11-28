//! The menus of [`super::stations`] and [`super::mount`] against vanilla's own
//! (`stations_golden.rs`, from a private tool that runs vanilla's menus headless on
//! the same random cases), and their rules one by one.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as K, EntityType, Item};
use ruthenium_prepare::exporters::ItemClassData;
use ruthenium_protocol::item::{Component, ComponentPatch, ItemStack};
use ruthenium_protocol::packets::configuration::RegistryEntry;

use super::super::mount::{Mount, MountState};
use super::super::*;
use super::golden_m as golden;
use crate::item::{IdTags, Items, MenuTags, RecipeSets};

struct NoRegistries;
impl ruthenium_protocol::hashed::SyncedRegistries for NoRegistries {
    fn key(&self, _: &str, _: i32) -> Option<&str> {
        None
    }
}

const SLOTS: [&str; 8] = [
    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
];

/// Items as vanilla has them, for the golden pool, with the tags and the enchantments.
fn items() -> Items {
    let mut classes = Vec::new();
    let mut reports = HashMap::new();
    for &(id, max, damage, equip, allowed, fuel, enchantable) in golden::POOL {
        let name = Item::from_id(id as u32).unwrap().name();
        classes.push(ItemClassData {
            id: id as u32,
            key: name.to_owned(),
            classes: vec!["Item".to_owned()],
            flags: 0,
            block: None,
            fields: Vec::new(),
        });
        let mut c = serde_json::Map::new();
        c.insert("minecraft:max_stack_size".into(), max.into());
        c.insert("minecraft:enchantments".into(), serde_json::json!({}));
        c.insert("minecraft:repair_cost".into(), 0.into());
        if name == "minecraft:enchanted_book" {
            c.insert(
                "minecraft:stored_enchantments".into(),
                serde_json::json!({}),
            );
        }
        if damage > 0 {
            c.insert("minecraft:max_damage".into(), damage.into());
            c.insert("minecraft:damage".into(), 0.into());
        }
        if equip >= 0 {
            let mut e = serde_json::json!({ "slot": SLOTS[equip as usize] });
            if let Some(tag) = allowed.strip_prefix('#') {
                e["allowed_entities"] = format!("#{tag}").into();
            } else if !allowed.is_empty() {
                let names: Vec<&str> = allowed
                    .split('|')
                    .map(|i| EntityType::from_id(i.parse().unwrap()).unwrap().name())
                    .collect();
                e["allowed_entities"] = names.into();
            }
            c.insert("minecraft:equippable".into(), e);
        }
        if fuel {
            c.insert("minecraft:brewing_fuel".into(), serde_json::json!({}));
        }
        if enchantable {
            c.insert(
                "minecraft:enchantable".into(),
                serde_json::json!({ "value": 1 }),
            );
        }
        reports.insert(name.to_owned(), serde_json::json!({ "components": c }));
    }
    let mut items = Items::new(&classes, &reports);
    // The enchantments by the cases' ids; the curse of binding keeps armour on.
    let files = golden::ENCHANTMENTS.iter().map(|n| {
        let json = if *n == "binding_curse" {
            r#"{"effects": {"minecraft:prevent_armor_change": {}}}"#
        } else {
            "{}"
        };
        (format!("enchantment/{n}.json"), json.as_bytes().to_vec())
    });
    let vanilla = crate::enchantment::Enchantments::from_json_files(files);
    let entries: Vec<RegistryEntry> = golden::ENCHANTMENTS
        .iter()
        .map(|n| RegistryEntry {
            id: format!("minecraft:{n}"),
            data: None,
        })
        .collect();
    items.enchantments = crate::enchantment::Enchantments::from_registry(&entries, &vanilla);
    let mut tags = MenuTags::default();
    for &(registry, tag, ids) in golden::TAGS {
        match registry {
            "item" => tags.items.set(
                tag,
                ids.iter().map(|&i| Item::from_id(i as u32).unwrap().name()),
            ),
            "entity_type" => tags.entity_types.set(tag, ids.iter().copied()),
            _ => tags.enchantments.set(tag, ids.iter().copied()),
        }
    }
    let _: &IdTags = &tags.enchantments;
    items.tags = Arc::new(tags);
    items
}

fn id(name: &str) -> i32 {
    Item::from_name(name).unwrap().id() as i32
}

fn recipes() -> RecipeSets {
    let mut r = RecipeSets::new();
    r.set(
        "minecraft:brewing_input",
        [id("minecraft:potion"), id("minecraft:splash_potion")],
    );
    r.set(
        "minecraft:brewing_reagent",
        [
            id("minecraft:nether_wart"),
            id("minecraft:redstone"),
            id("minecraft:blaze_powder"),
        ],
    );
    r
}

/// The tool's generator (a 64-bit LCG).
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

/// The tool's `randomStack`.
fn random_stack(r: &mut Lcg, items: &Items) -> Stack {
    let pool = golden::POOL;
    let (item, max, damage, ..) = pool[r.next(pool.len() as i32) as usize];
    let count = 1 + r.next(max);
    let mut added = Vec::new();
    if damage > 0 && r.next(2) == 0 {
        let d = 1 + r.next(damage / 2 + 1);
        added.push((K::Damage.id() as i32, Component::Damage(d.min(damage))));
    }
    let e = r.next(6);
    let book = item == id("minecraft:enchanted_book");
    if e < 2 || book && e < 4 {
        let n = 1 + r.next(2);
        let mut map: Vec<(i32, i32)> = Vec::new();
        for _ in 0..n {
            let (which, level) = (r.next(golden::ENCHANTMENTS.len() as i32), 1 + r.next(3));
            match map.iter_mut().find(|(k, _)| *k == which) {
                Some(here) => here.1 = here.1.max(level),
                None => map.push((which, level)),
            }
        }
        map.sort_unstable();
        added.push(if book {
            (
                K::StoredEnchantments.id() as i32,
                Component::StoredEnchantments(map),
            )
        } else {
            (K::Enchantments.id() as i32, Component::Enchantments(map))
        });
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

fn enchantments(v: &[(i32, i32)]) -> String {
    let mut l: Vec<String> = v
        .iter()
        .map(|(e, lvl)| format!("{}:{lvl}", golden::ENCHANTMENTS[*e as usize]))
        .collect();
    l.sort();
    l.join(",")
}

/// The tool's `stack`: item, count, and the patch's entries by type id.
fn describe(s: &Stack) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let mut out = format!("{}*{}", s.item, s.count);
    let p = &s.patch;
    if p.added.is_empty() && p.removed.is_empty() {
        return out;
    }
    let mut entries: Vec<(i32, String)> = p.removed.iter().map(|k| (*k, "!".to_owned())).collect();
    for (k, c) in &p.added {
        let v = match c {
            Component::Damage(n) | Component::MaxDamage(n) => n.to_string(),
            Component::Enchantments(e) | Component::StoredEnchantments(e) => enchantments(e),
            Component::Other { data, .. } if *k == K::RepairCost.id() as i32 => {
                ruthenium_protocol::wire::Reader::new(data)
                    .varint()
                    .unwrap()
                    .to_string()
            }
            _ => "?".into(),
        };
        entries.push((*k, v));
    }
    entries.sort();
    out.push('{');
    for (k, v) in entries {
        out += &format!("{k}={v};");
    }
    out + "}"
}

fn contents(m: &Menu, inv: &Inventory) -> String {
    (0..m.slots.len())
        .filter(|&i| !m.item(inv, i).is_empty())
        .map(|i| format!("{i}={};", describe(m.item(inv, i))))
        .collect()
}

fn data(m: &Menu) -> String {
    if m.data.is_empty() {
        return String::new();
    }
    let mut s = "#".to_owned();
    for d in &m.data {
        s += &format!("{d},");
    }
    s
}

/// The click packet's changed slots and cursor, as the tool writes them.
fn packet(body: &[u8]) -> String {
    let mut r = ruthenium_protocol::wire::Reader::new(body);
    let stack = |r: &mut ruthenium_protocol::wire::Reader<'_>| {
        if !r.bool().unwrap() {
            return "-".to_owned();
        }
        let (item, count) = (r.varint().unwrap(), r.varint().unwrap());
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
        added.sort_unstable();
        removed.sort_unstable();
        let list = |v: &[i32]| {
            let s: Vec<String> = v.iter().map(i32::to_string).collect();
            format!("[{}]", s.join(", "))
        };
        format!("{item}*{count}+{}-{}", list(&added), list(&removed))
    };
    r.varint().unwrap();
    let mut out = r.varint().unwrap().to_string();
    r.bytes(3).unwrap();
    r.varint().unwrap();
    for _ in 0..r.varint().unwrap() {
        let slot = i16::from_be_bytes(r.bytes(2).unwrap().try_into().unwrap());
        out += &format!(";{slot}={}", stack(&mut r));
    }
    out += &format!(";c={}", stack(&mut r));
    out
}

const KINDS: [&str; 6] = ["beacon", "brewing", "enchant", "grind", "lectern", "mount"];

fn run_case(c: u64, items: &Arc<Items>, recipes: &RecipeSets) -> String {
    let mut r = Lcg(0x5EED_1000 + c);
    let kind = r.next(6);
    let mount = r.next(golden::MOUNTS.len() as i32) as usize;
    let mut columns = 0;
    if mount == 1 || mount == 2 {
        columns = if r.next(2) == 0 { 0 } else { 5 };
    }
    if mount == 3 || mount == 4 {
        columns = if r.next(2) == 0 { 0 } else { 1 + r.next(5) };
    }
    let state = MountState {
        tamed: r.next(3) != 0,
        baby: r.next(5) == 0,
        alive: r.next(10) != 0,
    };
    let creative = r.next(5) == 0;
    let k = match kind {
        0 => Kind::Beacon,
        1 => Kind::BrewingStand,
        2 => Kind::Enchantment,
        3 => Kind::Grindstone,
        4 => Kind::Lectern,
        _ => {
            let t = golden::MOUNTS[mount];
            let name = EntityType::from_id(t as u32).unwrap().name();
            Kind::Mount(Mount::new(name, t, columns, state, &items.tags).unwrap())
        }
    };
    let mut m = Menu::new(k, 1);
    m.items = super::MenuItems(Some(items.clone()));
    let mut inv = Inventory::default();
    let who = Who {
        items,
        recipes,
        creative,
    };
    let n = m.slots.len() as i32;
    let mut out = KINDS[kind as usize].to_owned();
    if kind == 5 {
        out += &format!(
            " {mount}/{columns}/{}{}{}",
            state.tamed, state.baby, state.alive
        );
    }
    out += if creative { " creative\n" } else { "\n" };
    for i in 0..m.slots.len() {
        if r.next(3) == 0 {
            let s = random_stack(&mut r, items);
            m.set(&mut inv, i, s);
        }
    }
    for i in 0..m.data.len() {
        if r.next(2) == 0 {
            let v = r.next(40) - 1;
            m.set_data(i as i32, v);
        }
    }
    inv.selected = r.next(9);
    if r.next(3) == 0 {
        m.carried = random_stack(&mut r, items);
    }
    let all: Vec<Stack> = (0..m.slots.len()).map(|i| m.item(&inv, i).copy()).collect();
    let carried = m.carried.clone();
    let state_id = r.next(100);
    m.initialize(&mut inv, state_id, all, carried);
    out += &format!(
        "selected {} state {}\n{}{}\n",
        inv.selected,
        m.state_id,
        contents(&m, &inv),
        data(&m)
    );
    let mut clicks = Vec::new();
    for _ in 0..14 {
        let input = r.next(7);
        if input == 5 && r.next(10) < 7 {
            let t = r.next(3);
            clicks.push((-999, quickcraft_mask(0, t), 5));
            for _ in 0..1 + r.next(4) {
                clicks.push((r.next(n), quickcraft_mask(1, t), 5));
            }
            clicks.push((-999, quickcraft_mask(2, t), 5));
            continue;
        }
        let (slot, button) = match input {
            2 => {
                let slot = r.next(n);
                let b = r.next(10);
                (slot, if b == 9 { 40 } else { b })
            }
            5 => {
                let h = r.next(3);
                let t = r.next(3);
                let slot = if h == 1 { r.next(n) } else { -999 };
                (slot, quickcraft_mask(h, t))
            }
            _ => {
                let x = r.next(20);
                let slot = if x == 0 {
                    -999
                } else if x == 1 && input == 0 {
                    -1
                } else {
                    r.next(n)
                };
                let button = match input {
                    0 | 1 => [0, 0, 1, 1, 2][r.next(5) as usize],
                    3 => r.next(3),
                    _ => r.next(2),
                };
                (slot, button)
            }
        };
        clicks.push((slot, button, input));
    }
    for (slot, button, input) in clicks {
        let i = Input::from_id(input).unwrap();
        let (bytes, outcome) = m
            .handle_container_input(&mut inv, &who, 1, slot, button, i, &NoRegistries)
            .unwrap();
        out += &format!(
            "{input}:{slot}:{button}|{}{}|c={}|d={}|p={}\n",
            contents(&m, &inv),
            data(&m),
            describe(&m.carried),
            outcome.dropped.len(),
            packet(&bytes[1..])
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

#[test]
fn clicks_do_what_vanillas_station_and_mount_menus_do() {
    let (items, recipes) = (Arc::new(items()), recipes());
    for &(id, max, damage, equip, _, fuel, enchantable) in golden::POOL {
        let s = items.new_stack(id, 1);
        assert_eq!(s.max_stack_size(), max);
        assert_eq!(s.c.max_damage.unwrap_or(0), damage);
        assert_eq!(
            s.c.equippable.map_or(-1, |e| e.slot as i32),
            equip,
            "{}",
            s.c.name
        );
        assert_eq!((s.c.brewing_fuel, s.c.enchantable), (fuel, enchantable));
    }
    let mut wrong = Vec::new();
    for (c, &want) in golden::CASES.iter().enumerate() {
        let got = run_case(c as u64, &items, &recipes);
        if fnv(&got) != want {
            wrong.push(c);
            if wrong.len() == 1 {
                eprintln!("case {c} differs (the tool prints vanilla's with its number):\n{got}");
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ: {:?}",
        wrong.len(),
        golden::CASES.len(),
        &wrong[..wrong.len().min(20)]
    );
}

/// The tool's `grindStack`.
fn grind_stack(r: &mut Lcg, items: &Items, pool: &[&str], same: Option<i32>) -> Stack {
    let item = match same {
        Some(s) if r.next(2) == 0 => s,
        _ => id(pool[r.next(pool.len() as i32) as usize]),
    };
    let count = if r.next(8) == 0 { 2 } else { 1 };
    let plain = items.new_stack(item, 1);
    let mut added = Vec::new();
    if plain.c.damageable() && r.next(3) != 0 {
        let d = r.next(plain.c.max_damage.unwrap() + 1);
        if d != 0 {
            added.push((K::Damage.id() as i32, Component::Damage(d)));
        }
    }
    let mut map: Vec<(i32, i32)> = Vec::new();
    for _ in 0..r.next(4) {
        let (which, level) = (r.next(golden::ENCHANTMENTS.len() as i32), 1 + r.next(3));
        match map.iter_mut().find(|(k, _)| *k == which) {
            Some(here) => here.1 = here.1.max(level),
            None => map.push((which, level)),
        }
    }
    if !map.is_empty() {
        map.sort_unstable();
        added.push(if item == id("minecraft:enchanted_book") {
            (
                K::StoredEnchantments.id() as i32,
                Component::StoredEnchantments(map),
            )
        } else {
            (K::Enchantments.id() as i32, Component::Enchantments(map))
        });
    }
    if r.next(10) == 0 {
        let cost = r.next(5);
        if cost != 0 {
            let mut data = Vec::new();
            ruthenium_protocol::wire::write_varint(&mut data, cost);
            added.push((
                K::RepairCost.id() as i32,
                Component::Other {
                    kind: K::RepairCost.id() as i32,
                    data,
                },
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

#[test]
fn a_grindstone_makes_what_vanillas_makes() {
    let items = Arc::new(items());
    let pool = [
        "minecraft:diamond_sword",
        "minecraft:iron_pickaxe",
        "minecraft:bow",
        "minecraft:enchanted_book",
        "minecraft:book",
        "minecraft:diamond_helmet",
        "minecraft:stone",
        "minecraft:shield",
        "minecraft:wolf_armor",
    ];
    let mut wrong = Vec::new();
    for (c, &want) in golden::GRIND_CASES.iter().enumerate() {
        let mut r = Lcg(0x6121_D000 + c as u64);
        let a = if r.next(6) == 0 {
            Stack::empty()
        } else {
            grind_stack(&mut r, &items, &pool, None)
        };
        let same = (!a.is_empty()).then(|| a.item());
        let b = if r.next(4) == 0 {
            Stack::empty()
        } else {
            grind_stack(&mut r, &items, &pool, same)
        };
        // Through the slots, as the tool's are set (the container keeps one of a tool).
        let mut m = Menu::new(Kind::Grindstone, 1);
        m.items = super::MenuItems(Some(items.clone()));
        let mut inv = Inventory::default();
        m.set(&mut inv, 0, a);
        m.set(&mut inv, 1, b);
        let got = format!(
            "{} + {} = {}",
            describe(&m.own[0]),
            describe(&m.own[1]),
            describe(&m.own[2])
        );
        if fnv(&got) != want {
            if wrong.is_empty() {
                eprintln!(
                    "grindstone case {c} differs (the tool prints vanilla's with g{c}):\n{got}"
                );
            }
            wrong.push(c);
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ: {:?}",
        wrong.len(),
        golden::GRIND_CASES.len(),
        &wrong[..wrong.len().min(20)]
    );
}
