//! Item stacks as the server sends them, with their data component patches.
//!
//! A stack is `ItemStack.OPTIONAL_STREAM_CODEC`: a count (0 or less is the empty stack),
//! the item's registry id, then the patch over the item's default components
//! (`DataComponentPatch.STREAM_CODEC`): how many components are added, how many removed,
//! each added one as its type id and value, then each removed type id. The values are
//! not length-prefixed (that is only the untrusted serverbound codec), so every value
//! must be read exactly: one byte too few or too many loses everything after it.
//!
//! Every component type has a reader here. The ones the client simulation uses are
//! [`Component`]'s typed variants; the rest are kept as their raw bytes
//! ([`Component::Other`]). A typed value's bytes are kept too ([`RawValues`]), for
//! what it leaves out. Types without a network codec of their own (custom data,
//! locks, recipes...) go over the wire as one network NBT tag, as vanilla's fallback
//! (`ByteBufCodecs.fromCodecWithRegistries`) sends them.
//!
//! Encodings that recur:
//! - a *registry holder* (`ByteBufCodecs.holderRegistry`) is the plain VarInt id;
//! - a *holder* (`ByteBufCodecs.holder`) is a VarInt: 0 means the value follows inline,
//!   anything else is the registry id plus one;
//! - a *holder set* (`ByteBufCodecs.holderSet`) is a VarInt n: 0 means a tag
//!   identifier follows, otherwise n - 1 registry ids follow;
//! - text components and other codec-backed values are one network NBT tag.

use ruthenium_data::v26_3::registries::DataComponentType as Kind;

use crate::profile::ResolvableProfile;
use crate::wire::{MAX_STRING, Reader, WireError};

/// How deeply stacks and recursive values (a stack in a bundle in a bundle, an effect's
/// hidden effect) may nest before the reader refuses them, so that a hostile packet
/// can't exhaust the stack.
const MAX_DEPTH: u32 = 64;

/// An item stack: the item (`minecraft:item` registry id), how many, and the changes to
/// the item's default components.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemStack {
    pub item: i32,
    pub count: i32,
    pub patch: ComponentPatch,
}

impl ItemStack {
    /// The empty stack (air, none of it).
    pub const EMPTY: ItemStack = ItemStack {
        item: 0,
        count: 0,
        patch: ComponentPatch::EMPTY,
    };

    /// Whether this is the empty stack: air, or a count of zero or less (as vanilla's
    /// `ItemStack.isEmpty`).
    pub fn is_empty(&self) -> bool {
        self.item == 0 || self.count <= 0
    }

    /// The component of type `kind` the patch adds, if it adds one.
    pub fn get(&self, kind: Kind) -> Option<&Component> {
        self.patch.get(kind)
    }

    /// A stack that may be empty (`ItemStack.OPTIONAL_STREAM_CODEC`).
    pub fn read_optional(r: &mut Reader<'_>) -> Result<ItemStack, WireError> {
        read_optional(r, 0)
    }

    /// A stack that must not be empty (`ItemStack.STREAM_CODEC`).
    pub fn read(r: &mut Reader<'_>) -> Result<ItemStack, WireError> {
        let stack = read_optional(r, 0)?;
        if stack.is_empty() {
            return Err(WireError::Invalid("empty item stack"));
        }
        Ok(stack)
    }

    /// An `ItemStackTemplate` (what bundles, crossbows and use remainders hold): the item
    /// first, then the count, then the patch; never empty.
    pub fn read_template(r: &mut Reader<'_>) -> Result<ItemStack, WireError> {
        read_template(r, 0)
    }
}

fn deeper(depth: u32) -> Result<u32, WireError> {
    if depth >= MAX_DEPTH {
        return Err(WireError::Invalid("item data nested too deeply"));
    }
    Ok(depth + 1)
}

fn read_optional(r: &mut Reader<'_>, depth: u32) -> Result<ItemStack, WireError> {
    let count = r.varint()?;
    if count <= 0 {
        return Ok(ItemStack::EMPTY);
    }
    let item = r.varint()?;
    let patch = ComponentPatch::read(r, deeper(depth)?)?;
    Ok(ItemStack { item, count, patch })
}

fn read_template(r: &mut Reader<'_>, depth: u32) -> Result<ItemStack, WireError> {
    let item = r.varint()?;
    let count = r.varint()?;
    // `ItemStackTemplate` refuses air and a count of zero.
    if item == 0 || count == 0 {
        return Err(WireError::Invalid("empty item stack template"));
    }
    let patch = ComponentPatch::read(r, deeper(depth)?)?;
    Ok(ItemStack { item, count, patch })
}

/// The changes a stack makes to its item's default components: the components it adds
/// or replaces (type id, value) and the types it removes, in the server's order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComponentPatch {
    pub added: Vec<(i32, Component)>,
    pub removed: Vec<i32>,
    /// The bytes each typed component was read from (see [`RawValues`]).
    pub raw: RawValues,
}

/// The bytes the typed components of a patch were read from, for what their typed value
/// leaves out. Patches compare equal whatever their bytes.
#[derive(Clone, Default)]
pub struct RawValues(Vec<(i32, Box<[u8]>)>);

impl RawValues {
    /// The bytes the component of type `kind` was read from, if it was read.
    pub fn get(&self, kind: i32) -> Option<&[u8]> {
        self.0.iter().find(|(k, _)| *k == kind).map(|(_, b)| &b[..])
    }
}

impl PartialEq for RawValues {
    fn eq(&self, _: &RawValues) -> bool {
        true
    }
}

impl std::fmt::Debug for RawValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RawValues({})", self.0.len())
    }
}

impl ComponentPatch {
    pub const EMPTY: ComponentPatch = ComponentPatch {
        added: Vec::new(),
        removed: Vec::new(),
        raw: RawValues(Vec::new()),
    };

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// The value the patch gives `kind`, if it adds one.
    pub fn get(&self, kind: Kind) -> Option<&Component> {
        let id = kind.id() as i32;
        self.added.iter().find(|(k, _)| *k == id).map(|(_, c)| c)
    }

    /// Whether the patch removes `kind` from the item's defaults.
    pub fn removes(&self, kind: Kind) -> bool {
        self.removed.contains(&(kind.id() as i32))
    }

    fn read(r: &mut Reader<'_>, depth: u32) -> Result<ComponentPatch, WireError> {
        let added_count = r.count(usize::MAX)?;
        let removed_count = r.count(usize::MAX)?;
        let mut added = Vec::with_capacity(added_count.min(64));
        let mut raw = Vec::new();
        for _ in 0..added_count {
            let kind = r.varint()?;
            let mut start = r.clone();
            let c = read_component(r, kind, depth)?;
            if !matches!(c, Component::Other { .. }) {
                let used = start.remaining() - r.remaining();
                raw.push((kind, start.bytes(used)?.into()));
            }
            added.push((kind, c));
        }
        let mut removed = Vec::with_capacity(removed_count.min(64));
        for _ in 0..removed_count {
            let kind = r.varint()?;
            component_kind(kind)?;
            removed.push(kind);
        }
        Ok(ComponentPatch {
            added,
            removed,
            raw: RawValues(raw),
        })
    }
}

/// A registry entry given by id or written out inline (`ByteBufCodecs.holder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// The registry id.
    Id(i32),
    /// A value sent inline, not kept.
    Inline,
}

