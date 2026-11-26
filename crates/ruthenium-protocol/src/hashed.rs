//! The hashed stacks a container click carries (`HashedStack`, `HashedPatchMap`).
//!
//! `ServerboundContainerClickPacket` sends, for each slot the click changed and for the
//! carried item, the item, the count and a hash of each component the stack's patch
//! adds (and the types it removes). The server compares them with its own stacks and
//! sends back any slot that differs, so a wrong hash is a visible resync.
//!
//! The 26.3 client hashes a component as `type.codec().encodeStart(ops, value)` with
//! `ops` its registries' `RegistryOps` over `HashOps.CRC32C_INSTANCE`
//! (`ClientPacketListener.decoratedHashOpsGenerator`), and sends `HashCode.asInt()`. The
//! server's generator (`ServerPlayer`'s container synchroniser) is the same code over the
//! server's registries; both write a registry entry as its key, so they agree whenever
//! the keys do.
//!
//! `values` reads each component's network encoding and builds the hash its
//! persistent codec would write; `text` does text components; `nbt_form` the
//! values the network sends as NBT. A component the client can't encode (a transient
//! type such as `creative_slot_lock`, a number its codec refuses) makes vanilla throw
//! "Failed to hash" and crash; [`try_hash_stack`] reports it instead.
//!
//! The typed components of [`crate::item`] leave parts out (a consumable's effects, a
//! potion's custom effects...), so a patch keeps the bytes each was read from
//! ([`crate::item::RawValues`]) and a component is hashed from them while they still
//! read back as its value. A value built or changed by the client (a bundle a click
//! put an item in) is written out again from its typed value first.
//!
//! Not written as vanilla writes them (see `nbt_form`): booleans and int lists inside
//! the NBT-form values nested in `lock`, in adventure-mode predicates' partial
//! predicates and in hover events' items; a dialog written inline in a click event
//! (`HashError::Unsupported`).

mod enums;
mod nbt_form;
pub mod ops;
mod text;
mod values;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use ruthenium_data::v26_3::registries as builtin;
use ruthenium_data::v26_3::registries::DataComponentType as Kind;

use crate::item::{Component, ItemStack};
use crate::wire::{self, Reader, WireError};
use ops::{Hash, Rec};

/// Vanilla's `HashedStack.ActualItem`: an empty stack is `None` (written as the
/// optional's absent flag).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HashedStack {
    /// The `minecraft:item` id.
    pub item: i32,
    pub count: i32,
    /// (data component type id, hash), in the patch's order (vanilla's is an
    /// `IdentityHashMap`'s, which differs from run to run).
    pub added: Vec<(i32, i32)>,
    /// Data component type ids.
    pub removed: Vec<i32>,
}

impl HashedStack {
    /// `HashedStack.STREAM_CODEC`: whether there is a stack, then the item (a registry
    /// id), the count, the added components (type id and a big-endian int each) and
    /// the removed types.
    pub fn write(stack: Option<&HashedStack>, out: &mut Vec<u8>) {
        let Some(s) = stack else {
            wire::write_bool(out, false);
            return;
        };
        wire::write_bool(out, true);
        wire::write_varint(out, s.item);
        wire::write_varint(out, s.count);
        wire::write_varint(out, s.added.len() as i32);
        for &(kind, hash) in &s.added {
            wire::write_varint(out, kind);
            wire::write_i32(out, hash);
        }
        wire::write_varint(out, s.removed.len() as i32);
        for &kind in &s.removed {
            wire::write_varint(out, kind);
        }
    }
}

/// The keys of the registries the server synchronised in configuration (e.g.
/// `minecraft:enchantment`), by network id. Built-in registries (items, data component
/// types, mob effects, sound events, ...) are resolved here from ruthenium-data.
pub trait SyncedRegistries {
    fn key(&self, registry: &str, id: i32) -> Option<&str>;
}

/// Registry name to its entries' keys in id order, as the registry data packets list
/// them.
impl SyncedRegistries for HashMap<String, Vec<String>> {
    fn key(&self, registry: &str, id: i32) -> Option<&str> {
        let entries = self.get(registry)?;
        entries.get(usize::try_from(id).ok()?).map(String::as_str)
    }
}

/// Why a component can't be hashed.
#[derive(Debug, Clone, PartialEq)]
pub enum HashError {
    /// A type with no persistent codec (`creative_slot_lock`, `additional_trade_cost`,
    /// `map_post_processing`): vanilla's `encodeValue` fails and the client crashes.
    NotPersistent(&'static str),
    /// The codec refuses the value (a number out of its range, a string too long...):
    /// vanilla crashes with "Failed to hash".
    Encode(String),
    /// An id the registry doesn't have.
    UnknownId { registry: &'static str, id: i32 },
    /// The bytes don't read as the component.
    Wire(WireError),
    /// Something this module doesn't write (see its notes).
    Unsupported(&'static str),
}

impl From<WireError> for HashError {
    fn from(e: WireError) -> Self {
        HashError::Wire(e)
    }
}

impl std::fmt::Display for HashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashError::NotPersistent(k) => write!(f, "{k} is not a persistent component"),
            HashError::Encode(m) => write!(f, "the codec refuses the value: {m}"),
            HashError::UnknownId { registry, id } => write!(f, "no entry {id} in {registry}"),
            HashError::Wire(e) => write!(f, "{e:?}"),
            HashError::Unsupported(m) => write!(f, "can't hash {m}"),
        }
    }
}

