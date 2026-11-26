//! Each data component's value, read from its network encoding and hashed the way its
//! persistent codec writes it (`DataComponents`, each type's `CODEC`).
//!
//! The network sends what the stream codec writes; the client keeps the decoded value
//! and hashes `codec.encodeStart(registryOps(HashOps), value)`. So each reader below
//! follows the stream codec field by field and builds the codec's output: field names,
//! optional fields left out at their defaults (`optionalFieldOf(name, default)` drops a
//! value `equals` to the default, and so skips its validation), validated numbers that
//! make the encoder fail (vanilla's "Failed to hash" crash), holders written as their
//! registry key, holder sets as `#tag`, one key, or a list of keys.

use std::cmp::Ordering;

use ruthenium_data::v26_3::registries::DataComponentType as Kind;

use super::enums::{self};
use super::ops::{self, Hash, Rec};
use super::{Cx, HashError, IdEnum, Reg, nbt_form};
use crate::nbt::Tag;
use crate::wire::{MAX_STRING, Reader};

pub(super) type Res = Result<Hash, HashError>;

fn encode_err<T>(what: impl Into<String>) -> Result<T, HashError> {
    Err(HashError::Encode(what.into()))
}

// --- Numbers, as the validating codecs in ExtraCodecs and DFU check them on encode. ---

/// `intRange(lo, hi)`, `NON_NEGATIVE_INT`, `POSITIVE_INT`.
pub(super) fn int_in(v: i32, lo: i32, hi: i32) -> Res {
    if v < lo || v > hi {
        return encode_err(format!("int {v} not in [{lo}; {hi}]"));
    }
    Ok(ops::int(v))
}

pub(super) fn non_neg(v: i32) -> Res {
    int_in(v, 0, i32::MAX)
}

pub(super) fn positive(v: i32) -> Res {
    int_in(v, 1, i32::MAX)
}

/// `Float.compare`: -0.0 is below 0.0, NaN above everything and equal to itself.
pub(super) fn jcmp(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let bits = |f: f32| {
        if f.is_nan() {
            0x7fc0_0000
        } else {
            f.to_bits() as i32
        }
    };
    bits(a).cmp(&bits(b))
}

/// `Float.equals` (what a record's equality and `Objects.equals` use).
pub(super) fn feq(a: f32, b: f32) -> bool {
    jcmp(a, b) == Ordering::Equal
}

/// A float codec checked with `compareTo` against an inclusive range.
pub(super) fn float_in(v: f32, lo: f32, hi: f32) -> Res {
    if jcmp(v, lo) == Ordering::Less || jcmp(v, hi) == Ordering::Greater {
        return encode_err(format!("float {v} not in [{lo}; {hi}]"));
    }
    Ok(ops::float(v))
}

pub(super) fn non_neg_f(v: f32) -> Res {
    float_in(v, 0.0, f32::MAX)
}

/// `POSITIVE_FLOAT`: above zero (exclusive), at most `Float.MAX_VALUE`.
pub(super) fn positive_f(v: f32) -> Res {
    if jcmp(v, 0.0) != Ordering::Greater || jcmp(v, f32::MAX) == Ordering::Greater {
        return encode_err(format!("float {v} not positive"));
    }
    Ok(ops::float(v))
}

/// `UNSIGNED_BYTE`: written as a byte; above 255 is refused (a negative int wraps).
pub(super) fn unsigned_byte(v: i32) -> Res {
    if v > 255 {
        return encode_err(format!("unsigned byte {v}"));
    }
    Ok(ops::byte(v as i8))
}

/// `Codec.string(lo, hi)`: a length in UTF-16 units.
pub(super) fn string_in(s: &str, lo: usize, hi: usize) -> Res {
    let n = s.encode_utf16().count();
    if n < lo || n > hi {
        return encode_err(format!("string of {n} units not in [{lo}; {hi}]"));
    }
    Ok(ops::string(s))
}

/// An enum sent by id and written by name (`ByteBufCodecs.idMapper` over `ByIdMap`).
pub(super) fn id_enum(e: &IdEnum, id: i32) -> Hash {
    ops::string(e.name(id))
}

// --- Small readers. ---

pub(super) fn s<'a>(r: &mut Reader<'a>) -> Result<&'a str, HashError> {
    Ok(r.string(MAX_STRING)?)
}