/// A trim's material or pattern, or a banner's pattern: a registry entry, or one written
/// out, of which the picture's name (a material's palette, a pattern's asset) and whether
/// a trim's pattern is a decal are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrimPart {
    Id(i32),
    Inline { asset: String, decal: bool },
}

/// A set of registry entries (`HolderSet`): a tag, or entries by registry id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderSet {
    /// A tag's identifier, without the `#`.
    Tag(String),
    Ids(Vec<i32>),
}

/// One rule of a [`Component::Tool`] (`Tool.Rule`): the blocks it covers, the mining
/// speed it gives them, and whether it makes them drop.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRule {
    pub blocks: HolderSet,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

/// How a [`PropertyMatcher`] matches a value (`StatePropertiesPredicate`'s matchers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueMatcher {
    Exact(String),
    /// Either end may be open.
    Range {
        min: Option<String>,
        max: Option<String>,
    },
}

/// A block state property's condition in a [`BlockPredicate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyMatcher {
    pub name: String,
    pub value: ValueMatcher,
}

/// One entry of an adventure-mode predicate (`BlockPredicate`): the blocks and state
/// properties it asks for; the NBT and component conditions on the block entity are
/// only noted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPredicate {
    pub blocks: Option<HolderSet>,
    pub properties: Option<Vec<PropertyMatcher>>,
    /// It has an NBT condition.
    pub nbt: bool,
    /// It has exact or partial component conditions.
    pub components: bool,
}

/// An item's swing (`SwingAnimation`): the `SwingAnimationType` id and its length in
/// ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwingAnimation {
    pub kind: i32,
    pub duration: i32,
}

/// A condition of a [`KineticWeapon`] effect (`KineticWeapon.Condition`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KineticCondition {
    pub max_duration_ticks: i32,
    pub min_speed: f32,
    pub min_relative_speed: f32,
}

/// A charged weapon (`KineticWeapon`, the spear); its sounds are skipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KineticWeapon {
    pub contact_cooldown_ticks: i32,
    pub delay_ticks: i32,
    pub dismount_conditions: Option<KineticCondition>,
    pub knockback_conditions: Option<KineticCondition>,
    pub damage_conditions: Option<KineticCondition>,
    pub forward_movement: f32,
    pub damage_multiplier: f32,
}

/// A component's value: the typed ones the client uses, and the raw bytes of the rest.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    MaxStackSize(i32),
    MaxDamage(i32),
    Damage(i32),
    Unbreakable,
    UseEffects {
        can_sprint: bool,
        interact_vibrations: bool,
        speed_multiplier: f32,
    },
    MinimumAttackCharge(f32),
    /// (`minecraft:enchantment` id, level) pairs.
    Enchantments(Vec<(i32, i32)>),
    Food {
        nutrition: i32,
        saturation: f32,
        can_always_eat: bool,
    },
    /// The sound and the effects are skipped; `animation` is the `ItemUseAnimation` id.
    Consumable {
        consume_seconds: f32,
        animation: i32,
        has_consume_particles: bool,
    },
    UseRemainder(Box<ItemStack>),
    UseCooldown {
        seconds: f32,
        cooldown_group: Option<String>,
    },
    Weapon {
        item_damage_per_attack: i32,
        disable_blocking_for_seconds: f32,
    },
    AttackRange {
        min_reach: f32,
        max_reach: f32,
        min_creative_reach: f32,
        max_creative_reach: f32,
        hitbox_margin: f32,
        mob_factor: f32,
    },
    /// `slot` is the `EquipmentSlot` id and `asset` what it looks like worn
    /// (`minecraft:diamond`); the sounds, the camera overlay and the allowed entities are
    /// skipped.
    Equippable {
        slot: i32,
        asset: Option<String>,
        swappable: bool,
        damage_on_hurt: bool,
        equip_on_interact: bool,
        can_be_sheared: bool,
    },
    Glider,
    /// Its effects are skipped.
    DeathProtection,
    /// The damage reductions, item damage, bypassing types and sounds are skipped.
    BlocksAttacks {
        block_delay_seconds: f32,
        disable_cooldown_scale: f32,
    },
    /// Its sounds are skipped.
    PiercingWeapon {
        deals_knockback: bool,
        dismounts: bool,
    },
    KineticWeapon(KineticWeapon),
    AttackAnimation(SwingAnimation),
    InteractAnimation(SwingAnimation),
    StoredEnchantments(Vec<(i32, i32)>),
    ChargedProjectiles(Vec<ItemStack>),
    BundleContents(Vec<ItemStack>),
    /// The potion (`minecraft:potion` id), if any, and the colour it is given
    /// (0xRRGGBB) and the name it goes by; the custom effects are skipped.
    PotionContents {
        potion: Option<i32>,
        custom_color: Option<i32>,
        custom_name: Option<String>,
    },
    /// The item definition a stack is drawn by (`minecraft:diamond_sword`).
    ItemModel(String),
    /// 0xRRGGBB.
    DyedColor(i32),
    EnchantmentGlintOverride(bool),
    /// An armour trim: its material and its pattern.
    Trim {
        material: TrimPart,
        pattern: TrimPart,
    },
    /// A banner's or shield's ground: a `DyeColor` id.
    BaseColor(i32),
    /// A banner's or shield's patterns, each with its `DyeColor` id.
    BannerPatterns(Vec<(TrimPart, i32)>),
    Profile(Box<ResolvableProfile>),
    Instrument(Holder),
    /// The block transformer (`minecraft:block_transformer` registry id).
    BlockTransformer(i32),
    /// A container's slots in order; an empty slot is the empty stack.
    Container(Vec<ItemStack>),
    /// The mining rules; `damage_per_block` is the server's.
    Tool {
        rules: Vec<ToolRule>,
        default_mining_speed: f32,
        damage_per_block: i32,
        can_destroy_blocks_in_creative: bool,
    },
    /// The blocks an adventure-mode player may break with the item.
    CanBreak(Vec<BlockPredicate>),
    /// The blocks an adventure-mode player may place the item against.
    CanPlaceOn(Vec<BlockPredicate>),
    /// `block_state`: properties the placed block takes (name, value).
    BlockState(Vec<(String, String)>),
    /// Any other component: its type id and its value's bytes as sent.
    Other {
        kind: i32,
        data: Vec<u8>,
    },
}

fn component_kind(kind: i32) -> Result<Kind, WireError> {
    u32::try_from(kind)
        .ok()
        .and_then(Kind::from_id)
        .ok_or(WireError::Invalid("data component type"))
}

