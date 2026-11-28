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
use ruthenium_protocol::item::{
    BlockPredicate, Component, ComponentPatch, Holder, HolderSet, ItemStack, TrimPart,
};
use ruthenium_protocol::nbt::{Compound, Tag};

use crate::transform::TransformerRef;

/// `SwingAnimation.DEFAULT`: a whack of six ticks.
pub const DEFAULT_SWING: (i32, i32) = (1, 6);

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
    /// `EquipmentSlot.VALUES`.
    pub const VALUES: [EquipmentSlot; 8] = [
        EquipmentSlot::MainHand,
        EquipmentSlot::Feet,
        EquipmentSlot::Legs,
        EquipmentSlot::Chest,
        EquipmentSlot::Head,
        EquipmentSlot::OffHand,
        EquipmentSlot::Body,
        EquipmentSlot::Saddle,
    ];

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

/// A shield's patterns in their order.
pub type BannerPatterns = Arc<[(Arc<str>, u8)]>;

/// An armour trim as it is drawn: its material's key and palette and its pattern's
/// pictures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trim {
    /// The material's key in its registry (`minecraft:gold`); empty for one written out.
    pub material: Arc<str>,
    /// The palette its pictures are coloured by (`minecraft:trim/gold`).
    pub palette: Arc<str>,
    /// The pattern's key (`minecraft:bolt`); empty for one written out.
    pub pattern: Arc<str>,
    /// The pattern's pictures (`minecraft:bolt`).
    pub asset: Arc<str>,
    /// Drawn only where the armour itself is.
    pub decal: bool,
}

/// The trim materials and patterns of the session's registries: for each its key, its
/// picture's name (a material's palette, a pattern's asset) and, for a pattern, whether it
/// is a decal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trims {
    materials: Vec<(Arc<str>, Arc<str>)>,
    patterns: Vec<(Arc<str>, Arc<str>, bool)>,
}

impl Trims {
    /// From the registries' entries as the configuration gave them. An entry the server
    /// left to the client's own copy of the game is the game's: a material `gold` has the
    /// palette `trim/gold`, a pattern is named as its key and is no decal.
    pub fn from_registry(
        materials: &[ruthenium_protocol::packets::configuration::RegistryEntry],
        patterns: &[ruthenium_protocol::packets::configuration::RegistryEntry],
    ) -> Trims {
        let string = |e: &ruthenium_protocol::packets::configuration::RegistryEntry, key: &str| {
            let c = e.data.as_ref()?.as_compound()?;
            Some(Arc::<str>::from(full_identifier(c.string(key)?)))
        };
        Trims {
            materials: materials
                .iter()
                .map(|e| {
                    let palette = string(e, "palette_id").unwrap_or_else(|| {
                        let (namespace, path) =
                            e.id.split_once(':').unwrap_or(("minecraft", &e.id));
                        format!("{namespace}:trim/{path}").into()
                    });
                    (e.id.as_str().into(), palette)
                })
                .collect(),
            patterns: patterns
                .iter()
                .map(|e| {
                    let asset = string(e, "asset_id").unwrap_or_else(|| e.id.as_str().into());
                    let decal = e
                        .data
                        .as_ref()
                        .and_then(|d| d.as_compound()?.boolean("decal"))
                        .unwrap_or(false);
                    (e.id.as_str().into(), asset, decal)
                })
                .collect(),
        }
    }

    fn trim(&self, material: &TrimPart, pattern: &TrimPart) -> Option<Trim> {
        let (material, palette) = match material {
            TrimPart::Id(id) => self.materials.get(usize::try_from(*id).ok()?)?.clone(),
            TrimPart::Inline { asset, .. } => ("".into(), asset.as_str().into()),
        };
        let (pattern, asset, decal) = match pattern {
            TrimPart::Id(id) => self.patterns.get(usize::try_from(*id).ok()?)?.clone(),
            TrimPart::Inline { asset, decal } => ("".into(), asset.as_str().into(), *decal),
        };
        Some(Trim {
            material,
            palette,
            pattern,
            asset,
            decal,
        })
    }
}

/// An item's instrument (`InstrumentComponent`): a registry entry by id or name, or
/// one written out inline (whose values aren't kept).
#[derive(Debug, Clone, PartialEq)]
pub enum InstrumentRef {
    Id(i32),
    Name(String),
    Inline,
}

/// `AttackRange`: how near and how far an item attacks from the eyes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttackRange {
    pub min_reach: f32,
    pub max_reach: f32,
    pub min_creative_reach: f32,
    pub max_creative_reach: f32,
    pub hitbox_margin: f32,
    pub mob_factor: f32,
}

impl AttackRange {
    /// The codec's defaults.
    pub const DEFAULT: AttackRange = AttackRange {
        min_reach: 0.0,
        max_reach: 3.0,
        min_creative_reach: 0.0,
        max_creative_reach: 5.0,
        hitbox_margin: 0.3,
        mob_factor: 1.0,
    };

    /// `isInRange(attacker, location)` for a player attacker (`isCreative` by its
    /// player-list game mode): the distance from the eyes to `location` within the
    /// reach, widened by the hitbox margin.
    pub fn is_in_range(&self, eye: [f64; 3], location: [f64; 3], creative: bool) -> bool {
        let d = [
            eye[0] - location[0],
            eye[1] - location[1],
            eye[2] - location[2],
        ];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        // Float sums, widened (the extra buffer is 0 here).
        let min_reach = f64::from(self.min_range(creative) - self.hitbox_margin);
        let max_reach = f64::from(self.max_range(creative) + self.hitbox_margin);
        distance >= min_reach && distance <= max_reach
    }

    /// `effectiveMinRange` for a player.
    pub fn min_range(&self, creative: bool) -> f32 {
        if creative {
            self.min_creative_reach
        } else {
            self.min_reach
        }
    }

    /// `effectiveMaxRange` for a player.
    pub fn max_range(&self, creative: bool) -> f32 {
        if creative {
            self.max_creative_reach
        } else {
            self.max_reach
        }
    }
}

/// Blocks a tool rule names: a block tag, or blocks by registry id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockSet {
    /// The tag's identifier, without the `#`.
    Tag(String),
    Blocks(Vec<u32>),
}

impl BlockSet {
    fn from_holder_set(h: &HolderSet) -> BlockSet {
        match h {
            HolderSet::Tag(t) => BlockSet::Tag(t.clone()),
            HolderSet::Ids(ids) => {
                BlockSet::Blocks(ids.iter().filter_map(|&i| u32::try_from(i).ok()).collect())
            }
        }
    }