impl std::error::Error for HashError {}

/// `HashedStack.create(stack, hasher)` as the 26.3 client computes it (`None` for an
/// empty stack). A component that can't be hashed (see [`HashError`]; vanilla would
/// crash) is sent with the hash 0, so the server corrects the slot.
pub fn hash_stack(stack: &ItemStack, registries: &dyn SyncedRegistries) -> Option<HashedStack> {
    if stack.is_empty() {
        return None;
    }
    let cx = Cx {
        regs: registries,
        depth: 0,
    };
    Some(HashedStack {
        item: stack.item,
        count: stack.count,
        added: stack
            .patch
            .added
            .iter()
            .map(|(kind, c)| {
                let h = cx.component(*kind, c, stack.patch.raw.get(*kind));
                (*kind, h.map(Hash::as_int).unwrap_or(0))
            })
            .collect(),
        removed: stack.patch.removed.clone(),
    })
}

/// [`hash_stack`], failing where vanilla's hasher throws.
pub fn try_hash_stack(
    stack: &ItemStack,
    registries: &dyn SyncedRegistries,
) -> Result<Option<HashedStack>, HashError> {
    if stack.is_empty() {
        return Ok(None);
    }
    let cx = Cx {
        regs: registries,
        depth: 0,
    };
    let mut added = Vec::with_capacity(stack.patch.added.len());
    for (kind, c) in &stack.patch.added {
        let h = cx.component(*kind, c, stack.patch.raw.get(*kind))?;
        added.push((*kind, h.as_int()));
    }
    Ok(Some(HashedStack {
        item: stack.item,
        count: stack.count,
        added,
        removed: stack.patch.removed.clone(),
    }))
}

/// One component's hash, as the client's generator gives it.
pub fn hash_component(
    kind: i32,
    value: &Component,
    registries: &dyn SyncedRegistries,
) -> Result<i32, HashError> {
    let cx = Cx {
        regs: registries,
        depth: 0,
    };
    cx.component(kind, value, None).map(Hash::as_int)
}

/// An enum the network sends by id: its names, and what an id out of range reads as
/// (`ByIdMap`).
pub(crate) enum IdEnum {
    Continuous(&'static [&'static str], Out),
    /// (id, name) pairs, and the name of any other id.
    Sparse(&'static [(i32, &'static str)], &'static str),
}

/// `ByIdMap.OutOfBoundsStrategy`.
pub(crate) enum Out {
    Zero,
    Clamp,
    Wrap,
}

impl IdEnum {
    pub(crate) fn name(&self, id: i32) -> &'static str {
        match self {
            IdEnum::Continuous(names, out) => {
                let n = names.len() as i32;
                let i = if (0..n).contains(&id) {
                    id
                } else {
                    match out {
                        Out::Zero => 0,
                        Out::Clamp => id.clamp(0, n - 1),
                        Out::Wrap => id.rem_euclid(n),
                    }
                };
                names[i as usize]
            }
            IdEnum::Sparse(pairs, default) => pairs
                .iter()
                .find(|(i, _)| *i == id)
                .map_or(*default, |(_, n)| n),
        }
    }
}

/// A registry a value refers to by id.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reg {
    Item,
    Sound,
    MobEffect,
    Potion,
    Attribute,
    EntityType,
    Block,
    BlockEntityType,
    VillagerType,
    ConsumeEffect,
    PredicateType,
    /// One the server sends (`minecraft:enchantment`, ...).
    Synced(&'static str),
}

impl Reg {
    fn builtin(self) -> Result<(&'static str, &'static [&'static str]), &'static str> {
        Ok(match self {
            Reg::Item => ("minecraft:item", builtin::Item::NAMES),
            Reg::Sound => ("minecraft:sound_event", builtin::SoundEvent::NAMES),
            Reg::MobEffect => ("minecraft:mob_effect", builtin::MobEffect::NAMES),
            Reg::Potion => ("minecraft:potion", builtin::Potion::NAMES),
            Reg::Attribute => ("minecraft:attribute", builtin::Attribute::NAMES),
            Reg::EntityType => ("minecraft:entity_type", builtin::EntityType::NAMES),
            Reg::Block => ("minecraft:block", builtin::Block::NAMES),
            Reg::BlockEntityType => (
                "minecraft:block_entity_type",
                builtin::BlockEntityType::NAMES,
            ),
            Reg::VillagerType => ("minecraft:villager_type", builtin::VillagerType::NAMES),
            Reg::ConsumeEffect => (
                "minecraft:consume_effect_type",
                builtin::ConsumeEffectType::NAMES,
            ),
            Reg::PredicateType => (
                "minecraft:data_component_predicate_type",
                enums::DATA_COMPONENT_PREDICATE_TYPE,
            ),
            Reg::Synced(name) => return Err(name),
        })
    }
}

/// What hashing needs as it goes: the registries, and how deeply values are nested.
pub(crate) struct Cx<'a> {
    regs: &'a dyn SyncedRegistries,
    depth: u32,
}

/// How deeply stacks may nest in each other's components.
const MAX_DEPTH: u32 = 64;

impl<'a> Cx<'a> {
    fn deeper(&self) -> Result<Cx<'a>, HashError> {
        if self.depth >= MAX_DEPTH {
            return Err(HashError::Wire(WireError::Invalid(
                "item data nested too deeply",
            )));
        }
        Ok(Cx {
            regs: self.regs,
            depth: self.depth + 1,
        })
    }

