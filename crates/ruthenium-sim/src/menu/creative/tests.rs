use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as K, Item};
use ruthenium_prepare::exporters::ItemClassData;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::wire::{self, Reader};

use super::*;
use crate::item::RecipeSets;
use crate::menu::quickcraft_mask;

#[path = "golden.rs"]
mod golden;

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

fn items() -> Items {
    let mut classes = Vec::new();
    let mut reports = HashMap::new();
    for &(id, max, damage, equip, fuel) in golden::POOL {
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
        if damage > 0 {
            c.insert("minecraft:max_damage".into(), damage.into());
            c.insert("minecraft:damage".into(), 0.into());
        }
        if equip >= 0 {
            let slot = ["mainhand", "feet", "legs", "chest", "head", "offhand"][equip as usize];
            c.insert(
                "minecraft:equippable".into(),
                serde_json::json!({ "slot": slot }),
            );
        }
        if fuel {
            c.insert("minecraft:cooking_fuel".into(), serde_json::json!({}));
        }
        reports.insert(name.to_owned(), serde_json::json!({ "components": c }));
    }
    Items::new(&classes, &reports)
}

/// A stack as the server would send it: `components` as (type, value bytes).
fn stack(items: &Items, item: i32, count: i32, components: &[(K, &[u8])]) -> Stack {
    let mut b = Vec::new();
    wire::write_varint(&mut b, count);
    wire::write_varint(&mut b, item);
    wire::write_varint(&mut b, components.len() as i32);
    wire::write_varint(&mut b, 0);
    for (k, v) in components {
        wire::write_varint(&mut b, k.id() as i32);
        b.extend_from_slice(v);
    }
    items.stack(&ItemStack::read_optional(&mut Reader::new(&b)).unwrap())
}

/// CreativeGolden's `randomStack`.
fn random_stack(r: &mut Lcg, items: &Items, one: bool) -> Stack {
    let (id, max, damage, ..) = golden::POOL[r.next(golden::POOL.len() as i32) as usize];
    let count = 1 + r.next(max);
    let count = if one { 1 } else { count };
    if damage > 0 && r.next(2) == 0 {
        let mut v = Vec::new();
        wire::write_varint(&mut v, 1 + r.next(20));
        return stack(items, id, count, &[(K::Damage, &v)]);
    }
    stack(items, id, count, &[])
}

fn describe(s: &Stack) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let p = if s.patch.added.is_empty() && s.patch.removed.is_empty() {
        String::new()
    } else {
        format!("p{}", s.c.damage.unwrap_or(0))
    };
    format!("{}*{}{p}", s.item, s.count)
}

fn contents(m: &Menu, inv: &Inventory) -> String {
    (0..m.slots.len())
        .filter(|&i| !m.item(inv, i).is_empty())
        .map(|i| format!("{i}={};", describe(m.item(inv, i))))
        .collect()
}

fn hex(packets: &[Vec<u8>]) -> String {
    packets
        .iter()
        .map(|p| {
            let body: String = p[1..].iter().map(|b| format!("{b:02x}")).collect();
            body + ","
        })
        .collect()
}

fn tab(key: &str, kind: TabType) -> Tab {
    Tab {
        key: key.into(),
        row: Row::Top,
        column: 0,
        kind,
        aligned_right: false,
        show_title: true,
        can_scroll: true,
        title: Arc::new(Tag::String(String::new())),
        background: String::new(),
        icon: Stack::empty(),
        display: Vec::new(),
        search_display: Vec::new(),
    }
}

fn creative() -> Creative {
    let mut c = Creative {
        tabs: Arc::new(Tabs {
            tabs: vec![
                tab("minecraft:building_blocks", TabType::Category),
                tab("minecraft:inventory", TabType::Inventory),
            ],
        }),
        ..Creative::default()
    };
    c.new_player();
    c.set_selected("minecraft:building_blocks");
    c
}

static FEATURES: &[String] = &[];

fn run_case(case: u64, items: &Items) -> String {
    let mut r = Lcg(0xC0EA_0000 + case);
    let tab = r.next(2);
    let recipes = RecipeSets::new();
    let mut creative = creative();
    let mut menus = Menus::default();
    let mut inv = Inventory::default();
    let registries = HashMap::new();
    let lang: HashMap<String, String> = HashMap::new();
    let item_tags = ItemTags::new();
    let mut s = CreativeScreen::new(false);
    let mut screen = Screen::new(Kind::Creative, 400, 300);
    let mut c = Ctx {
        creative: &mut creative,
        menus: &mut menus,
        inv: &mut inv,
        who: Who {
            items,
            recipes: &recipes,
            creative: true,
        },
        infinite: true,
        features: FEATURES,
        registries: &registries,
        lang: &lang,
        item_tags: &item_tags,
        sent: Sent::default(),
    };
    if tab == 1 {
        select_tab(&mut s, &mut screen, &mut c, 1);
    } else {
        let n = r.next(130);
        for _ in 0..n {
            let st = if r.next(15) == 0 {
                stack(items, Item::Paper as i32, 1, &[(K::CreativeSlotLock, &[])])
            } else {
                random_stack(&mut r, items, true)
            };
            s.items.push(st);
        }
        s.scroll_to(r.next(5) as f32 * 0.25);
    }
    for i in 0..46 {
        if r.next(3) == 0 {
            let st = random_stack(&mut r, items, false);
            c.menus.inventory.set(c.inv, i, st);
        }
    }
    if r.next(3) == 0 {
        c.menus.inventory.carried = random_stack(&mut r, items, false);
    }
    s.sync(&c.menus.inventory);
    let mut out = format!("{tab}|{}\n", contents(s.menu(), c.inv));
    c.broadcast(true);
    out += &format!("init|{}\n", hex(&std::mem::take(&mut c.sent.packets)));
    let n = s.menu().slots.len() as i32;
    let mut clicks = Vec::new();
    for _ in 0..14 {
        let input = r.next(7);
        if input == 5 && r.next(10) < 7 {
            let t = r.next(3);
            clicks.push((-999, quickcraft_mask(0, t), 5));
            let count = 1 + r.next(4);
            for _ in 0..count {
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
                (if h == 1 { r.next(n) } else { -999 }, quickcraft_mask(h, t))
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
        s.has_clicked_outside = r.next(2) == 0;
        let at = usize::try_from(slot).ok();
        let i = Input::from_id(input).unwrap();
        slot_clicked(&mut s, &mut c, at, slot, button, i);
        let sent = std::mem::take(&mut c.sent);
        out += &format!(
            "{input}:{slot}:{button}|{}|c={}|d={}|p={}\n",
            contents(&c.menus.inventory, c.inv),
            describe(&c.menus.inventory.carried),
            sent.dropped,
            hex(&sent.packets)
        );
    }
    out
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// 2000 random tabs, inventories and click runs through vanilla's own
/// `slotClicked` (`research/tools`' CreativeGolden): every slot, the cursor, the drops
/// and every packet's bytes the same.
#[test]
fn clicks_do_what_vanillas_creative_inventory_does() {
    let items = items();
    let mut bad = Vec::new();
    for (case, &want) in golden::CASES.iter().enumerate() {
        let got = run_case(case as u64, &items);
        if fnv(&got) != want {
            bad.push(case);
            if bad.len() == 1 {
                eprintln!("case {case} differs; ours:\n{got}");
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} cases differ: {:?}",
        bad.len(),
        golden::CASES.len(),
        &bad[..bad.len().min(20)]
    );
}
