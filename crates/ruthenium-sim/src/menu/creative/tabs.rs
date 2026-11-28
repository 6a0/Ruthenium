//! The creative inventory's tabs (`CreativeModeTabs`, `CreativeModeTab`): their place,
//! look and title, and what they hold as `buildAllTabContents` builds it for the
//! server's enabled features, the player's permission and the server's registries.
//! What the generators give that doesn't come from the registries is the export's
//! (`creative_tabs`); the enchanted books, the paintings, the goat horns and the
//! ominous banner's patterns are made here from the registries the server sent, with
//! vanilla's own entries for those it sent without data.

use std::sync::Arc;

use ruthenium_data::v26_3::registries::{DataComponentType as Kind, Item};
use ruthenium_prepare::exporters::{CreativeEntryData, CreativeTabsData};
use ruthenium_protocol::item::{Component, ComponentPatch, ItemStack, TrimPart};
use ruthenium_protocol::nbt::{self, Tag};
use ruthenium_protocol::packets::configuration::{RegistryEntry, RegistryTags};
use ruthenium_protocol::wire::{self, Reader};

use crate::item::{Items, Stack};
use crate::item_name::{styled_name, translation};

/// `CreativeModeTab.Row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Top,
    Bottom,
}

/// `CreativeModeTab.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabType {
    Category,
    Inventory,
    Hotbar,
    Search,
}

/// A stack a tab shows, with the tooltip lines the name search reads of it
/// (`getTooltipLines` with the creative flag, before the language turns them to text).
#[derive(Debug, Clone)]
pub struct CreativeItem {
    pub stack: Stack,
    pub lines: Arc<[Tag]>,
}

/// `CreativeModeTab`.
#[derive(Debug, Clone)]
pub struct Tab {
    /// e.g. `minecraft:building_blocks`.
    pub key: String,
    pub row: Row,
    pub column: i32,
    pub kind: TabType,
    pub aligned_right: bool,
    pub show_title: bool,
    pub can_scroll: bool,
    pub title: Arc<Tag>,
    /// `getBackgroundTexture`'s path in the pack (`textures/gui/container/...`).
    pub background: String,
    pub icon: Stack,
    /// `getDisplayItems`.
    pub display: Vec<CreativeItem>,
    /// `getSearchTabDisplayItems`.
    pub search_display: Vec<CreativeItem>,
}

impl Tab {
    /// `shouldDisplay`.
    pub fn should_display(&self) -> bool {
        self.kind != TabType::Category || !self.display.is_empty()
    }

    /// `contains`: the stack is among those it gives the search tab.
    pub fn contains(&self, stack: &Stack) -> bool {
        self.search_display
            .iter()
            .any(|i| i.stack.same_item_same_components(stack))
    }
}

/// What the tabs are built for (`ItemDisplayParameters`).
#[derive(Debug)]
pub struct Parameters<'a> {
    pub data: &'a CreativeTabsData,
    pub items: &'a Items,
    pub registries: &'a [(String, Vec<RegistryEntry>)],
    pub tags: &'a [RegistryTags],
    pub features: &'a [String],
    pub has_permissions: bool,
}

impl Parameters<'_> {
    fn registry(&self, name: &str) -> &[RegistryEntry] {
        self.registries
            .iter()
            .find(|(r, _)| r == name)
            .map_or(&[][..], |(_, e)| e.as_slice())
    }

    /// The members of a tag of a registry, by network id, in the tag's order.
    fn tag(&self, registry: &str, tag: &str) -> Option<&[i32]> {
        let r = self.tags.iter().find(|t| t.registry == registry)?;
        r.tags
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, ids)| ids.as_slice())
    }

    /// `FeatureFlagSet.isSubsetOf` of the enabled features.
    fn enabled(&self, needs: &[String]) -> bool {
        needs.iter().all(|f| self.features.iter().any(|e| e == f))
    }

    fn item_enabled(&self, item: i32) -> bool {
        let needs = usize::try_from(item)
            .ok()
            .and_then(|i| self.data.item_features.get(i));
        needs.is_none_or(|n| self.enabled(n))
    }
}

