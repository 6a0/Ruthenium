//! Block transformers (`BlockTransformer`, the `minecraft:block_transformer` registry):
//! what axes, shovels and hoes turn a block into when used on it, as data. Each
//! transformer is a list of transforms tried in order; a transform names the faces it
//! may not be used from and a block state provider that answers the new state, if any.
//!
//! The providers and block predicates are the world-generation ones
//! (`levelgen/feature/stateproviders`, `levelgen/blockpredicates`); the subset
//! vanilla's transformers use is read: rule-based providers, property-copying
//! providers and plain states, with `all_of`, `any_of`, `not`, `true`,
//! `matching_blocks` and `matching_block_tag` predicates. A transformer using
//! anything else is left out, so the client predicts nothing for it and waits for the
//! server.
//!
//! With vanilla's known packs the server sends the registry's entries without data and
//! the client takes them from the jar (`data/minecraft/block_transformer/*.json`);
//! otherwise each entry's data comes as NBT.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::configuration::RegistryEntry;
use serde_json::Value;

use crate::block::{Pos, state};
use crate::direction::Direction;
use crate::interact::World;

/// One registry entry: its transforms, in the order they are tried.
#[derive(Debug, Clone, PartialEq)]
pub struct Transformer {
    pub transforms: Vec<Transform>,
}

/// `BlockTransformData`, the parts the client acts on (the sound, particles, loot and
/// item damage are the server's).
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub provider: Provider,
    /// Faces the transform can't be used from (`disallowed_faces`).
    pub disallowed_faces: Vec<Direction>,
    /// The new state takes its neighbours into account first (`update_from_neighbors`,
    /// true by default).
    pub update_from_neighbors: bool,
    /// A stackable item is used up by one (`consume_on_use`, true by default).
    pub consume_on_use: bool,
}

/// A block state provider (`BlockStateProvider`).
#[derive(Debug, Clone, PartialEq)]
pub enum Provider {
    /// `SimpleStateProvider`, or a state written in place of a provider.
    State(u32),
    /// `RuleBasedStateProvider`: the first rule whose predicate holds and whose
    /// provider has a state, else the fallback's.
    Rules {
        rules: Vec<(Predicate, Provider)>,
        fallback: Option<Box<Provider>>,
    },
    /// `CopyPropertiesProvider`: the source's state with the properties it shares
    /// with the block that is there.
    CopyProperties(Box<Provider>),
}

/// A block predicate (`BlockPredicate`), tested at the block's position.
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    True,
    AllOf(Vec<Predicate>),
    AnyOf(Vec<Predicate>),
    Not(Box<Predicate>),
    /// `matching_blocks`: the block at the offset is one of these.
    Blocks {
        offset: Pos,
        blocks: BlockSet,
    },
    /// `matching_block_tag`: the block at the offset is in the tag.
    Tag {
        offset: Pos,
        tag: String,
    },
}

/// A set of blocks (`HolderSet<Block>`): a tag, or blocks by registry id.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockSet {
    Tag(String),
    Blocks(Vec<u32>),
}

impl Provider {
    /// `getOptionalState`: the state the provider gives at `pos`, if any.
    pub fn optional_state(&self, w: &World<'_>, pos: Pos) -> Option<u32> {
        match self {
            Provider::State(s) => Some(*s),
            Provider::Rules { rules, fallback } => rules
                .iter()
                .filter(|(when, _)| when.test(w, pos))
                .find_map(|(_, then)| then.optional_state(w, pos))
                .or_else(|| fallback.as_ref()?.optional_state(w, pos)),
            Provider::CopyProperties(_) => Some(self.state(w, pos)),
        }
    }

    /// `getState`: always a state; a rule-based provider with nothing to give
    /// leaves the block that is there.
    pub fn state(&self, w: &World<'_>, pos: Pos) -> u32 {
        match self {
            Provider::State(s) => *s,
            Provider::Rules { .. } => self.optional_state(w, pos).unwrap_or_else(|| w.get(pos)),
            Provider::CopyProperties(source) => {
                state::with_properties_of(source.state(w, pos), w.get(pos))
            }
        }
    }
}

impl Predicate {
    /// `test(level, pos)`.
    pub fn test(&self, w: &World<'_>, pos: Pos) -> bool {
        let at = |offset: &Pos| [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]];
        match self {
            Predicate::True => true,
            Predicate::AllOf(all) => all.iter().all(|p| p.test(w, pos)),
            Predicate::AnyOf(any) => any.iter().any(|p| p.test(w, pos)),
            Predicate::Not(p) => !p.test(w, pos),
            Predicate::Blocks { offset, blocks } => {
                let s = w.get(at(offset));
                match blocks {
                    BlockSet::Tag(tag) => w.tags.is(s, tag),
                    BlockSet::Blocks(ids) => ids.contains(&state::block_id(s)),
                }
            }
            Predicate::Tag { offset, tag } => w.tags.is(w.get(at(offset)), tag),
        }
    }
}