    /// A report's `"blocks"`: `#tag`, one block, or a list of blocks.
    fn from_json(v: &serde_json::Value) -> Option<BlockSet> {
        let block =
            |n: &str| ruthenium_data::v26_3::Block::from_name(&full_identifier(n)).map(|b| b.id());
        match v {
            serde_json::Value::String(s) => Some(match s.strip_prefix('#') {
                Some(tag) => BlockSet::Tag(full_identifier(tag)),
                None => BlockSet::Blocks(block(s).into_iter().collect()),
            }),
            serde_json::Value::Array(a) => Some(BlockSet::Blocks(
                a.iter()
                    .filter_map(|n| n.as_str().and_then(block))
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// One of a tool's rules (`Tool.Rule`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRule {
    pub blocks: BlockSet,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

/// The `tool` component: how fast the item mines which blocks, and which it may
/// harvest.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub rules: Vec<ToolRule>,
    pub default_mining_speed: f32,
    pub can_destroy_blocks_in_creative: bool,
}

/// The delay and effect windows used by a held spear, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpearTiming {
    pub delay: i32,
    pub dismount: i32,
    pub knockback: i32,
    pub damage: i32,
}

/// A stack's effective components, the ones item use reads, and its item's class.
#[derive(Debug, Clone, Default)]
pub struct Components {
    /// `minecraft:stone`.
    pub name: &'static str,
    pub class: Class,
    pub block_item: bool,
    /// The block a block item places (`BlockItem.getBlock`, a block registry id).
    pub block: Option<u32>,
    /// The simple names of the item's class and its superclasses up to `Item`.
    pub classes: Arc<[String]>,
    /// The instance fields the item's classes declare (`Class.field`, value), as the
    /// exporter writes them: a standing-and-wall item's wall block, a bucket's fluid.
    pub fields: Arc<[(String, String)]>,
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
    pub spear: Option<SpearTiming>,
    /// How a hand swings when the item attacks and when it is used
    /// (`minecraft:attack_animation`, `minecraft:interact_animation`): none (0), a
    /// whack (1) or a stab (2), and its ticks. `None`: the usual whack.
    pub attack_animation: Option<(i32, i32)>,
    pub interact_animation: Option<(i32, i32)>,
    pub attack_range: Option<AttackRange>,
    /// `minimum_attack_charge` (0 without it).
    pub minimum_attack_charge: f32,
    /// `piercing_weapon`: attacks are jabs (spears).
    pub piercing_weapon: bool,
    /// `block_transformer`: what the item turns blocks into (axes, shovels, hoes).
    pub block_transformer: Option<TransformerRef>,
    pub instrument: Option<InstrumentRef>,
    /// Charged projectiles, any; and a firework rocket among them.
    pub charged: bool,
    pub charged_rocket: bool,
    /// (`minecraft:enchantment` id, level).
    pub enchantments: Vec<(i32, i32)>,
    /// `EnchantmentHelper.getTridentSpinAttackStrength`: above 0, a riptide trident.
    pub spin_attack_strength: f32,
    /// `glider`: worn in its equippable slot, it lets the player glide.
    pub glider: bool,
    /// `getUseDuration`, worked out when the stack was made.
    pub use_duration: i32,
    /// Loading time after applying this stack's enchantments.
    pub crossbow_charge_ticks: i32,
    /// The use remainder as a stack (a prototype: copy it).
    pub remainder: Option<Arc<Stack>>,
    /// `tool`: mining speeds and harvesting.
    pub tool: Option<Arc<Tool>>,
    /// `can_break`: what an adventure-mode player may break with it.
    pub can_break: Option<Arc<Vec<BlockPredicate>>>,
    /// `can_place_on`: what an adventure-mode player may place it against.
    pub can_place_on: Option<Arc<Vec<BlockPredicate>>>,
    /// `block_state`: properties the block it places takes (name, value).
    pub block_state: Option<Arc<Vec<(String, String)>>>,
    /// `compostable`: a composter takes it.
    pub compostable: bool,
    /// `jukebox_playable`: a jukebox plays it.
    pub jukebox_playable: bool,
    /// `item_model`: the item definition it is drawn by (`minecraft:diamond_sword`).
    pub item_model: Option<Arc<str>>,
    /// `equippable`'s asset: what it looks like worn (`minecraft:diamond`).
    pub equipment_asset: Option<Arc<str>>,
    /// `dyed_color`, 0xRRGGBB.
    pub dyed_color: Option<u32>,
    /// `enchantment_glint_override`.
    pub glint_override: Option<bool>,
    /// `trim`.
    pub trim: Option<Arc<Trim>>,
    /// `potion_contents`' own colour, 0xRRGGBB.
    pub potion_color: Option<u32>,
    /// `base_color`: a shield's ground, a `DyeColor` id.
    pub base_color: Option<u8>,
    /// `banner_patterns`: each pattern's asset (`minecraft:creeper`) and `DyeColor` id.
    /// Those of the registry are named as their keys.
    pub banner_patterns: Option<BannerPatterns>,
    /// `profile`: whose head it is.
    pub profile: Option<Arc<ruthenium_protocol::profile::ResolvableProfile>>,
    /// `getDescriptionId`: the translation key of the item's own name.
    pub description: Option<Arc<str>>,
    /// `custom_name` and `item_name`.
    pub custom_name: Option<Arc<Tag>>,
    pub item_name: Option<Arc<Tag>>,
    /// `rarity`'s id: common, uncommon, rare or epic.
    pub rarity: u8,
    /// `lodestone_tracker`.
    pub lodestone: bool,
    /// `potion_contents`, and what they are called in a potion's name (`swiftness`).
    pub potion_name: Option<Arc<str>>,
    /// `written_book_content`'s title.
    pub book_title: Option<Arc<str>>,
    /// `cooking_fuel`: a furnace burns it.
    pub cooking_fuel: bool,
    /// One of its enchantments has `prevent_armor_change` (curse of binding).
    pub prevent_armor_change: bool,
    /// `brewing_fuel`: a brewing stand burns it.
    pub brewing_fuel: bool,
    /// `enchantable`, and `enchantments` removed by the patch (`isEnchantable`).
    pub enchantable: bool,
    pub enchantments_removed: bool,
    /// `stored_enchantments` (an enchanted book's): (`minecraft:enchantment` id, level).
    pub stored_enchantments: Vec<(i32, i32)>,
    /// `equippable`'s `allowed_entities` (`canBeEquippedBy`): any, without it.
    pub equip_allowed: Option<Arc<HolderSet>>,
}

/// What the registered potion `id` is called in names (`Potion.name`): longer and
/// stronger ones go by the plain one's.
fn potion_name(id: i32) -> Option<&'static str> {
    let name = ruthenium_data::v26_3::registries::Potion::from_id(u32::try_from(id).ok()?)?
        .name()
        .trim_start_matches("minecraft:");
    Some(
        name.strip_prefix("long_")
            .or_else(|| name.strip_prefix("strong_"))
            .unwrap_or(name),
    )
}

/// A component read from a report, as the NBT a packet would bring it in.
fn json_to_tag(v: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match v {
        Value::Null => Tag::String(String::new()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(n) => Tag::Int(n as i32),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Tag::String(s.clone()),
        Value::Array(items) => Tag::List(items.iter().map(json_to_tag).collect()),
        Value::Object(o) => {
            let mut c = Compound::new();
            for (k, v) in o {
                c.put(k, json_to_tag(v));
            }
            Tag::Compound(c)
        }
    }
}

impl Components {
    /// `ItemStack.hasFoil`: it glints if it says so, or else if it is enchanted.
    pub fn glints(&self) -> bool {
        self.glint_override.unwrap_or(!self.enchantments.is_empty())
    }

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
                None if self.blocks_attacks || self.spear.is_some() => 72000,
                None => 0,
            },
        }
    }

