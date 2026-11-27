//! Item models: what each item definition (`items/*.json`) draws for a stack, as 26.3
//! resolves it (`client/renderer/item/**`), and the models' quads (`models/item/**`,
//! block models, and the flat models the game makes of an item's pictures).
//!
//! A definition is a tree: models at its leaves, and above them choices by what the
//! stack is and what its holder does. [`ItemModels::resolve`] walks it for one stack and
//! gives the leaves to draw.

use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;

use crate::atlas::{Atlas, Sprite};
use crate::id::Id;
use crate::image::{Image, alpha};
use crate::limits::Limits;
use crate::model::{
    BakedQuad, Element, Face, Layer, Material, ModelFiles, ModelRotation, RawModel,
    TransparencyCache, Vec3, bake_elements_with, bake_quad_with, parse_model_limited,
};
use crate::resources::Resources;

/// How a model is placed where it is shown (`ItemTransform`): turned by `rotation`
/// (degrees about x, y and z), moved by `translation` (blocks) and scaled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemTransform {
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

fn finite_f32(v: &Value, default: f32) -> f32 {
    v.as_f64()
        .map(|f| f as f32)
        .filter(|f| f.is_finite())
        .unwrap_or(default)
}

fn floats(v: Option<&Value>, default: [f32; 3]) -> [f32; 3] {
    match v.and_then(Value::as_array) {
        Some(a) if a.len() == 3 => std::array::from_fn(|i| finite_f32(&a[i], default[i])),
        _ => default,
    }
}

/// A model file's `display`: the places it names, each with its transform. A file that
/// places the right hand and not the left places the left as the right.
pub(crate) fn parse_display(v: Option<&Value>) -> Vec<(String, ItemTransform)> {
    let Some(map) = v.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<(String, ItemTransform)> = map
        .iter()
        .filter(|(_, t)| t.is_object())
        .map(|(place, t)| {
            let translation = floats(t.get("translation"), [0.0; 3]);
            let scale = floats(t.get("scale"), [1.0; 3]);
            let transform = ItemTransform {
                rotation: floats(t.get("rotation"), [0.0; 3]).map(|c| c.clamp(-3600.0, 3600.0)),
                // Sixteenths, and never farther than five blocks or larger than four times.
                translation: translation.map(|c| (c * 0.0625).clamp(-5.0, 5.0)),
                scale: scale.map(|c| c.clamp(-4.0, 4.0)),
            };
            (place.clone(), transform)
        })
        .collect();
    for (left, right) in [
        ("thirdperson_lefthand", "thirdperson_righthand"),
        ("firstperson_lefthand", "firstperson_righthand"),
    ] {
        if !out.iter().any(|(p, _)| p == left)
            && let Some(t) = out.iter().find(|(p, _)| p == right).map(|(_, t)| *t)
        {
            out.push((left.to_owned(), t));
        }
    }
    out
}

/// A transformation a definition gives its models: rows of a matrix, the last column
/// what it moves by.
pub type Transformation = [[f32; 4]; 3];

pub const NO_TRANSFORMATION: Transformation = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn compose(a: &Transformation, b: &Transformation) -> Transformation {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            let moved = if c == 3 { a[r][3] } else { 0.0 };
            a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + moved
        })
    })
}

/// A rotation as a matrix: a quaternion (x, y, z, w), or an axis and an angle in radians.
fn rotation(v: Option<&Value>) -> Transformation {
    let q = match v {
        Some(Value::Array(a)) if a.len() == 4 => {
            let q: [f32; 4] = std::array::from_fn(|i| finite_f32(&a[i], 0.0));
            let n = q.iter().map(|c| c * c).sum::<f32>().sqrt();
            if n > 0.0 && n.is_finite() {
                q.map(|c| c / n)
            } else {
                return NO_TRANSFORMATION;
            }
        }
        Some(Value::Object(o)) => {
            let axis = floats(o.get("axis"), [0.0, 1.0, 0.0]);
            let angle = o.get("angle").map_or(0.0, |a| finite_f32(a, 0.0));
            let n = axis.iter().map(|c| c * c).sum::<f32>().sqrt();
            if n == 0.0 || !n.is_finite() {
                return NO_TRANSFORMATION;
            }
            let (s, c) = (angle / 2.0).sin_cos();
            [axis[0] / n * s, axis[1] / n * s, axis[2] / n * s, c]
        }
        _ => return NO_TRANSFORMATION,
    };
    let [x, y, z, w] = q;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            0.0,
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            0.0,
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
        ],
    ]
}

/// A definition's `transformation`: sixteen numbers, a matrix by rows, or its parts (a
/// translation, a rotation, a scale and another rotation, done from the last).
/// Entries are kept finite and bounded so extreme scales cannot cause full-screen overdraw.
fn parse_transformation(v: &Value) -> Option<Transformation> {
    let m = match v {
        Value::Array(a) if a.len() == 16 => {
            let mut m = [[0.0f32; 4]; 3];
            for r in 0..3 {
                for c in 0..4 {
                    let val = a[r * 4 + c].as_f64()? as f32;
                    if !val.is_finite() {
                        return None;
                    }
                    m[r][c] = val.clamp(-1024.0, 1024.0);
                }
            }
            m
        }
        Value::Object(o) => {
            let t = floats(o.get("translation"), [0.0; 3]).map(|c| c.clamp(-1024.0, 1024.0));
            let s = floats(o.get("scale"), [1.0; 3]).map(|c| c.clamp(-16.0, 16.0));
            let moved = [
                [1.0, 0.0, 0.0, t[0]],
                [0.0, 1.0, 0.0, t[1]],
                [0.0, 0.0, 1.0, t[2]],
            ];
            let scaled = [
                [s[0], 0.0, 0.0, 0.0],
                [0.0, s[1], 0.0, 0.0],
                [0.0, 0.0, s[2], 0.0],
            ];
            let left = compose(&moved, &rotation(o.get("left_rotation")));
            compose(&compose(&left, &scaled), &rotation(o.get("right_rotation")))
        }
        _ => return None,
    };
    m.iter().flatten().all(|x| x.is_finite()).then_some(m)
}

/// One quad of an item's model.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemQuad {
    /// The corners in the model's space (a block spans 0 to 1), in the order they are
    /// drawn.
    pub positions: [Vec3; 4],
    /// Their texture coordinates, 0 to 1 over the atlas.
    pub uvs: [[f32; 2]; 4],
    /// The direction it faces (a `Direction` ordinal).
    pub direction: u8,
}

/// Quads of a model that are drawn alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// Their pictures are in the block atlas; else in the items'.
    pub block_atlas: bool,
    /// Some of what they show is seen through.
    pub translucent: bool,
    /// The tint that colours them, or -1.
    pub tint_index: i32,
    pub first: u32,
    pub count: u32,
}

/// A model's quads, and how it is held.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemMesh {
    pub quads: Vec<ItemQuad>,
    pub runs: Vec<Run>,
    /// In a right hand and in a left one, seen from outside.
    pub hands: [Option<ItemTransform>; 2],
    /// In a right hand and in a left one, seen by the one who holds it.
    pub first_person: [Option<ItemTransform>; 2],
    /// On a head.
    pub head: Option<ItemTransform>,
    /// In a slot of a GUI.
    pub gui: Option<ItemTransform>,
    /// How a GUI lights it (`gui_light`, the first of the model and its parents that
    /// says): from the side, as a block, unless from the front, as a flat picture.
    pub gui_side_lit: bool,
}

/// What colours the quads of one tint index (`ItemTintSource`). Colours are 0xRRGGBB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    Constant(u32),
    /// The stack's dye, or this.
    Dye(u32),
    /// The colour of the stack's potion, or this.
    Potion(u32),
    /// What needs more of the stack or the world than is known here: this.
    Default(u32),
}