/// The transformers the session knows, by registry id and by name.
#[derive(Debug, Clone, Default)]
pub struct Transformers {
    by_id: Vec<Option<Arc<Transformer>>>,
    by_name: HashMap<String, Arc<Transformer>>,
}

/// How an item names its transformer (the `block_transformer` component).
#[derive(Debug, Clone, PartialEq)]
pub enum TransformerRef {
    /// A registry id, as the server sends it.
    Id(i32),
    /// A registry name, as the item's default components give it.
    Name(String),
    /// Written out in place.
    Inline(Arc<Transformer>),
}

impl Transformers {
    /// Vanilla's transformers from the jar's data pack: each file's name
    /// (`axe.json`, or its path in the jar) and contents. Files that don't read are
    /// left out.
    pub fn from_json_files<N: AsRef<str>, B: AsRef<[u8]>>(
        files: impl IntoIterator<Item = (N, B)>,
    ) -> Transformers {
        let mut by_name = HashMap::new();
        for (name, bytes) in files {
            let file = name.as_ref().rsplit('/').next().unwrap_or_default();
            let Some(stem) = file.strip_suffix(".json") else {
                continue;
            };
            let parsed = serde_json::from_slice::<Value>(bytes.as_ref())
                .ok()
                .and_then(|v| Transformer::from_json(&v));
            if let Some(t) = parsed {
                by_name.insert(full_name(stem), Arc::new(t));
            }
        }
        Transformers {
            by_id: Vec::new(),
            by_name,
        }
    }

    /// The registry as the server sent it: an entry's own data, else vanilla's entry
    /// of that name (a known pack's, sent without data).
    pub fn from_registry(entries: &[RegistryEntry], vanilla: &Transformers) -> Transformers {
        let mut by_id = Vec::with_capacity(entries.len());
        let mut by_name = HashMap::new();
        for e in entries {
            let t = match &e.data {
                Some(data) => Transformer::from_json(&nbt_to_json(data)).map(Arc::new),
                None => vanilla.by_name.get(&e.id).cloned(),
            };
            if let Some(t) = &t {
                by_name.insert(e.id.clone(), t.clone());
            }
            by_id.push(t);
        }
        Transformers { by_id, by_name }
    }

    /// How many transformers are known.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// A transformer by name.
    pub fn named(&self, name: &str) -> Option<&Transformer> {
        self.by_name.get(name).map(|t| &**t)
    }

    /// The transformer an item names (an inline one needs no registry).
    pub fn get<'s>(&'s self, r: &'s TransformerRef) -> Option<&'s Transformer> {
        match r {
            TransformerRef::Id(id) => usize::try_from(*id)
                .ok()
                .and_then(|i| self.by_id.get(i)?.as_deref()),
            TransformerRef::Name(n) => self.named(n),
            TransformerRef::Inline(t) => Some(t),
        }
    }
}

impl TransformerRef {
    /// The default components' form: a name, or the transformer written out.
    pub fn from_json(v: &Value) -> Option<TransformerRef> {
        match v {
            Value::String(s) => Some(TransformerRef::Name(full_name(s))),
            other => Transformer::from_json(other).map(|t| TransformerRef::Inline(Arc::new(t))),
        }
    }
}

impl Transformer {
    /// A transformer as the codec writes it: a list of 1 to 200 transforms.
    pub fn from_json(v: &Value) -> Option<Transformer> {
        let list = v.as_array()?;
        if list.is_empty() || list.len() > 200 {
            return None;
        }
        let transforms = list
            .iter()
            .map(read_transform)
            .collect::<Option<Vec<_>>>()?;
        Some(Transformer { transforms })
    }
}

/// One `BlockTransformData`; the fields the client doesn't use are skipped.
fn read_transform(v: &Value) -> Option<Transform> {
    let provider = read_provider(v.get("block_state_provider")?)?;
    let disallowed_faces = match v.get("disallowed_faces") {
        None => Vec::new(),
        Some(faces) => faces
            .as_array()?
            .iter()
            .map(|f| Direction::from_name(f.as_str()?))
            .collect::<Option<Vec<_>>>()?,
    };
    Some(Transform {
        provider,
        disallowed_faces,
        update_from_neighbors: flag(v, "update_from_neighbors", true)?,
        consume_on_use: flag(v, "consume_on_use", true)?,
    })
}

/// A boolean field (JSON's `true`, or NBT's byte), with its default when absent.
fn flag(v: &Value, key: &str, default: bool) -> Option<bool> {
    match v.get(key) {
        None => Some(default),
        Some(Value::Bool(b)) => Some(*b),
        Some(n) => Some(n.as_i64()? != 0),
    }
}