/// `ByteBufCodecs.optional`.
pub(super) fn opt<T>(
    r: &mut Reader<'_>,
    f: impl FnOnce(&mut Reader<'_>) -> Result<T, HashError>,
) -> Result<Option<T>, HashError> {
    if r.bool()? { f(r).map(Some) } else { Ok(None) }
}

/// `ByteBufCodecs.list(max)`.
pub(super) fn list<T>(
    r: &mut Reader<'_>,
    max: usize,
    mut f: impl FnMut(&mut Reader<'_>) -> Result<T, HashError>,
) -> Result<Vec<T>, HashError> {
    let n = r.count(max)?;
    let mut out = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

/// A list of hashes, as `listOf()` writes it.
pub(super) fn hlist(r: &mut Reader<'_>, max: usize, f: impl FnMut(&mut Reader<'_>) -> Res) -> Res {
    Ok(ops::list(list(r, max, f)?))
}

pub(super) fn tag(r: &mut Reader<'_>) -> Result<Tag, HashError> {
    r.nbt()?
        .ok_or(HashError::Wire(crate::wire::WireError::Invalid(
            "missing NBT tag",
        )))
}

/// `BlockPos.STREAM_CODEC` (a packed long) as `BlockPos.CODEC` writes it: `[x, y, z]`.
fn block_pos(r: &mut Reader<'_>) -> Res {
    let l = r.i64()?;
    let x = (l >> 38) as i32;
    let y = ((l << 52) >> 52) as i32;
    let z = ((l << 26) >> 38) as i32;
    Ok(ops::int_list([x, y, z]))
}

/// `UUIDUtil.CODEC`: four ints, most significant first.
pub(super) fn uuid_hash(u: u128) -> Hash {
    ops::int_list([
        (u >> 96) as u32 as i32,
        (u >> 64) as u32 as i32,
        (u >> 32) as u32 as i32,
        u as u32 as i32,
    ])
}

impl Cx<'_> {
    /// A holder (`ByteBufCodecs.holder`): 0 and the value written out, or the id plus
    /// one. A registry entry is written as its key; an inline one by `inline`.
    pub(super) fn holder(
        &self,
        r: &mut Reader<'_>,
        reg: Reg,
        inline: impl FnOnce(&mut Reader<'_>, &Self) -> Res,
    ) -> Res {
        match r.varint()? {
            0 => inline(r, self),
            n => self.key_hash(reg, n - 1),
        }
    }

    /// A holder whose codec (`RegistryFixedCodec`) can't write a value inline.
    fn fixed_holder(
        &self,
        r: &mut Reader<'_>,
        reg: Reg,
        skip: impl FnOnce(&mut Reader<'_>, &Self) -> Res,
    ) -> Res {
        self.holder(r, reg, |r, cx| {
            skip(r, cx)?;
            encode_err("an inline value of a registry that can't be written inline")
        })
    }

    /// `ByteBufCodecs.holderRegistry`: the plain id, written as the key.
    pub(super) fn reg_key(&self, r: &mut Reader<'_>, reg: Reg) -> Res {
        let id = r.varint()?;
        self.key_hash(reg, id)
    }

    /// A holder set (`ByteBufCodecs.holderSet`) as `HolderSetCodec` writes it: `#tag`,
    /// or the entries' keys, one of them alone as itself.
    pub(super) fn holder_set(&self, r: &mut Reader<'_>, reg: Reg) -> Res {
        match r.varint()? {
            0 => Ok(ops::string(&format!("#{}", r.identifier()?))),
            n if n > 0 => {
                let mut keys = Vec::with_capacity((n as usize - 1).min(256));
                for _ in 0..n - 1 {
                    let id = r.varint()?;
                    keys.push(self.key_hash(reg, id)?);
                }
                Ok(compact(keys))
            }
            _ => Err(HashError::Wire(crate::wire::WireError::Invalid(
                "holder set size",
            ))),
        }
    }

    /// `SoundEvent.STREAM_CODEC`.
    pub(super) fn sound(&self, r: &mut Reader<'_>) -> Res {
        self.holder(r, Reg::Sound, |r, _| sound_direct(r))
    }

    /// A sound field with a default (`optionalFieldOf(name, SoundEvents.X)`): `None` when
    /// it is that registry entry (the same holder object, so an inline copy differs).
    fn sound_or_default(
        &self,
        r: &mut Reader<'_>,
        default: &str,
    ) -> Result<Option<Hash>, HashError> {
        match r.varint()? {
            0 => sound_direct(r).map(Some),
            n => {
                let key = self.key(Reg::Sound, n - 1)?;
                Ok((key != default).then(|| ops::string(key)))
            }
        }
    }

    pub(super) fn text(&self, r: &mut Reader<'_>) -> Res {
        super::text::hash(&tag(r)?, self)
    }

    /// `ItemStackTemplate.STREAM_CODEC`, written by `ItemStackTemplate.CODEC`.
    pub(super) fn template(&self, r: &mut Reader<'_>) -> Res {
        let stack = crate::item::read_template_at(r, self.depth)?;
        self.template_hash(&stack)
    }

    /// `MobEffectInstance.STREAM_CODEC`.
    fn mob_effect(&self, r: &mut Reader<'_>) -> Res {
        let id = self.reg_key(r, Reg::MobEffect)?;
        let mut rec = Rec::new();
        rec.put("id", id).extend(self.effect_details(r, 0)?);
        Ok(rec.build())
    }

    /// `MobEffectInstance.Details`: every field but `show_icon` left out at its default.
    fn effect_details(&self, r: &mut Reader<'_>, depth: u32) -> Result<Rec, HashError> {
        if depth > 64 {
            return Err(HashError::Wire(crate::wire::WireError::Invalid(
                "effects nested too deeply",
            )));
        }
        let amplifier = r.varint()?;
        let duration = r.varint()?;
        let ambient = r.bool()?;
        let show_particles = r.bool()?;
        let show_icon = r.bool()?;
        let hidden = opt(r, |r| self.effect_details(r, depth + 1))?;
        let mut rec = Rec::new();
        if amplifier != 0 {
            rec.put("amplifier", unsigned_byte(amplifier)?);
        }
        if duration != 0 {
            rec.put("duration", ops::int(duration));
        }
        if ambient {
            rec.put("ambient", ops::boolean(true));
        }
        if !show_particles {
            rec.put("show_particles", ops::boolean(false));
        }
        rec.put("show_icon", ops::boolean(show_icon));
        rec.opt("hidden_effect", hidden.map(Rec::build));
        Ok(rec)
    }

    /// `ConsumeEffect.STREAM_CODEC`, written as `{type, ...fields}`.
    fn consume_effect(&self, r: &mut Reader<'_>) -> Res {
        let id = r.varint()?;
        let ty = self.key(Reg::ConsumeEffect, id)?;
        let mut rec = Rec::new();
        rec.put("type", ops::string(ty));
        match ty {
            "minecraft:apply_effects" => {
                rec.put("effects", hlist(r, usize::MAX, |r| self.mob_effect(r))?);
                let p = r.f32()?;
                if !feq(p, 1.0) {
                    rec.put("probability", float_in(p, 0.0, 1.0)?);
                }
            }
            "minecraft:remove_effects" => {
                rec.put("effects", self.holder_set(r, Reg::MobEffect)?);
            }
            "minecraft:clear_all_effects" => {}
            "minecraft:teleport_randomly" => {
                let d = r.f32()?;
                if !feq(d, 16.0) {
                    rec.put("diameter", positive_f(d)?);
                }
                if !r.bool()? {
                    rec.put("directional_particles", ops::boolean(false));
                }
            }
            "minecraft:play_sound" => {
                rec.put("sound", self.sound(r)?);
            }
            _ => return Err(HashError::Unsupported("consume effect type")),
        }
        Ok(rec.build())
    }

    /// `BlockPredicate`, an adventure-mode predicate's entry.
    fn block_predicate(&self, r: &mut Reader<'_>) -> Res {
        let mut rec = Rec::new();
        rec.opt("blocks", opt(r, |r| self.holder_set(r, Reg::Block))?);
        rec.opt("state", opt(r, state_properties)?);
        // `NbtPredicate.CODEC` is `TagParser.LENIENT_CODEC`, which writes SNBT.
        rec.opt(
            "nbt",
            opt(r, |r| Ok(ops::string(&nbt_form::snbt(&tag(r)?))))?,
        );
        rec.extend(self.component_matchers(r)?);
        Ok(rec.build())
    }

    /// `DataComponentMatchers`: the exact components (`components`) and the partial
    /// predicates (`predicates`), each left out when empty.
    pub(super) fn component_matchers(&self, r: &mut Reader<'_>) -> Result<Rec, HashError> {
        let mut rec = Rec::new();
        // Exact: a list of typed components (`TypedDataComponent.STREAM_CODEC`), written
        // as a map of the persistent ones.
        let n = r.count(usize::MAX)?;
        let mut exact = Rec::new();
        let mut any = false;
        for _ in 0..n {
            let kind = r.varint()?;
            let k = super::kind(kind)?;
            let h = self.deeper()?.value(k, r)?;
            // The codec leaves transient types out.
            if super::is_transient(k) {
                continue;
            }
            exact.entry(ops::string(k.name()), h);
            any = true;
        }
        if any {
            rec.put("components", exact.build());
        }
        // Partial: (a predicate type or "any value" of a component type, its NBT).
        let n = r.count(64)?;
        let mut partial = Rec::new();
        for _ in 0..n {
            let predicate_type = r.bool()?;
            let id = r.varint()?;
            let value = tag(r)?;
            if predicate_type {
                let name = self.key(Reg::PredicateType, id)?;
                partial.entry(ops::string(name), nbt_form::predicate(name, &value, self)?);
            } else {
                let k = super::kind(id)?;
                partial.entry(ops::string(k.name()), ops::empty_map());
            }
        }
        if n > 0 {
            rec.put("predicates", partial.build());
        }
        Ok(rec)
    }

    /// `ResolvableProfile.STREAM_CODEC`, written by its `FULL_CODEC`.
    pub(super) fn profile(&self, r: &mut Reader<'_>) -> Res {
        let mut rec = Rec::new();
        if r.bool()? {
            // A whole game profile: id, name, properties.
            let id = r.uuid()?;
            let name = s(r)?.to_owned();
            let props = properties(r)?;
            rec.put("id", uuid_hash(id));
            rec.put("name", player_name(&name)?);
            if !props.is_empty() {
                rec.put("properties", properties_hash(&props)?);
            }
        } else {
            let name = opt(r, |r| Ok(s(r)?.to_owned()))?;
            let id = opt(r, |r| Ok(r.uuid()?))?;
            let props = properties(r)?;
            if let Some(name) = name {
                rec.put("name", player_name(&name)?);
            }
            rec.opt("id", id.map(uuid_hash));
            if !props.is_empty() {
                rec.put("properties", properties_hash(&props)?);
            }
        }
        // The skin patch: texture, cape, elytra, model.
        rec.opt("texture", opt(r, |r| Ok(ops::string(&r.identifier()?)))?);
        rec.opt("cape", opt(r, |r| Ok(ops::string(&r.identifier()?)))?);
        rec.opt("elytra", opt(r, |r| Ok(ops::string(&r.identifier()?)))?);
        rec.opt(
            "model",
            opt(r, |r| {
                Ok(ops::string(if r.bool()? { "slim" } else { "wide" }))
            })?,
        );
        Ok(rec.build())
    }

    /// The value of a component of type `kind` (`DataComponentType.streamCodec`), hashed
    /// by its persistent codec.
    pub(super) fn value(&self, kind: Kind, r: &mut Reader<'_>) -> Res {
        use Kind as K;
        Ok(match kind {
            // No network codec: the persistent codec's NBT.
            K::CustomData => custom_data(&tag(r)?)?,
            K::IntangibleProjectile => {
                tag(r)?;
                ops::empty_map()
            }
            K::MapDecorations => nbt_form::map_decorations(&tag(r)?)?,
            K::DebugStickState => nbt_form::debug_stick_state(&tag(r)?)?,
            K::Recipes => nbt_form::recipes(&tag(r)?)?,
            K::Lock => nbt_form::item_predicate(&tag(r)?, self)?,
            K::ContainerLoot => nbt_form::container_loot(&tag(r)?)?,
            // Transient types have a network codec and no persistent one.
            K::CreativeSlotLock => ops::empty(),
            K::AdditionalTradeCost | K::MapPostProcessing => {
                r.varint()?;
                ops::empty()
            }
            K::MaxStackSize => int_in(r.varint()?, 1, 99)?,
            K::MaxDamage => positive(r.varint()?)?,
            K::Damage | K::RepairCost => non_neg(r.varint()?)?,
            K::Unbreakable | K::Glider | K::Waxed => ops::empty_map(),
            K::UseEffects => {
                let mut rec = Rec::new();
                if r.bool()? {
                    rec.put("can_sprint", ops::boolean(true));
                }
                if !r.bool()? {
                    rec.put("interact_vibrations", ops::boolean(false));
                }
                let m = r.f32()?;
                if !feq(m, 0.2) {
                    rec.put("speed_multiplier", float_in(m, 0.0, 1.0)?);
                }
                rec.build()
            }
            K::CustomName | K::ItemName => self.text(r)?,
            K::MinimumAttackCharge => float_in(r.f32()?, 0.0, 1.0)?,
            K::DamageType => self.reg_key(r, Reg::Synced("minecraft:damage_type"))?,
            K::ItemModel | K::TooltipStyle | K::NoteBlockSound => ops::string(&r.identifier()?),
            K::Lore => hlist(r, 256, |r| self.text(r))?,
            K::Rarity => id_enum(&enums::RARITY, r.varint()?),
            K::Enchantments | K::StoredEnchantments => {
                let n = r.count(usize::MAX)?;
                let mut entries: Vec<(i32, i32)> = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    let (e, level) = (r.varint()?, r.varint()?);
                    // A map: a repeated enchantment keeps its last level.
                    match entries.iter_mut().find(|(k, _)| *k == e) {
                        Some(slot) => slot.1 = level,
                        None => entries.push((e, level)),
                    }
                }
                let mut rec = Rec::new();
                for (e, level) in entries {
                    rec.entry(
                        self.key_hash(Reg::Synced("minecraft:enchantment"), e)?,
                        int_in(level, 1, 255)?,
                    );
                }
                rec.build()
            }
            K::CanPlaceOn | K::CanBreak => {
                compact(list(r, usize::MAX, |r| self.deeper()?.block_predicate(r))?)
            }
            K::AttributeModifiers => hlist(r, usize::MAX, |r| {
                let mut rec = Rec::new();
                rec.put("type", self.reg_key(r, Reg::Attribute)?);
                rec.put("id", ops::string(&r.identifier()?));
                rec.put("amount", ops::double(r.f64()?));
                rec.put(
                    "operation",
                    id_enum(&enums::ATTRIBUTE_OPERATION, r.varint()?),
                );
                let slot = r.varint()?;
                if enums::EQUIPMENT_SLOT_GROUP.name(slot) != "any" {
                    rec.put("slot", id_enum(&enums::EQUIPMENT_SLOT_GROUP, slot));
                }
                match DISPLAY.name(r.varint()?) {
                    "default" => {}
                    "hidden" => {
                        let mut d = Rec::new();
                        d.put("type", ops::string("hidden"));
                        rec.put("display", d.build());
                    }
                    _ => {
                        let mut d = Rec::new();
                        d.put("type", ops::string("override"));
                        d.put("value", self.text(r)?);
                        rec.put("display", d.build());
                    }
                }
                Ok(rec.build())
            })?,
            K::CustomModelData => {
                let mut rec = Rec::new();
                let floats = list(r, usize::MAX, |r| Ok(ops::float(r.f32()?)))?;
                let flags = list(r, usize::MAX, |r| Ok(ops::boolean(r.bool()?)))?;
                let strings = list(r, usize::MAX, |r| Ok(ops::string(s(r)?)))?;
                let colors = list(r, usize::MAX, |r| Ok(ops::int(r.i32()?)))?;
                for (name, v) in [
                    ("floats", floats),
                    ("flags", flags),
                    ("strings", strings),
                    ("colors", colors),
                ] {
                    if !v.is_empty() {
                        rec.put(name, ops::list(v));
                    }
                }
                rec.build()
            }
            K::TooltipDisplay => {
                let mut rec = Rec::new();
                if r.bool()? {
                    rec.put("hide_tooltip", ops::boolean(true));
                }
                // A linked set: a repeated type keeps its first place.
                let ids = list(r, usize::MAX, |r| Ok(r.varint()?))?;
                let mut seen = Vec::new();
                for id in ids {
                    if !seen.contains(&id) {
                        seen.push(id);
                    }
                }
                if !seen.is_empty() {
                    let mut keys = Vec::new();
                    for id in seen {
                        keys.push(ops::string(super::kind(id)?.name()));
                    }
                    rec.put("hidden_components", ops::list(keys));
                }
                rec.build()
            }
            K::EnchantmentGlintOverride => ops::boolean(r.bool()?),
            K::Food => {
                let mut rec = Rec::new();
                rec.put("nutrition", non_neg(r.varint()?)?);
                rec.put("saturation", ops::float(r.f32()?));
                if r.bool()? {
                    rec.put("can_always_eat", ops::boolean(true));
                }
                rec.build()
            }
            K::Consumable => {
                let mut rec = Rec::new();
                let secs = r.f32()?;
                if !feq(secs, 1.6) {
                    rec.put("consume_seconds", non_neg_f(secs)?);
                }
                let anim = enums::ITEM_USE_ANIMATION.name(r.varint()?);
                if anim != "eat" {
                    rec.put("animation", ops::string(anim));
                }
                rec.opt(
                    "sound",
                    self.sound_or_default(r, "minecraft:entity.generic.eat")?,
                );
                if !r.bool()? {
                    rec.put("has_consume_particles", ops::boolean(false));
                }
                let effects = list(r, usize::MAX, |r| self.consume_effect(r))?;
                if !effects.is_empty() {
                    rec.put("on_consume_effects", ops::list(effects));
                }
                rec.build()
            }
            K::UseRemainder | K::SulfurCubeContent => self.template(r)?,
            K::UseCooldown => {
                let mut rec = Rec::new();
                rec.put("seconds", positive_f(r.f32()?)?);
                rec.opt(
                    "cooldown_group",
                    opt(r, |r| Ok(ops::string(&r.identifier()?)))?,
                );
                rec.build()
            }
            K::DamageResistant => {
                let mut rec = Rec::new();
                rec.put(
                    "types",
                    self.holder_set(r, Reg::Synced("minecraft:damage_type"))?,
                );
                rec.build()
            }
            K::Tool => {
                let mut rec = Rec::new();
                rec.put(
                    "rules",
                    hlist(r, usize::MAX, |r| {
                        let mut rule = Rec::new();
                        rule.put("blocks", self.holder_set(r, Reg::Block)?);
                        rule.opt("speed", opt(r, |r| positive_f(r.f32()?))?);
                        rule.opt(
                            "correct_for_drops",
                            opt(r, |r| Ok(ops::boolean(r.bool()?)))?,
                        );
                        Ok(rule.build())
                    })?,
                );
                let speed = r.f32()?;
                if !feq(speed, 1.0) {
                    rec.put("default_mining_speed", ops::float(speed));
                }
                let damage = r.varint()?;
                if damage != 1 {
                    rec.put("damage_per_block", non_neg(damage)?);
                }
                if !r.bool()? {
                    rec.put("can_destroy_blocks_in_creative", ops::boolean(false));
                }
                rec.build()
            }
            K::Weapon => {
                let mut rec = Rec::new();
                let damage = r.varint()?;
                if damage != 1 {
                    rec.put("item_damage_per_attack", non_neg(damage)?);
                }
                let secs = r.f32()?;
                if !feq(secs, 0.0) {
                    rec.put("disable_blocking_for_seconds", non_neg_f(secs)?);
                }
                rec.build()
            }
            K::AttackRange => {
                let mut rec = Rec::new();
                for (name, default, hi) in [
                    ("min_reach", 0.0, 64.0),
                    ("max_reach", 3.0, 64.0),
                    ("min_creative_reach", 0.0, 64.0),
                    ("max_creative_reach", 5.0, 64.0),
                    ("hitbox_margin", 0.3, 1.0),
                    ("mob_factor", 1.0, 2.0),
                ] {
                    let v = r.f32()?;
                    if !feq(v, default) {
                        rec.put(name, float_in(v, 0.0, hi)?);
                    }
                }
                rec.build()
            }
            K::Enchantable => {
                let mut rec = Rec::new();
                rec.put("value", positive(r.varint()?)?);
                rec.build()
            }
            K::Equippable => {
                let mut rec = Rec::new();
                rec.put("slot", id_enum(&enums::EQUIPMENT_SLOT, r.varint()?));
                rec.opt(
                    "equip_sound",
                    self.sound_or_default(r, "minecraft:item.armor.equip_generic")?,
                );
                rec.opt("asset_id", opt(r, |r| Ok(ops::string(&r.identifier()?)))?);
                rec.opt(
                    "camera_overlay",
                    opt(r, |r| Ok(ops::string(&r.identifier()?)))?,
                );
                rec.opt(
                    "allowed_entities",
                    opt(r, |r| self.holder_set(r, Reg::EntityType))?,
                );
                for (name, default) in [
                    ("dispensable", true),
                    ("swappable", true),
                    ("damage_on_hurt", true),
                    ("equip_on_interact", false),
                    ("can_be_sheared", false),
                ] {
                    let v = r.bool()?;
                    if v != default {
                        rec.put(name, ops::boolean(v));
                    }
                }
                rec.opt(
                    "shearing_sound",
                    self.sound_or_default(r, "minecraft:item.shears.snip")?,
                );
                rec.build()
            }
            K::Repairable => {
                let mut rec = Rec::new();
                rec.put("items", self.holder_set(r, Reg::Item)?);
                rec.build()
            }
            K::DeathProtection => {
                let mut rec = Rec::new();
                let effects = list(r, usize::MAX, |r| self.consume_effect(r))?;
                if !effects.is_empty() {
                    rec.put("death_effects", ops::list(effects));
                }
                rec.build()
            }
            K::BlocksAttacks => {
                let mut rec = Rec::new();
                let delay = r.f32()?;
                if !feq(delay, 0.0) {
                    rec.put("block_delay_seconds", non_neg_f(delay)?);
                }
                let scale = r.f32()?;
                if !feq(scale, 1.0) {
                    rec.put("disable_cooldown_scale", non_neg_f(scale)?);
                }
                // Damage reductions; the default is the one 90 degree, full reduction.
                let reductions = list(r, usize::MAX, |r| {
                    let angle = r.f32()?;
                    let ty = opt(r, |r| {
                        self.holder_set(r, Reg::Synced("minecraft:damage_type"))
                    })?;
                    let base = r.f32()?;
                    let factor = r.f32()?;
                    let is_default =
                        feq(angle, 90.0) && ty.is_none() && feq(base, 0.0) && feq(factor, 1.0);
                    let mut d = Rec::new();
                    if !feq(angle, 90.0) {
                        d.put("horizontal_blocking_angle", positive_f(angle)?);
                    }
                    d.opt("type", ty);
                    d.put("base", ops::float(base));
                    d.put("factor", ops::float(factor));
                    Ok((is_default, d.build()))
                })?;
                if !(reductions.len() == 1 && reductions[0].0) {
                    rec.put(
                        "damage_reductions",
                        ops::list(reductions.into_iter().map(|(_, h)| h)),
                    );
                }
                let (threshold, base, factor) = (r.f32()?, r.f32()?, r.f32()?);
                if !(feq(threshold, 1.0) && feq(base, 0.0) && feq(factor, 1.0)) {
                    let mut d = Rec::new();
                    d.put("threshold", non_neg_f(threshold)?);
                    d.put("base", ops::float(base));
                    d.put("factor", ops::float(factor));
                    rec.put("item_damage", d.build());
                }
                rec.opt(
                    "bypassed_by",
                    opt(r, |r| {
                        self.holder_set(r, Reg::Synced("minecraft:damage_type"))
                    })?,
                );
                rec.opt("block_sound", opt(r, |r| self.sound(r))?);
                rec.opt("disabled_sound", opt(r, |r| self.sound(r))?);
                rec.build()
            }
            K::PiercingWeapon => {
                let mut rec = Rec::new();
                if !r.bool()? {
                    rec.put("deals_knockback", ops::boolean(false));
                }
                if r.bool()? {
                    rec.put("dismounts", ops::boolean(true));
                }
                rec.opt("sound", opt(r, |r| self.sound(r))?);
                rec.opt("hit_sound", opt(r, |r| self.sound(r))?);
                rec.build()
            }
            K::KineticWeapon => {
                let mut rec = Rec::new();
                let cooldown = r.varint()?;
                if cooldown != 10 {
                    rec.put("contact_cooldown_ticks", non_neg(cooldown)?);
                }
                let delay = r.varint()?;
                if delay != 0 {
                    rec.put("delay_ticks", non_neg(delay)?);
                }
                for name in [
                    "dismount_conditions",
                    "knockback_conditions",
                    "damage_conditions",
                ] {
                    let c = opt(r, |r| {
                        let mut c = Rec::new();
                        c.put("max_duration_ticks", non_neg(r.varint()?)?);
                        let min_speed = r.f32()?;
                        if !feq(min_speed, 0.0) {
                            c.put("min_speed", ops::float(min_speed));
                        }
                        let min_relative = r.f32()?;
                        if !feq(min_relative, 0.0) {
                            c.put("min_relative_speed", ops::float(min_relative));
                        }
                        Ok(c.build())
                    })?;
                    rec.opt(name, c);
                }
                let forward = r.f32()?;
                if !feq(forward, 0.0) {
                    rec.put("forward_movement", ops::float(forward));
                }
                let mult = r.f32()?;
                if !feq(mult, 1.0) {
                    rec.put("damage_multiplier", ops::float(mult));
                }
                rec.opt("sound", opt(r, |r| self.sound(r))?);
                rec.opt("hit_sound", opt(r, |r| self.sound(r))?);
                rec.build()
            }
            K::AttackAnimation | K::InteractAnimation => {
                let mut rec = Rec::new();
                let ty = enums::SWING_ANIMATION_TYPE.name(r.varint()?);
                if ty != "whack" {
                    rec.put("type", ops::string(ty));
                }
                let duration = r.varint()?;
                if duration != 6 {
                    rec.put("duration", non_neg(duration)?);
                }
                rec.build()
            }
            K::BlockTransformer => self.reg_key(r, Reg::Synced("minecraft:block_transformer"))?,
            K::VillagerFood => {
                let mut rec = Rec::new();
                rec.put("nutrition", positive(r.varint()?)?);
                rec.build()
            }
            K::Dye
            | K::BaseColor
            | K::WolfCollar
            | K::TropicalFishBaseColor
            | K::TropicalFishPatternColor
            | K::CatCollar
            | K::SheepColor
            | K::ShulkerColor
            | K::CushionColor => id_enum(&enums::DYE_COLOR, r.varint()?),
            K::DyedColor => ops::int(r.i32()?),
            K::MapId => ops::int(r.varint()?),
            K::ChargedProjectiles => hlist(r, 1024, |r| self.template(r))?,
            K::BundleContents => hlist(r, usize::MAX, |r| self.template(r))?,
            K::Container => {
                let slots = list(r, 256, |r| opt(r, |r| self.template(r)))?;
                ops::list(slots.into_iter().enumerate().filter_map(|(i, item)| {
                    let item = item?;
                    let mut rec = Rec::new();
                    rec.put("slot", ops::int(i as i32));
                    rec.put("item", item);
                    Some(rec.build())
                }))
            }
            K::PotionContents => {
                let mut rec = Rec::new();
                rec.opt("potion", opt(r, |r| self.reg_key(r, Reg::Potion))?);
                rec.opt("custom_color", opt(r, |r| Ok(ops::int(r.i32()?)))?);
                let effects = list(r, usize::MAX, |r| self.mob_effect(r))?;
                if !effects.is_empty() {
                    rec.put("custom_effects", ops::list(effects));
                }
                rec.opt("custom_name", opt(r, |r| Ok(ops::string(s(r)?)))?);
                rec.build()
            }
            K::PotionDurationScale => non_neg_f(r.f32()?)?,
            K::SuspiciousStewEffects => hlist(r, usize::MAX, |r| {
                let mut rec = Rec::new();
                rec.put("id", self.reg_key(r, Reg::MobEffect)?);
                let duration = r.varint()?;
                if duration != 160 {
                    rec.put("duration", ops::int(duration));
                }
                Ok(rec.build())
            })?,
            K::WritableBookContent => {
                let pages = list(r, 100, |r| {
                    let raw = r.string(1024)?.to_owned();
                    let filtered = opt(r, |r| Ok(r.string(1024)?.to_owned()))?;
                    let mut rec = Rec::new();
                    rec.put("raw", string_in(&raw, 0, 1024)?);
                    if let Some(f) = filtered {
                        rec.put("filtered", string_in(&f, 0, 1024)?);
                    }
                    Ok(rec.build())
                })?;
                let mut rec = Rec::new();
                if !pages.is_empty() {
                    rec.put("pages", ops::list(pages));
                }
                rec.build()
            }
            K::WrittenBookContent => {
                let mut rec = Rec::new();
                let raw = r.string(32)?.to_owned();
                let filtered = opt(r, |r| Ok(r.string(32)?.to_owned()))?;
                let mut title = Rec::new();
                title.put("raw", string_in(&raw, 0, 32)?);
                if let Some(f) = filtered {
                    title.put("filtered", string_in(&f, 0, 32)?);
                }
                rec.put("title", title.build());
                rec.put("author", ops::string(s(r)?));
                let generation = r.varint()?;
                if generation != 0 {
                    rec.put("generation", int_in(generation, 0, 3)?);
                }
                let pages = list(r, usize::MAX, |r| {
                    let mut page = Rec::new();
                    page.put("raw", self.text(r)?);
                    page.opt("filtered", opt(r, |r| self.text(r))?);
                    Ok(page.build())
                })?;
                if !pages.is_empty() {
                    rec.put("pages", ops::list(pages));
                }
                if r.bool()? {
                    rec.put("resolved", ops::boolean(true));
                }
                rec.build()
            }
            K::Trim => {
                let mut rec = Rec::new();
                rec.put("material", self.trim_material(r)?);
                rec.put(
                    "pattern",
                    self.holder(r, Reg::Synced("minecraft:trim_pattern"), |r, cx| {
                        let mut rec = Rec::new();
                        rec.put("asset_id", ops::string(&r.identifier()?));
                        rec.put("description", cx.text(r)?);
                        rec.put("decal", ops::boolean(r.bool()?));
                        Ok(rec.build())
                    })?,
                );
                rec.build()
            }
            K::ProvidesTrimMaterial => self.trim_material(r)?,
            K::EntityData => {
                let ty = self.key(Reg::EntityType, r.varint()?)?;
                typed_entity_data(ty, &tag(r)?)?
            }
            K::BlockEntityData => {
                let ty = self.key(Reg::BlockEntityType, r.varint()?)?;
                typed_entity_data(ty, &tag(r)?)?
            }
            K::BucketEntityData => custom_data(&tag(r)?)?,
            K::Instrument => self.holder(r, Reg::Synced("minecraft:instrument"), |r, cx| {
                let mut rec = Rec::new();
                rec.put("sound_event", cx.sound(r)?);
                rec.put("use_duration", non_neg_f(r.f32()?)?);
                rec.put("range", positive_f(r.f32()?)?);
                let damage = r.varint()?;
                if damage != 0 {
                    rec.put("durability_damage", non_neg(damage)?);
                }
                rec.put("description", cx.text(r)?);
                Ok(rec.build())
            })?,
            K::OminousBottleAmplifier => int_in(r.varint()?, 0, 4)?,
            // `JukeboxSong.CODEC` is a fixed one: a song sent inline can't be written.
            K::JukeboxPlayable => {
                self.fixed_holder(r, Reg::Synced("minecraft:jukebox_song"), |r, cx| {
                    // Sound, description, length, comparator output.
                    cx.sound(r)?;
                    cx.text(r)?;
                    r.f32()?;
                    r.varint()?;
                    Ok(ops::empty())
                })?
            }
            K::ProvidesBannerPatterns => {
                self.holder_set(r, Reg::Synced("minecraft:banner_pattern"))?
            }
            K::LodestoneTracker => {
                let mut rec = Rec::new();
                rec.opt(
                    "target",
                    opt(r, |r| {
                        let mut pos = Rec::new();
                        pos.put("dimension", ops::string(&r.identifier()?));
                        pos.put("pos", block_pos(r)?);
                        Ok(pos.build())
                    })?,
                );
                if !r.bool()? {
                    rec.put("tracked", ops::boolean(false));
                }
                rec.build()
            }
            K::FireworkExplosion => firework_explosion(r)?,
            K::Fireworks => {
                let mut rec = Rec::new();
                let flight = r.varint()?;
                if flight != 0 {
                    rec.put("flight_duration", unsigned_byte(flight)?);
                }
                let explosions = list(r, 256, firework_explosion)?;
                if !explosions.is_empty() {
                    rec.put("explosions", ops::list(explosions));
                }
                rec.build()
            }
            K::Profile => self.profile(r)?,
            K::BannerPatterns => hlist(r, usize::MAX, |r| {
                let mut rec = Rec::new();
                rec.put(
                    "pattern",
                    self.holder(r, Reg::Synced("minecraft:banner_pattern"), |r, _| {
                        let mut rec = Rec::new();
                        rec.put("asset_id", ops::string(&r.identifier()?));
                        rec.put("translation_key", ops::string(s(r)?));
                        Ok(rec.build())
                    })?,
                );
                rec.put("color", id_enum(&enums::DYE_COLOR, r.varint()?));
                Ok(rec.build())
            })?,
            K::PotDecorations => {
                let mut rec = Rec::new();
                for side in ["back", "left", "right", "front"] {
                    rec.opt(side, opt(r, |r| self.template(r))?);
                }
                rec.build()
            }
            K::BlockState => {
                let n = r.count(usize::MAX)?;
                let mut entries: Vec<(String, String)> = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    let (k, v) = (s(r)?.to_owned(), s(r)?.to_owned());
                    match entries.iter_mut().find(|(ek, _)| *ek == k) {
                        Some(e) => e.1 = v,
                        None => entries.push((k, v)),
                    }
                }
                let mut rec = Rec::new();
                for (k, v) in entries {
                    rec.put(&k, ops::string(&v));
                }
                rec.build()
            }
            K::Bees => hlist(r, usize::MAX, |r| {
                let ty = self.key(Reg::EntityType, r.varint()?)?;
                let data = typed_entity_data(ty, &tag(r)?)?;
                let mut rec = Rec::new();
                rec.put("entity_data", data);
                rec.put("ticks_in_hive", ops::int(r.varint()?));
                rec.put("min_ticks_in_hive", ops::int(r.varint()?));
                Ok(rec.build())
            })?,
            K::BreakSound => self.sound(r)?,
            K::Compostable => {
                let mut rec = Rec::new();
                rec.put("layers", resolvable(r, false)?);
                rec.build()
            }
            K::CookingFuel => {
                let mut rec = Rec::new();
                rec.put("burn_time", resolvable(r, false)?);
                rec.put("speed_multiplier", resolvable(r, true)?);
                rec.build()
            }
            K::BrewingFuel => {
                let mut rec = Rec::new();
                rec.put("uses", resolvable(r, false)?);
                rec.put("speed_multiplier", resolvable(r, true)?);
                rec.build()
            }
            K::MobVisibility => {
                let mut rec = Rec::new();
                rec.put(
                    "targeting_entity_types",
                    self.holder_set(r, Reg::EntityType)?,
                );
                rec.put("visibility", float_in(r.f32()?, 0.0, 10.0)?);
                rec.build()
            }
            K::VillagerVariant => self.reg_key(r, Reg::VillagerType)?,
            K::WolfVariant => self.reg_key(r, Reg::Synced("minecraft:wolf_variant"))?,
            K::WolfSoundVariant => self.reg_key(r, Reg::Synced("minecraft:wolf_sound_variant"))?,
            K::PigVariant => self.reg_key(r, Reg::Synced("minecraft:pig_variant"))?,
            K::PigSoundVariant => self.reg_key(r, Reg::Synced("minecraft:pig_sound_variant"))?,
            K::CowVariant => self.reg_key(r, Reg::Synced("minecraft:cow_variant"))?,
            K::CowSoundVariant => self.reg_key(r, Reg::Synced("minecraft:cow_sound_variant"))?,
            K::ChickenVariant => self.reg_key(r, Reg::Synced("minecraft:chicken_variant"))?,
            K::ChickenSoundVariant => {
                self.reg_key(r, Reg::Synced("minecraft:chicken_sound_variant"))?
            }
            K::ZombieNautilusVariant => {
                self.reg_key(r, Reg::Synced("minecraft:zombie_nautilus_variant"))?
            }
            K::FrogVariant => self.reg_key(r, Reg::Synced("minecraft:frog_variant"))?,
            K::CatVariant => self.reg_key(r, Reg::Synced("minecraft:cat_variant"))?,
            K::CatSoundVariant => self.reg_key(r, Reg::Synced("minecraft:cat_sound_variant"))?,
            K::FoxVariant => id_enum(&enums::FOX_VARIANT, r.varint()?),
            K::SalmonSize => id_enum(&enums::SALMON_SIZE, r.varint()?),
            K::ParrotVariant => id_enum(&enums::PARROT_VARIANT, r.varint()?),
            K::TropicalFishPattern => id_enum(&enums::TROPICAL_FISH_PATTERN, r.varint()?),
            K::MooshroomVariant => id_enum(&enums::MOOSHROOM_VARIANT, r.varint()?),
            K::RabbitVariant => id_enum(&enums::RABBIT_VARIANT, r.varint()?),
            K::HorseVariant => id_enum(&enums::HORSE_VARIANT, r.varint()?),
            K::LlamaVariant => id_enum(&enums::LLAMA_VARIANT, r.varint()?),
            K::AxolotlVariant => id_enum(&enums::AXOLOTL_VARIANT, r.varint()?),
            K::PaintingVariant => {
                self.fixed_holder(r, Reg::Synced("minecraft:painting_variant"), |r, cx| {
                    // Width, height, asset, optional title and author.
                    r.varint()?;
                    r.varint()?;
                    r.identifier()?;
                    opt(r, |r| cx.text(r))?;
                    opt(r, |r| cx.text(r))?;
                    Ok(ops::empty())
                })?
            }
            K::ProvidesPotteryPattern => {
                self.reg_key(r, Reg::Synced("minecraft:decorated_pot_pattern"))?
            }
            K::SignTextFront | K::SignTextBack => {
                let lines = |r: &mut Reader<'_>| {
                    (0..4).map(|_| self.text(r)).collect::<Result<Vec<_>, _>>()
                };
                let messages = lines(r)?;
                let filtered = opt(r, lines)?;
                let mut rec = Rec::new();
                rec.put("messages", ops::list(messages.iter().copied()));
                // Filtered lines are written only when they differ from the raw ones
                // (`Component.equals`, compared here by what they write).
                if let Some(f) = filtered
                    && f != messages
                {
                    rec.put("filtered_messages", ops::list(f));
                }
                rec.put("color", id_enum(&enums::DYE_COLOR, r.varint()?));
                rec.put("has_glowing_text", ops::boolean(r.bool()?));
                rec.build()
            }
        })
    }

    /// `TrimMaterial.STREAM_CODEC`: a registry entry, or a palette and description.
    fn trim_material(&self, r: &mut Reader<'_>) -> Res {
        self.holder(r, Reg::Synced("minecraft:trim_material"), |r, cx| {
            let mut rec = Rec::new();
            rec.put("palette_id", ops::string(&r.identifier()?));
            rec.put("description", cx.text(r)?);
            Ok(rec.build())
        })
    }
}