/// A model the game draws by code of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    Shield,
    Trident,
    /// One that is not drawn yet.
    Other,
}

/// A question with two answers (`ConditionalItemModelProperty`).
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    UsingItem,
    Broken,
    Damaged,
    HasComponent {
        component: String,
        ignore_default: bool,
    },
    /// What is never so for a thing in another's hand, or is not known here.
    Never,
}

/// A question with named answers (`SelectItemModelProperty`).
#[derive(Debug, Clone, PartialEq)]
pub enum Select {
    DisplayContext,
    MainHand,
    ChargeType,
    TrimMaterial,
    BlockState(String),
    /// One whose answer is not known here: no case is taken.
    Unknown,
}

/// A question with a number for an answer (`RangeSelectItemModelProperty`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Range {
    UseDuration {
        remaining: bool,
    },
    UseCycle {
        period: f32,
    },
    CrossbowPull,
    Damage {
        normalize: bool,
    },
    Count {
        normalize: bool,
    },
    /// One whose answer is not known here: nothing.
    Unknown,
}

/// A definition, or a part of one.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Empty,
    Model {
        mesh: u32,
        tints: Vec<Tint>,
        transformation: Transformation,
    },
    Special {
        special: Special,
        /// The model whose placing it takes.
        base: u32,
        transformation: Transformation,
    },
    Composite(Vec<Node>),
    Condition {
        property: Condition,
        on_true: Box<Node>,
        on_false: Box<Node>,
    },
    Select {
        property: Select,
        cases: Vec<(Vec<String>, Node)>,
        fallback: Box<Node>,
    },
    Range {
        property: Range,
        scale: f32,
        /// By their thresholds, lowest first.
        entries: Vec<(f32, Node)>,
        fallback: Box<Node>,
    },
}

/// What of a stack and its holder a definition asks about.
pub struct Held<'a> {
    /// Where it is shown (`thirdperson_righthand`).
    pub context: &'a str,
    /// Its holder's main hand is the left one.
    pub left_handed: bool,
    /// Its holder is using this very stack.
    pub using: bool,
    /// For how many ticks, and how many are left.
    pub use_ticks: i32,
    pub use_remaining: i32,
    /// Its damage and the most it takes, if it takes any.
    pub damage: Option<(i32, i32)>,
    /// How many there are, and how many there can be.
    pub count: (i32, i32),
    /// What a crossbow is loaded with: `none`, `arrow` or `rocket`.
    pub charge: &'a str,
    /// The ticks a crossbow takes to load.
    pub charge_ticks: f32,
    /// The key of its trim's material (`minecraft:gold`).
    pub trim_material: Option<&'a str>,
    /// The properties it gives the block it places.
    pub block_state: &'a [(String, String)],
    /// Whether it has a component of this name; with `true`, one that is not its
    /// item's own.
    pub has_component: &'a dyn Fn(&str, bool) -> bool,
}

impl std::fmt::Debug for Held<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held")
            .field("context", &self.context)
            .field("using", &self.using)
            .field("use_ticks", &self.use_ticks)
            .field("damage", &self.damage)
            .field("charge", &self.charge)
            .finish_non_exhaustive()
    }
}

/// A leaf of a definition to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drawn<'a> {
    Model {
        mesh: u32,
        tints: &'a [Tint],
        transformation: &'a Transformation,
    },
    Special {
        special: Special,
        base: u32,
        transformation: &'a Transformation,
    },
}

impl Node {
    /// Gives each leaf that is drawn for `held` to `out`, in order, stopping once `left`
    /// resolved models have been emitted.
    fn resolve<'a>(&'a self, held: &Held<'_>, left: &mut usize, out: &mut dyn FnMut(Drawn<'a>)) {
        if *left == 0 {
            return;
        }
        match self {
            Node::Empty => {}
            Node::Model {
                mesh,
                tints,
                transformation,
            } => {
                *left -= 1;
                out(Drawn::Model {
                    mesh: *mesh,
                    tints,
                    transformation,
                });
            }
            Node::Special {
                special,
                base,
                transformation,
            } => {
                *left -= 1;
                out(Drawn::Special {
                    special: *special,
                    base: *base,
                    transformation,
                });
            }
            Node::Composite(nodes) => {
                for n in nodes {
                    if *left == 0 {
                        break;
                    }
                    n.resolve(held, left, out);
                }
            }
            Node::Condition {
                property,
                on_true,
                on_false,
            } => {
                let holds = match property {
                    Condition::UsingItem => held.using,
                    Condition::Broken => held.damage.is_some_and(|(d, max)| d >= max - 1),
                    Condition::Damaged => held.damage.is_some_and(|(d, _)| d > 0),
                    Condition::HasComponent {
                        component,
                        ignore_default,
                    } => (held.has_component)(component, *ignore_default),
                    Condition::Never => false,
                };
                if holds { on_true } else { on_false }.resolve(held, left, out);
            }
            Node::Select {
                property,
                cases,
                fallback,
            } => {
                let value: Option<&str> = match property {
                    Select::DisplayContext => Some(held.context),
                    Select::MainHand => Some(if held.left_handed { "left" } else { "right" }),
                    Select::ChargeType => Some(held.charge),
                    Select::TrimMaterial => held.trim_material,
                    Select::BlockState(name) => held
                        .block_state
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.as_str()),
                    Select::Unknown => None,
                };
                let chosen = value.and_then(|v| {
                    cases
                        .iter()
                        .find(|(when, _)| when.iter().any(|w| w == v))
                        .map(|(_, node)| node)
                });
                chosen.unwrap_or(fallback).resolve(held, left, out);
            }
            Node::Range {
                property,
                scale,
                entries,
                fallback,
            } => {
                let value = match *property {
                    Range::UseDuration { remaining } if held.using => {
                        (if remaining {
                            held.use_remaining
                        } else {
                            held.use_ticks
                        }) as f32
                    }
                    Range::UseCycle { period } if held.using => held.use_remaining as f32 % period,
                    Range::CrossbowPull if held.using && held.charge == "none" => {
                        held.use_ticks as f32 / held.charge_ticks
                    }
                    Range::Damage { normalize } => match (held.damage, normalize) {
                        (Some((d, max)), true) => (d as f32 / max as f32).clamp(0.0, 1.0),
                        (Some((d, max)), false) => d.clamp(0, max) as f32,
                        (None, _) => 0.0,
                    },
                    Range::Count { normalize } => {
                        let (count, max) = held.count;
                        if normalize {
                            (count as f32 / max as f32).clamp(0.0, 1.0)
                        } else {
                            count.clamp(0, max) as f32
                        }
                    }
                    _ => 0.0,
                } * scale;
                // The last entry that is not over the value.
                let chosen = entries
                    .iter()
                    .take_while(|(threshold, _)| *threshold <= value)
                    .last()
                    .map(|(_, node)| node);
                chosen
                    .filter(|_| !value.is_nan())
                    .unwrap_or(fallback)
                    .resolve(held, left, out);
            }
        }
    }
}

/// Every item definition and the models they draw.
#[derive(Debug, Default)]
pub struct ItemModels {
    definitions: HashMap<String, Node>,
    /// How much farther a hand sinks to change to or from each item than usual
    /// (`swap_animation_scale`), for those that say.
    swap_scales: HashMap<String, f32>,
    meshes: Vec<ItemMesh>,
    /// Maximum models resolved per item stack ([`Limits::item_resolved_models`]).
    max_resolved: usize,
    /// What went wrong on the way, for the log.
    pub warnings: Vec<String>,
}

/// The parent of the models the game makes of an item's pictures.
const GENERATED: &str = "minecraft:builtin/generated";
/// The pictures such a model is made of, in order.
const LAYERS: [&str; 5] = ["layer0", "layer1", "layer2", "layer3", "layer4"];
/// What a grass tint is where the colour map has nothing (`GrassColor`'s default).
const NO_GRASS: u32 = 0xFF_00FF;

