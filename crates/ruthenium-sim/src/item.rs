//! Items as the client's item use sees them: each stack's effective components (the
//! item's defaults, from the data generator's component reports, with the stack's patch
//! from the server applied), the item's class (from the exporters), and stacks that
//! keep Java's object identity where vanilla compares stacks with `==`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ruthenium_data::v26_3::registries::{DataComponentType as Kind, Item};
use ruthenium_prepare::exporters::{ItemClassData, read_item_classes};
use ruthenium_protocol::item::{Component, ComponentPatch, Holder, ItemStack};

/// `ItemUseAnimation` ids.
pub mod animation {
    pub const NONE: i32 = 0;
    pub const EAT: i32 = 1;
    pub const DRINK: i32 = 2;
    pub const BLOCK: i32 = 3;
    pub const BOW: i32 = 4;
    pub const TRIDENT: i32 = 5;
    pub const CROSSBOW: i32 = 6;
    pub const SPYGLASS: i32 = 7;
    pub const TOOT_HORN: i32 = 8;
    pub const BRUSH: i32 = 9;
    pub const BUNDLE: i32 = 10;
    pub const SPEAR: i32 = 11;

    /// The id of an animation's serialized name.
    pub fn from_name(name: &str) -> i32 {
        const NAMES: [&str; 12] = [
            "none",
            "eat",
            "drink",
            "block",
            "bow",
            "trident",
            "crossbow",
            "spyglass",
            "toot_horn",
            "brush",
            "bundle",
            "spear",
        ];
        NAMES.iter().position(|n| *n == name).unwrap_or(0) as i32
    }
}

/// `EquipmentSlot` by its id (`EquipmentSlot.id`, as the network sends it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipmentSlot {
    MainHand,
    Feet,
    Legs,
    Chest,
    Head,
    OffHand,
    Body,
    Saddle,
}

impl EquipmentSlot {
    pub fn from_id(id: i32) -> EquipmentSlot {
        match id {
            1 => EquipmentSlot::Feet,
            2 => EquipmentSlot::Legs,
            3 => EquipmentSlot::Chest,
            4 => EquipmentSlot::Head,
            5 => EquipmentSlot::OffHand,
            6 => EquipmentSlot::Body,
            7 => EquipmentSlot::Saddle,
            // `ByIdMap.OutOfBoundsStrategy.ZERO`.
            _ => EquipmentSlot::MainHand,
        }
    }

    pub fn from_name(name: &str) -> EquipmentSlot {
        match name {
            "feet" => EquipmentSlot::Feet,
            "legs" => EquipmentSlot::Legs,
            "chest" => EquipmentSlot::Chest,
            "head" => EquipmentSlot::Head,
            "offhand" => EquipmentSlot::OffHand,
            "body" => EquipmentSlot::Body,
            "saddle" => EquipmentSlot::Saddle,
            _ => EquipmentSlot::MainHand,
        }
    }
}

/// What an item's class does when used (`Item.use` and its overrides), as far as the
/// client predicts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Class {
    /// `Item.use`: consumables, equippables, blocking and kinetic weapons by their
    /// components.
    #[default]
    Plain,
    Bow,
    Crossbow,
    Trident,
    Spyglass,
    /// Goat horns.
    Instrument,
    Bundle,
    /// Snowballs, eggs, ender pearls, experience bottles, splash and lingering
    /// potions, wind charges: thrown and used up at once.
    Throwable,
    EnderEye,
    FishingRod,
    FireworkRocket,
    /// Written and writable books (they open a screen).
    Book,
    KnowledgeBook,
    EmptyMap,
    FoodOnAStick,
    Bucket,
    Boat,
    SpawnEgg,
    /// Glass bottles.
    Bottle,
    /// Lily pads and frogspawn.
    PlaceOnWater,
    /// A class that overrides `use` and isn't one of the above.
    Unknown,
}