/// `ItemAttributeModifiers.Display.Type`'s ids.
const DISPLAY: IdEnum = IdEnum::Continuous(&["default", "hidden", "override"], super::Out::Zero);

/// `ExtraCodecs.compactListCodec` (holder sets, adventure predicates): one element is
/// written as itself, anything else as a list.
pub(super) fn compact(mut items: Vec<Hash>) -> Hash {
    if items.len() == 1 {
        items.pop().unwrap_or_else(ops::empty)
    } else {
        ops::list(items)
    }
}

/// `SoundEvent.DIRECT_CODEC`: the sound's id and, if it has one, its fixed range.
fn sound_direct(r: &mut Reader<'_>) -> Res {
    let mut rec = Rec::new();
    rec.put("sound_id", ops::string(&r.identifier()?));
    rec.opt("range", opt(r, |r| Ok(ops::float(r.f32()?)))?);
    Ok(rec.build())
}

/// `StatePropertiesPredicate.STREAM_CODEC` as its map codec writes it.
fn state_properties(r: &mut Reader<'_>) -> Res {
    let n = r.count(usize::MAX)?;
    let mut rec = Rec::new();
    let mut names: Vec<String> = Vec::new();
    for _ in 0..n {
        let name = s(r)?.to_owned();
        let value = if r.bool()? {
            ops::string(s(r)?)
        } else {
            let mut range = Rec::new();
            range.opt("min", opt(r, |r| Ok(ops::string(s(r)?)))?);
            range.opt("max", opt(r, |r| Ok(ops::string(s(r)?)))?);
            range.build()
        };
        // `Collectors.toMap` throws on a repeated name.
        if names.contains(&name) {
            return encode_err(format!("duplicate state property {name}"));
        }
        rec.put(&name, value);
        names.push(name);
    }
    Ok(rec.build())
}