fn read_tag(bytes: &[u8]) -> Tag {
    nbt::read_network(bytes)
        .ok()
        .and_then(|(t, _)| t)
        .unwrap_or_else(|| Tag::String(String::new()))
}

/// A patch as the server sends one (read as a stack's, behind a count and an item).
fn read_patch(bytes: &[u8]) -> ComponentPatch {
    let mut b = vec![1, 1];
    b.extend_from_slice(bytes);
    ItemStack::read_optional(&mut Reader::new(&b))
        .map_or_else(|_| ComponentPatch::default(), |s| s.patch)
}

/// A patch of one component, as the server would send it.
fn one_component(kind: Kind, value: &[u8]) -> ComponentPatch {
    let mut b = Vec::new();
    wire::write_varint(&mut b, 1);
    wire::write_varint(&mut b, 0);
    wire::write_varint(&mut b, kind.id() as i32);
    b.extend_from_slice(value);
    read_patch(&b)
}

fn stack(items: &Items, item: i32, patch: ComponentPatch) -> Stack {
    items.stack(&ItemStack {
        item,
        count: 1,
        patch,
    })
}

/// `CreativeModeTab.TabVisibility`, by its ordinal.
const BOTH: u8 = 0;
const PARENT_ONLY: u8 = 1;
const SEARCH_ONLY: u8 = 2;

/// `ItemDisplayBuilder`: the tab's and the search tab's lists, each without a second
/// stack of the same item and components (`ItemStackLinkedSet`), and only of enabled
/// items.
struct Output<'p, 'a> {
    p: &'p Parameters<'a>,
    tab: Vec<CreativeItem>,
    search: Vec<CreativeItem>,
}

impl Output<'_, '_> {
    fn accept(&mut self, item: CreativeItem, visibility: u8) {
        if !self.p.item_enabled(item.stack.item) {
            return;
        }
        let add = |list: &mut Vec<CreativeItem>, item: &CreativeItem| {
            if !list
                .iter()
                .any(|i| i.stack.same_item_same_components(&item.stack))
            {
                list.push(item.clone());
            }
        };
        match visibility {
            PARENT_ONLY => add(&mut self.tab, &item),
            SEARCH_ONLY => add(&mut self.search, &item),
            _ => {
                add(&mut self.tab, &item);
                add(&mut self.search, &item);
            }
        }
    }
}

/// An enchantment as the books need it: its id, highest level and description.
struct EnchantmentInfo {
    id: i32,
    max_level: i32,
    description: Tag,
    curse: bool,
}

fn enchantments(p: &Parameters<'_>) -> Vec<EnchantmentInfo> {
    let curses = p
        .tag("minecraft:enchantment", "minecraft:curse")
        .unwrap_or(&[]);
    p.registry("minecraft:enchantment")
        .iter()
        .enumerate()
        .filter_map(|(id, e)| {
            let own = e.data.as_ref().and_then(|d| d.as_compound());
            let (max_level, description) = match own {
                Some(c) => (c.int("max_level")?, c.get("description")?.clone()),
                None => {
                    let (_, max, d) = p.data.enchantments.iter().find(|(k, ..)| *k == e.id)?;
                    (*max as i32, read_tag(d))
                }
            };
            let id = id as i32;
            Some(EnchantmentInfo {
                id,
                max_level,
                description,
                curse: curses.contains(&id),
            })
        })
        .collect()
}