/// Reads the value of a component of type `kind`.
fn read_component(r: &mut Reader<'_>, kind: i32, depth: u32) -> Result<Component, WireError> {
    let k = component_kind(kind)?;
    let mut start = r.clone();
    let c = match k {
        Kind::MaxStackSize => Component::MaxStackSize(r.varint()?),
        Kind::MaxDamage => Component::MaxDamage(r.varint()?),
        Kind::Damage => Component::Damage(r.varint()?),
        Kind::Unbreakable => Component::Unbreakable,
        Kind::UseEffects => Component::UseEffects {
            can_sprint: r.bool()?,
            interact_vibrations: r.bool()?,
            speed_multiplier: r.f32()?,
        },
        Kind::MinimumAttackCharge => Component::MinimumAttackCharge(r.f32()?),
        Kind::Enchantments => Component::Enchantments(enchantments(r)?),
        Kind::StoredEnchantments => Component::StoredEnchantments(enchantments(r)?),
        Kind::Food => Component::Food {
            nutrition: r.varint()?,
            saturation: r.f32()?,
            can_always_eat: r.bool()?,
        },
        Kind::Consumable => {
            let consume_seconds = r.f32()?;
            let animation = r.varint()?;
            sound(r)?;
            let has_consume_particles = r.bool()?;
            skip_list(r, usize::MAX, |r| consume_effect(r, depth))?;
            Component::Consumable {
                consume_seconds,
                animation,
                has_consume_particles,
            }
        }
        Kind::UseRemainder => Component::UseRemainder(Box::new(read_template(r, depth)?)),
        Kind::UseCooldown => Component::UseCooldown {
            seconds: r.f32()?,
            cooldown_group: opt(r, |r| r.identifier())?,
        },
        Kind::Weapon => Component::Weapon {
            item_damage_per_attack: r.varint()?,
            disable_blocking_for_seconds: r.f32()?,
        },
        Kind::AttackRange => Component::AttackRange {
            min_reach: r.f32()?,
            max_reach: r.f32()?,
            min_creative_reach: r.f32()?,
            max_creative_reach: r.f32()?,
            hitbox_margin: r.f32()?,
            mob_factor: r.f32()?,
        },
        Kind::Equippable => {
            let slot = r.varint()?;
            sound(r)?;
            // The asset id, the camera overlay, the entities that may wear it.
            let asset = opt(r, |r| r.identifier())?;
            opt(r, |r| r.identifier())?;
            opt(r, holder_set)?;
            let _dispensable = r.bool()?;
            let c = Component::Equippable {
                slot,
                asset,
                swappable: r.bool()?,
                damage_on_hurt: r.bool()?,
                equip_on_interact: r.bool()?,
                can_be_sheared: r.bool()?,
            };
            sound(r)?;
            c
        }
        Kind::Glider => Component::Glider,
        Kind::DeathProtection => {
            skip_list(r, usize::MAX, |r| consume_effect(r, depth))?;
            Component::DeathProtection
        }
        Kind::BlocksAttacks => {
            let c = Component::BlocksAttacks {
                block_delay_seconds: r.f32()?,
                disable_cooldown_scale: r.f32()?,
            };
            // Damage reductions: angle, optional types, base, factor.
            skip_list(r, usize::MAX, |r| {
                r.f32()?;
                opt(r, holder_set)?;
                r.f32()?;
                r.f32()?;
                Ok(())
            })?;
            // Item damage: threshold, base, factor.
            for _ in 0..3 {
                r.f32()?;
            }
            opt(r, holder_set)?;
            opt(r, sound)?;
            opt(r, sound)?;
            c
        }
        Kind::PiercingWeapon => {
            let c = Component::PiercingWeapon {
                deals_knockback: r.bool()?,
                dismounts: r.bool()?,
            };
            opt(r, sound)?;
            opt(r, sound)?;
            c
        }
        Kind::KineticWeapon => {
            let condition = |r: &mut Reader<'_>| {
                opt(r, |r| {
                    Ok(KineticCondition {
                        max_duration_ticks: r.varint()?,
                        min_speed: r.f32()?,
                        min_relative_speed: r.f32()?,
                    })
                })
            };
            let w = KineticWeapon {
                contact_cooldown_ticks: r.varint()?,
                delay_ticks: r.varint()?,
                dismount_conditions: condition(r)?,
                knockback_conditions: condition(r)?,
                damage_conditions: condition(r)?,
                forward_movement: r.f32()?,
                damage_multiplier: r.f32()?,
            };
            opt(r, sound)?;
            opt(r, sound)?;
            Component::KineticWeapon(w)
        }
        Kind::Tool => Component::Tool {
            rules: list(r, usize::MAX, |r| {
                Ok(ToolRule {
                    blocks: read_holder_set(r)?,
                    speed: opt(r, |r| r.f32())?,
                    correct_for_drops: opt(r, |r| r.bool())?,
                })
            })?,
            default_mining_speed: r.f32()?,
            damage_per_block: r.varint()?,
            can_destroy_blocks_in_creative: r.bool()?,
        },
        Kind::CanBreak => Component::CanBreak(list(r, usize::MAX, |r| block_predicate(r, depth))?),
        Kind::CanPlaceOn => {
            Component::CanPlaceOn(list(r, usize::MAX, |r| block_predicate(r, depth))?)
        }
        Kind::BlockState => Component::BlockState(list(r, usize::MAX, |r| {
            Ok((
                r.string(MAX_STRING)?.to_owned(),
                r.string(MAX_STRING)?.to_owned(),
            ))
        })?),
        Kind::AttackAnimation => Component::AttackAnimation(swing_animation(r)?),
        Kind::InteractAnimation => Component::InteractAnimation(swing_animation(r)?),
        Kind::ChargedProjectiles => {
            Component::ChargedProjectiles(list(r, 1024, |r| read_template(r, depth))?)
        }
        Kind::BundleContents => {
            Component::BundleContents(list(r, usize::MAX, |r| read_template(r, depth))?)
        }
        Kind::Container => Component::Container(list(r, 256, |r| {
            Ok(opt(r, |r| read_template(r, depth))?.unwrap_or_default())
        })?),
        Kind::PotionContents => {
            let potion = opt(r, |r| r.varint())?;
            // Custom colour, custom effects, custom name.
            let custom_color = opt(r, |r| r.i32())?;
            skip_list(r, usize::MAX, |r| mob_effect(r, depth))?;
            let custom_name = opt(r, |r| Ok(r.string(MAX_STRING)?.to_owned()))?;
            Component::PotionContents {
                potion,
                custom_color,
                custom_name,
            }
        }
        Kind::ItemModel => Component::ItemModel(r.identifier()?),
        Kind::DyedColor => Component::DyedColor(r.i32()?),
        Kind::EnchantmentGlintOverride => Component::EnchantmentGlintOverride(r.bool()?),
        Kind::Trim => {
            // A material: its palette and description. A pattern: its asset, its
            // description, and whether it is a decal.
            let material = trim_part(r, |r| {
                let asset = r.identifier()?;
                nbt(r)?;
                Ok((asset, false))
            })?;
            let pattern = trim_part(r, |r| {
                let asset = r.identifier()?;
                nbt(r)?;
                Ok((asset, r.bool()?))
            })?;
            Component::Trim { material, pattern }
        }
        Kind::BaseColor => Component::BaseColor(r.varint()?),
        Kind::BannerPatterns => Component::BannerPatterns(list(r, 1024, |r| {
            // A pattern: its asset and its translation key.
            let pattern = trim_part(r, |r| {
                let asset = r.identifier()?;
                r.string(MAX_STRING)?;
                Ok((asset, false))
            })?;
            Ok((pattern, r.varint()?))
        })?),
        Kind::Profile => Component::Profile(Box::new(ResolvableProfile::read(r)?)),
        Kind::BlockTransformer => Component::BlockTransformer(r.varint()?),
        Kind::Instrument => Component::Instrument(holder(r, |r| {
            // Sound, use duration, range, durability damage, description.
            sound(r)?;
            r.f32()?;
            r.f32()?;
            r.varint()?;
            nbt(r)
        })?),
        other => {
            skip_value(r, other, depth)?;
            let used = start.remaining() - r.remaining();
            return Ok(Component::Other {
                kind,
                data: start.bytes(used)?.to_vec(),
            });
        }
    };
    Ok(c)
}