    /// The key of entry `id` of `reg`.
    pub(crate) fn key(&self, reg: Reg, id: i32) -> Result<&'a str, HashError> {
        match reg.builtin() {
            Ok((name, names)) => usize::try_from(id)
                .ok()
                .and_then(|i| names.get(i).copied())
                .ok_or(HashError::UnknownId { registry: name, id }),
            Err(name) => self
                .regs
                .key(name, id)
                .ok_or(HashError::UnknownId { registry: name, id }),
        }
    }

    pub(crate) fn key_hash(&self, reg: Reg, id: i32) -> Result<Hash, HashError> {
        Ok(ops::string(self.key(reg, id)?))
    }

    /// Checks a built-in registry has `key` (a decoder looks it up).
    pub(crate) fn key_id(&self, reg: Reg, key: &str) -> Result<(), HashError> {
        match reg.builtin() {
            Ok((name, names)) if !names.contains(&key) => Err(HashError::UnknownId {
                registry: name,
                id: -1,
            }),
            _ => Ok(()),
        }
    }

    /// A component of the stack's patch (`HashedPatchMap.create`): a transient type
    /// fails, as `encodeValue` does.
    fn component(
        &self,
        kind: i32,
        value: &Component,
        raw: Option<&[u8]>,
    ) -> Result<Hash, HashError> {
        let k = self::kind(kind)?;
        if is_transient(k) {
            return Err(HashError::NotPersistent(k.name()));
        }
        self.persistent(k, value, raw)
    }

    /// A component's value hashed by its codec, from the bytes it was read from when
    /// they still read back as `value`, else from `value` written out again.
    fn persistent(
        &self,
        k: Kind,
        value: &Component,
        raw: Option<&[u8]>,
    ) -> Result<Hash, HashError> {
        let owned;
        let bytes: &[u8] = match value {
            Component::Other { data, .. } => data,
            typed => match raw.filter(|r| crate::item::reads_as(k, r, typed)) {
                Some(r) => r,
                None => {
                    owned = crate::item::write_value(typed).map_err(HashError::Unsupported)?;
                    &owned
                }
            },
        };
        let mut r = Reader::new(bytes);
        let h = self.value(k, &mut r)?;
        r.finish()?;
        Ok(h)
    }

    /// `ItemStackTemplate.CODEC`: `{id, count, components}`, the count left out at 1 and
    /// the components when the patch is empty. In this nested patch, transient types
    /// are left out (`DataComponentPatch.CODEC`) and a removed type is `"!type": {}`.
    pub(crate) fn template_hash(&self, stack: &ItemStack) -> Result<Hash, HashError> {
        let cx = self.deeper()?;
        let mut rec = Rec::new();
        rec.put("id", self.key_hash(Reg::Item, stack.item)?);
        if stack.count != 1 {
            rec.put("count", values::int_in(stack.count, 1, 99)?);
        }
        let patch = &stack.patch;
        if !patch.is_empty() {
            let mut comps = Rec::new();
            for (kind, c) in &patch.added {
                let k = self::kind(*kind)?;
                if is_transient(k) {
                    continue;
                }
                comps.put(k.name(), cx.persistent(k, c, patch.raw.get(*kind))?);
            }
            for kind in &patch.removed {
                let k = self::kind(*kind)?;
                if is_transient(k) {
                    continue;
                }
                comps.put(&format!("!{}", k.name()), ops::empty_map());
            }
            rec.put("components", comps.build());
        }
        Ok(rec.build())
    }
}

fn kind(id: i32) -> Result<Kind, HashError> {
    u32::try_from(id)
        .ok()
        .and_then(Kind::from_id)
        .ok_or(HashError::UnknownId {
            registry: "minecraft:data_component_type",
            id,
        })
}

/// Types with a network codec and no persistent one.
fn is_transient(k: Kind) -> bool {
    matches!(
        k,
        Kind::CreativeSlotLock | Kind::AdditionalTradeCost | Kind::MapPostProcessing
    )
}