/// `FireworkExplosion.STREAM_CODEC`.
fn firework_explosion(r: &mut Reader<'_>) -> Res {
    let mut rec = Rec::new();
    rec.put("shape", id_enum(&enums::FIREWORK_SHAPE, r.varint()?));
    let colors = list(r, usize::MAX, |r| Ok(ops::int(r.i32()?)))?;
    if !colors.is_empty() {
        rec.put("colors", ops::list(colors));
    }
    let fade = list(r, usize::MAX, |r| Ok(ops::int(r.i32()?)))?;
    if !fade.is_empty() {
        rec.put("fade_colors", ops::list(fade));
    }
    if r.bool()? {
        rec.put("has_trail", ops::boolean(true));
    }
    if r.bool()? {
        rec.put("has_twinkle", ops::boolean(true));
    }
    Ok(rec.build())
}

/// `ResolvableInt` / `ResolvableFloat` (`ByteBufCodecs.either`): a constant, or a key.
fn resolvable(r: &mut Reader<'_>, float: bool) -> Res {
    if r.bool()? {
        Ok(if float {
            ops::float(r.f32()?)
        } else {
            ops::int(r.i32()?)
        })
    } else {
        Ok(ops::string(&r.identifier()?))
    }
}

/// A game profile's properties (`ByteBufCodecs.GAME_PROFILE_PROPERTIES`).
fn properties(r: &mut Reader<'_>) -> Result<Vec<(String, String, Option<String>)>, HashError> {
    list(r, 16, |r| {
        let name = s(r)?.to_owned();
        let value = s(r)?.to_owned();
        let signature = opt(r, |r| Ok(s(r)?.to_owned()))?;
        Ok((name, value, signature))
    })
}

