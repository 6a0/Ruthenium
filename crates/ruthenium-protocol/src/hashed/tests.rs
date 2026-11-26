use std::collections::HashMap;

use super::ops;
use super::*;
use crate::item::{ComponentPatch, Holder, RawValues};

/// A stack as vanilla's classes encoded it, and what the client's hasher gave.
pub(super) struct Case {
    pub name: &'static str,
    pub bytes: &'static [u8],
    pub expect: Expect,
}

pub(super) enum Expect {
    Empty,
    /// The hasher threw ("Failed to hash").
    Error,
    Stack {
        item: i32,
        count: i32,
        added: &'static [(i32, i32)],
        removed: &'static [i32],
    },
}

fn registries() -> HashMap<String, Vec<String>> {
    golden::REGISTRIES
        .iter()
        .map(|(name, keys)| {
            (
                name.to_string(),
                keys.iter().map(|k| k.to_string()).collect(),
            )
        })
        .collect()
}

fn read(c: &Case) -> ItemStack {
    let mut r = Reader::new(c.bytes);
    let s = ItemStack::read_optional(&mut r).unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
    r.finish().unwrap_or_else(|e| panic!("{}: {e:?}", c.name));
    s
}

/// What a case's hashes come to: `Ok(None)` empty, sorted for comparison.
fn outcome(
    s: &ItemStack,
    regs: &HashMap<String, Vec<String>>,
) -> Result<Option<HashedStack>, HashError> {
    try_hash_stack(s, regs).map(|h| {
        h.map(|mut h| {
            h.added.sort_unstable();
            h.removed.sort_unstable();
            h
        })
    })
}

fn matches(expect: &Expect, got: &Result<Option<HashedStack>, HashError>) -> bool {
    match (expect, got) {
        (Expect::Empty, Ok(None)) => true,
        (Expect::Error, Err(_)) => true,
        (
            Expect::Stack {
                item,
                count,
                added,
                removed,
            },
            Ok(Some(h)),
        ) => h.item == *item && h.count == *count && h.added == *added && h.removed == *removed,
        _ => false,
    }
}

#[test]
fn crc32c_and_hash_ops_primitives_match_the_jvm() {
    let all: Vec<u8> = (0..=255).collect();
    let mut seen = 0;
    for &(what, want) in golden::PRIMITIVES {
        let got = match what {
            "crc32c" => ops::crc32c(&[]),
            "crc32c a" => ops::crc32c(b"a"),
            "crc32c 123456789" => ops::crc32c(b"123456789"),
            "crc32c 0..256" => ops::crc32c(&all),
            "empty" => ops::empty(),
            "empty map" => ops::empty_map(),
            "empty list" => ops::empty_list(),
            "true" => ops::boolean(true),
            "false" => ops::boolean(false),
            "byte -1" => ops::byte(-1),
            "byte 0" => ops::byte(0),
            "byte 127" => ops::byte(127),
            "short -2" => ops::short(-2),
            "short 30000" => ops::short(30000),
            "int 0" => ops::int(0),
            "int -1" => ops::int(-1),
            "int 123456789" => ops::int(123_456_789),
            "int min" => ops::int(i32::MIN),
            "long min" => ops::long(i64::MIN),
            "long 42" => ops::long(42),
            "float 1.5" => ops::float(1.5),
            "float -0" => ops::float(-0.0),
            "float nan" => ops::float(f32::from_bits(0x7fc0_0000)),
            "float max" => ops::float(f32::MAX),
            "double -0" => ops::double(-0.0),
            "double 1e300" => ops::double(1e300),
            "double nan" => ops::double(f64::from_bits(0x7ff8_0000_0000_0000)),
            "string empty" => ops::string(""),
            "string h-e-acute" => ops::string("h\u{e9}"),
            "string smile" => ops::string("\u{1f600}"),
            "string minecraft:stone" => ops::string("minecraft:stone"),
            "map a=1 b=2" => {
                let mut r = Rec::new();
                // In the other order: a map's hash doesn't depend on it.
                r.put("b", ops::int(2)).put("a", ops::int(1));
                r.build()
            }
            "list 1 2 3" => ops::list([ops::int(1), ops::int(2), ops::int(3)]),
            "byte list 1 2 255" => ops::byte_list(&[1, 2, 255]),
            "int list 1 -1" => ops::int_list([1, -1]),
            "long list 1 -1" => ops::long_list([1, -1]),
            other => panic!("no Rust side for {other:?}"),
        };
        assert_eq!(got.as_int(), want, "{what}");
        seen += 1;
    }
    assert_eq!(seen, golden::PRIMITIVES.len());
    // The usual check value of CRC-32C.
    assert_eq!(ops::crc32c(b"123456789").0, 0xE306_9283);
}