/// `EnchantmentHelper.createBook` with `Enchantment.getFullname`'s line.
fn book(p: &Parameters<'_>, e: &EnchantmentInfo, level: i32) -> CreativeItem {
    let mut v = Vec::new();
    wire::write_varint(&mut v, 1);
    wire::write_varint(&mut v, e.id);
    wire::write_varint(&mut v, level);
    let s = stack(
        p.items,
        Item::EnchantedBook as i32,
        one_component(Kind::StoredEnchantments, &v),
    );
    let mut name = ruthenium_protocol::nbt::Compound::new();
    let colour = if e.curse { "red" } else { "gray" };
    name.put("color", Tag::String(colour.into()));
    let mut extra = vec![e.description.clone()];
    if level != 1 || e.max_level != 1 {
        extra.push(Tag::String(" ".into()));
        extra.push(translation(
            &format!("enchantment.level.{level}"),
            Vec::new(),
        ));
    }
    name.put("text", Tag::String(String::new()));
    name.put("extra", Tag::List(extra));
    CreativeItem {
        lines: Arc::from(vec![styled_name(&s), Tag::Compound(name)]),
        stack: s,
    }
}

/// The painting variants of `#placeable` (or the others), sorted by area then width
/// (`PAINTING_COMPARATOR`, a stable sort), each with `HangingEntityItem`'s lines.
fn paintings(p: &Parameters<'_>, placeable: bool) -> Vec<CreativeItem> {
    let tagged = p
        .tag("minecraft:painting_variant", "minecraft:placeable")
        .unwrap_or(&[]);
    let mut list: Vec<(i32, i32, i32, Vec<Tag>)> = p
        .registry("minecraft:painting_variant")
        .iter()
        .enumerate()
        .filter(|(id, _)| tagged.contains(&(*id as i32)) == placeable)
        .filter_map(|(id, e)| {
            let own = e.data.as_ref().and_then(|d| d.as_compound());
            let (w, h, title, author) = match own {
                Some(c) => (
                    c.int("width")?,
                    c.int("height")?,
                    c.get("title").cloned(),
                    c.get("author").cloned(),
                ),
                None => {
                    let v = p.data.paintings.iter().find(|v| v.key == e.id)?;
                    (
                        i32::from(v.width),
                        i32::from(v.height),
                        v.title.as_deref().map(read_tag),
                        v.author.as_deref().map(read_tag),
                    )
                }
            };
            let mut lines: Vec<Tag> = title.into_iter().chain(author).collect();
            lines.push(translation(
                "painting.dimensions",
                vec![Tag::Int(w), Tag::Int(h)],
            ));
            Some((id as i32, w, h, lines))
        })
        .collect();
    list.sort_by_key(|&(_, w, h, _)| (w * h, w));
    list.into_iter()
        .map(|(id, _, _, extra)| {
            let mut v = Vec::new();
            wire::write_varint(&mut v, id + 1);
            let s = stack(
                p.items,
                Item::Painting as i32,
                one_component(Kind::PaintingVariant, &v),
            );
            let mut lines = vec![styled_name(&s)];
            lines.extend(extra);
            CreativeItem {
                lines: Arc::from(lines),
                stack: s,
            }
        })
        .collect()
}

/// `generateInstrumentTypes`: the tag's instruments on the item, in the tag's order.
fn instruments(p: &Parameters<'_>, item: u32, tag: &str) -> Vec<CreativeItem> {
    let Some(ids) = p.tag("minecraft:instrument", tag) else {
        return Vec::new();
    };
    let registry = p.registry("minecraft:instrument");
    ids.iter()
        .filter_map(|&id| {
            let e = registry.get(usize::try_from(id).ok()?)?;
            let description = match e.data.as_ref().and_then(|d| d.as_compound()) {
                Some(c) => c.get("description").cloned(),
                None => p
                    .data
                    .instruments
                    .iter()
                    .find(|(k, _)| *k == e.id)
                    .map(|(_, d)| read_tag(d)),
            };
            // (`set` of the item's default leaves the patch empty: ponder's horn.)
            let default = &p.items.defaults(item as i32).instrument;
            let patch = if matches!(default, Some(crate::item::InstrumentRef::Name(n)) if *n == e.id) {
                ComponentPatch::default()
            } else {
                let mut v = Vec::new();
                wire::write_varint(&mut v, id + 1);
                one_component(Kind::Instrument, &v)
            };
            let s = stack(p.items, item as i32, patch);
            let mut lines = vec![styled_name(&s)];
            lines.extend(description);
            Some(CreativeItem {
                lines: Arc::from(lines),
                stack: s,
            })
        })
        .collect()
}