    /// Whether the item's class is or extends `class` (a simple name).
    pub fn is(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c == class)
    }

    /// One of the item's fields (`Class.field`), as the exporter wrote it.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// `getAttackAnimation`: the kind of swing and its ticks.
    pub fn attack_swing(&self) -> (i32, i32) {
        self.attack_animation.unwrap_or(DEFAULT_SWING)
    }

    /// `getInteractAnimation`.
    pub fn interact_swing(&self) -> (i32, i32) {
        self.interact_animation.unwrap_or(DEFAULT_SWING)
    }

    /// `getUseAnimation` ([`animation`]).
    pub fn use_animation(&self) -> i32 {
        match self.class {
            Class::Bow => animation::BOW,
            Class::Crossbow => animation::CROSSBOW,
            Class::Trident => animation::TRIDENT,
            Class::Spyglass => animation::SPYGLASS,
            Class::Instrument => animation::TOOT_HORN,
            Class::Bundle => animation::BUNDLE,
            _ if self.is("BrushItem") => animation::BRUSH,
            _ => match self.consumable {
                Some(c) => c.animation,
                None if self.blocks_attacks => animation::BLOCK,
                None if self.spear.is_some() => animation::SPEAR,
                None => animation::NONE,
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
                Some(Kind::KineticWeapon) => self.spear = None,
                Some(Kind::AttackAnimation) => self.attack_animation = None,
                Some(Kind::InteractAnimation) => self.interact_animation = None,
                Some(Kind::AttackRange) => self.attack_range = None,
                Some(Kind::MinimumAttackCharge) => self.minimum_attack_charge = 0.0,
                Some(Kind::PiercingWeapon) => self.piercing_weapon = false,
                Some(Kind::BlockTransformer) => self.block_transformer = None,
                Some(Kind::Instrument) => self.instrument = None,
                Some(Kind::ChargedProjectiles) => {
                    self.charged = false;
                    self.charged_rocket = false;
                }
                Some(Kind::Enchantments) => {
                    self.enchantments.clear();
                    self.enchantments_removed = true;
                }
                Some(Kind::StoredEnchantments) => self.stored_enchantments.clear(),
                Some(Kind::Enchantable) => self.enchantable = false,
                Some(Kind::BrewingFuel) => self.brewing_fuel = false,
                Some(Kind::Tool) => self.tool = None,
                Some(Kind::CanBreak) => self.can_break = None,
                Some(Kind::CanPlaceOn) => self.can_place_on = None,
                Some(Kind::BlockState) => self.block_state = None,
                Some(Kind::Compostable) => self.compostable = false,
                Some(Kind::JukeboxPlayable) => self.jukebox_playable = false,
                Some(Kind::Glider) => self.glider = false,
                Some(Kind::ItemModel) => self.item_model = None,
                Some(Kind::DyedColor) => self.dyed_color = None,
                Some(Kind::EnchantmentGlintOverride) => self.glint_override = None,
                Some(Kind::Trim) => self.trim = None,
                Some(Kind::PotionContents) => {
                    self.potion_color = None;
                    self.potion_name = None;
                }
                Some(Kind::CustomName) => self.custom_name = None,
                Some(Kind::ItemName) => self.item_name = None,
                Some(Kind::Rarity) => self.rarity = 0,
                Some(Kind::LodestoneTracker) => self.lodestone = false,
                Some(Kind::WrittenBookContent) => self.book_title = None,
                Some(Kind::BaseColor) => self.base_color = None,
                Some(Kind::BannerPatterns) => self.banner_patterns = None,
                Some(Kind::Profile) => self.profile = None,
                Some(Kind::CookingFuel) => self.cooking_fuel = false,
                _ => {}
            }
        }
        // (A removed equippable takes its asset along.)
        if self.equippable.is_none() {
            self.equipment_asset = None;
            self.equip_allowed = None;
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
                    slot,
                    swappable,
                    asset,
                    allowed_entities,
                    ..
                } => {
                    self.equippable = Some(Equippable {
                        slot: EquipmentSlot::from_id(*slot),
                        swappable: *swappable,
                    });
                    self.equipment_asset = asset.as_deref().map(Arc::from);
                    self.equip_allowed = allowed_entities.clone().map(Arc::new);
                }
                Component::ItemModel(id) => self.item_model = Some(id.as_str().into()),
                Component::DyedColor(rgb) => self.dyed_color = Some(*rgb as u32 & 0xFF_FFFF),
                Component::EnchantmentGlintOverride(on) => self.glint_override = Some(*on),
                Component::Trim { material, pattern } => {
                    self.trim = items.trims.trim(material, pattern).map(Arc::new);
                }
                Component::PotionContents {
                    potion,
                    custom_color,
                    custom_name,
                } => {
                    self.potion_name = Some(
                        custom_name
                            .as_deref()
                            .or_else(|| potion.and_then(potion_name))
                            .unwrap_or("empty")
                            .into(),
                    );
                    // Its own colour, else its potion's. (One made of effects of its
                    // own alone is not mixed here.)
                    self.potion_color = custom_color
                        .map(|c| c as u32 & 0xFF_FFFF)
                        .or_else(|| potion_colour(potion.unwrap_or(-1)));
                }
                Component::BaseColor(dye) => self.base_color = u8::try_from(*dye).ok(),
                Component::BannerPatterns(patterns) => {
                    self.banner_patterns = Some(
                        patterns
                            .iter()
                            .filter_map(|(pattern, dye)| {
                                let asset: Arc<str> = match pattern {
                                    TrimPart::Id(id) => items
                                        .banner_patterns
                                        .get(usize::try_from(*id).ok()?)?
                                        .clone(),
                                    TrimPart::Inline { asset, .. } => asset.as_str().into(),
                                };
                                Some((asset, u8::try_from(*dye).ok()?))
                            })
                            .collect(),
                    );
                }
                Component::Profile(p) => self.profile = Some(Arc::new((**p).clone())),
                Component::BlocksAttacks { .. } => self.blocks_attacks = true,
                Component::KineticWeapon(w) => {
                    self.spear = Some(SpearTiming {
                        delay: w.delay_ticks,
                        dismount: w.dismount_conditions.map_or(0, |c| c.max_duration_ticks),
                        knockback: w.knockback_conditions.map_or(0, |c| c.max_duration_ticks),
                        damage: w.damage_conditions.map_or(0, |c| c.max_duration_ticks),
                    })
                }
                Component::AttackAnimation(s) => {
                    self.attack_animation = Some((s.kind, s.duration));
                }
                Component::InteractAnimation(s) => {
                    self.interact_animation = Some((s.kind, s.duration));
                }
                Component::AttackRange {
                    min_reach,
                    max_reach,
                    min_creative_reach,
                    max_creative_reach,
                    hitbox_margin,
                    mob_factor,
                } => {
                    self.attack_range = Some(AttackRange {
                        min_reach: *min_reach,
                        max_reach: *max_reach,
                        min_creative_reach: *min_creative_reach,
                        max_creative_reach: *max_creative_reach,
                        hitbox_margin: *hitbox_margin,
                        mob_factor: *mob_factor,
                    })
                }
                Component::MinimumAttackCharge(v) => self.minimum_attack_charge = *v,
                Component::PiercingWeapon { .. } => self.piercing_weapon = true,
                Component::Instrument(h) => {
                    self.instrument = Some(match h {
                        Holder::Id(id) => InstrumentRef::Id(*id),
                        Holder::Inline => InstrumentRef::Inline,
                    })
                }
                Component::ChargedProjectiles(p) => {
                    self.charged = !p.is_empty();
                    let rocket =
                        Item::from_name("minecraft:firework_rocket").map(|i| i.id() as i32);
                    self.charged_rocket = p.iter().any(|s| Some(s.item) == rocket);
                }
                Component::Enchantments(e) => {
                    self.enchantments = e.clone();
                    self.enchantments_removed = false;
                }
                Component::StoredEnchantments(e) => self.stored_enchantments = e.clone(),
                Component::Glider => self.glider = true,
                Component::Tool {
                    rules,
                    default_mining_speed,
                    can_destroy_blocks_in_creative,
                    ..
                } => {
                    self.tool = Some(Arc::new(Tool {
                        rules: rules
                            .iter()
                            .map(|r| ToolRule {
                                blocks: BlockSet::from_holder_set(&r.blocks),
                                speed: r.speed,
                                correct_for_drops: r.correct_for_drops,
                            })
                            .collect(),
                        default_mining_speed: *default_mining_speed,
                        can_destroy_blocks_in_creative: *can_destroy_blocks_in_creative,
                    }))
                }
                Component::CanBreak(p) => self.can_break = Some(Arc::new(p.clone())),
                Component::CanPlaceOn(p) => self.can_place_on = Some(Arc::new(p.clone())),
                Component::BlockState(p) => self.block_state = Some(Arc::new(p.clone())),
                Component::BlockTransformer(id) => {
                    self.block_transformer = Some(TransformerRef::Id(*id))
                }
                _ => {}
            }
        }
        for (kind, c) in &patch.added {
            let kind = u32::try_from(*kind).ok().and_then(Kind::from_id);
            let text = |c: &Component| match c {
                Component::Other { data, .. } => ruthenium_protocol::nbt::read_network(data)
                    .ok()
                    .and_then(|(tag, _)| tag.map(Arc::new)),
                _ => None,
            };
            match kind {
                Some(Kind::Compostable) => self.compostable = true,
                Some(Kind::CookingFuel) => self.cooking_fuel = true,
                Some(Kind::BrewingFuel) => self.brewing_fuel = true,
                Some(Kind::Enchantable) => self.enchantable = true,
                Some(Kind::JukeboxPlayable) => self.jukebox_playable = true,
                Some(Kind::LodestoneTracker) => self.lodestone = true,
                Some(Kind::CustomName) => self.custom_name = text(c),
                Some(Kind::ItemName) => self.item_name = text(c),
                Some(Kind::Rarity) => {
                    if let Component::Other { data, .. } = c {
                        // (Unknown ones read as the first.)
                        self.rarity = data.first().copied().filter(|r| *r < 4).unwrap_or(0);
                    }
                }
                Some(Kind::WrittenBookContent) => {
                    if let Component::Other { data, .. } = c {
                        self.book_title = ruthenium_protocol::wire::Reader::new(data)
                            .string(32)
                            .ok()
                            .map(Arc::from);
                    }
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

    /// `ItemStack.matches`: the same object, or as many of the same item with the same
    /// components.
    pub fn matches(&self, other: &Stack) -> bool {
        self.same_object(other)
            || self.count == other.count && self.same_item_same_components(other)
    }

    /// The same object (Java's `==`).
    pub fn same_object(&self, other: &Stack) -> bool {
        self.identity == other.identity
    }

    /// `ItemStack.matchesIgnoringComponents` for the components a hand does not sink
    /// for (`ignoreSwapAnimation`: the damage): the same object, or as many of the same
    /// item with the same components but for that.
    pub fn matches_but_damage(&self, other: &Stack) -> bool {
        if self.same_object(other) {
            return true;
        }
        if self.count != other.count || !self.same_item(other) {
            return false;
        }
        if self.is_empty() && other.is_empty() {
            return true;
        }
        fn added(s: &Stack) -> impl Iterator<Item = &(i32, Component)> {
            let damage = Kind::Damage.id() as i32;
            s.patch.added.iter().filter(move |(k, _)| *k != damage)
        }
        fn removed(s: &Stack) -> impl Iterator<Item = &i32> {
            let damage = Kind::Damage.id() as i32;
            s.patch.removed.iter().filter(move |k| **k != damage)
        }
        added(self).eq(added(other)) && removed(self).eq(removed(other))
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

    /// `grow`.
    pub fn grow(&mut self, n: i32) {
        self.count += n;
    }

    /// `copyWithCount`: a new object (the empty stack stays the empty one).
    pub fn copy_with_count(&self, count: i32) -> Stack {
        if self.is_empty() {
            return Stack::empty();
        }
        let mut s = self.copy();
        s.count = count;
        s
    }

    /// `split`: up to `n` of this stack as a new one, the rest staying here.
    pub fn split(&mut self, n: i32) -> Stack {
        let n = n.min(self.count);
        let taken = self.copy_with_count(n);
        self.shrink(n);
        taken
    }

    /// `getMaxStackSize` (1 without the component, as the empty stack).
    pub fn max_stack_size(&self) -> i32 {
        self.c.max_stack_size()
    }

    /// `isDamaged`: damageable, with damage above 0.
    pub fn is_damaged(&self) -> bool {
        self.c.damageable() && self.c.damage.unwrap_or(0) > 0
    }

    /// `isStackable`: more than one fits, and it is not a damaged tool.
    pub fn is_stackable(&self) -> bool {
        self.max_stack_size() > 1 && !self.is_damaged()
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

/// Item tags, from the server's tags packet (`ItemStack.is(TagKey)`).
#[derive(Debug, Clone, Default)]
pub struct ItemTags {
    /// Each tag's items, by item registry id.
    all: HashMap<String, Vec<bool>>,
}

impl ItemTags {
    pub fn new() -> ItemTags {
        ItemTags::default()
    }

    /// Sets a tag's items, by item name (`minecraft:book`); unknown names are skipped.
    pub fn set<'a>(&mut self, tag: &str, items: impl IntoIterator<Item = &'a str>) {
        let mut by_item = Vec::new();
        for i in items.into_iter().filter_map(Item::from_name) {
            let i = i.id() as usize;
            if by_item.len() <= i {
                by_item.resize(i + 1, false);
            }
            by_item[i] = true;
        }
        self.all.insert(tag.to_owned(), by_item);
    }

    /// The tags an item is in (`Holder.tags`, in no particular order).
    pub fn tags_of(&self, item: i32) -> Vec<&str> {
        let Ok(i) = usize::try_from(item) else {
            return Vec::new();
        };
        self.all
            .iter()
            .filter(|(_, by)| by.get(i).copied().unwrap_or(false))
            .map(|(t, _)| t.as_str())
            .collect()
    }

    /// Every tag's name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.all.keys().map(String::as_str)
    }

    /// `ItemStack.is(tag)`: an empty stack, or an unknown tag, holds nothing.
    pub fn is(&self, stack: &Stack, tag: &str) -> bool {
        !stack.is_empty()
            && usize::try_from(stack.item())
                .ok()
                .and_then(|i| self.all.get(tag)?.get(i).copied())
                .unwrap_or(false)
    }
}

/// The recipe property sets the server sent (`RecipePropertySet`: the items a
/// campfire cooks, a furnace smelts...), by key.
#[derive(Debug, Clone, Default)]
pub struct RecipeSets {
    sets: HashMap<String, Vec<bool>>,
}

impl RecipeSets {
    pub fn new() -> RecipeSets {
        RecipeSets::default()
    }

    /// Sets a property set's items, by item registry id.
    pub fn set(&mut self, key: &str, items: impl IntoIterator<Item = i32>) {
        let mut by_item = Vec::new();
        for i in items.into_iter().filter_map(|i| usize::try_from(i).ok()) {
            if by_item.len() <= i {
                by_item.resize(i + 1, false);
            }
            by_item[i] = true;
        }
        self.sets.insert(key.to_owned(), by_item);
    }

    /// `RecipePropertySet.test`: the stack's item is in the set (an unknown set holds
    /// nothing).
    pub fn contains(&self, key: &str, stack: &Stack) -> bool {
        usize::try_from(stack.item())
            .ok()
            .and_then(|i| self.sets.get(key)?.get(i).copied())
            .unwrap_or(false)
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
    /// The `minecraft:enchantment` registry (empty until the server sends it).
    pub enchantments: crate::enchantment::Enchantments,
    /// The `minecraft:trim_material` and `minecraft:trim_pattern` registries.
    pub trims: Trims,
    /// The `minecraft:banner_pattern` registry: each entry's asset, by id.
    pub banner_patterns: Vec<Arc<str>>,
    /// What the creative inventory's tabs hold (the `creative_tabs` export; empty
    /// without it).
    pub creative: Arc<ruthenium_prepare::exporters::CreativeTabsData>,
    /// The server's tags that menus read: items', entity types' and enchantments'.
    pub tags: Arc<MenuTags>,
}

/// The server's tags the menus read (`ItemStack.is`, `Entity.is`, `Holder.is`): a
/// beacon's payment, a brewing stand's potions, who wears a mount's armour, curses.
#[derive(Debug, Clone, Default)]
pub struct MenuTags {
    pub items: ItemTags,
    pub entity_types: IdTags,
    pub enchantments: IdTags,
}

/// A registry's tags by entry id (`HolderSet.Named` as the client has them).
#[derive(Debug, Clone, Default)]
pub struct IdTags {
    all: HashMap<String, Vec<bool>>,
}

impl IdTags {
    /// Sets a tag's entries, by registry id.
    pub fn set(&mut self, tag: &str, ids: impl IntoIterator<Item = i32>) {
        let mut by_id = Vec::new();
        for i in ids.into_iter().filter_map(|i| usize::try_from(i).ok()) {
            if by_id.len() <= i {
                by_id.resize(i + 1, false);
            }
            by_id[i] = true;
        }
        self.all.insert(tag.to_owned(), by_id);
    }

    /// Whether entry `id` is in `tag` (an unknown tag holds nothing).
    pub fn contains(&self, tag: &str, id: i32) -> bool {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.all.get(tag)?.get(i).copied())
            .unwrap_or(false)
    }

    /// `HolderSet.contains`: a tag's entries, or those listed.
    pub fn holds(&self, set: &HolderSet, id: i32) -> bool {
        match set {
            HolderSet::Tag(t) => self.contains(t, id),
            HolderSet::Ids(ids) => ids.contains(&id),
        }
    }
}

impl Items {
    /// `Equippable.canBeEquippedBy`: an entity of type `entity_type` may wear `stack`.
    pub fn can_be_equipped_by(&self, stack: &Stack, entity_type: i32) -> bool {
        stack
            .c
            .equip_allowed
            .as_deref()
            .is_none_or(|set| self.tags.entity_types.holds(set, entity_type))
    }
}

/// An `allowed_entities` of an item's report: a `#` tag, one type or a list of them.
fn entity_set(v: &serde_json::Value) -> HolderSet {
    use ruthenium_data::v26_3::registries::EntityType;
    let id = |s: &str| EntityType::from_name(&full_identifier(s)).map(|t| t.id() as i32);
    match v {
        serde_json::Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => HolderSet::Tag(full_identifier(tag)),
            None => HolderSet::Ids(id(s).into_iter().collect()),
        },
        serde_json::Value::Array(a) => {
            HolderSet::Ids(a.iter().filter_map(|e| id(e.as_str()?)).collect())
        }
        _ => HolderSet::Ids(Vec::new()),
    }
}

#[cfg(test)]
mod menu_tag_tests {
    use super::*;
    use ruthenium_data::v26_3::registries::EntityType;

    fn horse_items() -> Items {
        let mut classes = Vec::new();
        let mut reports = HashMap::new();
        let equip = |slot: &str, allowed: serde_json::Value| {
            serde_json::json!({"components": {
                "minecraft:max_stack_size": 1,
                "minecraft:enchantments": {},
                "minecraft:equippable": {"slot": slot, "allowed_entities": allowed},
            }})
        };
        for (name, report) in [
            (
                "minecraft:iron_horse_armor",
                equip("body", "#minecraft:can_wear_horse_armor".into()),
            ),
            (
                "minecraft:white_carpet",
                equip(
                    "body",
                    serde_json::json!(["minecraft:llama", "trader_llama"]),
                ),
            ),
            (
                "minecraft:wolf_armor",
                equip("body", "minecraft:wolf".into()),
            ),
            (
                "minecraft:diamond_helmet",
                serde_json::json!({"components": {"minecraft:equippable": {"slot": "head"}}}),
            ),
            (
                "minecraft:blaze_powder",
                serde_json::json!({"components": {"minecraft:brewing_fuel": {}, "minecraft:enchantments": {}}}),
            ),
            (
                "minecraft:book",
                serde_json::json!({"components": {"minecraft:enchantable": {"value": 1}, "minecraft:enchantments": {}}}),
            ),
        ] {
            classes.push(ItemClassData {
                id: Item::from_name(name).unwrap().id(),
                key: name.to_owned(),
                classes: vec!["Item".to_owned()],
                flags: 0,
                block: None,
                fields: Vec::new(),
            });
            reports.insert(name.to_owned(), report);
        }
        let mut items = Items::new(&classes, &reports);
        let mut tags = MenuTags::default();
        tags.entity_types.set(
            "minecraft:can_wear_horse_armor",
            [EntityType::Horse.id() as i32],
        );
        items.tags = Arc::new(tags);
        items
    }

    #[test]
    fn who_may_wear_it_and_what_brews_and_enchants() {
        let items = horse_items();
        let stack = |name: &str| items.new_stack(Item::from_name(name).unwrap().id() as i32, 1);
        let (horse, llama, trader, wolf) = (
            EntityType::Horse.id() as i32,
            EntityType::Llama.id() as i32,
            EntityType::TraderLlama.id() as i32,
            EntityType::Wolf.id() as i32,
        );
        let armour = stack("minecraft:iron_horse_armor");
        assert!(items.can_be_equipped_by(&armour, horse));
        assert!(!items.can_be_equipped_by(&armour, llama));
        let carpet = stack("minecraft:white_carpet");
        assert!(
            items.can_be_equipped_by(&carpet, llama) && items.can_be_equipped_by(&carpet, trader)
        );
        assert!(!items.can_be_equipped_by(&carpet, horse));
        assert!(items.can_be_equipped_by(&stack("minecraft:wolf_armor"), wolf));
        assert!(!items.can_be_equipped_by(&stack("minecraft:wolf_armor"), horse));
        // No list: anyone.
        assert!(items.can_be_equipped_by(&stack("minecraft:diamond_helmet"), horse));
        assert!(stack("minecraft:blaze_powder").c.brewing_fuel);
        let book = stack("minecraft:book");
        assert!(book.c.enchantable && !book.c.enchantments_removed);
        assert!(!stack("minecraft:wolf_armor").c.enchantable);
        // A patch can name others, or take the component away.
        let patched = items.stack(&ItemStack {
            item: armour.item,
            count: 1,
            patch: ComponentPatch {
                added: vec![(
                    Kind::Equippable.id() as i32,
                    Component::Equippable {
                        slot: 6,
                        asset: None,
                        allowed_entities: Some(HolderSet::Ids(vec![wolf])),
                        swappable: true,
                        damage_on_hurt: true,
                        equip_on_interact: false,
                        can_be_sheared: false,
                    },
                )],
                removed: vec![Kind::Enchantments.id() as i32],
                ..Default::default()
            },
        });
        assert!(items.can_be_equipped_by(&patched, wolf));
        assert!(!items.can_be_equipped_by(&patched, horse));
        assert!(patched.c.enchantments_removed);
    }
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
        let mut items = Items::new(&classes, &defaults);
        let path = exports.join(ruthenium_prepare::exporters::CREATIVE_TABS);
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| ruthenium_prepare::exporters::read_creative_tabs(&b))
        {
            Ok(c) => items.creative = Arc::new(c),
            Err(e) => eprintln!(
                "[ruthenium] no creative tabs ({}: {e}); run `ruthenium prepare`. The creative inventory is empty",
                path.display()
            ),
        }
        Ok(items)
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
                block: data.block,
                classes: data.classes.clone().into(),
                fields: data.fields.clone().into(),
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
            enchantments: crate::enchantment::Enchantments::default(),
            trims: Trims::default(),
            banner_patterns: Vec::new(),
            creative: Arc::default(),
            tags: Arc::default(),
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
        if c.class == Class::Crossbow {
            c.crossbow_charge_ticks = self.enchantments.crossbow_charge_ticks(&c.enchantments);
        }
        c.spin_attack_strength = self.enchantments.spin_attack_strength(&c.enchantments);
        c.prevent_armor_change = self.enchantments.prevents_armor_change(&c.enchantments);
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
    c.compostable = has("minecraft:compostable");
    c.jukebox_playable = has("minecraft:jukebox_playable");
    c.glider = has("minecraft:glider");
    c.cooking_fuel = has("minecraft:cooking_fuel");
    c.brewing_fuel = has("minecraft:brewing_fuel");
    c.enchantable = has("minecraft:enchantable");
    c.enchantments_removed = !has("minecraft:enchantments");
    if let Some(o) = v.get("minecraft:block_state").and_then(|o| o.as_object()) {
        let props = o
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect();
        c.block_state = Some(Arc::new(props));
    }
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
        c.equipment_asset = o
            .get("asset_id")
            .and_then(|a| a.as_str())
            .map(|a| full_identifier(a).into());
        c.equip_allowed = o.get("allowed_entities").map(|a| Arc::new(entity_set(a)));
    }
    c.item_model = v
        .get("minecraft:item_model")
        .and_then(|m| m.as_str())
        .map(|m| full_identifier(m).into());
    c.glint_override = v
        .get("minecraft:enchantment_glint_override")
        .and_then(serde_json::Value::as_bool);
    c.dyed_color = v
        .get("minecraft:dyed_color")
        .and_then(serde_json::Value::as_i64)
        .map(|rgb| rgb as u32 & 0xFF_FFFF);
    c.potion_color = v.get("minecraft:potion_contents").and_then(|p| {
        let custom = p.get("custom_color").and_then(serde_json::Value::as_i64);
        custom.map(|c| c as u32 & 0xFF_FFFF).or_else(|| {
            let name = full_identifier(p.get("potion")?.as_str()?);
            let potion = ruthenium_data::v26_3::registries::Potion::from_name(&name)?;
            potion_colour(potion.id() as i32)
        })
    });
    c.potion_name = v.get("minecraft:potion_contents").map(|p| {
        let potion = p.get("potion").and_then(|n| n.as_str());
        let custom = p.get("custom_name").and_then(|n| n.as_str());
        custom
            .or_else(|| {
                let name = full_identifier(potion?);
                let potion = ruthenium_data::v26_3::registries::Potion::from_name(&name)?;
                potion_name(potion.id() as i32)
            })
            .unwrap_or("empty")
            .into()
    });
    c.item_name = v
        .get("minecraft:item_name")
        .map(|n| Arc::new(json_to_tag(n)));
    c.description = v
        .pointer("/minecraft:item_name/translate")
        .and_then(|k| k.as_str())
        .map(Arc::from);
    c.rarity = match v.get("minecraft:rarity").and_then(|r| r.as_str()) {
        Some("uncommon") => 1,
        Some("rare") => 2,
        Some("epic") => 3,
        _ => 0,
    };
    c.base_color = v
        .get("minecraft:base_color")
        .and_then(|d| d.as_str())
        .and_then(|name| DYE_NAMES.iter().position(|n| *n == name))
        .map(|i| i as u8);
    c.blocks_attacks = has("minecraft:blocks_attacks");
    c.spear = v.get("minecraft:kinetic_weapon").map(|w| {
        let ticks = |field: &str| w.pointer(field).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        SpearTiming {
            delay: ticks("/delay_ticks"),
            dismount: ticks("/dismount_conditions/max_duration_ticks"),
            knockback: ticks("/knockback_conditions/max_duration_ticks"),
            damage: ticks("/damage_conditions/max_duration_ticks"),
        }
    });
    let swing = |k: &str| {
        let o = v.get(k)?;
        let kind = match o.get("type").and_then(|t| t.as_str()) {
            Some("none") => 0,
            Some("stab") => 2,
            _ => DEFAULT_SWING.0,
        };
        let duration = o
            .get("duration")
            .and_then(serde_json::Value::as_i64)
            .map_or(DEFAULT_SWING.1, |d| d as i32);
        Some((kind, duration))
    };
    c.attack_animation = swing("minecraft:attack_animation");
    c.interact_animation = swing("minecraft:interact_animation");
    c.attack_range = v.get("minecraft:attack_range").map(|o| {
        let d = AttackRange::DEFAULT;
        AttackRange {
            min_reach: f32_or(o, "min_reach", d.min_reach),
            max_reach: f32_or(o, "max_reach", d.max_reach),
            min_creative_reach: f32_or(o, "min_creative_reach", d.min_creative_reach),
            max_creative_reach: f32_or(o, "max_creative_reach", d.max_creative_reach),
            hitbox_margin: f32_or(o, "hitbox_margin", d.hitbox_margin),
            mob_factor: f32_or(o, "mob_factor", d.mob_factor),
        }
    });
    c.minimum_attack_charge = v
        .get("minecraft:minimum_attack_charge")
        .and_then(serde_json::Value::as_f64)
        .map_or(0.0, |f| f as f32);
    c.piercing_weapon = has("minecraft:piercing_weapon");
    c.block_transformer = v
        .get("minecraft:block_transformer")
        .and_then(TransformerRef::from_json);
    c.instrument = v.get("minecraft:instrument").map(|i| match i.as_str() {
        Some(name) => InstrumentRef::Name(full_identifier(name)),
        None => InstrumentRef::Inline,
    });
    c.charged = v
        .get("minecraft:charged_projectiles")
        .and_then(|p| p.as_array())
        .is_some_and(|p| !p.is_empty());
    c.tool = v.get("minecraft:tool").map(|o| {
        let rules = o
            .get("rules")
            .and_then(|r| r.as_array())
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|r| {
                        Some(ToolRule {
                            blocks: BlockSet::from_json(r.get("blocks")?)?,
                            speed: r
                                .get("speed")
                                .and_then(serde_json::Value::as_f64)
                                .map(|s| s as f32),
                            correct_for_drops: r
                                .get("correct_for_drops")
                                .and_then(serde_json::Value::as_bool),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Arc::new(Tool {
            rules,
            default_mining_speed: f32_or(o, "default_mining_speed", 1.0),
            can_destroy_blocks_in_creative: bool_or(o, "can_destroy_blocks_in_creative", true),
        })
    });
}

/// The colour of a potion of the registry, if it has effects.
fn potion_colour(id: i32) -> Option<u32> {
    let colours = &crate::potion_colours::POTION_COLOURS;
    *usize::try_from(id).ok().and_then(|i| colours.get(i))?
}

/// `DyeColor` names, by id.
pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

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
            block: None,
            fields: Vec::new(),
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
                ..Default::default()
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
        // `matches`: a copy matches until its count changes.
        let mut copy = shield.copy();
        assert!(copy.matches(&shield));
        copy.count = 2;
        assert!(!copy.matches(&shield));
    }

    #[test]
    fn stacks_are_named_by_their_items_and_their_own_components() {
        use crate::item_name::{hover_name, styled_name};
        let report = |v: serde_json::Value| serde_json::json!({ "components": v });
        let reports = HashMap::from([
            (
                "minecraft:diamond_sword".to_owned(),
                report(serde_json::json!({
                    "minecraft:item_name": {"translate": "item.minecraft.diamond_sword"},
                    "minecraft:rarity": "uncommon",
                })),
            ),
            (
                "minecraft:potion".to_owned(),
                report(serde_json::json!({
                    "minecraft:item_name": {"translate": "item.minecraft.potion"},
                    "minecraft:potion_contents": {"potion": "minecraft:long_swiftness"},
                })),
            ),
        ]);
        let class = |key: &str, class: &str| ItemClassData {
            id: Item::from_name(key).unwrap().id(),
            key: key.to_owned(),
            classes: vec![class.to_owned()],
            flags: 0,
            block: None,
            fields: Vec::new(),
        };
        let items = Items::new(
            &[
                class("minecraft:diamond_sword", "Item"),
                class("minecraft:potion", "PotionItem"),
            ],
            &reports,
        );
        let key = |t: &Tag| {
            let c = t.as_compound().expect("a translation");
            c.string("translate").unwrap_or_default().to_owned()
        };
        let sword = with(&items, "minecraft:diamond_sword", Vec::new());
        assert_eq!(key(&hover_name(&sword)), "item.minecraft.diamond_sword");
        let colour = |s: &Stack| {
            styled_name(s)
                .as_compound()
                .unwrap()
                .string("color")
                .map(str::to_owned)
        };
        assert_eq!(colour(&sword).as_deref(), Some("yellow"));
        // Longer and stronger potions go by the plain one's name.
        let potion = with(&items, "minecraft:potion", Vec::new());
        assert_eq!(
            key(&hover_name(&potion)),
            "item.minecraft.potion.effect.swiftness"
        );
        let contents = |potion: Option<&str>, custom_name: Option<&str>| {
            let c = Component::PotionContents {
                potion: potion.map(|p| {
                    ruthenium_data::v26_3::registries::Potion::from_name(p)
                        .unwrap()
                        .id() as i32
                }),
                custom_color: None,
                custom_name: custom_name.map(str::to_owned),
            };
            let s = with(&items, "minecraft:potion", vec![(Kind::PotionContents, c)]);
            key(&hover_name(&s)).replace("item.minecraft.potion.effect.", "")
        };
        assert_eq!(contents(Some("minecraft:strong_healing"), None), "healing");
        assert_eq!(contents(Some("minecraft:water"), Some("fizz")), "fizz");
        assert_eq!(contents(None, None), "empty");
        // Components that are kept as they came.
        let other = |kind: Kind, data: Vec<u8>| {
            let component = Component::Other {
                kind: kind.id() as i32,
                data,
            };
            (kind, component)
        };
        let nbt = |s: &str| {
            let mut out = Vec::new();
            ruthenium_protocol::nbt::write_network(&mut out, &Tag::String(s.into())).unwrap();
            out
        };
        let given = with(
            &items,
            "minecraft:diamond_sword",
            vec![
                other(Kind::CustomName, nbt("Mine")),
                other(Kind::ItemName, nbt("Its")),
                other(Kind::Rarity, vec![3]),
                other(Kind::LodestoneTracker, vec![0, 0]),
            ],
        );
        assert_eq!(hover_name(&given), Tag::String("Mine".into()));
        assert_eq!(
            given.c.item_name.as_deref(),
            Some(&Tag::String("Its".into()))
        );
        assert_eq!(colour(&given).as_deref(), Some("light_purple"));
        assert!(given.c.lodestone && !sword.c.lodestone);
        let mut title = vec![5];
        title.extend(b"Title");
        let book = with(
            &items,
            "minecraft:diamond_sword",
            vec![other(Kind::WrittenBookContent, title)],
        );
        assert_eq!(hover_name(&book), Tag::String("Title".into()));
        let unknown = with(
            &items,
            "minecraft:diamond_sword",
            vec![other(Kind::Rarity, vec![9])],
        );
        assert_eq!(unknown.c.rarity, 0);
        // Taken away, there is none.
        let bare = items.stack(&ItemStack {
            item: sword.item,
            count: 1,
            patch: ComponentPatch {
                added: Vec::new(),
                removed: vec![Kind::ItemName.id() as i32, Kind::Rarity.id() as i32],
                ..Default::default()
            },
        });
        assert_eq!(hover_name(&bare), Tag::String(String::new()));
        assert_eq!(colour(&bare).as_deref(), Some("white"));
        let bare = items.stack(&ItemStack {
            item: potion.item,
            count: 1,
            patch: ComponentPatch {
                added: Vec::new(),
                removed: vec![Kind::PotionContents.id() as i32],
                ..Default::default()
            },
        });
        assert_eq!(key(&hover_name(&bare)), "item.minecraft.potion");
    }

    #[test]
    fn attack_ranges_widen_by_the_margin() {
        // A spear's range: 2 to 4.5 blocks, 0.125 either side.
        let spear = AttackRange {
            min_reach: 2.0,
            max_reach: 4.5,
            min_creative_reach: 2.0,
            max_creative_reach: 6.5,
            hitbox_margin: 0.125,
            mob_factor: 0.5,
        };
        let eye = [0.0, 1.62, 0.0];
        let at = |x: f64| [x, 1.62, 0.0];
        assert!(!spear.is_in_range(eye, at(1.8), false));
        assert!(spear.is_in_range(eye, at(1.875), false));
        assert!(spear.is_in_range(eye, at(4.625), false));
        assert!(!spear.is_in_range(eye, at(4.7), false));
        assert!(spear.is_in_range(eye, at(6.0), true));
    }

    /// A stack of `name` with these components added.
    fn with(items: &Items, name: &str, added: Vec<(Kind, Component)>) -> Stack {
        items.stack(&ItemStack {
            item: Item::from_name(name).unwrap().id() as i32,
            count: 1,
            patch: ComponentPatch {
                added: added.into_iter().map(|(k, c)| (k.id() as i32, c)).collect(),
                removed: Vec::new(),
                ..Default::default()
            },
        })
    }

    #[test]
    fn what_a_stack_looks_like_comes_from_its_item_and_its_patch() {
        use ruthenium_protocol::nbt::Tag;
        use ruthenium_protocol::packets::configuration::RegistryEntry;
        let class = |name: &str| ItemClassData {
            id: Item::from_name(name).unwrap().id(),
            key: name.to_owned(),
            classes: vec!["Item".to_owned()],
            flags: 0,
            block: None,
            fields: Vec::new(),
        };
        let classes = vec![
            class("minecraft:air"),
            class("minecraft:leather_chestplate"),
            class("minecraft:enchanted_golden_apple"),
            class("minecraft:potion"),
            class("minecraft:crossbow"),
        ];
        let mut reports = HashMap::new();
        reports.insert(
            "minecraft:leather_chestplate".to_owned(),
            serde_json::json!({"components": {
                "minecraft:equippable": {"asset_id": "minecraft:leather", "slot": "chest"},
                "minecraft:item_model": "minecraft:leather_chestplate",
                "minecraft:enchantments": {}
            }}),
        );
        reports.insert(
            "minecraft:enchanted_golden_apple".to_owned(),
            serde_json::json!({"components": {"minecraft:enchantment_glint_override": true}}),
        );
        reports.insert(
            "minecraft:potion".to_owned(),
            serde_json::json!({"components": {"minecraft:potion_contents": {"potion": "minecraft:healing"}}}),
        );
        let mut items = Items::new(&classes, &reports);
        let entry = |id: &str, data: Option<Tag>| RegistryEntry {
            id: id.to_owned(),
            data,
        };
        let mut own = ruthenium_protocol::nbt::Compound::new();
        own.put("palette_id", Tag::String("example:shiny".into()));
        let mut lines = ruthenium_protocol::nbt::Compound::new();
        lines.put("asset_id", Tag::String("example:lines".into()));
        lines.put("decal", Tag::Byte(1));
        items.trims = Trims::from_registry(
            &[
                entry("minecraft:gold", None),
                entry("example:mithril", Some(Tag::Compound(own))),
            ],
            &[
                entry("minecraft:bolt", None),
                entry("example:striped", Some(Tag::Compound(lines))),
            ],
        );
        items.banner_patterns = vec!["minecraft:creeper".into(), "minecraft:border".into()];

        // The item's own: what it looks like worn and held, no dye, no glint.
        let plain = with(&items, "minecraft:leather_chestplate", Vec::new());
        assert_eq!(
            plain.c.equipment_asset.as_deref(),
            Some("minecraft:leather")
        );
        assert_eq!(
            plain.c.item_model.as_deref(),
            Some("minecraft:leather_chestplate")
        );
        assert_eq!((plain.c.dyed_color, plain.c.glints()), (None, false));
        assert!(plain.c.trim.is_none());
        // Dyed, enchanted and trimmed by entries the client knows only by name.
        let worn = with(
            &items,
            "minecraft:leather_chestplate",
            vec![
                (
                    Kind::DyedColor,
                    Component::DyedColor(-0x100_0000 | 0x12_3456),
                ),
                (Kind::Enchantments, Component::Enchantments(vec![(3, 1)])),
                (
                    Kind::Trim,
                    Component::Trim {
                        material: TrimPart::Id(0),
                        pattern: TrimPart::Id(0),
                    },
                ),
                (Kind::ItemModel, Component::ItemModel("example:robe".into())),
            ],
        );
        assert_eq!(
            (worn.c.dyed_color, worn.c.glints()),
            (Some(0x12_3456), true)
        );
        assert_eq!(worn.c.item_model.as_deref(), Some("example:robe"));
        let trim = worn.c.trim.as_deref().unwrap();
        assert_eq!(
            (&*trim.material, &*trim.palette),
            ("minecraft:gold", "minecraft:trim/gold")
        );
        assert_eq!(
            (&*trim.pattern, &*trim.asset, trim.decal),
            ("minecraft:bolt", "minecraft:bolt", false)
        );
        // By entries the server described, and written out in the stack itself.
        let of = |material, pattern| {
            let s = with(
                &items,
                "minecraft:leather_chestplate",
                vec![(Kind::Trim, Component::Trim { material, pattern })],
            );
            s.c.trim.as_deref().cloned()
        };
        let described = of(TrimPart::Id(1), TrimPart::Id(1)).unwrap();
        assert_eq!(
            (&*described.palette, &*described.asset, described.decal),
            ("example:shiny", "example:lines", true)
        );
        let inline = TrimPart::Inline {
            asset: "example:own".into(),
            decal: true,
        };
        let written = of(inline.clone(), inline).unwrap();
        assert_eq!(
            (&*written.material, &*written.palette, written.decal),
            ("", "example:own", true)
        );
        assert_eq!(of(TrimPart::Id(7), TrimPart::Id(0)), None);
        // A glint an item has by itself, and one a stack is told not to have.
        let apple = with(&items, "minecraft:enchanted_golden_apple", Vec::new());
        assert!(apple.c.glints());
        let dull = with(
            &items,
            "minecraft:leather_chestplate",
            vec![
                (Kind::Enchantments, Component::Enchantments(vec![(3, 1)])),
                (
                    Kind::EnchantmentGlintOverride,
                    Component::EnchantmentGlintOverride(false),
                ),
            ],
        );
        assert!(!dull.c.glints());
        // A potion's colour: its item's own contents', its potion's, or the one it is given.
        let healing = with(&items, "minecraft:potion", Vec::new());
        assert_eq!(healing.c.potion_color, Some(0xF8_2423));
        let contents = |potion, custom_color| {
            let c = Component::PotionContents {
                potion,
                custom_color,
                custom_name: None,
            };
            with(&items, "minecraft:potion", vec![(Kind::PotionContents, c)])
                .c
                .potion_color
        };
        let id = |name: &str| {
            ruthenium_data::v26_3::registries::Potion::from_name(name).map(|p| p.id() as i32)
        };
        assert_eq!(contents(id("minecraft:swiftness"), None), Some(0x33_EBFF));
        assert_eq!(contents(id("minecraft:water"), None), None);
        assert_eq!(
            contents(id("minecraft:swiftness"), Some(0x0A_0B0C)),
            Some(0x0A_0B0C)
        );
        assert_eq!(contents(None, None), None);
        // A crossbow by what is in it; a shield by its ground and patterns.
        let loaded = |name: &str| {
            let projectile = ItemStack {
                item: Item::from_name(name).unwrap().id() as i32,
                count: 1,
                patch: ComponentPatch::EMPTY,
            };
            let c = Component::ChargedProjectiles(vec![projectile]);
            let s = with(
                &items,
                "minecraft:crossbow",
                vec![(Kind::ChargedProjectiles, c)],
            );
            (s.c.charged, s.c.charged_rocket)
        };
        assert_eq!(loaded("minecraft:arrow"), (true, false));
        assert_eq!(loaded("minecraft:firework_rocket"), (true, true));
        let shield = with(
            &items,
            "minecraft:crossbow",
            vec![
                (Kind::BaseColor, Component::BaseColor(14)),
                (
                    Kind::BannerPatterns,
                    Component::BannerPatterns(vec![
                        (TrimPart::Id(1), 4),
                        (TrimPart::Id(9), 4),
                        (
                            TrimPart::Inline {
                                asset: "example:star".into(),
                                decal: false,
                            },
                            15,
                        ),
                    ]),
                ),
            ],
        );
        assert_eq!(shield.c.base_color, Some(14));
        let patterns: Vec<(&str, u8)> = shield
            .c
            .banner_patterns
            .as_deref()
            .unwrap()
            .iter()
            .map(|(asset, dye)| (&**asset, *dye))
            .collect();
        assert_eq!(patterns, [("minecraft:border", 4), ("example:star", 15)]);
    }
}