/// A provider: a block state written out (no `type`), or a typed provider. A
/// registry reference (a bare name) isn't resolved.
fn read_provider(v: &Value) -> Option<Provider> {
    let Some(kind) = v.get("type") else {
        return read_state(v).map(Provider::State);
    };
    match full_name(kind.as_str()?).as_str() {
        "minecraft:simple_state_provider" => {
            let s = v.get("state")?;
            let state = match s {
                Value::String(block) => {
                    state::block_named(&full_name(block)).map(state::default_of_block)?
                }
                other => read_state(other)?,
            };
            Some(Provider::State(state))
        }
        "minecraft:rule_based" => {
            let rules = v
                .get("rules")?
                .as_array()?
                .iter()
                .map(|r| {
                    Some((
                        read_predicate(r.get("if_true")?)?,
                        read_provider(r.get("then")?)?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            let fallback = match v.get("fallback") {
                None => None,
                Some(f) => Some(Box::new(read_provider(f)?)),
            };
            Some(Provider::Rules { rules, fallback })
        }
        "minecraft:copy_properties" => Some(Provider::CopyProperties(Box::new(read_provider(
            v.get("source")?,
        )?))),
        _ => None,
    }
}

/// `BlockState`'s codec: `{"id": block, "properties": {name: value}}`; an unknown
/// block or property value fails the whole transformer.
fn read_state(v: &Value) -> Option<u32> {
    let block = state::block_named(&full_name(v.get("id")?.as_str()?))?;
    let mut s = state::default_of_block(block);
    if let Some(props) = v.get("properties") {
        for (k, value) in props.as_object()? {
            let value = match value {
                Value::String(t) => t.clone(),
                other => other.to_string(),
            };
            if !state::values(s, k).contains(&value.as_str()) {
                return None;
            }
            s = state::set(s, k, &value);
        }
    }
    Some(s)
}

fn read_predicate(v: &Value) -> Option<Predicate> {
    let list = |key: &str| {
        v.get(key)?
            .as_array()?
            .iter()
            .map(read_predicate)
            .collect::<Option<Vec<_>>>()
    };
    match full_name(v.get("type")?.as_str()?).as_str() {
        "minecraft:true" => Some(Predicate::True),
        "minecraft:all_of" => Some(Predicate::AllOf(list("predicates")?)),
        "minecraft:any_of" => Some(Predicate::AnyOf(list("predicates")?)),
        "minecraft:not" => Some(Predicate::Not(Box::new(read_predicate(
            v.get("predicate")?,
        )?))),
        "minecraft:matching_blocks" => Some(Predicate::Blocks {
            offset: read_offset(v)?,
            blocks: read_block_set(v.get("blocks")?)?,
        }),
        "minecraft:matching_block_tag" => {
            let tag = v.get("tag")?.as_str()?;
            Some(Predicate::Tag {
                offset: read_offset(v)?,
                tag: full_name(tag.strip_prefix('#').unwrap_or(tag)),
            })
        }
        _ => None,
    }
}

/// A predicate's `offset` (`[x, y, z]`, each at most 16 away; none by default).
fn read_offset(v: &Value) -> Option<Pos> {
    let Some(o) = v.get("offset") else {
        return Some([0, 0, 0]);
    };
    let parts = o
        .as_array()?
        .iter()
        .map(|n| i32::try_from(n.as_i64()?).ok().filter(|n| n.abs() <= 16))
        .collect::<Option<Vec<_>>>()?;
    parts.try_into().ok()
}

/// A block holder set: `#tag`, one block, or a list of blocks.
fn read_block_set(v: &Value) -> Option<BlockSet> {
    let block = |n: &Value| state::block_named(&full_name(n.as_str()?));
    match v {
        Value::String(s) => match s.strip_prefix('#') {
            Some(tag) => Some(BlockSet::Tag(full_name(tag))),
            None => Some(BlockSet::Blocks(vec![block(v)?])),
        },
        Value::Array(list) => Some(BlockSet::Blocks(
            list.iter().map(block).collect::<Option<_>>()?,
        )),
        _ => None,
    }
}

/// `minecraft:` for a name without a namespace.
fn full_name(n: &str) -> String {
    if n.contains(':') {
        n.to_owned()
    } else {
        format!("minecraft:{n}")
    }
}

/// An NBT value as the JSON the same codec reads (bytes stay numbers; booleans are
/// read from either).
pub fn nbt_to_json(t: &Tag) -> Value {
    let numbers = |v: Vec<i64>| Value::Array(v.into_iter().map(Value::from).collect());
    match t {
        Tag::Byte(n) => Value::from(*n),
        Tag::Short(n) => Value::from(*n),
        Tag::Int(n) => Value::from(*n),
        Tag::Long(n) => Value::from(*n),
        Tag::Float(n) => Value::from(f64::from(*n)),
        Tag::Double(n) => Value::from(*n),
        Tag::String(s) => Value::from(s.clone()),
        Tag::ByteArray(v) => numbers(v.iter().map(|&n| i64::from(n)).collect()),
        Tag::IntArray(v) => numbers(v.iter().map(|&n| i64::from(n)).collect()),
        Tag::LongArray(v) => numbers(v.clone()),
        Tag::List(v) => Value::Array(v.iter().map(nbt_to_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_owned(), nbt_to_json(v)))
                .collect(),
        ),
    }
}