/// Reads past the value of a component that isn't typed.
fn skip_value(r: &mut Reader<'_>, k: Kind, depth: u32) -> Result<(), WireError> {
    match k {
        // Types with no network codec: the persistent codec's NBT.
        Kind::CustomData
        | Kind::IntangibleProjectile
        | Kind::MapDecorations
        | Kind::DebugStickState
        | Kind::Recipes
        | Kind::Lock
        | Kind::ContainerLoot => nbt(r)?,
        // Text components.
        Kind::CustomName | Kind::ItemName => nbt(r)?,
        Kind::BucketEntityData => compound(r)?,
        Kind::Lore => skip_list(r, 256, nbt)?,
        // Units.
        Kind::CreativeSlotLock | Kind::Waxed => {}
        Kind::PotionDurationScale => {
            r.f32()?;
        }
        // One VarInt: a number, an enum id or a registry id.
        Kind::DamageType
        | Kind::Rarity
        | Kind::RepairCost
        | Kind::Enchantable
        | Kind::AdditionalTradeCost
        | Kind::VillagerFood
        | Kind::Dye
        | Kind::MapId
        | Kind::MapPostProcessing
        | Kind::OminousBottleAmplifier
        | Kind::VillagerVariant
        | Kind::WolfVariant
        | Kind::WolfSoundVariant
        | Kind::WolfCollar
        | Kind::FoxVariant
        | Kind::SalmonSize
        | Kind::ParrotVariant
        | Kind::TropicalFishPattern
        | Kind::TropicalFishBaseColor
        | Kind::TropicalFishPatternColor
        | Kind::MooshroomVariant
        | Kind::RabbitVariant
        | Kind::PigVariant
        | Kind::PigSoundVariant
        | Kind::CowVariant
        | Kind::CowSoundVariant
        | Kind::ChickenVariant
        | Kind::ChickenSoundVariant
        | Kind::ZombieNautilusVariant
        | Kind::FrogVariant
        | Kind::HorseVariant
        | Kind::LlamaVariant
        | Kind::AxolotlVariant
        | Kind::CatVariant
        | Kind::CatSoundVariant
        | Kind::CatCollar
        | Kind::SheepColor
        | Kind::ShulkerColor
        | Kind::ProvidesPotteryPattern
        | Kind::CushionColor => {
            r.varint()?;
        }
        Kind::TooltipStyle | Kind::NoteBlockSound => {
            r.identifier()?;
        }
        Kind::AttributeModifiers => skip_list(r, usize::MAX, |r| {
            // Attribute, modifier id, amount, operation, slot group, display.
            r.varint()?;
            r.identifier()?;
            r.f64()?;
            r.varint()?;
            r.varint()?;
            // Display: default (0) and hidden (1) carry nothing, an override (2) its
            // text; ids out of range fall back to default.
            if r.varint()? == 2 {
                nbt(r)?;
            }
            Ok(())
        })?,
        Kind::CustomModelData => {
            skip_list(r, usize::MAX, |r| r.f32().map(drop))?;
            skip_list(r, usize::MAX, |r| r.bool().map(drop))?;
            skip_list(r, usize::MAX, |r| r.string(MAX_STRING).map(drop))?;
            skip_list(r, usize::MAX, |r| r.i32().map(drop))?;
        }
        Kind::TooltipDisplay => {
            r.bool()?;
            skip_list(r, usize::MAX, |r| r.varint().map(drop))?;
        }
        Kind::DamageResistant | Kind::Repairable | Kind::ProvidesBannerPatterns => holder_set(r)?,
        Kind::SuspiciousStewEffects => skip_list(r, usize::MAX, |r| {
            r.varint()?;
            r.varint().map(drop)
        })?,
        Kind::WritableBookContent => {
            skip_list(r, 100, |r| filterable(r, |r| r.string(1024).map(drop)))?
        }
        Kind::WrittenBookContent => {
            filterable(r, |r| r.string(32).map(drop))?;
            r.string(MAX_STRING)?;
            r.varint()?;
            skip_list(r, usize::MAX, |r| filterable(r, nbt))?;
            r.bool()?;
        }
        Kind::ProvidesTrimMaterial => trim_material(r)?,
        Kind::EntityData | Kind::BlockEntityData => {
            r.varint()?;
            compound(r)?;
        }
        Kind::JukeboxPlayable => {
            holder(r, |r| {
                // Sound, description, length in seconds, comparator output.
                sound(r)?;
                nbt(r)?;
                r.f32()?;
                r.varint().map(drop)
            })?;
        }
        Kind::LodestoneTracker => {
            opt(r, |r| {
                r.identifier()?;
                r.i64().map(drop)
            })?;
            r.bool()?;
        }
        Kind::FireworkExplosion => firework_explosion(r)?,
        Kind::Fireworks => {
            r.varint()?;
            skip_list(r, 256, firework_explosion)?;
        }
        Kind::PotDecorations => {
            for _ in 0..4 {
                opt(r, |r| read_template(r, depth))?;
            }
        }
        Kind::Bees => skip_list(r, usize::MAX, |r| {
            // Entity type and data, ticks in the hive, minimum ticks.
            r.varint()?;
            compound(r)?;
            r.varint()?;
            r.varint().map(drop)
        })?,
        Kind::SulfurCubeContent => {
            read_template(r, depth)?;
        }
        Kind::BreakSound => sound(r)?,
        Kind::Compostable => resolvable_int(r)?,
        Kind::CookingFuel | Kind::BrewingFuel => {
            resolvable_int(r)?;
            resolvable_float(r)?;
        }
        Kind::MobVisibility => {
            holder_set(r)?;
            r.f32()?;
        }
        Kind::PaintingVariant => {
            holder(r, |r| {
                // Width, height, asset, optional title and author.
                r.varint()?;
                r.varint()?;
                r.identifier()?;
                opt(r, nbt)?;
                opt(r, nbt).map(drop)
            })?;
        }
        Kind::SignTextFront | Kind::SignTextBack => {
            // Four messages, optionally four filtered ones, the colour, glowing.
            for _ in 0..4 {
                nbt(r)?;
            }
            opt(r, |r| (0..4).try_for_each(|_| nbt(r)))?;
            r.varint()?;
            r.bool()?;
        }
        // The typed ones are read by `read_component`.
        Kind::MaxStackSize
        | Kind::MaxDamage
        | Kind::Damage
        | Kind::Unbreakable
        | Kind::UseEffects
        | Kind::MinimumAttackCharge
        | Kind::Enchantments
        | Kind::StoredEnchantments
        | Kind::Food
        | Kind::Consumable
        | Kind::UseRemainder
        | Kind::UseCooldown
        | Kind::Weapon
        | Kind::AttackRange
        | Kind::Equippable
        | Kind::Glider
        | Kind::DeathProtection
        | Kind::BlocksAttacks
        | Kind::PiercingWeapon
        | Kind::KineticWeapon
        | Kind::AttackAnimation
        | Kind::InteractAnimation
        | Kind::ChargedProjectiles
        | Kind::BundleContents
        | Kind::Container
        | Kind::PotionContents
        | Kind::ItemModel
        | Kind::DyedColor
        | Kind::EnchantmentGlintOverride
        | Kind::Trim
        | Kind::BaseColor
        | Kind::BannerPatterns
        | Kind::Profile
        | Kind::Instrument
        | Kind::BlockTransformer
        | Kind::Tool
        | Kind::CanBreak
        | Kind::CanPlaceOn
        | Kind::BlockState => {
            read_component(r, k.id() as i32, depth)?;
        }
    }
    Ok(())
}