/// `Raid.getOminousBannerInstance`: the patterns the server's registry has
/// (`addIfRegistered`) put back in the patch where they were.
fn ominous_banner(
    p: &Parameters<'_>,
    item: u32,
    patch: &[u8],
    at: u8,
    lines: &[Vec<u8>],
    patterns: &[(String, u8)],
) -> CreativeItem {
    let registry = p.registry("minecraft:banner_pattern");
    let layers: Vec<(TrimPart, i32)> = patterns
        .iter()
        .filter_map(|(key, colour)| {
            let id = registry.iter().position(|e| e.id == *key)?;
            Some((TrimPart::Id(id as i32), i32::from(*colour)))
        })
        .collect();
    let mut patch = read_patch(patch);
    let at = usize::from(at).min(patch.added.len());
    patch.added.insert(
        at,
        (
            Kind::BannerPatterns.id() as i32,
            Component::BannerPatterns(layers),
        ),
    );
    CreativeItem {
        stack: stack(p.items, item as i32, patch),
        lines: lines.iter().map(|l| read_tag(l)).collect(),
    }
}

/// The creative inventory's tabs as built for some parameters.
#[derive(Debug, Clone, Default)]
pub struct Tabs {
    /// In registry order (`allTabs`).
    pub tabs: Vec<Tab>,
}

impl Tabs {
    /// `CreativeModeTabs.buildAllTabContents`.
    pub fn build(p: &Parameters<'_>) -> Tabs {
        let mut tabs: Vec<Tab> = p
            .data
            .tabs
            .iter()
            .map(|t| {
                let kind = match t.kind {
                    1 => TabType::Inventory,
                    2 => TabType::Hotbar,
                    3 => TabType::Search,
                    _ => TabType::Category,
                };
                let mut out = Output {
                    p,
                    tab: Vec::new(),
                    search: Vec::new(),
                };
                if kind == TabType::Category && (!t.operator || p.has_permissions) {
                    for e in &t.entries {
                        generate(p, e, &mut out);
                    }
                }
                let background = t
                    .background
                    .strip_prefix("minecraft:")
                    .unwrap_or(&t.background)
                    .to_owned();
                Tab {
                    key: t.key.clone(),
                    row: if t.row == 0 { Row::Top } else { Row::Bottom },
                    column: i32::from(t.column),
                    kind,
                    aligned_right: t.flags & 1 != 0,
                    show_title: t.flags & 2 != 0,
                    can_scroll: t.flags & 4 != 0,
                    title: Arc::new(read_tag(&t.title)),
                    background,
                    icon: p.items.new_stack(t.icon as i32, 1),
                    display: out.tab,
                    search_display: out.search,
                }
            })
            .collect();
        // The search tab: every other tab's search stacks, in registry order, once each.
        let mut all = Output {
            p,
            tab: Vec::new(),
            search: Vec::new(),
        };
        for t in tabs.iter().filter(|t| t.kind != TabType::Search) {
            for i in &t.search_display {
                all.accept(i.clone(), BOTH);
            }
        }
        if let Some(search) = tabs.iter_mut().find(|t| t.kind == TabType::Search) {
            search.display = all.tab;
            search.search_display = all.search;
        }
        Tabs { tabs }
    }

    /// `CreativeModeTabs.tabs`: those that show.
    pub fn shown(&self) -> impl Iterator<Item = (usize, &Tab)> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.should_display())
    }

    /// `getDefaultTab`: the building blocks.
    pub fn default_tab(&self) -> usize {
        self.tabs
            .iter()
            .position(|t| t.key == "minecraft:building_blocks")
            .unwrap_or(0)
    }

    /// `searchTab`.
    pub fn search_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.kind == TabType::Search)
    }
}