/// `ExtraCodecs.PROPERTY_MAP`: the properties as a list, grouped by name in the order
/// the names first appear (a `PropertyMap` is a multimap).
fn properties_hash(props: &[(String, String, Option<String>)]) -> Res {
    let mut names: Vec<&str> = Vec::new();
    for (n, _, _) in props {
        if !names.contains(&n.as_str()) {
            names.push(n);
        }
    }
    let mut out = Vec::with_capacity(props.len());
    for name in names {
        for (n, v, sig) in props.iter().filter(|(n, _, _)| n == name) {
            let mut rec = Rec::new();
            rec.put("name", string_in(n, 0, 64)?);
            rec.put("value", string_in(v, 0, 32767)?);
            if let Some(sig) = sig {
                rec.put("signature", string_in(sig, 0, 1024)?);
            }
            out.push(rec.build());
        }
    }
    Ok(ops::list(out))
}

/// `ExtraCodecs.PLAYER_NAME`: at most 16 units, each a printable ASCII character.
pub(super) fn player_name(name: &str) -> Res {
    if name.encode_utf16().count() > 16 || !name.chars().all(|c| c > ' ' && c < '\u{7f}') {
        return encode_err(format!("player name {name:?}"));
    }
    Ok(ops::string(name))
}

/// `CustomData.CODEC`: the compound as it is (`CompoundTag.CODEC`).
fn custom_data(t: &Tag) -> Res {
    match t {
        Tag::Compound(_) => Ok(nbt_form::generic(t)),
        _ => Err(HashError::Unsupported("custom data that isn't a compound")),
    }
}

/// `TypedEntityData.codec`: the data with its type put back under `id`.
fn typed_entity_data(ty: &str, t: &Tag) -> Res {
    let Tag::Compound(c) = t else {
        return Err(HashError::Wire(crate::wire::WireError::Invalid(
            "expected an NBT compound",
        )));
    };
    let mut c = c.clone();
    c.put("id", Tag::String(ty.to_owned()));
    Ok(nbt_form::generic(&Tag::Compound(c)))
}