impl Class {
    fn of(data: &ItemClassData) -> Class {
        const KNOWN: [(&str, Class); 23] = [
            ("BowItem", Class::Bow),
            ("CrossbowItem", Class::Crossbow),
            ("TridentItem", Class::Trident),
            ("SpyglassItem", Class::Spyglass),
            ("InstrumentItem", Class::Instrument),
            ("BundleItem", Class::Bundle),
            ("SnowballItem", Class::Throwable),
            ("EggItem", Class::Throwable),
            ("EnderpearlItem", Class::Throwable),
            ("ExperienceBottleItem", Class::Throwable),
            ("ThrowablePotionItem", Class::Throwable),
            ("WindChargeItem", Class::Throwable),
            ("EnderEyeItem", Class::EnderEye),
            ("FishingRodItem", Class::FishingRod),
            ("FireworkRocketItem", Class::FireworkRocket),
            ("WrittenBookItem", Class::Book),
            ("WritableBookItem", Class::Book),
            ("KnowledgeBookItem", Class::KnowledgeBook),
            ("EmptyMapItem", Class::EmptyMap),
            ("FoodOnAStickItem", Class::FoodOnAStick),
            ("BucketItem", Class::Bucket),
            ("BoatItem", Class::Boat),
            ("SpawnEggItem", Class::SpawnEgg),
        ];
        if let Some((_, c)) = KNOWN.iter().find(|(name, _)| data.is(name)) {
            return *c;
        }
        if data.is("BottleItem") {
            Class::Bottle
        } else if data.is("PlaceOnWaterBlockItem") {
            Class::PlaceOnWater
        } else if data.flags & ItemClassData::USE != 0 {
            Class::Unknown
        } else {
            Class::Plain
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Consumable {
    pub consume_seconds: f32,
    pub animation: i32,
}

impl Consumable {
    /// `consumeTicks`.
    pub fn ticks(&self) -> i32 {
        (self.consume_seconds * 20.0) as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Food {
    pub nutrition: i32,
    pub saturation: f32,
    pub can_always_eat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseEffects {
    pub can_sprint: bool,
    pub speed_multiplier: f32,
}

impl UseEffects {
    /// `UseEffects.DEFAULT`.
    pub const DEFAULT: UseEffects = UseEffects {
        can_sprint: false,
        speed_multiplier: 0.2,
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct UseCooldown {
    pub seconds: f32,
    pub group: Option<String>,
}

impl UseCooldown {
    pub fn ticks(&self) -> i32 {
        (self.seconds * 20.0) as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equippable {
    pub slot: EquipmentSlot,
    pub swappable: bool,
}

/// An item's instrument (`InstrumentComponent`): a registry entry by id or name, or
/// one written out inline (whose values aren't kept).
#[derive(Debug, Clone, PartialEq)]
pub enum InstrumentRef {
    Id(i32),
    Name(String),
    Inline,
}

/// A stack's effective components, the ones item use reads, and its item's class.
#[derive(Debug, Clone, Default)]
pub struct Components {
    /// `minecraft:stone`.
    pub name: &'static str,
    pub class: Class,
    pub block_item: bool,
    /// The class overrides `useOn`.
    pub use_on: bool,
    pub max_stack_size: Option<i32>,
    pub max_damage: Option<i32>,
    pub damage: Option<i32>,
    pub unbreakable: bool,
    pub consumable: Option<Consumable>,
    pub food: Option<Food>,
    pub use_effects: Option<UseEffects>,
    pub use_cooldown: Option<UseCooldown>,
    pub use_remainder: Option<Arc<ItemStack>>,
    pub equippable: Option<Equippable>,
    pub blocks_attacks: bool,
    pub kinetic_weapon: bool,
    pub attack_range: bool,
    pub block_transformer: bool,
    pub instrument: Option<InstrumentRef>,
    /// Charged projectiles, any.
    pub charged: bool,
    /// (`minecraft:enchantment` id, level).
    pub enchantments: Vec<(i32, i32)>,
    /// An enchantment with a riptide spin (`getTridentSpinAttackStrength > 0`).
    pub riptide: bool,
    /// `getUseDuration`, worked out when the stack was made.
    pub use_duration: i32,
    /// The use remainder as a stack (a prototype: copy it).
    pub remainder: Option<Arc<Stack>>,
}

impl Components {
    /// `getOrDefault(USE_EFFECTS, UseEffects.DEFAULT)`.
    pub fn use_effects(&self) -> UseEffects {
        self.use_effects.unwrap_or(UseEffects::DEFAULT)
    }

    /// `getMaxStackSize` (1 without the component).
    pub fn max_stack_size(&self) -> i32 {
        self.max_stack_size.unwrap_or(1)
    }

    /// `isDamageableItem`.
    pub fn damageable(&self) -> bool {
        self.max_damage.is_some() && !self.unbreakable && self.damage.is_some()
    }

    /// `Item.getUseDuration` and its overrides.
    fn compute_use_duration(
        &self,
        instrument_seconds: impl Fn(&InstrumentRef) -> Option<f32>,
    ) -> i32 {
        match self.class {
            Class::Bow | Class::Crossbow | Class::Trident => 72000,
            Class::Spyglass => 1200,
            Class::Bundle => 200,
            Class::EnderEye => 0,
            Class::Instrument => self
                .instrument
                .as_ref()
                .and_then(instrument_seconds)
                .map_or(0, |s| (s * 20.0).floor() as i32),
            _ => match self.consumable {
                Some(c) => c.ticks(),
                None if self.blocks_attacks || self.kinetic_weapon => 72000,
                None => 0,
            },
        }
    }

    /// `useOnRelease`: crossbows.
    pub fn use_on_release(&self) -> bool {
        self.class == Class::Crossbow
    }

    /// Applies a patch from the server.
    fn apply(&mut self, patch: &ComponentPatch, items: &Items) {
        for &kind in &patch.removed {
            match u32::try_from(kind).ok().and_then(Kind::from_id) {
                Some(Kind::MaxStackSize) => self.max_stack_size = None,
                Some(Kind::MaxDamage) => self.max_damage = None,
                Some(Kind::Damage) => self.damage = None,
                Some(Kind::Unbreakable) => self.unbreakable = false,
                Some(Kind::Consumable) => self.consumable = None,
                Some(Kind::Food) => self.food = None,
                Some(Kind::UseEffects) => self.use_effects = None,
                Some(Kind::UseCooldown) => self.use_cooldown = None,
                Some(Kind::UseRemainder) => self.use_remainder = None,
                Some(Kind::Equippable) => self.equippable = None,
                Some(Kind::BlocksAttacks) => self.blocks_attacks = false,
                Some(Kind::KineticWeapon) => self.kinetic_weapon = false,
                Some(Kind::AttackRange) => self.attack_range = false,
                Some(Kind::BlockTransformer) => self.block_transformer = false,
                Some(Kind::Instrument) => self.instrument = None,
                Some(Kind::ChargedProjectiles) => self.charged = false,
                Some(Kind::Enchantments) => self.enchantments.clear(),
                _ => {}
            }
        }
        for (_, c) in &patch.added {
            match c {
                Component::MaxStackSize(n) => self.max_stack_size = Some(*n),
                Component::MaxDamage(n) => self.max_damage = Some(*n),
                Component::Damage(n) => self.damage = Some(*n),
                Component::Unbreakable => self.unbreakable = true,
                Component::Consumable {
                    consume_seconds,
                    animation,
                    ..
                } => {
                    self.consumable = Some(Consumable {
                        consume_seconds: *consume_seconds,
                        animation: *animation,
                    })
                }
                Component::Food {
                    nutrition,
                    saturation,
                    can_always_eat,
                } => {
                    self.food = Some(Food {
                        nutrition: *nutrition,
                        saturation: *saturation,
                        can_always_eat: *can_always_eat,
                    })
                }
                Component::UseEffects {
                    can_sprint,
                    speed_multiplier,
                    ..
                } => {
                    self.use_effects = Some(UseEffects {
                        can_sprint: *can_sprint,
                        speed_multiplier: *speed_multiplier,
                    })
                }
                Component::UseCooldown {
                    seconds,
                    cooldown_group,
                } => {
                    self.use_cooldown = Some(UseCooldown {
                        seconds: *seconds,
                        group: cooldown_group.clone(),
                    })
                }
                Component::UseRemainder(s) => self.use_remainder = Some(Arc::new((**s).clone())),
                Component::Equippable {
                    slot, swappable, ..
                } => {
                    self.equippable = Some(Equippable {
                        slot: EquipmentSlot::from_id(*slot),
                        swappable: *swappable,
                    })
                }
                Component::BlocksAttacks { .. } => self.blocks_attacks = true,
                Component::KineticWeapon(_) => self.kinetic_weapon = true,
                Component::AttackRange { .. } => self.attack_range = true,
                Component::Instrument(h) => {
                    self.instrument = Some(match h {
                        Holder::Id(id) => InstrumentRef::Id(*id),
                        Holder::Inline => InstrumentRef::Inline,
                    })
                }
                Component::ChargedProjectiles(p) => self.charged = !p.is_empty(),
                Component::Enchantments(e) => self.enchantments = e.clone(),
                Component::Other { kind, .. }
                    if u32::try_from(*kind).ok().and_then(Kind::from_id)
                        == Some(Kind::BlockTransformer) =>
                {
                    self.block_transformer = true
                }
                _ => {}
            }
        }
        let _ = items;
    }
}

static NEXT_STACK: AtomicU64 = AtomicU64::new(1);

/// A new object identity.
fn new_identity() -> u64 {
    NEXT_STACK.fetch_add(1, Ordering::Relaxed)
}

/// An item stack on the client. `identity` stands for the Java object: a copy or a
/// stack from a packet is a new one; using it up in place (`consume`, `shrink`) keeps
/// it, as vanilla's `==` checks expect.
#[derive(Debug, Clone)]
pub struct Stack {
    pub identity: u64,
    pub item: i32,
    pub count: i32,
    pub patch: Arc<ComponentPatch>,
    pub c: Arc<Components>,
}

/// Equal stacks: the same object holding the same.
impl PartialEq for Stack {
    fn eq(&self, other: &Stack) -> bool {
        self.identity == other.identity
            && self.item == other.item
            && self.count == other.count
            && self.patch == other.patch
    }
}

impl Default for Stack {
    fn default() -> Stack {
        Stack::empty()
    }
}

impl Stack {
    /// `ItemStack.EMPTY` (one shared object).
    pub fn empty() -> Stack {
        Stack {
            identity: 0,
            item: 0,
            count: 0,
            patch: Arc::new(ComponentPatch::EMPTY),
            c: Arc::new(Components {
                name: "minecraft:air",
                ..Components::default()
            }),
        }
    }

    /// `isEmpty`: air, or none of it.
    pub fn is_empty(&self) -> bool {
        self.item == 0 || self.count <= 0
    }

    /// `getItem` (air when empty).
    pub fn item(&self) -> i32 {
        if self.is_empty() { 0 } else { self.item }
    }

    /// `ItemStack.isSameItem`.
    pub fn same_item(&self, other: &Stack) -> bool {
        self.item() == other.item()
    }

    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &Stack) -> bool {
        self.same_item(other) && (self.is_empty() || self.patch == other.patch)
    }

    /// The same object (Java's `==`).
    pub fn same_object(&self, other: &Stack) -> bool {
        self.identity == other.identity
    }

    /// `copy`: a new object.
    pub fn copy(&self) -> Stack {
        if self.is_empty() {
            return Stack::empty();
        }
        Stack {
            identity: new_identity(),
            ..self.clone()
        }
    }

    /// `shrink`.
    pub fn shrink(&mut self, n: i32) {
        self.count -= n;
    }

    /// `ItemStack.consume`: used up unless the player has infinite materials.
    pub fn consume(&mut self, n: i32, infinite: bool) {
        if !infinite {
            self.shrink(n);
        }
    }

    /// `nextDamageWillBreak`.
    pub fn next_damage_will_break(&self) -> bool {
        self.c.damageable() && self.c.damage.unwrap_or(0) >= self.c.max_damage.unwrap_or(0) - 1
    }

    /// The cooldown group (`ItemCooldowns.getCooldownGroup`).
    pub fn cooldown_group(&self) -> String {
        match self.c.use_cooldown.as_ref().and_then(|c| c.group.clone()) {
            Some(g) => g,
            None => self.c.name.to_owned(),
        }
    }

    /// The stack as `id*count`, empty as "" (as the golden traces write hands).
    pub fn describe(&self) -> String {
        if self.is_empty() {
            String::new()
        } else {
            format!("{}*{}", self.c.name, self.count)
        }
    }
}

/// Every item's defaults and class, and what the session knows of the synced
/// registries the client's item use reads.
#[derive(Debug, Clone, Default)]
pub struct Items {
    defaults: Vec<Arc<Components>>,
    /// `minecraft:instrument` entries by id: name, and use duration in seconds when
    /// the server sent the entry's data.
    pub instruments: Vec<(String, Option<f32>)>,
    /// `minecraft:enchantment` ids whose effects include a riptide spin.
    pub riptide: Vec<i32>,
}

impl Items {
    /// Loads the item classes (exports) and the default components (the data
    /// generator's `reports/minecraft/components/item`).
    pub fn load(exports: &Path, reports: &Path) -> Result<Items, String> {
        let path = exports.join(ruthenium_prepare::exporters::ITEM_CLASSES);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let classes = read_item_classes(&bytes)?;
        let dir = reports.join("minecraft/components/item");
        let mut defaults = HashMap::new();
        for c in &classes {
            let short = c.key.strip_prefix("minecraft:").unwrap_or(&c.key);
            let file = dir.join(format!("{short}.json"));
            if let Ok(text) = std::fs::read_to_string(&file) {
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
                defaults.insert(c.key.clone(), v);
            }
        }
        Ok(Items::new(&classes, &defaults))
    }

    /// From the exported classes and each item's report (`{"components": {...}}`).
    pub fn new(classes: &[ItemClassData], reports: &HashMap<String, serde_json::Value>) -> Items {
        let size = classes.iter().map(|c| c.id as usize + 1).max().unwrap_or(0);
        let mut defaults: Vec<Arc<Components>> = (0..size).map(|_| Arc::default()).collect();
        for data in classes {
            let Some(item) = Item::from_id(data.id) else {
                continue;
            };
            let mut c = Components {
                name: item.name(),
                class: Class::of(data),
                block_item: data.flags & ItemClassData::BLOCK_ITEM != 0,
                use_on: data.flags & ItemClassData::USE_ON != 0,
                ..Components::default()
            };
            if let Some(v) = reports.get(&data.key).and_then(|r| r.get("components")) {
                read_defaults(&mut c, v);
            }
            if let Some(slot) = defaults.get_mut(data.id as usize) {
                *slot = Arc::new(c);
            }
        }
        Items {
            defaults,
            instruments: Vec::new(),
            riptide: Vec::new(),
        }
    }

    /// The item's default components.
    pub fn defaults(&self, item: i32) -> Arc<Components> {
        usize::try_from(item)
            .ok()
            .and_then(|i| self.defaults.get(i))
            .cloned()
            .unwrap_or_default()
    }

    /// A stack from the server: a new object.
    pub fn stack(&self, s: &ItemStack) -> Stack {
        if s.is_empty() {
            return Stack::empty();
        }
        let mut c = (*self.defaults(s.item)).clone();
        c.apply(&s.patch, self);
        c.use_duration = c.compute_use_duration(|i| self.instrument_seconds(i));
        c.riptide = c
            .enchantments
            .iter()
            .any(|(id, level)| *level > 0 && self.riptide.contains(id));
        c.remainder = c
            .use_remainder
            .as_ref()
            .filter(|t| !t.is_empty())
            .map(|t| Arc::new(self.stack(t)));
        Stack {
            identity: new_identity(),
            item: s.item,
            count: s.count,
            patch: Arc::new(s.patch.clone()),
            c: Arc::new(c),
        }
    }

    /// A new stack of an item with its default components.
    pub fn new_stack(&self, item: i32, count: i32) -> Stack {
        self.stack(&ItemStack {
            item,
            count,
            patch: ComponentPatch::EMPTY,
        })
    }

    /// An instrument's use duration in seconds; vanilla's goat horns all play for 7
    /// when the server left the entry's data to the client's own copy.
    pub fn instrument_seconds(&self, i: &InstrumentRef) -> Option<f32> {
        match i {
            InstrumentRef::Id(id) => usize::try_from(*id)
                .ok()
                .and_then(|i| self.instruments.get(i))
                .map(|(_, s)| s.unwrap_or(7.0)),
            InstrumentRef::Name(_) => Some(7.0),
            InstrumentRef::Inline => None,
        }
    }

    /// `getUseDuration` of a stack.
    pub fn use_duration(&self, s: &Stack) -> i32 {
        s.c.use_duration
    }
}

/// An item's report (`"components"`): the defaults item use reads, written with the
/// persistent codecs' names and defaults.
fn read_defaults(c: &mut Components, v: &serde_json::Value) {
    let int = |k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_i64)
            .map(|n| n as i32)
    };
    let f32_or = |o: &serde_json::Value, k: &str, d: f32| {
        o.get(k)
            .and_then(serde_json::Value::as_f64)
            .map_or(d, |f| f as f32)
    };
    let bool_or = |o: &serde_json::Value, k: &str, d: bool| {
        o.get(k).and_then(serde_json::Value::as_bool).unwrap_or(d)
    };
    let has = |k: &str| v.get(k).is_some();
    c.max_stack_size = int("minecraft:max_stack_size");
    c.max_damage = int("minecraft:max_damage");
    c.damage = int("minecraft:damage");
    c.unbreakable = has("minecraft:unbreakable");
    if let Some(o) = v.get("minecraft:consumable") {
        c.consumable = Some(Consumable {
            consume_seconds: f32_or(o, "consume_seconds", 1.6),
            animation: animation::from_name(
                o.get("animation").and_then(|a| a.as_str()).unwrap_or("eat"),
            ),
        });
    }
    if let Some(o) = v.get("minecraft:food") {
        c.food = Some(Food {
            nutrition: o.get("nutrition").and_then(|n| n.as_i64()).unwrap_or(0) as i32,
            saturation: f32_or(o, "saturation", 0.0),
            can_always_eat: bool_or(o, "can_always_eat", false),
        });
    }
    if let Some(o) = v.get("minecraft:use_effects") {
        c.use_effects = Some(UseEffects {
            can_sprint: bool_or(o, "can_sprint", false),
            speed_multiplier: f32_or(o, "speed_multiplier", 0.2),
        });
    }
    if let Some(o) = v.get("minecraft:use_cooldown") {
        c.use_cooldown = Some(UseCooldown {
            seconds: f32_or(o, "seconds", 0.0),
            group: o
                .get("cooldown_group")
                .and_then(|g| g.as_str())
                .map(full_identifier),
        });
    }
    if let Some(o) = v.get("minecraft:use_remainder") {
        let id = o
            .get("id")
            .and_then(|i| i.as_str())
            .and_then(|n| Item::from_name(&full_identifier(n)));
        if let Some(item) = id {
            c.use_remainder = Some(Arc::new(ItemStack {
                item: item.id() as i32,
                count: o.get("count").and_then(|n| n.as_i64()).unwrap_or(1) as i32,
                patch: ComponentPatch::EMPTY,
            }));
        }
    }
    if let Some(o) = v.get("minecraft:equippable") {
        c.equippable = Some(Equippable {
            slot: EquipmentSlot::from_name(o.get("slot").and_then(|s| s.as_str()).unwrap_or("")),
            swappable: bool_or(o, "swappable", true),
        });
    }
    c.blocks_attacks = has("minecraft:blocks_attacks");
    c.kinetic_weapon = has("minecraft:kinetic_weapon");
    c.attack_range = has("minecraft:attack_range");
    c.block_transformer = has("minecraft:block_transformer");
    c.instrument = v.get("minecraft:instrument").map(|i| match i.as_str() {
        Some(name) => InstrumentRef::Name(full_identifier(name)),
        None => InstrumentRef::Inline,
    });
    c.charged = v
        .get("minecraft:charged_projectiles")
        .and_then(|p| p.as_array())
        .is_some_and(|p| !p.is_empty());
}

fn full_identifier(s: &str) -> String {
    if s.contains(':') {
        s.to_owned()
    } else {
        format!("minecraft:{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Items {
        let class = |name: &str, classes: &[&str], flags: u8| ItemClassData {
            id: Item::from_name(name).unwrap().id(),
            key: name.to_owned(),
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            flags,
        };
        let classes = vec![
            class("minecraft:air", &["AirItem", "Item"], 0),
            class("minecraft:golden_apple", &["Item"], 0),
            class(
                "minecraft:bow",
                &["BowItem", "ProjectileWeaponItem", "Item"],
                9,
            ),
            class("minecraft:shield", &["ShieldItem", "Item"], 0),
        ];
        let mut reports = HashMap::new();
        reports.insert(
            "minecraft:golden_apple".to_owned(),
            serde_json::json!({"components": {
                "minecraft:consumable": {},
                "minecraft:food": {"can_always_eat": true, "nutrition": 4, "saturation": 9.6},
                "minecraft:max_stack_size": 64,
                "minecraft:use_effects": {}
            }}),
        );
        reports.insert(
            "minecraft:shield".to_owned(),
            serde_json::json!({"components": {
                "minecraft:blocks_attacks": {"block_delay_seconds": 0.25},
                "minecraft:equippable": {"slot": "offhand", "swappable": false},
                "minecraft:use_effects": {}
            }}),
        );
        Items::new(&classes, &reports)
    }

    #[test]
    fn defaults_and_patches_make_the_components() {
        let items = items();
        let apple = Item::from_name("minecraft:golden_apple").unwrap().id() as i32;
        let s = items.new_stack(apple, 3);
        assert_eq!(s.c.consumable.unwrap().ticks(), 32);
        assert!(s.c.food.unwrap().can_always_eat);
        assert_eq!(s.c.use_effects(), UseEffects::DEFAULT);
        assert_eq!(items.use_duration(&s), 32);
        // A patch that makes it quick to eat and lets the eater sprint.
        let patched = items.stack(&ItemStack {
            item: apple,
            count: 1,
            patch: ComponentPatch {
                added: vec![
                    (
                        Kind::Consumable.id() as i32,
                        Component::Consumable {
                            consume_seconds: 0.5,
                            animation: animation::DRINK,
                            has_consume_particles: false,
                        },
                    ),
                    (
                        Kind::UseEffects.id() as i32,
                        Component::UseEffects {
                            can_sprint: true,
                            interact_vibrations: true,
                            speed_multiplier: 1.0,
                        },
                    ),
                ],
                removed: vec![Kind::Food.id() as i32],
            },
        });
        assert_eq!(items.use_duration(&patched), 10);
        assert!(patched.c.use_effects().can_sprint && patched.c.food.is_none());
        assert!(!patched.same_item_same_components(&s) && patched.same_item(&s));
    }

    #[test]
    fn classes_and_identity() {
        let items = items();
        let bow = items.new_stack(Item::from_name("minecraft:bow").unwrap().id() as i32, 1);
        assert_eq!(bow.c.class, Class::Bow);
        assert_eq!(items.use_duration(&bow), 72000);
        let shield = items.new_stack(Item::from_name("minecraft:shield").unwrap().id() as i32, 1);
        assert!(shield.c.blocks_attacks);
        assert_eq!(items.use_duration(&shield), 72000);
        assert_eq!(shield.c.equippable.unwrap().slot, EquipmentSlot::OffHand);
        let mut a = shield.clone();
        assert!(a.same_object(&shield));
        a.shrink(1);
        assert!(a.is_empty() && a.same_object(&shield));
        assert!(!shield.copy().same_object(&shield));
        assert!(Stack::empty().same_item(&a));
    }
}