/// A flat picture's quads, shared by every model that shows it.
type FlatQuads = Option<Rc<Vec<BakedQuad>>>;

struct Baker<'a> {
    files: &'a dyn ModelFiles,
    blocks: &'a Atlas,
    items: &'a Atlas,
    grass: Option<&'a Image>,
    raw: HashMap<Id, Option<Rc<RawModel>>>,
    baked: HashMap<Id, u32>,
    meshes: Vec<ItemMesh>,
    warnings: Vec<String>,
    limits: Limits,
    /// Quads baked so far, against [`Limits::baked_quads`].
    quads: usize,
    /// Definition nodes read so far, and how deep the one being read is.
    nodes: usize,
    depth: usize,
    /// A flat picture's quads, by its atlas, sprite and layer: each picture is looked
    /// through once, however many models show it. `None`: more quads than may be baked.
    flat: HashMap<(usize, usize, i32), FlatQuads>,
    transparency: TransparencyCache,
}

fn colour(v: Option<&Value>, default: u32) -> u32 {
    match v {
        Some(Value::Array(a)) if a.len() == 3 => a.iter().fold(0, |rgb, c| {
            rgb << 8 | (c.as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).floor() as u32
        }),
        Some(n) => n.as_i64().map_or(default, |c| c as u32 & 0xFF_FFFF),
        None => default,
    }
}

fn kind(v: &Value, key: &str) -> String {
    let name = v.get(key).and_then(Value::as_str).unwrap_or("");
    name.strip_prefix("minecraft:").unwrap_or(name).to_owned()
}