#[test]
fn stacks_hash_as_the_26_3_client_hashes_them() {
    let regs = registries();
    let mut wrong = Vec::new();
    for c in golden::CASES {
        let got = outcome(&read(c), &regs);
        if !matches(&c.expect, &got) {
            wrong.push(format!("{}: {got:?}", c.name));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ:\n{}",
        wrong.len(),
        golden::CASES.len(),
        wrong.join("\n")
    );
}

/// Drops the bytes the typed components were read from, here and in nested stacks.
fn without_raw(s: &ItemStack) -> ItemStack {
    let mut s = s.clone();
    s.patch.raw = RawValues::default();
    for (_, c) in &mut s.patch.added {
        match c {
            Component::BundleContents(v)
            | Component::ChargedProjectiles(v)
            | Component::Container(v) => {
                for i in v.iter_mut() {
                    *i = without_raw(i);
                }
            }
            Component::UseRemainder(b) => **b = without_raw(b),
            _ => {}
        }
    }
    s
}

/// Whether a typed value keeps everything its component's codec writes.
fn lossless(c: &Component) -> bool {
    match c {
        Component::Consumable { .. }
        | Component::Equippable { .. }
        | Component::DeathProtection
        | Component::BlocksAttacks { .. }
        | Component::PiercingWeapon { .. }
        | Component::KineticWeapon(_)
        | Component::PotionContents { .. }
        | Component::Trim { .. }
        | Component::BannerPatterns(_)
        | Component::Instrument(_)
        | Component::CanBreak(_)
        | Component::CanPlaceOn(_) => false,
        Component::BundleContents(v)
        | Component::ChargedProjectiles(v)
        | Component::Container(v) => v
            .iter()
            .all(|s| s.patch.added.iter().all(|(_, c)| lossless(c))),
        Component::UseRemainder(s) => s.patch.added.iter().all(|(_, c)| lossless(c)),
        _ => true,
    }
}

#[test]
fn typed_values_written_out_again_hash_the_same() {
    // A stack built or changed by the client has no bytes to read back: its typed
    // values are written as the server would send them. Where they keep everything,
    // the hash can't tell.
    let regs = registries();
    let mut checked = 0;
    let mut wrong = Vec::new();
    for c in golden::CASES {
        let s = read(c);
        if !s.patch.added.iter().all(|(_, c)| lossless(c)) {
            continue;
        }
        let got = outcome(&without_raw(&s), &regs);
        if !matches(&c.expect, &got) {
            wrong.push(format!("{}: {got:?}", c.name));
        }
        checked += 1;
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked > 150, "{checked}");
}

#[test]
fn a_changed_value_is_hashed_from_what_it_is_now() {
    // A bundle read from the server, then given one more stone by a click: its old
    // bytes no longer read as its value, so the new value is written out.
    let regs = registries();
    let stone = ruthenium_data::v26_3::registries::Item::Stone.id() as i32;
    let bundle = golden::CASES.iter().find(|c| c.name == "bundle").unwrap();
    let mut s = read(bundle);
    let kind = Kind::BundleContents.id() as i32;
    assert!(s.patch.raw.get(kind).is_some());
    let one = ItemStack {
        item: stone,
        count: 1,
        patch: ComponentPatch::EMPTY,
    };
    s.patch.added = vec![(kind, Component::BundleContents(vec![one]))];
    let h = try_hash_stack(&s, &regs).unwrap().unwrap();
    // [{id: "minecraft:stone"}]
    let mut t = Rec::new();
    t.put("id", ops::string("minecraft:stone"));
    assert_eq!(h.added, [(kind, ops::list([t.build()]).as_int())]);
}

#[test]
fn transient_components_fail_at_the_top_and_are_left_out_inside() {
    let regs = registries();
    let lock = Kind::CreativeSlotLock.id() as i32;
    let stick = ruthenium_data::v26_3::registries::Item::Stick.id() as i32;
    let locked = ItemStack {
        item: stick,
        count: 1,
        patch: ComponentPatch {
            added: vec![(
                lock,
                Component::Other {
                    kind: lock,
                    data: Vec::new(),
                },
            )],
            ..Default::default()
        },
    };
    assert_eq!(
        try_hash_stack(&locked, &regs),
        Err(HashError::NotPersistent("minecraft:creative_slot_lock"))
    );
    // `hash_stack` sends it with the hash 0, for the server to correct.
    assert_eq!(hash_stack(&locked, &regs).unwrap().added, [(lock, 0)]);
    // In a bundle: `{id, components: {}}`.
    let kind = Kind::BundleContents.id() as i32;
    let bundle = ItemStack {
        item: stick,
        count: 1,
        patch: ComponentPatch {
            added: vec![(kind, Component::BundleContents(vec![locked]))],
            ..Default::default()
        },
    };
    let mut t = Rec::new();
    t.put("id", ops::string("minecraft:stick"))
        .put("components", ops::empty_map());
    let h = try_hash_stack(&bundle, &regs).unwrap().unwrap();
    assert_eq!(h.added, [(kind, ops::list([t.build()]).as_int())]);
}

#[test]
fn the_empty_stack_hashes_to_nothing() {
    let regs = registries();
    assert_eq!(hash_stack(&ItemStack::EMPTY, &regs), None);
    let mut out = Vec::new();
    HashedStack::write(None, &mut out);
    assert_eq!(out, [0]);
}

#[test]
fn hashed_stacks_are_written_as_the_server_reads_them() {
    let h = HashedStack {
        item: 5,
        count: 64,
        added: vec![(3, -2), (300, 7)],
        removed: vec![4],
    };
    let mut out = Vec::new();
    HashedStack::write(Some(&h), &mut out);
    // Present, item, count, two added (VarInt type, big-endian int), one removed.
    assert_eq!(
        out,
        [
            1, 5, 64, 2, 3, 0xFF, 0xFF, 0xFF, 0xFE, 0xAC, 0x02, 0, 0, 0, 7, 1, 4
        ]
    );
}

#[test]
fn ids_out_of_range_read_as_vanilla_reads_them() {
    assert_eq!(enums::RARITY.name(9), "common");
    assert_eq!(enums::SALMON_SIZE.name(-1), "small");
    assert_eq!(enums::SALMON_SIZE.name(7), "large");
    assert_eq!(enums::HORSE_VARIANT.name(-1), "dark_brown");
    assert_eq!(enums::HORSE_VARIANT.name(7), "white");
    assert_eq!(enums::RABBIT_VARIANT.name(99), "evil");
    assert_eq!(enums::RABBIT_VARIANT.name(98), "brown");
}

#[test]
fn typed_values_read_back_from_what_is_written() {
    // Every typed value in the golden stacks, nested ones too, written as the server
    // would send it, reads back as itself.
    fn walk(s: &ItemStack, n: &mut usize) {
        for (kind, c) in &s.patch.added {
            if matches!(c, Component::Other { .. }) {
                continue;
            }
            let k = super::kind(*kind).unwrap();
            let bytes = match crate::item::write_value(c) {
                Ok(b) => b,
                // What a typed value can't say: an adventure predicate's NBT or
                // component conditions, an instrument written inline.
                Err(_) => {
                    assert!(
                        matches!(
                            c,
                            Component::CanBreak(_)
                                | Component::CanPlaceOn(_)
                                | Component::Instrument(Holder::Inline)
                        ),
                        "{k:?} {c:?}"
                    );
                    continue;
                }
            };
            assert!(crate::item::reads_as(k, &bytes, c), "{k:?} {c:?}");
            *n += 1;
            match c {
                Component::BundleContents(v)
                | Component::ChargedProjectiles(v)
                | Component::Container(v) => v.iter().for_each(|s| walk(s, n)),
                Component::UseRemainder(s) => walk(s, n),
                _ => {}
            }
        }
    }
    let mut n = 0;
    for c in golden::CASES {
        walk(&read(c), &mut n);
    }
    assert!(n > 80, "{n}");
}

#[test]
fn a_text_list_is_its_first_element_with_the_rest_appended() {
    // `["a", {text: "b", bold: true}, "c"]` decodes as `a` with `b` and `c` as siblings
    // (vanilla's own server never sends the list form, so the golden stacks can't
    // show it).
    use crate::nbt::{Compound, Tag};
    let regs = registries();
    let cx = Cx {
        regs: &regs,
        depth: 0,
    };
    let mut b = Compound::new();
    b.put("text", Tag::String("b".into()));
    b.put("bold", Tag::Byte(1));
    let list = Tag::List(vec![
        Tag::String("a".into()),
        Tag::Compound(b.clone()),
        Tag::String("c".into()),
    ]);
    let mut whole = Compound::new();
    whole.put("text", Tag::String("a".into()));
    whole.put(
        "extra",
        Tag::List(vec![Tag::Compound(b), Tag::String("c".into())]),
    );
    let (l, w) = (
        text::hash(&list, &cx).unwrap(),
        text::hash(&Tag::Compound(whole), &cx).unwrap(),
    );
    assert_eq!(l, w);
    // And not in any other order.
    let mut back = Compound::new();
    back.put("text", Tag::String("a".into()));
    back.put(
        "extra",
        Tag::List(vec![Tag::String("c".into()), Tag::String("b".into())]),
    );
    assert_ne!(l, text::hash(&Tag::Compound(back), &cx).unwrap());
    // A list of one is that one; an empty list is no text.
    assert_eq!(
        text::hash(&Tag::List(vec![Tag::String("a".into())]), &cx).unwrap(),
        ops::string("a")
    );
    assert!(text::hash(&Tag::List(Vec::new()), &cx).is_err());
}