/// `Some` value if a true boolean comes first (`ByteBufCodecs.optional`).
fn opt<T>(
    r: &mut Reader<'_>,
    f: impl FnOnce(&mut Reader<'_>) -> Result<T, WireError>,
) -> Result<Option<T>, WireError> {
    if r.bool()? { f(r).map(Some) } else { Ok(None) }
}

/// A VarInt count of at most `max`, then that many values.
fn list<T>(
    r: &mut Reader<'_>,
    max: usize,
    mut f: impl FnMut(&mut Reader<'_>) -> Result<T, WireError>,
) -> Result<Vec<T>, WireError> {
    let n = r.count(max)?;
    let mut out = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

fn skip_list(
    r: &mut Reader<'_>,
    max: usize,
    mut f: impl FnMut(&mut Reader<'_>) -> Result<(), WireError>,
) -> Result<(), WireError> {
    let n = r.count(max)?;
    for _ in 0..n {
        f(r)?;
    }
    Ok(())
}

/// One network NBT tag; the end tag (vanilla's null) is refused, as `ByteBufCodecs.TAG`
/// does.
fn nbt(r: &mut Reader<'_>) -> Result<(), WireError> {
    match r.nbt()? {
        Some(_) => Ok(()),
        None => Err(WireError::Invalid("missing NBT tag")),
    }
}

/// One network NBT compound (`ByteBufCodecs.COMPOUND_TAG`).
fn compound(r: &mut Reader<'_>) -> Result<(), WireError> {
    match r.nbt()? {
        Some(crate::nbt::Tag::Compound(_)) => Ok(()),
        _ => Err(WireError::Invalid("expected an NBT compound")),
    }
}

/// A holder: `Holder::Id` for a registry entry, or the inline value read by `inline`.
fn holder(
    r: &mut Reader<'_>,
    inline: impl FnOnce(&mut Reader<'_>) -> Result<(), WireError>,
) -> Result<Holder, WireError> {
    match r.varint()? {
        0 => {
            inline(r)?;
            Ok(Holder::Inline)
        }
        n => Ok(Holder::Id(n - 1)),
    }
}

/// A holder of which an inline value's asset and decal flag are kept.
fn trim_part(
    r: &mut Reader<'_>,
    inline: impl FnOnce(&mut Reader<'_>) -> Result<(String, bool), WireError>,
) -> Result<TrimPart, WireError> {
    match r.varint()? {
        0 => {
            let (asset, decal) = inline(r)?;
            Ok(TrimPart::Inline { asset, decal })
        }
        n => Ok(TrimPart::Id(n - 1)),
    }
}

/// A holder set: a tag, or a list of registry ids.
fn holder_set(r: &mut Reader<'_>) -> Result<(), WireError> {
    read_holder_set(r).map(drop)
}

/// A holder set (`ByteBufCodecs.holderSet`): 0 and a tag, or the count plus one and
/// the ids.
fn read_holder_set(r: &mut Reader<'_>) -> Result<HolderSet, WireError> {
    match r.varint()? {
        0 => Ok(HolderSet::Tag(r.identifier()?)),
        n if n > 0 => {
            let n = (n - 1) as usize;
            let mut ids = Vec::with_capacity(n.min(256));
            for _ in 0..n {
                ids.push(r.varint()?);
            }
            Ok(HolderSet::Ids(ids))
        }
        _ => Err(WireError::Invalid("holder set size")),
    }
}

/// `SoundEvent.STREAM_CODEC`: a holder whose inline value is the sound's identifier and
/// an optional fixed range.
fn sound(r: &mut Reader<'_>) -> Result<(), WireError> {
    holder(r, |r| {
        r.identifier()?;
        opt(r, |r| r.f32()).map(drop)
    })
    .map(drop)
}

fn enchantments(r: &mut Reader<'_>) -> Result<Vec<(i32, i32)>, WireError> {
    // A map from a registry holder (the plain id) to the level.
    list(r, usize::MAX, |r| Ok((r.varint()?, r.varint()?)))
}

fn swing_animation(r: &mut Reader<'_>) -> Result<SwingAnimation, WireError> {
    Ok(SwingAnimation {
        kind: r.varint()?,
        duration: r.varint()?,
    })
}

/// `MobEffectInstance.STREAM_CODEC`: the effect id, then its details.
fn mob_effect(r: &mut Reader<'_>, depth: u32) -> Result<(), WireError> {
    r.varint()?;
    mob_effect_details(r, depth)
}

/// Amplifier, duration, ambient, particles, icon, then optionally the hidden effect's
/// details, recursively.
fn mob_effect_details(r: &mut Reader<'_>, depth: u32) -> Result<(), WireError> {
    r.varint()?;
    r.varint()?;
    r.bool()?;
    r.bool()?;
    r.bool()?;
    let depth = deeper(depth)?;
    opt(r, |r| mob_effect_details(r, depth)).map(drop)
}

/// `ConsumeEffect.STREAM_CODEC`: the `minecraft:consume_effect_type` id, then that
/// type's fields.
fn consume_effect(r: &mut Reader<'_>, depth: u32) -> Result<(), WireError> {
    match r.varint()? {
        // apply_effects: the effects and a probability.
        0 => {
            skip_list(r, usize::MAX, |r| mob_effect(r, depth))?;
            r.f32()?;
        }
        // remove_effects.
        1 => holder_set(r)?,
        // clear_all_effects.
        2 => {}
        // teleport_randomly: the diameter and whether particles follow the direction.
        3 => {
            r.f32()?;
            r.bool()?;
        }
        // play_sound.
        4 => sound(r)?,
        _ => return Err(WireError::Invalid("consume effect type")),
    }
    Ok(())
}

/// `BlockPredicate.STREAM_CODEC` (an adventure-mode predicate's entries): optional
/// blocks, optional state properties, optional NBT, then the component matchers.
fn block_predicate(r: &mut Reader<'_>, depth: u32) -> Result<BlockPredicate, WireError> {
    let blocks = opt(r, read_holder_set)?;
    let properties = opt(r, |r| {
        list(r, usize::MAX, |r| {
            let name = r.string(MAX_STRING)?.to_owned();
            // Either an exact value, or an optional minimum and maximum.
            let value = if r.bool()? {
                ValueMatcher::Exact(r.string(MAX_STRING)?.to_owned())
            } else {
                ValueMatcher::Range {
                    min: opt(r, |r| r.string(MAX_STRING).map(str::to_owned))?,
                    max: opt(r, |r| r.string(MAX_STRING).map(str::to_owned))?,
                }
            };
            Ok(PropertyMatcher { name, value })
        })
    })?;
    let nbt_condition = opt(r, compound)?.is_some();
    // `DataComponentMatchers`: exact components (type id and value)...
    let depth = deeper(depth)?;
    let exact = list(r, usize::MAX, |r| {
        let kind = r.varint()?;
        read_component(r, kind, depth).map(drop)
    })?;
    // ...and partial predicates: a predicate type (true) or a component type (false),
    // its id, and the predicate as NBT.
    let partial = list(r, 64, |r| {
        r.bool()?;
        r.varint()?;
        nbt(r)
    })?;
    Ok(BlockPredicate {
        blocks,
        properties,
        nbt: nbt_condition,
        components: !exact.is_empty() || !partial.is_empty(),
    })
}

/// `Filterable.streamCodec`: the raw value, then optionally the filtered one.
fn filterable(
    r: &mut Reader<'_>,
    mut f: impl FnMut(&mut Reader<'_>) -> Result<(), WireError>,
) -> Result<(), WireError> {
    f(r)?;
    if r.bool()? {
        f(r)?;
    }
    Ok(())
}

/// A trim material holder: inline, its palette id and description.
fn trim_material(r: &mut Reader<'_>) -> Result<(), WireError> {
    holder(r, |r| {
        r.identifier()?;
        nbt(r)
    })
    .map(drop)
}

/// Shape, colours, fade colours, trail, twinkle.
fn firework_explosion(r: &mut Reader<'_>) -> Result<(), WireError> {
    r.varint()?;
    skip_list(r, usize::MAX, |r| r.i32().map(drop))?;
    skip_list(r, usize::MAX, |r| r.i32().map(drop))?;
    r.bool()?;
    r.bool()?;
    Ok(())
}

/// `ResolvableInt`: a constant (true) or a context provider's key (false).
fn resolvable_int(r: &mut Reader<'_>) -> Result<(), WireError> {
    if r.bool()? {
        r.i32()?;
    } else {
        r.identifier()?;
    }
    Ok(())
}

fn resolvable_float(r: &mut Reader<'_>) -> Result<(), WireError> {
    if r.bool()? {
        r.f32()?;
    } else {
        r.identifier()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::{self, Compound, Tag};
    use crate::wire;

    /// Builds bytes as the server writes them.
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        fn v(mut self, v: i32) -> W {
            wire::write_varint(&mut self.0, v);
            self
        }
        fn kind(self, k: Kind) -> W {
            self.v(k.id() as i32)
        }
        fn f(mut self, v: f32) -> W {
            wire::write_f32(&mut self.0, v);
            self
        }
        fn i(mut self, v: i32) -> W {
            wire::write_i32(&mut self.0, v);
            self
        }
        fn b(mut self, v: bool) -> W {
            wire::write_bool(&mut self.0, v);
            self
        }
        fn s(mut self, v: &str) -> W {
            wire::write_string(&mut self.0, v);
            self
        }
        fn uuid(mut self, v: u128) -> W {
            wire::write_uuid(&mut self.0, v);
            self
        }
        fn text(mut self, v: &str) -> W {
            nbt::write_network(&mut self.0, &Tag::String(v.into())).unwrap();
            self
        }
        fn compound(mut self) -> W {
            let mut c = Compound::new();
            c.put("x", Tag::Int(1));
            nbt::write_network(&mut self.0, &Tag::Compound(c)).unwrap();
            self
        }
        fn raw(mut self, b: &[u8]) -> W {
            self.0.extend_from_slice(b);
            self
        }
    }

    /// Reads one optional stack and checks every byte was used.
    fn stack(w: W) -> ItemStack {
        let mut r = Reader::new(&w.0);
        let s = ItemStack::read_optional(&mut r).unwrap();
        r.finish().unwrap();
        s
    }

    /// A stack of one item 800 with a patch adding these components.
    fn with(n: i32, components: W) -> W {
        W::default().v(1).v(800).v(n).v(0).raw(&components.0)
    }

    #[test]
    fn empty_and_plain_stacks() {
        assert_eq!(stack(W::default().v(0)), ItemStack::EMPTY);
        assert!(stack(W::default().v(0)).is_empty());
        // Any count of zero or less is the empty stack, with nothing after it.
        assert_eq!(stack(W::default().v(-3)), ItemStack::EMPTY);
        let s = stack(W::default().v(5).v(42).v(0).v(0));
        assert_eq!((s.item, s.count, s.patch.is_empty()), (42, 5, true));
        assert!(!s.is_empty());
        // The non-optional codec refuses the empty stack.
        assert!(ItemStack::read(&mut Reader::new(&[0])).is_err());
    }

    #[test]
    fn typed_components() {
        let s = stack(with(
            6,
            W::default()
                .kind(Kind::MaxStackSize)
                .v(16)
                .kind(Kind::Food)
                .v(4)
                .f(2.4)
                .b(true)
                .kind(Kind::UseCooldown)
                .f(1.5)
                .b(true)
                .s("ender_pearl")
                .kind(Kind::Weapon)
                .v(1)
                .f(5.0)
                .kind(Kind::AttackAnimation)
                .v(2)
                .v(6)
                .kind(Kind::Equippable)
                .v(4) // head
                .v(0) // inline sound
                .s("item.armor.equip_generic")
                .b(true)
                .f(16.0)
                .b(true) // asset id
                .s("minecraft:iron")
                .b(false) // camera overlay
                .b(true) // allowed entities: a tag
                .v(0)
                .s("minecraft:skeletons")
                .b(true) // dispensable
                .b(true)
                .b(false)
                .b(true)
                .b(false)
                .v(12), // shearing sound by id
        ));
        let get = |k| s.get(k).cloned();
        assert_eq!(get(Kind::MaxStackSize), Some(Component::MaxStackSize(16)));
        assert_eq!(
            get(Kind::Food),
            Some(Component::Food {
                nutrition: 4,
                saturation: 2.4,
                can_always_eat: true
            })
        );
        assert_eq!(
            get(Kind::UseCooldown),
            Some(Component::UseCooldown {
                seconds: 1.5,
                cooldown_group: Some("minecraft:ender_pearl".into())
            })
        );
        assert_eq!(
            get(Kind::AttackAnimation),
            Some(Component::AttackAnimation(SwingAnimation {
                kind: 2,
                duration: 6
            }))
        );
        assert_eq!(
            get(Kind::Equippable),
            Some(Component::Equippable {
                slot: 4,
                asset: Some("minecraft:iron".into()),
                swappable: true,
                damage_on_hurt: false,
                equip_on_interact: true,
                can_be_sheared: false,
            })
        );
        assert!(get(Kind::Weapon).is_some());
    }

    #[test]
    fn consumable_with_effects_and_inline_sound() {
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Consumable)
                .f(1.6)
                .v(1) // eat
                .v(0) // inline sound
                .s("entity.generic.eat")
                .b(false)
                .b(true)
                .v(3) // three effects
                // apply_effects: regeneration II with a hidden weaker one, then the
                // probability.
                .v(0)
                .v(1)
                .v(9)
                .v(1)
                .v(100)
                .b(false)
                .b(true)
                .b(true)
                .b(true)
                .v(0)
                .v(600)
                .b(false)
                .b(true)
                .b(true)
                .b(false)
                .f(1.0)
                // remove_effects: a list of two effects.
                .v(1)
                .v(3)
                .v(18)
                .v(19)
                // play_sound by id.
                .v(4)
                .v(31)
                .kind(Kind::Damage)
                .v(3),
        ));
        assert_eq!(
            s.get(Kind::Consumable),
            Some(&Component::Consumable {
                consume_seconds: 1.6,
                animation: 1,
                has_consume_particles: true
            })
        );
        assert_eq!(s.get(Kind::Damage), Some(&Component::Damage(3)));
    }

    #[test]
    fn removed_components() {
        let s = stack(
            W::default()
                .v(1)
                .v(7)
                .v(1)
                .v(2)
                .kind(Kind::Unbreakable)
                .kind(Kind::Damage)
                .kind(Kind::Food),
        );
        assert_eq!(s.patch.added, [(4, Component::Unbreakable)]);
        assert_eq!(s.patch.removed, [3, 23]);
        assert!(s.patch.removes(Kind::Food) && !s.patch.removes(Kind::Glider));
    }

    #[test]
    fn nested_stacks() {
        // A bundle holding a damaged sword (a template: item, count, patch) and three
        // arrows, then a use remainder.
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::BundleContents)
                .v(2)
                .v(900)
                .v(1)
                .v(1)
                .v(0)
                .kind(Kind::Damage)
                .v(12)
                .v(901)
                .v(3)
                .v(0)
                .v(0)
                .kind(Kind::UseRemainder)
                .v(902)
                .v(1)
                .v(0)
                .v(0),
        ));
        let Some(Component::BundleContents(items)) = s.get(Kind::BundleContents) else {
            panic!("{s:?}")
        };
        assert_eq!((items[0].item, items[0].count), (900, 1));
        assert_eq!(items[0].get(Kind::Damage), Some(&Component::Damage(12)));
        assert_eq!((items[1].item, items[1].count), (901, 3));
        let Some(Component::UseRemainder(r)) = s.get(Kind::UseRemainder) else {
            panic!()
        };
        assert_eq!(r.item, 902);

        // A shulker box: a slot list where empty slots are absent templates.
        let s = stack(with(
            1,
            W::default()
                .kind(Kind::Container)
                .v(3)
                .b(true)
                .v(1)
                .v(64)
                .v(0)
                .v(0)
                .b(false)
                .b(true)
                .v(2)
                .v(1)
                .v(0)
                .v(0),
        ));
        let Some(Component::Container(slots)) = s.get(Kind::Container) else {
            panic!()
        };
        assert_eq!(slots.len(), 3);
        assert!(slots[1].is_empty() && slots[2].item == 2);

        // A stack in an item frame's entity data value nests the same way.
        let s = stack(with(
            1,
            W::default()
                .kind(Kind::ChargedProjectiles)
                .v(1)
                .v(5)
                .v(1)
                .v(1)
                .v(0)
                .kind(Kind::Glider),
        ));
        assert!(
            matches!(s.get(Kind::ChargedProjectiles), Some(Component::ChargedProjectiles(v)) if v.len() == 1)
        );
    }

    #[test]
    fn text_component_then_another() {
        let s = stack(with(
            3,
            W::default()
                .kind(Kind::CustomName)
                .text("Excalibur")
                .kind(Kind::Lore)
                .v(2)
                .text("one")
                .compound()
                .kind(Kind::Damage)
                .v(9),
        ));
        let Some(Component::Other { kind, data }) = s.get(Kind::CustomName) else {
            panic!()
        };
        assert_eq!(*kind, 6);
        // The raw value is the tag as sent: type 8, length, the string.
        assert_eq!(data[..3], [8, 0, 9]);
        assert_eq!(s.get(Kind::Damage), Some(&Component::Damage(9)));
    }

    #[test]
    fn holders_inline_and_by_id() {
        let s = stack(with(
            1,
            W::default()
                .kind(Kind::Instrument)
                .v(0)
                .v(0)
                .s("item.goat_horn.sound.0")
                .b(false)
                .f(7.0)
                .f(256.0)
                .v(1)
                .text("Ponder"),
        ));
        assert_eq!(
            s.get(Kind::Instrument),
            Some(&Component::Instrument(Holder::Inline))
        );
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Instrument)
                .v(4)
                .kind(Kind::BreakSound)
                .v(0)
                .s("entity.item.break")
                .b(true)
                .f(8.0),
        ));
        assert_eq!(
            s.get(Kind::Instrument),
            Some(&Component::Instrument(Holder::Id(3)))
        );
        // Enchantments are registry holders: plain ids, no offset.
        let s = stack(with(
            1,
            W::default()
                .kind(Kind::Enchantments)
                .v(2)
                .v(0)
                .v(5)
                .v(12)
                .v(1),
        ));
        assert_eq!(
            s.get(Kind::Enchantments),
            Some(&Component::Enchantments(vec![(0, 5), (12, 1)]))
        );
        // A trim: an inline material then a pattern by id.
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Trim)
                .v(0)
                .s("gold")
                .text("Gold")
                .v(3)
                .kind(Kind::Glider),
        ));
        assert_eq!(
            s.get(Kind::Trim),
            Some(&Component::Trim {
                material: TrimPart::Inline {
                    asset: "minecraft:gold".into(),
                    decal: false
                },
                pattern: TrimPart::Id(2),
            })
        );
        assert_eq!(s.get(Kind::Glider), Some(&Component::Glider));
    }

    #[test]
    fn what_a_stack_looks_like_is_read() {
        let s = stack(with(
            7,
            W::default()
                .kind(Kind::ItemModel)
                .s("minecraft:stick")
                .kind(Kind::DyedColor)
                .i(0x12_3456)
                .kind(Kind::EnchantmentGlintOverride)
                .b(true)
                .kind(Kind::BaseColor)
                .v(14)
                // A pattern by id in red, then one written out in white, a decal no
                // banner has.
                .kind(Kind::BannerPatterns)
                .v(2)
                .v(5)
                .v(14)
                .v(0)
                .s("example:star")
                .s("block.example.banner.star")
                .v(0)
                .kind(Kind::PotionContents)
                .b(true)
                .v(7)
                .b(true)
                .i(0xAB_CDEF)
                .v(0)
                .b(true)
                .s("fizz")
                // A trim with a pattern written out, which is a decal.
                .kind(Kind::Trim)
                .v(4)
                .v(0)
                .s("example:lines")
                .text("Lines")
                .b(true),
        ));
        let get = |k| s.get(k).cloned();
        assert_eq!(
            get(Kind::ItemModel),
            Some(Component::ItemModel("minecraft:stick".into()))
        );
        assert_eq!(get(Kind::DyedColor), Some(Component::DyedColor(0x12_3456)));
        assert_eq!(
            get(Kind::EnchantmentGlintOverride),
            Some(Component::EnchantmentGlintOverride(true))
        );
        assert_eq!(get(Kind::BaseColor), Some(Component::BaseColor(14)));
        assert_eq!(
            get(Kind::BannerPatterns),
            Some(Component::BannerPatterns(vec![
                (TrimPart::Id(4), 14),
                (
                    TrimPart::Inline {
                        asset: "example:star".into(),
                        decal: false
                    },
                    0
                ),
            ]))
        );
        assert_eq!(
            get(Kind::PotionContents),
            Some(Component::PotionContents {
                potion: Some(7),
                custom_color: Some(0xAB_CDEF),
                custom_name: Some("fizz".into()),
            })
        );
        assert_eq!(
            get(Kind::Trim),
            Some(Component::Trim {
                material: TrimPart::Id(3),
                pattern: TrimPart::Inline {
                    asset: "example:lines".into(),
                    decal: true
                },
            })
        );
    }

    #[test]
    fn holder_sets_tag_and_list() {
        let s = stack(with(
            3,
            W::default()
                .kind(Kind::Repairable)
                .v(0)
                .s("minecraft:wooden_tool_materials")
                .kind(Kind::DamageResistant)
                .v(3)
                .v(10)
                .v(11)
                .kind(Kind::Tool)
                .v(1)
                .v(1) // an empty list of blocks
                .b(true)
                .f(8.0)
                .b(false)
                .f(1.0)
                .v(1)
                .b(true),
        ));
        assert_eq!(
            s.get(Kind::Tool),
            Some(&Component::Tool {
                rules: vec![ToolRule {
                    blocks: HolderSet::Ids(vec![]),
                    speed: Some(8.0),
                    correct_for_drops: None,
                }],
                default_mining_speed: 1.0,
                damage_per_block: 1,
                can_destroy_blocks_in_creative: true,
            })
        );
        let Some(Component::Other { data, .. }) = s.get(Kind::Repairable) else {
            panic!()
        };
        assert_eq!(data[0], 0);
        let Some(Component::Other { data, .. }) = s.get(Kind::DamageResistant) else {
            panic!()
        };
        assert_eq!(data[..], [3, 10, 11]);
    }

    #[test]
    fn profiles() {
        // A full profile with a property, and a slim model.
        stack(with(
            2,
            W::default()
                .kind(Kind::Profile)
                .b(true)
                .uuid(7)
                .s("Steve")
                .v(1)
                .s("textures")
                .s("e30=")
                .b(true)
                .s("sig")
                .b(false)
                .b(true)
                .s("minecraft:entity/player/cape")
                .b(false)
                .b(true)
                .b(true)
                .kind(Kind::Glider),
        ));
        // A partial one: a name only.
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Profile)
                .b(false)
                .b(true)
                .s("Alex")
                .b(false)
                .v(0)
                .raw(&[0, 0, 0, 0])
                .kind(Kind::Damage)
                .v(1),
        ));
        assert_eq!(s.get(Kind::Damage), Some(&Component::Damage(1)));
    }

    #[test]
    fn adventure_predicates() {
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::CanBreak)
                .v(1)
                // Blocks: two ids.
                .b(true)
                .v(3)
                .v(1)
                .v(2)
                // State properties: one exact, one ranged with only a minimum.
                .b(true)
                .v(2)
                .s("facing")
                .b(true)
                .s("north")
                .s("age")
                .b(false)
                .b(true)
                .s("3")
                .b(false)
                // NBT.
                .b(true)
                .compound()
                // Exact components: damage 0 and an enchantment list.
                .v(2)
                .kind(Kind::Damage)
                .v(0)
                .kind(Kind::Enchantments)
                .v(1)
                .v(4)
                .v(2)
                // Partial predicates: a predicate type with its NBT, and "any value"
                // of a component type.
                .v(2)
                .b(true)
                .v(1)
                .compound()
                .b(false)
                .kind(Kind::Damage)
                .compound()
                .kind(Kind::Damage)
                .v(5),
        ));
        assert_eq!(s.get(Kind::Damage), Some(&Component::Damage(5)));
        assert_eq!(
            s.get(Kind::CanBreak),
            Some(&Component::CanBreak(vec![BlockPredicate {
                blocks: Some(HolderSet::Ids(vec![1, 2])),
                properties: Some(vec![
                    PropertyMatcher {
                        name: "facing".into(),
                        value: ValueMatcher::Exact("north".into()),
                    },
                    PropertyMatcher {
                        name: "age".into(),
                        value: ValueMatcher::Range {
                            min: Some("3".into()),
                            max: None,
                        },
                    },
                ]),
                nbt: true,
                components: true,
            }]))
        );
    }

    #[test]
    fn books() {
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::WrittenBookContent)
                .s("Title")
                .b(true)
                .s("T***e")
                .s("Author")
                .v(1)
                .v(2)
                .text("page one")
                .b(false)
                .text("page two")
                .b(true)
                .text("p*** two")
                .b(true)
                .kind(Kind::WritableBookContent)
                .v(1)
                .s("draft")
                .b(false),
        ));
        assert_eq!(s.patch.added.len(), 2);
    }

    #[test]
    fn fireworks() {
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Fireworks)
                .v(2)
                .v(1)
                .v(4) // creeper
                .v(2)
                .i(0xFF0000)
                .i(0x00FF00)
                .v(0)
                .b(true)
                .b(false)
                .kind(Kind::FireworkExplosion)
                .v(0)
                .v(0)
                .v(1)
                .i(0x123456)
                .b(false)
                .b(true),
        ));
        let Some(Component::Other { data, .. }) = s.get(Kind::Fireworks) else {
            panic!()
        };
        assert_eq!(data.len(), 1 + 1 + 1 + 1 + 8 + 1 + 2);
    }

    #[test]
    fn untyped_components_keep_their_bytes() {
        let s = stack(with(
            3,
            W::default()
                .kind(Kind::CustomData)
                .compound()
                .kind(Kind::SignTextFront)
                .text("a")
                .text("b")
                .text("c")
                .text("d")
                .b(false)
                .v(15)
                .b(true)
                .kind(Kind::PaintingVariant)
                .v(0)
                .v(2)
                .v(1)
                .s("kebab")
                .b(true)
                .text("Kebab")
                .b(false),
        ));
        assert_eq!(s.patch.added.len(), 3);
        let Some(Component::Other { data, .. }) = s.get(Kind::PaintingVariant) else {
            panic!()
        };
        assert_eq!(data[..3], [0, 2, 1]);
    }

    #[test]
    fn typed_components_keep_the_bytes_they_were_read_from() {
        let s = stack(with(
            2,
            W::default()
                .kind(Kind::Food)
                .v(4)
                .f(2.4)
                .b(true)
                .kind(Kind::CustomName)
                .text("x"),
        ));
        let food = W::default().v(4).f(2.4).b(true);
        assert_eq!(s.patch.raw.get(Kind::Food.id() as i32), Some(&food.0[..]));
        // An untyped one is its bytes already.
        assert_eq!(s.patch.raw.get(Kind::CustomName.id() as i32), None);
        // Patches compare by their values alone.
        let mut bare = s.patch.clone();
        bare.raw = RawValues::default();
        assert_eq!(bare, s.patch);
    }

    #[test]
    fn bad_input_is_refused() {
        // An unknown component type.
        let w = with(1, W::default().v(500));
        assert!(ItemStack::read_optional(&mut Reader::new(&w.0)).is_err());
        // A template of nothing.
        let w = with(1, W::default().kind(Kind::UseRemainder).v(0).v(1).v(0).v(0));
        assert!(ItemStack::read_optional(&mut Reader::new(&w.0)).is_err());
        // An unknown consume effect.
        let w = with(1, W::default().kind(Kind::DeathProtection).v(1).v(9));
        assert!(ItemStack::read_optional(&mut Reader::new(&w.0)).is_err());
        // Bundles nested past the limit.
        let mut w = W::default();
        for _ in 0..MAX_DEPTH + 1 {
            w = w.v(1).v(1).v(1).v(0).kind(Kind::BundleContents).v(1);
        }
        assert!(ItemStack::read_optional(&mut Reader::new(&w.0)).is_err());
    }
}