impl<'a> Baker<'a> {
    fn new(
        files: &'a dyn ModelFiles,
        blocks: &'a Atlas,
        items: &'a Atlas,
        grass: Option<&'a Image>,
    ) -> Baker<'a> {
        Baker {
            files,
            blocks,
            items,
            grass,
            raw: HashMap::new(),
            baked: HashMap::new(),
            meshes: Vec::new(),
            warnings: Vec::new(),
            limits: files.limits(),
            quads: 0,
            nodes: 0,
            depth: 0,
            flat: HashMap::new(),
            transparency: TransparencyCache::default(),
        }
    }

    fn bake_definition(
        &mut self,
        id: &str,
        text: &str,
        out: &mut HashMap<String, Node>,
        swap_scales: &mut HashMap<String, f32>,
    ) {
        let file = serde_json::from_str::<Value>(text).ok();
        if let Some(scale) = file
            .as_ref()
            .and_then(|v| v.get("swap_animation_scale")?.as_f64())
            .map(|s| s as f32)
            .filter(|s| s.is_finite())
        {
            swap_scales.insert(id.to_owned(), scale.clamp(-16.0, 16.0));
        }
        let root = file.and_then(|v| v.get("model").cloned());
        match root {
            Some(model) => {
                let node = self.node(&model, &NO_TRANSFORMATION);
                out.insert(id.to_owned(), node);
            }
            None => self.warn(format!("item {id}: not a definition")),
        }
    }

    fn warn(&mut self, text: String) {
        if self.warnings.len() < 200 {
            self.warnings.push(text);
        }
    }

    fn raw(&mut self, id: &Id) -> Option<Rc<RawModel>> {
        if let Some(m) = self.raw.get(id) {
            return m.clone();
        }
        let (max, textures) = (self.limits.model_elements, self.limits.model_textures);
        let model = match self
            .files
            .model(id)
            .map(|text| parse_model_limited(&text, max, textures))
        {
            Some(Ok(m)) => Some(Rc::new(m)),
            Some(Err(e)) => {
                self.warn(format!("model {id}: {e}"));
                None
            }
            None => {
                self.warn(format!("model {id} is missing"));
                None
            }
        };
        self.raw.insert(id.clone(), model.clone());
        model
    }

    /// A sprite and the atlas it is of: the items', the blocks', or the missing one.
    fn sprite(&self, id: &Id) -> (&'a Atlas, &'a Sprite, bool) {
        let (items, blocks) = (self.items, self.blocks);
        match (items.sprite(id), blocks.sprite(id)) {
            (Some(s), _) => (items, s, true),
            (None, Some(s)) => (blocks, s, true),
            (None, None) => (items, items.missing(), false),
        }
    }

    /// Bakes a model, once. A model that is missing or broken is an empty one.
    fn mesh(&mut self, id: &Id) -> u32 {
        if let Some(&i) = self.baked.get(id) {
            return i;
        }
        // The model and its parents, the model first, up to the one the game makes
        // flat models from.
        let mut chain: Vec<Rc<RawModel>> = Vec::new();
        let mut generated = false;
        let mut next = Some(id.clone());
        let mut seen = Vec::new();
        while let Some(at) = next.take() {
            if at.to_string() == GENERATED {
                generated = true;
                break;
            }
            if seen.contains(&at) {
                self.warn(format!("model {id}: its parents form a loop at {at}"));
                break;
            }
            if seen.len() > self.limits.model_parents {
                self.warn(format!(
                    "model {id}: more parents than a model may have ({})",
                    self.limits.model_parents
                ));
                break;
            }
            let Some(model) = self.raw(&at) else {
                break;
            };
            next.clone_from(&model.parent);
            chain.push(model);
            seen.push(at);
        }
        let resolved = crate::model::resolve_slots(&chain);
        let mut missing: Vec<String> = Vec::new();
        // Each quad with the atlas it is of.
        let mut quads: Vec<(BakedQuad, &Atlas)> = Vec::new();
        if generated {
            for (layer, slot) in LAYERS.iter().enumerate() {
                let Some(&(sprite_id, _)) = resolved.get(slot) else {
                    break;
                };
                let (atlas, sprite, found) = self.sprite(sprite_id);
                if !found {
                    missing.push(sprite_id.to_string());
                }
                let before = quads.len();
                let key = (
                    std::ptr::from_ref(atlas) as usize,
                    std::ptr::from_ref(sprite) as usize,
                    layer as i32,
                );
                let left = self
                    .limits
                    .baked_quads
                    .saturating_sub(self.quads)
                    .min(self.limits.item_mesh_quads.saturating_sub(quads.len()));
                let flat = match self.flat.get(&key) {
                    // (Baked for another mesh, which may have had more room left.)
                    Some(Some(f)) if f.len() <= left => Some(f.clone()),
                    Some(_) => None,
                    None => {
                        let f = flat_layer(atlas, sprite, layer as i32, left, &|| {
                            self.files.cancelled()
                        })
                        .map(Rc::new);
                        // A failure with less room than a whole mesh's says nothing of
                        // the next mesh, which may fit the layer.
                        if f.is_some() || left >= self.limits.item_mesh_quads {
                            self.flat.insert(key, f.clone());
                        }
                        f
                    }
                };
                let Some(flat) = flat else {
                    self.warn(format!(
                        "model {id}: {sprite_id} has more edges than may be baked ({left})"
                    ));
                    break;
                };
                quads.extend(flat.iter().map(|q| (q.clone(), atlas)));
                // All of a picture's quads are drawn as the picture is.
                for (q, _) in &mut quads[before..] {
                    q.layer = if sprite.transparency.translucent {
                        Layer::Translucent
                    } else {
                        Layer::Cutout
                    };
                }
            }
        } else if let Some(elements) = chain.iter().find_map(|m| m.elements.as_deref()) {
            // A model of elements takes all its pictures from one atlas (the game refuses
            // one that does not): the atlas of the first picture a face asks for.
            let (items_atlas, blocks_atlas, files) = (self.items, self.blocks, self.files);
            let of: std::cell::Cell<Option<&Atlas>> = std::cell::Cell::new(None);
            let unknown = std::cell::RefCell::new(&mut missing);
            let material = |slot: &str| {
                let slot = slot.strip_prefix('#').unwrap_or(slot);
                let found = resolved.get(slot).map(|&(id, force)| {
                    let s = match (items_atlas.sprite(id), blocks_atlas.sprite(id)) {
                        (Some(s), _) => (items_atlas, s, true),
                        (None, Some(s)) => (blocks_atlas, s, true),
                        (None, None) => (items_atlas, items_atlas.missing(), false),
                    };
                    (id, force, s)
                });
                let (atlas, sprite, force_translucent) = match found {
                    Some((_, force, (atlas, sprite, true))) => (atlas, sprite, force),
                    other => {
                        unknown.borrow_mut().push(match other {
                            Some((id, ..)) => id.to_string(),
                            None => format!("#{slot}"),
                        });
                        let atlas = of.get().unwrap_or(items_atlas);
                        (atlas, atlas.missing(), false)
                    }
                };
                if of.get().is_none() {
                    of.set(Some(atlas));
                }
                Material {
                    atlas,
                    sprite,
                    force_translucent,
                }
            };
            let groups = bake_elements_with(
                elements,
                ModelRotation::default(),
                &material,
                &mut self.transparency,
                &|| files.cancelled(),
            );
            let atlas = of.get().unwrap_or(items_atlas);
            // As `QuadCollection.getAll` lists them.
            quads.extend(groups.into_iter().flatten().map(|q| (q, atlas)));
        }
        let quads = if quads.len() > self.limits.item_mesh_quads {
            self.warn(format!(
                "model {id}: {} quads is more than an item model may have ({})",
                quads.len(),
                self.limits.item_mesh_quads
            ));
            Vec::new()
        } else if self.quads.saturating_add(quads.len()) > self.limits.baked_quads {
            self.warn(format!(
                "model {id}: the models have more quads than may be baked ({})",
                self.limits.baked_quads
            ));
            Vec::new()
        } else {
            self.quads += quads.len();
            quads
        };
        let mesh = self.finish(&chain, quads);
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            self.warn(format!("model {id}: no texture for {}", missing.join(", ")));
        }
        let index = self.meshes.len() as u32;
        self.meshes.push(mesh);
        self.baked.insert(id.clone(), index);
        index
    }

    /// A model's quads put in runs, with its placing from the models of `chain`.
    fn finish(&self, chain: &[Rc<RawModel>], quads: Vec<(BakedQuad, &Atlas)>) -> ItemMesh {
        let mut keyed: Vec<((bool, bool, i32), ItemQuad)> = quads
            .into_iter()
            .map(|(q, atlas)| {
                let in_blocks = std::ptr::eq(atlas, self.blocks);
                let size = [atlas.width as f32, atlas.height as f32];
                (
                    (in_blocks, q.layer == Layer::Translucent, q.tint_index),
                    ItemQuad {
                        positions: q.positions,
                        uvs: q.uvs.map(|uv| [uv[0] / size[0], uv[1] / size[1]]),
                        direction: q.direction,
                    },
                )
            })
            .collect();
        keyed.sort_by_key(|(key, _)| *key);
        let mut mesh = ItemMesh::default();
        for (key, quad) in keyed {
            match mesh.runs.last_mut() {
                Some(run) if (run.block_atlas, run.translucent, run.tint_index) == key => {
                    run.count += 1;
                }
                _ => mesh.runs.push(Run {
                    block_atlas: key.0,
                    translucent: key.1,
                    tint_index: key.2,
                    first: mesh.quads.len() as u32,
                    count: 1,
                }),
            }
            mesh.quads.push(quad);
        }
        // Each place from the first of the model and its parents that names it.
        let place = |name: &str| {
            chain
                .iter()
                .find_map(|m| m.display.iter().find(|(p, _)| p == name).map(|(_, t)| *t))
        };
        mesh.hands = [
            place("thirdperson_righthand"),
            place("thirdperson_lefthand"),
        ];
        mesh.first_person = [
            place("firstperson_righthand"),
            place("firstperson_lefthand"),
        ];
        mesh.head = place("head");
        mesh.gui = place("gui");
        mesh.gui_side_lit = chain.iter().find_map(|m| m.gui_side_lit).unwrap_or(true);
        mesh
    }

    fn tint(&self, v: &Value) -> Tint {
        let default = colour(v.get("default"), 0xFF_FFFF);
        match kind(v, "type").as_str() {
            "constant" => Tint::Constant(colour(v.get("value"), 0xFF_FFFF)),
            "dye" => Tint::Dye(default),
            "potion" => Tint::Potion(default),
            "grass" => {
                let number = |key: &str| v.get(key).and_then(Value::as_f64).unwrap_or(0.0);
                Tint::Constant(grass(self.grass, number("temperature"), number("downfall")))
            }
            _ => Tint::Default(default),
        }
    }

    /// A definition's node, no deeper than [`Limits::item_model_depth`] and no more of them
    /// than [`Limits::item_model_nodes`]: past either, nothing is drawn.
    fn node(&mut self, v: &Value, parent: &Transformation) -> Node {
        if self.files.cancelled() {
            return Node::Empty;
        }
        if self.depth >= self.limits.item_model_depth {
            self.warn(format!(
                "an item definition nests deeper than {}",
                self.limits.item_model_depth
            ));
            return Node::Empty;
        }
        if self.nodes >= self.limits.item_model_nodes {
            self.warn(format!(
                "the item definitions have more than {} nodes",
                self.limits.item_model_nodes
            ));
            return Node::Empty;
        }
        self.depth += 1;
        self.nodes += 1;
        let node = self.node_within(v, parent);
        self.depth -= 1;
        node
    }

    fn node_within(&mut self, v: &Value, parent: &Transformation) -> Node {
        let transformation = match v.get("transformation").and_then(parse_transformation) {
            Some(own) => compose(parent, &own),
            None => *parent,
        };
        let child = |b: &mut Self, key: &str| match v.get(key) {
            Some(model) => b.node(model, &transformation),
            None => Node::Empty,
        };
        let model = |b: &mut Self, key: &str| {
            let id = v.get(key).and_then(Value::as_str).and_then(Id::parse);
            id.map(|id| b.mesh(&id))
        };
        match kind(v, "type").as_str() {
            "model" => match model(self, "model") {
                Some(mesh) => Node::Model {
                    mesh,
                    tints: v
                        .get("tints")
                        .and_then(Value::as_array)
                        .map_or(&[][..], Vec::as_slice)
                        .iter()
                        .map(|t| self.tint(t))
                        .collect(),
                    transformation,
                },
                None => Node::Empty,
            },
            "special" => match model(self, "base") {
                Some(base) => Node::Special {
                    special: match v.get("model").map(|m| kind(m, "type")).as_deref() {
                        Some("shield") => Special::Shield,
                        Some("trident") => Special::Trident,
                        _ => Special::Other,
                    },
                    base,
                    transformation,
                },
                None => Node::Empty,
            },
            "composite" => Node::Composite(
                v.get("models")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .take(self.limits.item_resolved_models)
                    .map(|m| self.node(m, &transformation))
                    .collect(),
            ),
            "condition" => Node::Condition {
                property: match kind(v, "property").as_str() {
                    "using_item" => Condition::UsingItem,
                    "broken" => Condition::Broken,
                    "damaged" => Condition::Damaged,
                    "has_component" => Condition::HasComponent {
                        component: v
                            .get("component")
                            .and_then(Value::as_str)
                            .map(|c| {
                                if c.contains(':') {
                                    c.to_owned()
                                } else {
                                    format!("minecraft:{c}")
                                }
                            })
                            .unwrap_or_default(),
                        ignore_default: v
                            .get("ignore_default")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                    _ => Condition::Never,
                },
                on_true: Box::new(child(self, "on_true")),
                on_false: Box::new(child(self, "on_false")),
            },
            "select" => {
                let cases = v
                    .get("cases")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .filter_map(|case| {
                        let when = match case.get("when")? {
                            Value::String(s) => vec![s.clone()],
                            Value::Array(a) => a
                                .iter()
                                .take(256)
                                .filter_map(|s| Some(s.as_str()?.to_owned()))
                                .collect(),
                            _ => return None,
                        };
                        Some((when, self.node(case.get("model")?, &transformation)))
                    })
                    .collect();
                Node::Select {
                    property: match kind(v, "property").as_str() {
                        "display_context" => Select::DisplayContext,
                        "main_hand" => Select::MainHand,
                        "charge_type" => Select::ChargeType,
                        "trim_material" => Select::TrimMaterial,
                        "block_state" => Select::BlockState(
                            v.get("block_state_property")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                        ),
                        _ => Select::Unknown,
                    },
                    cases,
                    fallback: Box::new(child(self, "fallback")),
                }
            }
            "range_dispatch" => {
                let flag = |key: &str| v.get(key).and_then(Value::as_bool);
                let mut entries: Vec<(f32, Node)> = v
                    .get("entries")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .filter_map(|e| {
                        let threshold = e.get("threshold")?.as_f64()? as f32;
                        Some((threshold, self.node(e.get("model")?, &transformation)))
                    })
                    .collect();
                entries.sort_by(|a, b| a.0.total_cmp(&b.0));
                Node::Range {
                    property: match kind(v, "property").as_str() {
                        "use_duration" => Range::UseDuration {
                            remaining: flag("remaining").unwrap_or(false),
                        },
                        "use_cycle" => Range::UseCycle {
                            period: v.get("period").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                        },
                        "crossbow/pull" => Range::CrossbowPull,
                        "damage" => Range::Damage {
                            normalize: flag("normalize").unwrap_or(true),
                        },
                        "count" => Range::Count {
                            normalize: flag("normalize").unwrap_or(true),
                        },
                        _ => Range::Unknown,
                    },
                    scale: v.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32,
                    entries,
                    fallback: Box::new(child(self, "fallback")),
                }
            }
            _ => Node::Empty,
        }
    }
}