fn generate(p: &Parameters<'_>, e: &CreativeEntryData, out: &mut Output<'_, '_>) {
    match e {
        CreativeEntryData::Stack {
            item,
            patch,
            visibility,
            features,
            lines,
        } => {
            // `generatePotionEffectTypes`: a potion of a feature not enabled isn't made.
            if !p.enabled(features) {
                return;
            }
            let s = stack(p.items, *item as i32, read_patch(patch));
            let lines = lines.iter().map(|l| read_tag(l)).collect();
            out.accept(CreativeItem { stack: s, lines }, *visibility);
        }
        CreativeEntryData::EnchantedBooks => {
            let list = enchantments(p);
            for e in &list {
                out.accept(book(p, e, e.max_level), PARENT_ONLY);
            }
            for e in &list {
                for level in 1..=e.max_level {
                    out.accept(book(p, e, level), SEARCH_ONLY);
                }
            }
        }
        CreativeEntryData::Paintings { placeable } => {
            for i in paintings(p, *placeable) {
                out.accept(i, BOTH);
            }
        }
        CreativeEntryData::Instruments { item, tag } => {
            for i in instruments(p, *item, tag) {
                out.accept(i, BOTH);
            }
        }
        CreativeEntryData::OminousBanner {
            item,
            patch,
            at,
            visibility,
            lines,
            patterns,
        } => {
            let i = ominous_banner(p, *item, patch, *at, lines, patterns);
            out.accept(i, *visibility);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::creative::tabs_golden as golden;
    use ruthenium_assets::zip::Zip;

    fn fnv(s: &str) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    /// A tag of the jar's data pack, nested tags expanded, each member once.
    fn tag(jar: &Zip, registry: &str, name: &str, out: &mut Vec<String>) {
        let Some(i) = jar.find(&format!("data/minecraft/tags/{registry}/{name}.json")) else {
            return;
        };
        let v: serde_json::Value = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        for n in v["values"].as_array().unwrap() {
            let n = n.as_str().or_else(|| n["id"].as_str()).unwrap();
            match n.strip_prefix('#') {
                Some(inner) => tag(jar, registry, inner.trim_start_matches("minecraft:"), out),
                None if !out.iter().any(|o| o == n) => out.push(n.to_owned()),
                None => {}
            }
        }
    }

    /// TabsGolden's `describe`.
    fn describe(s: &Stack, regs: &[(String, Vec<RegistryEntry>)]) -> String {
        let key = |registry: &str, id: i32| -> String {
            regs.iter()
                .find(|(r, _)| r == registry)
                .and_then(|(_, e)| e.get(usize::try_from(id).ok()?))
                .map_or_else(|| format!("?{id}"), |e| e.id.clone())
        };
        let mut out = format!(
            "{}*{}",
            Item::from_id(s.item as u32).map_or("?", |i| i.name()),
            s.count
        );
        let mut b = Vec::new();
        s.patch.write_delimited(&mut b).unwrap();
        let mut r = Reader::new(&b);
        let added = r.varint().unwrap();
        let removed = r.varint().unwrap();
        for _ in 0..added {
            let kind = r.varint().unwrap();
            let len = r.varint().unwrap() as usize;
            let value = r.bytes(len).unwrap();
            let name = Kind::from_id(kind as u32).map_or("?", |k| k.name());
            out += &format!(" {name}=");
            let mut v = Reader::new(value);
            if kind == Kind::StoredEnchantments.id() as i32 {
                for _ in 0..v.varint().unwrap() {
                    let id = v.varint().unwrap();
                    let level = v.varint().unwrap();
                    out += &format!("{}:{level},", key("minecraft:enchantment", id));
                }
            } else if kind == Kind::PaintingVariant.id() as i32 {
                out += &key("minecraft:painting_variant", v.varint().unwrap() - 1);
            } else if kind == Kind::Instrument.id() as i32 {
                out += &key("minecraft:instrument", v.varint().unwrap() - 1);
            } else if kind == Kind::BannerPatterns.id() as i32 {
                for _ in 0..v.varint().unwrap() {
                    let id = v.varint().unwrap() - 1;
                    let colour = v.varint().unwrap();
                    out += &format!("{}/{colour},", key("minecraft:banner_pattern", id));
                }
            } else {
                out += &value.iter().map(|b| format!("{b:02x}")).collect::<String>();
            }
        }
        for _ in 0..removed {
            let kind = r.varint().unwrap();
            out += &format!(" !{}", Kind::from_id(kind as u32).map_or("?", |k| k.name()));
        }
        out
    }

    /// With the prepared game (`RUTHENIUM_GAME_DIR`): every tab's stacks, and its
    /// search tab stacks, as vanilla's own `CreativeModeTabs` builds them (TabsGolden),
    /// from the export and vanilla's registries as a vanilla server lists them.
    #[test]
    fn tabs_hold_what_vanillas_hold() {
        let Some(dir) = std::env::var_os("RUTHENIUM_GAME_DIR") else {
            eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let items = Items::load(
            &dir.join("versions/26.3/exports"),
            &dir.join("versions/26.3/generated/reports"),
        )
        .expect("items");
        let jar = Zip::open(&dir.join("versions/26.3/client.jar")).expect("jar");
        let data = items.creative.clone();
        let sorted = |mut keys: Vec<String>| {
            keys.sort();
            keys.into_iter()
                .map(|id| RegistryEntry { id, data: None })
                .collect::<Vec<_>>()
        };
        let banners: Vec<String> = jar
            .with_prefix("data/minecraft/banner_pattern/")
            .filter_map(|i| {
                let f = jar.name(i).strip_prefix("data/minecraft/banner_pattern/")?;
                Some(format!("minecraft:{}", f.strip_suffix(".json")?))
            })
            .collect();
        let regs = vec![
            (
                "minecraft:enchantment".to_owned(),
                sorted(data.enchantments.iter().map(|e| e.0.clone()).collect()),
            ),
            (
                "minecraft:painting_variant".to_owned(),
                sorted(data.paintings.iter().map(|p| p.key.clone()).collect()),
            ),
            (
                "minecraft:instrument".to_owned(),
                sorted(data.instruments.iter().map(|i| i.0.clone()).collect()),
            ),
            ("minecraft:banner_pattern".to_owned(), sorted(banners)),
        ];
        let tags: Vec<RegistryTags> = [
            ("minecraft:enchantment", "enchantment", "curse"),
            (
                "minecraft:painting_variant",
                "painting_variant",
                "placeable",
            ),
            ("minecraft:instrument", "instrument", "goat_horns"),
        ]
        .iter()
        .map(|&(registry, dir, name)| {
            let mut members = Vec::new();
            tag(&jar, dir, name, &mut members);
            let entries = &regs.iter().find(|(r, _)| r == registry).unwrap().1;
            let ids = members
                .iter()
                .filter_map(|m| entries.iter().position(|e| e.id == *m).map(|i| i as i32))
                .collect();
            RegistryTags {
                registry: registry.to_owned(),
                tags: vec![(format!("minecraft:{name}"), ids)],
            }
        })
        .collect();
        let features = vec!["minecraft:vanilla".to_owned()];
        let tabs = Tabs::build(&Parameters {
            data: &data,
            items: &items,
            registries: &regs,
            tags: &tags,
            features: &features,
            has_permissions: true,
        });
        assert_eq!(tabs.tabs.len(), golden::TABS.len());
        for (t, &(key, n, hash, sn, shash)) in tabs.tabs.iter().zip(golden::TABS) {
            assert_eq!(t.key, key);
            let list = |l: &[CreativeItem]| -> String {
                l.iter().map(|i| describe(&i.stack, &regs) + "\n").collect()
            };
            let (d, s) = (list(&t.display), list(&t.search_display));
            if std::env::var_os("RUTHENIUM_TABS_DUMP")
                .is_some_and(|v| key.ends_with(v.to_str().unwrap()))
            {
                eprintln!("{d}-----\n{s}");
            }
            assert_eq!((t.display.len(), fnv(&d)), (n, hash), "{key}'s stacks");
            assert_eq!(
                (t.search_display.len(), fnv(&s)),
                (sn, shash),
                "{key}'s search stacks"
            );
        }
    }
}