/// `GrassColor.get`: the grass colour map at a temperature and a downfall.
fn grass(map: Option<&Image>, temperature: f64, downfall: f64) -> u32 {
    let downfall = downfall * temperature;
    let x = ((1.0 - temperature) * 255.0) as i32;
    let y = ((1.0 - downfall) * 255.0) as i32;
    let index = y << 8 | x;
    map.and_then(|m| m.pixels.get(usize::try_from(index).ok()?))
        .map_or(NO_GRASS, |c| c & 0xFF_FFFF)
}

/// The flat model the game makes of one of an item's pictures
/// (`ItemModelGenerator.bakeExtrudedSprite`): the picture seen from the front and from
/// behind, a sixteenth of a block apart, and a side for every edge of what it shows.
/// `None` if it would have more than `max_quads` quads.
fn flat_layer(
    atlas: &Atlas,
    sprite: &Sprite,
    layer: i32,
    max_quads: usize,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<BakedQuad>> {
    if max_quads < 2 || cancelled() {
        return None;
    }
    let material = Material {
        atlas,
        sprite,
        force_translucent: false,
    };
    let quad = |from: Vec3, to: Vec3, uvs: [f32; 4], direction: u8| {
        let mut faces: [Option<Face>; 6] = Default::default();
        let face = Face {
            cull: None,
            tint_index: layer,
            texture: String::new(),
            uvs: Some(uvs),
            rotation: 0,
        };
        faces[usize::from(direction)] = Some(face.clone());
        let element = Element {
            from,
            to,
            faces,
            rotation: None,
            shade_direction: None,
            light_emission: 0,
        };
        // Skip rect_transparency: the caller overwrites `q.layer` from `sprite.transparency`.
        bake_quad_with(
            &element,
            &face,
            direction,
            &material,
            None,
            None,
            Some(sprite.transparency),
            None,
        )
    };
    const SOUTH: u8 = 3;
    const NORTH: u8 = 2;
    let mut out = vec![
        quad(
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [0.0, 0.0, 16.0, 16.0],
            SOUTH,
        ),
        quad(
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [16.0, 0.0, 0.0, 16.0],
            NORTH,
        ),
    ];
    let (width, height) = (sprite.width as i32, sprite.height as i32);
    // Which texels show something, in any of the picture's frames.
    let frames: Vec<(&Image, u32, u32)> = match sprite.animated.and_then(|i| atlas.animated.get(i))
    {
        Some(animated) => {
            let mut seen = Vec::new();
            let mut unique = std::collections::HashSet::new();
            for &(frame, _) in &animated.animation.frames {
                if unique.insert(frame) {
                    seen.push(frame);
                }
            }
            seen.into_iter()
                .map(|frame| {
                    let (fx, fy) = animated.animation.frame_pos(frame);
                    (&animated.levels[0], fx * sprite.width, fy * sprite.height)
                })
                .collect()
        }
        None => vec![(
            &atlas.levels[0],
            sprite.x + sprite.padding,
            sprite.y + sprite.padding,
        )],
    };
    let (x_scale, y_scale) = (16.0 / width as f32, 16.0 / height as f32);
    // Up, down, left, right: where the texel beside lies, and the direction of the side.
    const SIDES: [(i32, i32, u8); 4] = [(0, -1, 1), (0, 1, 0), (-1, 0, 5), (1, 0, 4)];
    let mut sides = std::collections::BTreeSet::new();
    for (image, ox, oy) in frames {
        if cancelled() {
            return None;
        }
        let clear = |x: i32, y: i32| {
            x < 0
                || y < 0
                || x >= width
                || y >= height
                || alpha(image.get(ox + x as u32, oy + y as u32)) == 0
        };
        for y in 0..height {
            if y & 15 == 0 && cancelled() {
                return None;
            }
            for x in 0..width {
                if clear(x, y) {
                    continue;
                }
                for (side, (dx, dy, _)) in SIDES.iter().enumerate() {
                    if clear(x + dx, y + dy) {
                        sides.insert((side, x, y));
                    }
                }
                if sides.len() + 2 > max_quads {
                    return None;
                }
            }
        }
    }
    for (step, (side, x, y)) in sides.into_iter().enumerate() {
        if step & 255 == 0 && cancelled() {
            return None;
        }
        let (x, y) = (x as f32, y as f32);
        let across = side < 2;
        let (u0, u1) = (x + 0.1, x + 1.0 - 0.1);
        let (v0, v1) = if across {
            (y + 0.1, y + 1.0 - 0.1)
        } else {
            (y + 1.0 - 0.1, y + 0.1)
        };
        let (mut start, mut end) = ([x, y], [x, y]);
        match side {
            0 => end[0] += 1.0,
            1 => {
                end[0] += 1.0;
                start[1] += 1.0;
                end[1] += 1.0;
            }
            2 => end[1] += 1.0,
            _ => {
                start[0] += 1.0;
                end[0] += 1.0;
                end[1] += 1.0;
            }
        }
        let (sx, ex) = (start[0] * x_scale, end[0] * x_scale);
        let (sy, ey) = (16.0 - start[1] * y_scale, 16.0 - end[1] * y_scale);
        let (from, to) = match side {
            0 => ([sx, sy, 7.5], [ex, sy, 8.5]),
            1 => ([sx, ey, 7.5], [ex, ey, 8.5]),
            2 => ([sx, sy, 7.5], [sx, ey, 8.5]),
            _ => ([ex, sy, 7.5], [ex, ey, 8.5]),
        };
        let uvs = [u0 * x_scale, v0 * y_scale, u1 * x_scale, v1 * y_scale];
        out.push(quad(from, to, uvs, SIDES[side].2));
    }
    Some(out)
}

impl ItemModels {
    /// Bakes every item definition of `resources` and the models they name, with the
    /// sprites of the block and item atlases and the grass colour map.
    /// Streams definitions one file at a time so huge packs don't buffer all JSON in RAM.
    pub fn bake(
        resources: &Resources,
        blocks: &Atlas,
        items: &Atlas,
        grass: Option<&Image>,
    ) -> ItemModels {
        let listed = resources.list("items", |p| p.ends_with(".json"));
        let mut baker = Baker::new(resources, blocks, items, grass);
        let mut out = HashMap::new();
        let mut swap_scales = HashMap::new();
        for (id, r) in &listed {
            if resources.cancelled() {
                baker.warn("the load was cancelled".into());
                break;
            }
            let Some(name) = id
                .path()
                .strip_prefix("items/")
                .and_then(|p| p.strip_suffix(".json"))
            else {
                continue;
            };
            let Ok(text) = r.text() else {
                continue;
            };
            let item_id = format!("{}:{name}", id.namespace());
            baker.bake_definition(&item_id, &text, &mut out, &mut swap_scales);
        }
        ItemModels {
            definitions: out,
            swap_scales,
            meshes: baker.meshes,
            max_resolved: baker.limits.item_resolved_models,
            warnings: baker.warnings,
        }
    }

    /// The same, from the definitions' ids (`minecraft:stick`) and texts.
    pub fn bake_from(
        files: &dyn ModelFiles,
        definitions: &[(String, String)],
        blocks: &Atlas,
        items: &Atlas,
        grass: Option<&Image>,
    ) -> ItemModels {
        let mut baker = Baker::new(files, blocks, items, grass);
        let mut out = HashMap::new();
        let mut swap_scales = HashMap::new();
        for (id, text) in definitions {
            if files.cancelled() {
                baker.warn("the load was cancelled".into());
                break;
            }
            baker.bake_definition(id, text, &mut out, &mut swap_scales);
        }
        ItemModels {
            definitions: out,
            swap_scales,
            meshes: baker.meshes,
            max_resolved: baker.limits.item_resolved_models,
            warnings: baker.warnings,
        }
    }

    /// What the definition `id` draws for a stack held so, in order.
    pub fn resolve<'a>(&'a self, id: &str, held: &Held<'_>, out: &mut Vec<Drawn<'a>>) {
        self.resolve_each(id, held, &mut |leaf| out.push(leaf));
    }

    /// The same, each leaf given to `each` as it is found.
    pub fn resolve_each<'a>(&'a self, id: &str, held: &Held<'_>, each: &mut dyn FnMut(Drawn<'a>)) {
        if let Some(node) = self.definitions.get(id) {
            let mut left = if self.max_resolved == 0 {
                usize::MAX
            } else {
                self.max_resolved
            };
            node.resolve(held, &mut left, each);
        }
    }

    pub fn mesh(&self, index: u32) -> Option<&ItemMesh> {
        self.meshes.get(index as usize)
    }

    /// `swapAnimationScale` of the definition `id`: 1 unless it says otherwise.
    pub fn swap_scale(&self, id: &str) -> f32 {
        self.swap_scales.get(id).copied().unwrap_or(1.0)
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// How many models and quads were baked.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.meshes.len(),
            self.meshes.iter().map(|m| m.quads.len()).sum(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::atlas::{ATLASES, missing_id};
    use crate::image::Transparency;

    fn sprite(x: u32, size: u32, transparent: bool, translucent: bool) -> Sprite {
        Sprite {
            x,
            y: 0,
            padding: 0,
            width: size,
            height: size,
            transparency: Transparency {
                transparent,
                translucent,
            },
            blur: false,
            clamp: false,
            gui: None,
            animated: None,
        }
    }

    fn id(path: &str) -> Id {
        Id::minecraft(path).unwrap()
    }

    /// The blocks: the missing sprite and an opaque `block/a`, 16 texels each in 32 x 16.
    fn blocks() -> Atlas {
        let mut level = Image::new(32, 16);
        level.pixels.fill(0xFF80_8080);
        Atlas {
            def: ATLASES[1],
            width: 32,
            height: 16,
            max_level: 0,
            levels: vec![level],
            sprites: BTreeMap::from([
                (missing_id(), sprite(0, 16, false, false)),
                (id("block/a"), sprite(16, 16, false, false)),
            ]),
            animated: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// The items, 4 texels each in 16 x 4: the missing sprite, `item/dot` (one texel
    /// shown, at x 1 and y 2), `item/bar` (its top row shown) and `item/haze` (all of it
    /// half seen through).
    fn items() -> Atlas {
        let mut level = Image::new(16, 4);
        for y in 0..4 {
            for x in 0..16 {
                let alpha = match x {
                    0..4 => 255,
                    4..8 => u32::from((x, y) == (5, 2)) * 255,
                    8..12 => u32::from(y == 0) * 255,
                    _ => 128,
                };
                level.set(x, y, alpha << 24 | 0x40_4040);
            }
        }
        Atlas {
            def: ATLASES[2],
            width: 16,
            height: 4,
            max_level: 0,
            levels: vec![level],
            sprites: BTreeMap::from([
                (missing_id(), sprite(0, 4, false, false)),
                (id("item/dot"), sprite(4, 4, true, false)),
                (id("item/bar"), sprite(8, 4, true, false)),
                (id("item/haze"), sprite(12, 4, false, true)),
            ]),
            animated: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[derive(Default)]
    struct Files(HashMap<String, String>);

    impl ModelFiles for Files {
        fn model(&self, id: &Id) -> Option<String> {
            self.0.get(&id.to_string()).cloned()
        }

        fn blockstates(&self, _: &Id) -> Vec<String> {
            Vec::new()
        }
    }

    fn files() -> Files {
        let mut f = Files::default();
        let mut add = |name: &str, text: &str| {
            f.0.insert(format!("minecraft:{name}"), text.to_owned());
        };
        add(
            "item/generated",
            r#"{"parent": "builtin/generated", "display": {
                "thirdperson_righthand": {"rotation": [0, 0, 0], "translation": [0, 3, 1], "scale": [0.55, 0.55, 0.55]},
                "head": {"translation": [0, 200, -90], "scale": [9, -9, 1]}}}"#,
        );
        add(
            "item/handheld",
            r#"{"parent": "item/generated", "display": {
                "thirdperson_righthand": {"rotation": [0, -90, 55]},
                "thirdperson_lefthand": {"rotation": [0, 90, -55]}}}"#,
        );
        add(
            "item/dot",
            r#"{"parent": "item/generated", "textures": {"layer0": "item/dot"}}"#,
        );
        add(
            "item/two",
            r#"{"parent": "item/handheld", "textures": {"layer0": "item/bar", "layer1": "item/haze"}}"#,
        );
        add(
            "block/cube",
            r##"{"textures": {"all": "block/a"},
                "display": {"thirdperson_righthand": {"rotation": [75, 45, 0], "scale": [0.375, 0.375, 0.375]}},
                "elements": [{"from": [0, 0, 0], "to": [16, 8, 16], "faces": {
                    "up": {"texture": "#all", "tintindex": 0},
                    "north": {"texture": "#all"}}}]}"##,
        );
        add("item/loop", r#"{"parent": "item/loop"}"#);
        f
    }

    fn bake(definitions: &[(&str, &str)]) -> ItemModels {
        let definitions: Vec<(String, String)> = definitions
            .iter()
            .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
            .collect();
        ItemModels::bake_from(&files(), &definitions, &blocks(), &items(), None)
    }

    fn held<'a>(has: &'a dyn Fn(&str, bool) -> bool) -> Held<'a> {
        Held {
            context: "thirdperson_righthand",
            left_handed: false,
            using: false,
            use_ticks: 0,
            use_remaining: 0,
            damage: None,
            count: (1, 64),
            charge: "none",
            charge_ticks: 25.0,
            trim_material: None,
            block_state: &[],
            has_component: has,
        }
    }

    const NONE: &dyn Fn(&str, bool) -> bool = &|_, _| false;

    fn model(models: &ItemModels, id: &str, held: &Held<'_>) -> Vec<u32> {
        let mut out = Vec::new();
        models.resolve(id, held, &mut out);
        out.iter()
            .map(|d| match d {
                Drawn::Model { mesh, .. } => *mesh,
                Drawn::Special { base, .. } => *base,
            })
            .collect()
    }

    #[test]
    fn a_picture_becomes_a_flat_model_with_sides_around_what_it_shows() {
        let models = bake(&[(
            "a:dot",
            r#"{"model": {"type": "model", "model": "item/dot"}}"#,
        )]);
        assert!(models.warnings.is_empty(), "{:?}", models.warnings);
        let mesh = models
            .mesh(model(&models, "a:dot", &held(NONE))[0])
            .unwrap();
        // The front, the back, and the four sides of its one texel.
        assert_eq!(mesh.quads.len(), 6);
        assert_eq!(
            mesh.runs,
            [Run {
                block_atlas: false,
                translucent: false,
                tint_index: 0,
                first: 0,
                count: 6
            }]
        );
        let (front, back) = (&mesh.quads[0], &mesh.quads[1]);
        assert_eq!((front.direction, back.direction), (3, 2));
        assert!(front.positions.iter().all(|p| p[2] == 8.5 / 16.0));
        assert!(back.positions.iter().all(|p| p[2] == 7.5 / 16.0));
        // Both show the whole picture: a quarter of the atlas's width, from its second
        // quarter on.
        for quad in [front, back] {
            let us: Vec<f32> = quad.uvs.iter().map(|uv| uv[0]).collect();
            assert!(us.iter().all(|u| *u == 0.25 || *u == 0.5), "{us:?}");
            assert!(quad.uvs.iter().all(|uv| uv[1] == 0.0 || uv[1] == 1.0));
        }
        // The texel at (1, 2) of four spans x 4 to 8 and y 4 to 8 of sixteen, y from
        // the top; its sides face up, down, east and west, each a sixteenth deep.
        let sides: Vec<u8> = mesh.quads[2..].iter().map(|q| q.direction).collect();
        assert_eq!(sides, [1, 0, 4, 5]);
        for side in &mesh.quads[2..] {
            for p in side.positions {
                assert!(p[0] == 0.25 || p[0] == 0.5, "{p:?}");
                assert!(p[1] == 0.25 || p[1] == 0.5, "{p:?}");
                assert!(p[2] == 7.5 / 16.0 || p[2] == 8.5 / 16.0);
            }
            // They read inside their texel.
            for uv in side.uvs {
                assert!(uv[0] > 0.3125 && uv[0] < 0.375, "{uv:?}");
                assert!(uv[1] > 0.5 && uv[1] < 0.75, "{uv:?}");
            }
        }
        let top = &mesh.quads[2];
        assert!(top.positions.iter().all(|p| p[1] == 0.5));
        let (west, east) = (&mesh.quads[4], &mesh.quads[5]);
        assert!(west.positions.iter().all(|p| p[0] == 0.25));
        assert!(east.positions.iter().all(|p| p[0] == 0.5));
    }

    #[test]
    fn a_model_is_placed_as_the_first_of_its_parents_says() {
        let models = bake(&[
            (
                "a:dot",
                r#"{"model": {"type": "model", "model": "item/dot"}}"#,
            ),
            (
                "a:two",
                r#"{"model": {"type": "model", "model": "item/two"}}"#,
            ),
        ]);
        let dot = models
            .mesh(model(&models, "a:dot", &held(NONE))[0])
            .unwrap();
        // The right hand's place, and the left as the right where only that is named.
        let right = dot.hands[0].unwrap();
        assert_eq!(right.translation, [0.0, 0.1875, 0.0625]);
        assert_eq!((right.rotation, right.scale), ([0.0; 3], [0.55; 3]));
        assert_eq!(dot.hands[1], dot.hands[0]);
        // Never farther than five blocks, nor larger than four times.
        let head = dot.head.unwrap();
        assert_eq!(head.translation, [0.0, 5.0, -5.0]);
        assert_eq!(head.scale, [4.0, -4.0, 1.0]);
        // A child's own places come first, the others are its parents'.
        let two = models
            .mesh(model(&models, "a:two", &held(NONE))[0])
            .unwrap();
        assert_eq!(two.hands[0].unwrap().rotation, [0.0, -90.0, 55.0]);
        assert_eq!(two.hands[0].unwrap().scale, [1.0; 3]);
        assert_eq!(two.hands[1].unwrap().rotation, [0.0, 90.0, -55.0]);
        assert_eq!(two.head, dot.head);
        // Two pictures: each a run of its own tint, the one seen through after the other.
        let runs: Vec<(bool, i32, u32)> = two
            .runs
            .iter()
            .map(|r| (r.translucent, r.tint_index, r.count))
            .collect();
        // The bar: front, back, four sides above, four below and one at each end. The
        // haze: front, back and four sides along each of its edges.
        assert_eq!(runs, [(false, 0, 12), (true, 1, 18)]);
    }

    #[test]
    fn a_model_of_elements_is_of_the_atlas_of_its_pictures() {
        let models = bake(&[(
            "a:cube",
            r#"{"model": {"type": "model", "model": "block/cube", "tints": [
                {"type": "grass", "temperature": 0.5, "downfall": 1.0},
                {"type": "dye", "default": -6265536},
                {"type": "potion", "default": [1.0, 0.5, 0.0]},
                {"type": "constant", "value": 255},
                {"type": "firework", "default": 7}]}}"#,
        )]);
        let mut out = Vec::new();
        models.resolve("a:cube", &held(NONE), &mut out);
        let Drawn::Model { mesh, tints, .. } = out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(
            tints,
            [
                Tint::Constant(NO_GRASS),
                Tint::Dye(0xA0_6540),
                Tint::Potion(0xFF_7F00),
                Tint::Constant(0xFF),
                Tint::Default(7)
            ]
        );
        let mesh = models.mesh(mesh).unwrap();
        assert_eq!(mesh.quads.len(), 2);
        let runs: Vec<(bool, i32)> = mesh
            .runs
            .iter()
            .map(|r| (r.block_atlas, r.tint_index))
            .collect();
        assert_eq!(runs, [(true, -1), (true, 0)]);
        // The block atlas's second half.
        assert!(
            mesh.quads
                .iter()
                .all(|q| q.uvs.iter().all(|uv| uv[0] >= 0.5))
        );
        assert_eq!(mesh.hands[0].unwrap().scale, [0.375; 3]);
        assert_eq!(mesh.head, None);
    }

    #[test]
    fn grass_is_read_from_its_map() {
        let mut map = Image::new(256, 256);
        // A temperature of a half and a downfall of one: the middle of the middle row.
        map.set(127, 127, 0xFF12_3456);
        assert_eq!(grass(Some(&map), 0.5, 1.0), 0x12_3456);
        assert_eq!(grass(Some(&map), 1.0, 1.0), 0);
        assert_eq!(grass(None, 0.5, 1.0), NO_GRASS);
    }

    #[test]
    fn a_definition_chooses_by_the_stack_and_its_holder() {
        let leaf = |model: &str| format!(r#"{{"type": "model", "model": "{model}"}}"#);
        let (dot, two, cube) = (leaf("item/dot"), leaf("item/two"), leaf("block/cube"));
        let models = bake(&[
            ("a:dot", &format!(r#"{{"model": {dot}}}"#)),
            ("a:two", &format!(r#"{{"model": {two}}}"#)),
            ("a:cube", &format!(r#"{{"model": {cube}}}"#)),
            (
                "a:using",
                &format!(
                    r#"{{"model": {{"type": "condition", "property": "using_item", "on_true": {two}, "on_false": {dot}}}}}"#
                ),
            ),
            (
                "a:broken",
                &format!(
                    r#"{{"model": {{"type": "condition", "property": "minecraft:broken", "on_true": {two}, "on_false": {dot}}}}}"#
                ),
            ),
            (
                "a:has",
                &format!(
                    r#"{{"model": {{"type": "condition", "property": "has_component", "component": "dyed_color", "on_true": {two}, "on_false": {dot}}}}}"#
                ),
            ),
            (
                "a:where",
                &format!(
                    r#"{{"model": {{"type": "select", "property": "display_context", "cases": [
                    {{"when": ["gui", "ground"], "model": {two}}}, {{"when": "head", "model": {cube}}}], "fallback": {dot}}}}}"#
                ),
            ),
            (
                "a:charge",
                &format!(
                    r#"{{"model": {{"type": "select", "property": "charge_type", "cases": [
                    {{"when": "rocket", "model": {two}}}]}}}}"#
                ),
            ),
            (
                "a:pull",
                &format!(
                    r#"{{"model": {{"type": "range_dispatch", "property": "use_duration", "scale": 0.05, "entries": [
                    {{"threshold": 0.9, "model": {cube}}}, {{"threshold": 0.65, "model": {two}}}], "fallback": {dot}}}}}"#
                ),
            ),
            (
                "a:both",
                &format!(
                    r#"{{"model": {{"type": "composite", "models": [{dot}, {{"type": "empty"}}, {cube}]}}}}"#
                ),
            ),
            (
                "a:pile",
                &format!(
                    r#"{{"model": {{"type": "range_dispatch", "property": "count", "normalize": false, "entries": [
                    {{"threshold": 2, "model": {two}}}], "fallback": {dot}}}}}"#
                ),
            ),
        ]);
        let none = held(NONE);
        let of = |id: &str| model(&models, id, &none)[0];
        let (dot, two, cube) = (of("a:dot"), of("a:two"), of("a:cube"));
        let using = |ticks| Held {
            using: true,
            use_ticks: ticks,
            ..held(NONE)
        };
        assert_eq!(model(&models, "a:using", &none), [dot]);
        assert_eq!(model(&models, "a:using", &using(0)), [two]);
        // Broken: one use left of what takes damage.
        let damaged = |d| Held {
            damage: Some((d, 10)),
            ..held(NONE)
        };
        assert_eq!(model(&models, "a:broken", &none), [dot]);
        assert_eq!(model(&models, "a:broken", &damaged(8)), [dot]);
        assert_eq!(model(&models, "a:broken", &damaged(9)), [two]);
        let dyed = |name: &str, _: bool| name == "minecraft:dyed_color";
        assert_eq!(model(&models, "a:has", &none), [dot]);
        assert_eq!(model(&models, "a:has", &held(&dyed)), [two]);
        // A case by one name or by several, else what is left.
        let at = |context| Held {
            context,
            ..held(NONE)
        };
        assert_eq!(model(&models, "a:where", &at("ground")), [two]);
        assert_eq!(model(&models, "a:where", &at("head")), [cube]);
        assert_eq!(model(&models, "a:where", &none), [dot]);
        // Without a case and without anything left: nothing.
        assert_eq!(model(&models, "a:charge", &none), [0u32; 0]);
        let loaded = Held {
            charge: "rocket",
            ..held(NONE)
        };
        assert_eq!(model(&models, "a:charge", &loaded), [two]);
        // The last threshold that is not over the value, whatever order they are in.
        assert_eq!(model(&models, "a:pull", &none), [dot]);
        assert_eq!(model(&models, "a:pull", &using(12)), [dot]);
        assert_eq!(model(&models, "a:pull", &using(13)), [two]);
        assert_eq!(model(&models, "a:pull", &using(17)), [two]);
        assert_eq!(model(&models, "a:pull", &using(18)), [cube]);
        // A value that is just a threshold has reached it.
        let pile = |count| Held {
            count: (count, 64),
            ..held(NONE)
        };
        assert_eq!(model(&models, "a:pile", &pile(1)), [dot]);
        assert_eq!(model(&models, "a:pile", &pile(2)), [two]);
        assert_eq!(model(&models, "a:both", &none), [dot, cube]);
        // What is not a definition, or has none, draws nothing.
        assert_eq!(model(&models, "a:none", &none), [0u32; 0]);
    }

    #[test]
    fn transformations_reach_the_leaves_one_after_the_other() {
        let models = bake(&[(
            "a:shield",
            r#"{"model": {"type": "condition", "property": "using_item",
                "transformation": {"translation": [1, 2, 3], "scale": [1, -1, -1]},
                "on_false": {"type": "special", "base": "item/dot", "model": {"type": "shield"},
                    "transformation": [1, 0, 0, 10,  0, 1, 0, 20,  0, 0, 1, 30,  0, 0, 0, 1]},
                "on_true": {"type": "special", "base": "item/dot", "model": {"type": "minecraft:trident"}}}}"#,
        )]);
        let mut out = Vec::new();
        models.resolve("a:shield", &held(NONE), &mut out);
        let Drawn::Special {
            special,
            transformation,
            ..
        } = out[0]
        else {
            panic!("{out:?}")
        };
        assert_eq!(special, Special::Shield);
        // Its own first, then its parent's: moved by (10, 20, 30), turned over, moved.
        assert_eq!(
            *transformation,
            [
                [1.0, 0.0, 0.0, 11.0],
                [0.0, -1.0, 0.0, -18.0],
                [0.0, 0.0, -1.0, -27.0]
            ]
        );
        let using = Held {
            using: true,
            ..held(NONE)
        };
        let mut out = Vec::new();
        models.resolve("a:shield", &using, &mut out);
        assert!(matches!(
            out[0],
            Drawn::Special {
                special: Special::Trident,
                transformation: [[1.0, 0.0, 0.0, 1.0], ..],
                ..
            }
        ));
        // A quarter turn about y by its axis and angle, and by its quaternion.
        let half = std::f32::consts::FRAC_PI_2;
        let by_axis = rotation(Some(&serde_json::json!({"axis": [0, 2, 0], "angle": half})));
        let s = (half / 2.0).sin();
        let by_quaternion = rotation(Some(&serde_json::json!([0.0, s, 0.0, s])));
        for r in [by_axis, by_quaternion] {
            assert!(
                (r[0][2] - 1.0).abs() < 1e-6 && (r[2][0] + 1.0).abs() < 1e-6,
                "{r:?}"
            );
            assert!(r[0][0].abs() < 1e-6 && (r[1][1] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn what_is_missing_or_broken_draws_nothing_and_is_said() {
        let models = bake(&[
            (
                "a:gone",
                r#"{"model": {"type": "model", "model": "item/gone"}}"#,
            ),
            (
                "a:loop",
                r#"{"model": {"type": "model", "model": "item/loop"}}"#,
            ),
            (
                "a:odd",
                r#"{"model": {"type": "something_new", "model": "item/dot"}}"#,
            ),
            ("a:broken", "{"),
        ]);
        let none = held(NONE);
        for id in ["a:gone", "a:loop"] {
            let mesh = models.mesh(model(&models, id, &none)[0]).unwrap();
            assert!(mesh.quads.is_empty(), "{id}");
        }
        assert_eq!(model(&models, "a:odd", &none), [0u32; 0]);
        assert_eq!(model(&models, "a:broken", &none), [0u32; 0]);
        assert_eq!(models.len(), 3);
        let said = models.warnings.join("\n");
        for part in [
            "item/gone is missing",
            "form a loop",
            "a:broken: not a definition",
        ] {
            assert!(said.contains(part), "{part}: {said}");
        }
    }
}
