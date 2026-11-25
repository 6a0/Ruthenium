//! Block models: every block state's quads as 26.3 bakes them from blockstate
//! definitions and model files, ready for a section mesher.
//!
//! Follows 26.3's `client/resources/model/**` (model files, parents, texture slots, the
//! face bakery) and `client/renderer/block/dispatch/**` (blockstate definitions, variants,
//! multipart). Facts kept from there, cited where they are used: the vertex order of each
//! face, the default texture coordinates, the order of rotations, and how a variant is
//! picked from a position's seed.
//!
//! What comes out per block state is a list of *parts* to draw (one per chosen variant,
//! several for a multipart block), each a list of quads grouped by the face of the block
//! they are culled with. A quad carries its four corners in block space, its texture
//! coordinates in atlas texels, the direction it faces, its tint layer, and which of the
//! three terrain layers it belongs to.

use std::collections::HashMap;
use std::rc::Rc;

use ruthenium_data::block::BlockInfo;
use ruthenium_data::v26_3::BlockState;
use ruthenium_data::v26_3::blocks::STATE_COUNT;
use serde_json::Value;

use crate::atlas::{Atlas, Sprite};
use crate::id::Id;
use crate::image::Transparency;
use crate::resources::Resources;

/// The six directions in `Direction`'s order: down, up, north, south, west, east.
pub const FACE_NAMES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

/// Unit vectors of the six directions.
pub const NORMALS: [[f32; 3]; 6] = [
    [0.0, -1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, -1.0],
    [0.0, 0.0, 1.0],
    [-1.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
];

fn face_by_name(name: &str) -> Option<u8> {
    FACE_NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}

/// Which of an element's two corners each vertex of a face takes its x, y and z from
/// (`false`: `from`, `true`: `to`), per direction (`client/renderer/FaceInfo.java`).
const FACE_VERTICES: [[[bool; 3]; 4]; 6] = {
    const F: bool = false;
    const T: bool = true;
    [
        [[F, F, T], [F, F, F], [T, F, F], [T, F, T]],
        [[F, T, F], [F, T, T], [T, T, T], [T, T, F]],
        [[T, T, F], [T, F, F], [F, F, F], [F, T, F]],
        [[F, T, T], [F, F, T], [T, F, T], [T, T, T]],
        [[F, T, F], [F, F, F], [F, F, T], [F, T, T]],
        [[T, T, T], [T, F, T], [T, F, F], [T, T, F]],
    ]
};

pub(crate) type Vec3 = [f32; 3];
/// Row-major.
type Mat3 = [[f32; 3]; 3];

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    m
}

fn apply(m: &Mat3, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn transpose(m: &Mat3) -> Mat3 {
    std::array::from_fn(|r| std::array::from_fn(|c| m[c][r]))
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `Direction.getApproximateNearest`: the direction a vector points along most, north
/// for one that points nowhere.
fn nearest(v: Vec3) -> u8 {
    let mut best = 2;
    let mut highest = f32::from_bits(1);
    for (i, n) in NORMALS.iter().enumerate() {
        let d = dot(v, *n);
        if d > highest {
            highest = d;
            best = i as u8;
        }
    }
    best
}

/// A right-handed rotation about an axis (0 x, 1 y, 2 z) by `radians`.
fn rotation(axis: usize, radians: f32) -> Mat3 {
    let (s, c) = (
        f64::from(radians).sin() as f32,
        f64::from(radians).cos() as f32,
    );
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// A quarter turn of a blockstate rotation about an axis: clockwise seen from the
/// axis's positive end (`y: 90` turns north to east, `x: 90` turns up to north), with
/// exact entries (`OctahedralGroup.BLOCK_ROT_*`).
fn quarter(axis: usize) -> Mat3 {
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        1 => [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        _ => [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    }
}

fn quarters(axis: usize, count: u8) -> Mat3 {
    (0..count % 4).fold(IDENTITY, |m, _| mul(&quarter(axis), &m))
}

/// The rotation a variant gives its model: quarter turns about x, then y, then z
/// (`Quadrant.fromXYZAngles`), and whether textures stay as they were in the world
/// (`uvlock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModelRotation {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub uv_lock: bool,
}

impl ModelRotation {
    fn matrix(&self) -> Mat3 {
        mul(
            &quarters(2, self.z),
            &mul(&quarters(1, self.y), &quarters(0, self.x)),
        )
    }

    fn is_identity(&self) -> bool {
        self.x.is_multiple_of(4) && self.y.is_multiple_of(4) && self.z.is_multiple_of(4)
    }
}

/// How a face's texture coordinates turn so that the texture keeps its place in the
/// world under a model rotation (`BlockMath.getFaceTransformation`, inverted as the
/// bakery uses it): the rotation is taken into the face's own frame, in which every
/// face looks south, on both sides.
fn uv_lock_transform(model: &Mat3, face: u8) -> Mat3 {
    // Each direction's frame: south as it is, the others turned onto it.
    let half = std::f32::consts::FRAC_PI_2;
    let local_to_global = |f: u8| match f {
        0 => rotation(0, half),
        1 => rotation(0, -half),
        2 => rotation(1, 2.0 * half),
        3 => IDENTITY,
        4 => rotation(1, -half),
        _ => rotation(1, half),
    };
    let action = mul(model, &local_to_global(face));
    let new_side = nearest(apply(&action, [0.0, 0.0, 1.0]));
    let face_transform = mul(&transpose(&local_to_global(new_side)), &action);
    transpose(&face_transform)
}

/// Which terrain layer a quad is drawn in (`ChunkSectionLayer.byTransparency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Every texel the quad shows is opaque.
    Solid,
    /// Some are fully transparent, none partly.
    Cutout,
    /// Some are partly transparent.
    Translucent,
}

/// One baked quad.
#[derive(Debug, Clone, PartialEq)]
pub struct BakedQuad {
    /// The corners in block space (a full block spans 0 to 1), in the order they are
    /// drawn: the quad is the triangles 0-1-2 and 0-2-3.
    pub positions: [Vec3; 4],
    /// Texture coordinates of the corners, in texels of the atlas's level 0.
    pub uvs: [[f32; 2]; 4],
    /// The direction the quad faces (a `Direction` ordinal); up for a quad without area.
    pub direction: u8,
    /// The tint layer that colours it, or -1.
    pub tint_index: i32,
    /// The direction whose shade it takes instead of its own
    /// (`shade_direction_override`).
    pub shade_direction: Option<u8>,
    /// Light the element gives off by itself, 0 to 15.
    pub light_emission: u8,
    pub layer: Layer,
    /// Its sprite is animated.
    pub animated: bool,
}

/// One model as a variant places it: its quads, grouped by the face they are culled
/// with.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelPart {
    /// Unculled quads first, then those culled with north, south, east, west, up, down
    /// (the order `QuadCollection` lists them in).
    quads: Vec<BakedQuad>,
    /// Where each group ends in `quads`: unculled, then the six directions by ordinal.
    ends: [u32; 7],
    /// The model's `ambientocclusion` (the topmost model that says; else on).
    pub ambient_occlusion: bool,
}

/// The storage order of the culled groups, by direction ordinal: north, south, east,
/// west, up and down are the groups 1 to 6.
const GROUP_OF_FACE: [usize; 6] = [6, 5, 1, 2, 4, 3];

impl ModelPart {
    fn group(&self, group: usize) -> &[BakedQuad] {
        let start = if group == 0 { 0 } else { self.ends[group - 1] };
        &self.quads[start as usize..self.ends[group] as usize]
    }

    /// The quads drawn whatever is next to the block.
    pub fn unculled(&self) -> &[BakedQuad] {
        self.group(0)
    }

    /// The quads left out when the neighbour in `direction` hides that face.
    pub fn culled(&self, direction: usize) -> &[BakedQuad] {
        self.group(GROUP_OF_FACE[direction])
    }

    /// Every quad, in `QuadCollection.getAll`'s order.
    pub fn all(&self) -> &[BakedQuad] {
        &self.quads
    }

    fn from_groups(groups: [Vec<BakedQuad>; 7], ambient_occlusion: bool) -> ModelPart {
        let mut quads = Vec::with_capacity(groups.iter().map(Vec::len).sum());
        let mut ends = [0; 7];
        for (i, g) in groups.into_iter().enumerate() {
            quads.extend(g);
            ends[i] = quads.len() as u32;
        }
        ModelPart {
            quads,
            ends,
            ambient_occlusion,
        }
    }
}

/// A texture slot of a model file: another slot's name, or a sprite.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Slot {
    Reference(String),
    Material { sprite: Id, force_translucent: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Face {
    pub(crate) cull: Option<u8>,
    pub(crate) tint_index: i32,
    pub(crate) texture: String,
    /// `[u0, v0, u1, v1]` in sixteenths of the sprite.
    pub(crate) uvs: Option<[f32; 4]>,
    /// Quarter turns of the texture.
    pub(crate) rotation: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementRotation {
    /// In blocks.
    origin: Vec3,
    matrix: Mat3,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Element {
    pub(crate) from: Vec3,
    pub(crate) to: Vec3,
    /// By direction ordinal: the order the faces are baked in, whatever order the file
    /// has them in.
    pub(crate) faces: [Option<Face>; 6],
    pub(crate) rotation: Option<ElementRotation>,
    pub(crate) shade_direction: Option<u8>,
    pub(crate) light_emission: u8,
}

/// A model file as it stands, before its parents are followed.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RawModel {
    pub(crate) parent: Option<Id>,
    pub(crate) elements: Option<Vec<Element>>,
    pub(crate) ambient_occlusion: Option<bool>,
    pub(crate) textures: Vec<(String, Slot)>,
    /// How it is placed where items are shown (`display`), by the place's name.
    pub(crate) display: Vec<(String, crate::item::ItemTransform)>,
}

fn number(v: &Value, what: &str) -> Result<f32, String> {
    v.as_f64()
        .map(|n| n as f32)
        .ok_or_else(|| format!("{what} is not a number"))
}

fn vec3(v: Option<&Value>, what: &str) -> Result<Vec3, String> {
    let a = v
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing {what}"))?;
    if a.len() != 3 {
        return Err(format!("expected 3 {what} values, found {}", a.len()));
    }
    Ok([
        number(&a[0], what)?,
        number(&a[1], what)?,
        number(&a[2], what)?,
    ])
}

/// `Quadrant.parseJson`: degrees as quarter turns.
fn quadrant(degrees: i64) -> Result<u8, String> {
    match degrees.rem_euclid(360) {
        0 => Ok(0),
        90 => Ok(1),
        180 => Ok(2),
        270 => Ok(3),
        _ => Err(format!(
            "invalid rotation {degrees}: only 0, 90, 180 and 270 are allowed"
        )),
    }
}

fn int(v: Option<&Value>, default: i64, what: &str) -> Result<i64, String> {
    match v {
        None => Ok(default),
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .ok_or_else(|| format!("{what} is not a number")),
    }
}

fn parse_face(v: &Value) -> Result<Face, String> {
    let cull = v
        .get("cullface")
        .and_then(Value::as_str)
        .and_then(face_by_name);
    let uvs = match v.get("uv") {
        None => None,
        Some(uv) => {
            let a = uv.as_array().ok_or("uv is not a list")?;
            if a.len() != 4 {
                return Err(format!("expected 4 uv values, found {}", a.len()));
            }
            Some([
                number(&a[0], "uv")?,
                number(&a[1], "uv")?,
                number(&a[2], "uv")?,
                number(&a[3], "uv")?,
            ])
        }
    };
    Ok(Face {
        cull,
        tint_index: int(v.get("tintindex"), -1, "tintindex")? as i32,
        texture: v
            .get("texture")
            .and_then(Value::as_str)
            .ok_or("a face without a texture")?
            .to_owned(),
        uvs,
        rotation: quadrant(int(v.get("rotation"), 0, "rotation")?)?,
    })
}

/// An element's rotation about its origin (`CuboidRotation`): about one axis by an
/// angle, or about x, then y, then z; with `rescale`, stretched so that the rotated
/// element reaches as far along the axes as it did before.
fn parse_rotation(v: &Value) -> Result<ElementRotation, String> {
    let mut origin = vec3(v.get("origin"), "origin")?;
    for c in &mut origin {
        *c *= 0.0625;
    }
    let degrees = 0.017_453_292_f32;
    let float = |key: &str| match v.get(key) {
        None => Ok(0.0),
        Some(n) => number(n, key),
    };
    let mut matrix = if v.get("axis").is_none() && v.get("angle").is_none() {
        if v.get("x").is_none() && v.get("y").is_none() && v.get("z").is_none() {
            return Err("a rotation needs an axis and an angle, or x, y and z".into());
        }
        let (x, y, z) = (float("x")?, float("y")?, float("z")?);
        mul(
            &rotation(2, z * degrees),
            &mul(&rotation(1, y * degrees), &rotation(0, x * degrees)),
        )
    } else {
        let axis = match v
            .get("axis")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("x") => 0,
            Some("y") => 1,
            Some("z") => 2,
            other => return Err(format!("invalid rotation axis {other:?}")),
        };
        let angle = number(
            v.get("angle").ok_or("a rotation without an angle")?,
            "angle",
        )?;
        if angle == 0.0 {
            IDENTITY
        } else {
            rotation(axis, angle * degrees)
        }
    };
    let rescale = v.get("rescale").and_then(Value::as_bool).unwrap_or(false);
    if rescale && matrix != IDENTITY {
        // Each axis by the inverse of the largest component its unit vector turns to.
        let scale: Vec3 = std::array::from_fn(|axis| {
            let turned = [matrix[0][axis], matrix[1][axis], matrix[2][axis]];
            1.0 / turned.iter().fold(0.0_f32, |m, c| m.max(c.abs()))
        });
        for row in &mut matrix {
            for (c, s) in row.iter_mut().zip(scale) {
                *c *= s;
            }
        }
    }
    Ok(ElementRotation { origin, matrix })
}

fn parse_element(v: &Value) -> Result<Element, String> {
    let position = |key: &str| -> Result<Vec3, String> {
        let p = vec3(v.get(key), key)?;
        if p.iter().any(|c| !(-16.0..=32.0).contains(c)) {
            return Err(format!("'{key}' exceeds the allowed boundaries: {p:?}"));
        }
        Ok(p)
    };
    let (from, to) = (position("from")?, position("to")?);
    let rotation = v.get("rotation").map(parse_rotation).transpose()?;
    let mut faces: [Option<Face>; 6] = Default::default();
    let listed = v
        .get("faces")
        .and_then(Value::as_object)
        .ok_or("an element without faces")?;
    for (name, face) in listed {
        let direction = face_by_name(name).ok_or_else(|| format!("unknown facing {name:?}"))?;
        faces[usize::from(direction)] = Some(parse_face(face)?);
    }
    if faces.iter().all(Option::is_none) {
        return Err("an element needs between 1 and 6 faces".into());
    }
    let shade_direction = match v.get("shade_direction_override") {
        None => None,
        Some(name) => Some(
            name.as_str()
                .and_then(face_by_name)
                .ok_or_else(|| format!("unknown shade direction override {name}"))?,
        ),
    };
    let light_emission = match v.get("light_emission") {
        None => 0,
        Some(n) => n
            .as_i64()
            .filter(|n| (0..=15).contains(n))
            .ok_or("'light_emission' is not an integer from 0 to 15")? as u8,
    };
    Ok(Element {
        from,
        to,
        faces,
        rotation,
        shade_direction,
        light_emission,
    })
}

pub(crate) fn parse_model(text: &str) -> Result<RawModel, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !v.is_object() {
        return Err("not an object".into());
    }
    let elements = match v.get("elements") {
        None => None,
        Some(list) => Some(
            list.as_array()
                .ok_or("elements is not a list")?
                .iter()
                .map(parse_element)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let parent = match v.get("parent").and_then(Value::as_str) {
        None | Some("") => None,
        Some(p) => Some(Id::parse(p).ok_or_else(|| format!("bad parent {p:?}"))?),
    };
    let mut textures = Vec::new();
    if let Some(map) = v.get("textures") {
        for (slot, value) in map.as_object().ok_or("textures is not an object")? {
            let contents = match value {
                Value::String(s) if s.starts_with('#') => Slot::Reference(s[1..].to_owned()),
                Value::String(s) => Slot::Material {
                    sprite: Id::parse(s).ok_or_else(|| format!("bad texture {s:?}"))?,
                    force_translucent: false,
                },
                Value::Object(o) => {
                    let s = o
                        .get("sprite")
                        .and_then(Value::as_str)
                        .ok_or("a material without a sprite")?;
                    Slot::Material {
                        sprite: Id::parse(s).ok_or_else(|| format!("bad texture {s:?}"))?,
                        force_translucent: o
                            .get("force_translucent")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }
                }
                other => return Err(format!("texture {slot} is {other}")),
            };
            textures.push((slot.clone(), contents));
        }
    }
    Ok(RawModel {
        parent,
        elements,
        ambient_occlusion: match v.get("ambientocclusion") {
            None => None,
            Some(b) => Some(b.as_bool().ok_or("ambientocclusion is not true or false")?),
        },
        textures,
        display: crate::item::parse_display(v.get("display")),
    })
}

/// The model every missing or broken model is replaced by: a full block showing the
/// missing sprite (`MissingCuboidModel`).
fn missing_model() -> RawModel {
    let face = |direction| Face {
        cull: Some(direction),
        tint_index: -1,
        texture: "missingno".into(),
        uvs: Some([0.0, 0.0, 16.0, 16.0]),
        rotation: 0,
    };
    RawModel {
        parent: None,
        elements: Some(vec![Element {
            from: [0.0; 3],
            to: [16.0; 3],
            faces: std::array::from_fn(|d| Some(face(d as u8))),
            rotation: None,
            shade_direction: None,
            light_emission: 0,
        }]),
        ambient_occlusion: None,
        display: Vec::new(),
        textures: vec![
            ("particle".into(), Slot::Reference("missingno".into())),
            (
                "missingno".into(),
                Slot::Material {
                    sprite: crate::atlas::missing_id(),
                    force_translucent: false,
                },
            ),
        ],
    }
}

/// The texture coordinates a face without `uv` takes from its element's extent
/// (`FaceBakery.defaultFaceUV`).
fn default_uvs(from: Vec3, to: Vec3, face: u8) -> [f32; 4] {
    match face {
        0 => [from[0], 16.0 - to[2], to[0], 16.0 - from[2]],
        1 => [from[0], from[2], to[0], to[2]],
        2 => [16.0 - to[0], 16.0 - to[1], 16.0 - from[0], 16.0 - from[1]],
        3 => [from[0], 16.0 - to[1], to[0], 16.0 - from[1]],
        4 => [from[2], 16.0 - to[1], to[2], 16.0 - from[1]],
        _ => [16.0 - to[2], 16.0 - to[1], 16.0 - from[2], 16.0 - from[1]],
    }
}

/// A sprite as the bakery needs it.
pub(crate) struct Material<'a> {
    /// The atlas the sprite is of.
    pub(crate) atlas: &'a Atlas,
    pub(crate) sprite: &'a Sprite,
    pub(crate) force_translucent: bool,
}

/// The transparency of the texels a face shows, over every frame of its sprite
/// (`SpriteContents.computeTransparency`); `rect` is `[u0, v0, u1, v1]` as fractions of
/// the sprite. Coordinates outside the sprite are clamped into it.
fn rect_transparency(atlas: &Atlas, sprite: &Sprite, rect: [f32; 4]) -> Transparency {
    if sprite.transparency.is_opaque() || rect == [0.0, 0.0, 1.0, 1.0] {
        return sprite.transparency;
    }
    let (w, h) = (sprite.width, sprite.height);
    let clamp = |v: f32, max: u32| (v.max(0.0) as u32).min(max);
    let x0 = clamp((rect[0] * w as f32).floor(), w);
    let y0 = clamp((rect[1] * h as f32).floor(), h);
    let x1 = clamp((rect[2] * w as f32).ceil(), w);
    let y1 = clamp((rect[3] * h as f32).ceil(), h);
    let mut t = Transparency::default();
    let mut scan = |image: &crate::Image, ox: u32, oy: u32| {
        for y in y0..y1 {
            for x in x0..x1 {
                match crate::image::alpha(image.get(ox + x, oy + y)) {
                    0 => t.transparent = true,
                    255 => {}
                    _ => t.translucent = true,
                }
            }
        }
    };
    match sprite.animated.and_then(|i| atlas.animated.get(i)) {
        // Every frame the animation shows, where it lies in the sprite's image.
        Some(animated) => {
            let mut seen = Vec::new();
            for &(frame, _) in &animated.animation.frames {
                if seen.contains(&frame) {
                    continue;
                }
                seen.push(frame);
                let (fx, fy) = animated.animation.frame_pos(frame);
                scan(&animated.levels[0], fx * w, fy * h);
            }
        }
        // Level 0 of the atlas holds the sprite's image (only the colour of fully
        // transparent texels differs from the file).
        None => scan(
            &atlas.levels[0],
            sprite.x + sprite.padding,
            sprite.y + sprite.padding,
        ),
    }
    t
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// The direction a quad faces: the one nearest the normal of its first three corners,
/// the first in `Direction`'s order on a tie; none for a quad without area.
fn facing(p: &[Vec3; 4]) -> Option<u8> {
    let (a, b) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let inv = 1.0 / dot(n, n).sqrt();
    let n = [n[0] * inv, n[1] * inv, n[2] * inv];
    if !n.iter().all(|c| c.is_finite()) {
        return None;
    }
    let mut best = None;
    let mut closest = 0.0;
    for (i, d) in NORMALS.iter().enumerate() {
        let product = dot(n, *d);
        if product >= 0.0 && product > closest {
            closest = product;
            best = Some(i as u8);
        }
    }
    best
}

/// Puts a quad's corners in the order of the face it ended up facing, so that an
/// unrotated element always starts at the same corner whatever its model rotation
/// (`FaceBakery.recalculateWinding`). `false` if a corner of the bounding box has no
/// vertex (the quad is then left as it was).
fn rewind(positions: &mut [Vec3; 4], uvs: &mut [[f32; 2]; 4], direction: u8) -> bool {
    let mut min = [999.0_f32; 3];
    let mut max = [-999.0_f32; 3];
    for p in positions.iter() {
        for a in 0..3 {
            if p[a] < min[a] {
                min[a] = p[a];
            }
            if p[a] > max[a] {
                max[a] = p[a];
            }
        }
    }
    let (before_p, before_uv) = (*positions, *uvs);
    for (vertex, pick) in FACE_VERTICES[usize::from(direction)].iter().enumerate() {
        let wanted: Vec3 = std::array::from_fn(|a| if pick[a] { max[a] } else { min[a] });
        let Some(found) = (vertex..4).find(|i| positions[*i] == wanted) else {
            *positions = before_p;
            *uvs = before_uv;
            return false;
        };
        positions.swap(found, vertex);
        uvs.swap(found, vertex);
    }
    true
}

/// Bakes one face of an element (`FaceBakery.bakeQuad`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bake_quad(
    element: &Element,
    face: &Face,
    direction: u8,
    material: &Material<'_>,
    model: Option<&Mat3>,
    uv_lock: Option<&Mat3>,
) -> BakedQuad {
    let uv = face
        .uvs
        .unwrap_or_else(|| default_uvs(element.from, element.to, direction));
    let transparency = if material.force_translucent {
        Transparency {
            transparent: false,
            translucent: true,
        }
    } else {
        rect_transparency(
            material.atlas,
            material.sprite,
            [
                uv[0].min(uv[2]) / 16.0,
                uv[1].min(uv[3]) / 16.0,
                uv[0].max(uv[2]) / 16.0,
                uv[1].max(uv[3]) / 16.0,
            ],
        )
    };
    let sprite = material.sprite;
    let origin = [
        (sprite.x + sprite.padding) as f32,
        (sprite.y + sprite.padding) as f32,
    ];
    let mut positions = [[0.0; 3]; 4];
    let mut uvs = [[0.0; 2]; 4];
    for i in 0..4 {
        let pick = FACE_VERTICES[usize::from(direction)][i];
        let mut v: Vec3 =
            std::array::from_fn(|a| if pick[a] { element.to[a] } else { element.from[a] } / 16.0);
        if let Some(r) = &element.rotation {
            v = add(apply(&r.matrix, sub(v, r.origin)), r.origin);
        }
        if let Some(m) = model {
            v = add(apply(m, sub(v, [0.5; 3])), [0.5; 3]);
        }
        positions[i] = v;
        // The corner of the texture rectangle this vertex shows: the first two vertices
        // are on its left, the first and last at its top; the face's own rotation
        // shifts which is which.
        let corner = (i + usize::from(face.rotation)) % 4;
        let mut u = if corner == 0 || corner == 1 {
            uv[0]
        } else {
            uv[2]
        } / 16.0;
        let mut w = if corner == 0 || corner == 3 {
            uv[1]
        } else {
            uv[3]
        } / 16.0;
        if let Some(t) = uv_lock {
            let turned = apply(t, [u - 0.5, w - 0.5, 0.0]);
            u = turned[0] + 0.5;
            w = turned[1] + 0.5;
        }
        uvs[i] = [
            origin[0] + u * sprite.width as f32,
            origin[1] + w * sprite.height as f32,
        ];
    }
    let direction = facing(&positions);
    if element.rotation.is_none()
        && let Some(d) = direction
    {
        rewind(&mut positions, &mut uvs, d);
    }
    BakedQuad {
        positions,
        uvs,
        direction: direction.unwrap_or(1),
        tint_index: face.tint_index,
        shade_direction: element.shade_direction,
        light_emission: element.light_emission,
        layer: if transparency.translucent {
            Layer::Translucent
        } else if transparency.transparent {
            Layer::Cutout
        } else {
            Layer::Solid
        },
        animated: sprite.animated.is_some(),
    }
}

/// Bakes a model's elements under a variant's rotation
/// (`UnbakedCuboidGeometry.bake`). `material` resolves a face's texture slot.
pub(crate) fn bake_elements<'a>(
    elements: &[Element],
    rotation: ModelRotation,
    material: &dyn Fn(&str) -> Material<'a>,
) -> [Vec<BakedQuad>; 7] {
    let matrix = rotation.matrix();
    let model = (!rotation.is_identity()).then_some(&matrix);
    let mut groups: [Vec<BakedQuad>; 7] = Default::default();
    for e in elements {
        // An element flat along an axis draws only the two faces across it.
        let flat: [bool; 3] = std::array::from_fn(|a| e.from[a] == e.to[a]);
        let draws = |axis: usize| (0..3).all(|other| other == axis || !flat[other]);
        for (d, face) in e.faces.iter().enumerate() {
            let Some(face) = face else { continue };
            // Directions come in pairs along y, z and x.
            let axis = [1, 2, 0][d / 2];
            if !draws(axis) {
                continue;
            }
            let lock = (rotation.uv_lock && model.is_some())
                .then(|| uv_lock_transform(&matrix, d as u8))
                .filter(|t| *t != IDENTITY);
            let quad = bake_quad(
                e,
                face,
                d as u8,
                &material(&face.texture),
                model,
                lock.as_ref(),
            );
            match face.cull {
                None => groups[0].push(quad),
                Some(cull) => {
                    let turned = match model {
                        Some(m) => nearest(apply(m, NORMALS[usize::from(cull)])),
                        None => cull,
                    };
                    groups[GROUP_OF_FACE[usize::from(turned)]].push(quad);
                }
            }
        }
    }
    groups
}

/// Where model files and blockstate definitions are read from.
pub trait ModelFiles {
    /// The text of the model `id` (`models/<path>.json`), from the top pack that has it.
    fn model(&self, id: &Id) -> Option<String>;
    /// Every pack's definition of a block's states (`blockstates/<path>.json`), bottom
    /// pack first.
    fn blockstates(&self, block: &Id) -> Vec<String>;
}

impl ModelFiles for Resources {
    fn model(&self, id: &Id) -> Option<String> {
        let bytes = self.get(&id.wrap("models/", ".json"))?.read().ok()?;
        String::from_utf8(bytes.into_owned()).ok()
    }

    fn blockstates(&self, block: &Id) -> Vec<String> {
        self.stack(&block.wrap("blockstates/", ".json"))
            .iter()
            .filter_map(|r| String::from_utf8(r.read().ok()?.into_owned()).ok())
            .collect()
    }
}

/// One entry of a variant list: a model, how it is turned, and its weight.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Variant {
    model: Id,
    rotation: ModelRotation,
    weight: u32,
}

/// The models one condition gives a state: one, or one picked by weight.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    /// `(weight, part)`.
    entries: Vec<(u32, u32)>,
    total: u32,
    /// Written as a list in the file: a random number is drawn even for one entry.
    weighted: bool,
}

/// What a block state is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateModel {
    /// One choice (an index into `BlockModels::choices`).
    Simple(u32),
    /// The choices of every multipart condition that holds, a range of
    /// `BlockModels::multipart`.
    Multipart(u32, u32),
}

fn parse_variant(v: &Value) -> Result<Variant, String> {
    let model = v
        .get("model")
        .and_then(Value::as_str)
        .and_then(Id::parse)
        .ok_or("a variant without a model")?;
    let turn = |key: &str| quadrant(int(v.get(key), 0, key)?);
    let weight = int(v.get("weight"), 1, "weight")?;
    if weight < 1 {
        return Err(format!("weight {weight} is not positive"));
    }
    Ok(Variant {
        model,
        rotation: ModelRotation {
            x: turn("x")?,
            y: turn("y")?,
            z: turn("z")?,
            uv_lock: v.get("uvlock").and_then(Value::as_bool).unwrap_or(false),
        },
        weight: weight as u32,
    })
}

/// A variant or a list of them (`BlockStateModel.Unbaked.CODEC`).
fn parse_variants(v: &Value) -> Result<(Vec<Variant>, bool), String> {
    match v {
        Value::Array(list) if list.is_empty() => Err("an empty list of variants".into()),
        Value::Array(list) => Ok((
            list.iter().map(parse_variant).collect::<Result<_, _>>()?,
            true,
        )),
        other => Ok((vec![parse_variant(other)?], false)),
    }
}

/// A multipart condition (`Condition.CODEC`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    And(Vec<Condition>),
    Or(Vec<Condition>),
    /// Property index, and for each of its values whether it passes.
    Property(usize, Vec<bool>),
}

fn parse_condition(v: &Value, block: &BlockInfo) -> Result<Condition, String> {
    let map = v.as_object().ok_or("a condition is not an object")?;
    if map.len() == 1
        && let Some((key @ ("AND" | "OR"), terms)) = map.iter().next().map(|(k, t)| (k.as_str(), t))
    {
        let terms = terms
            .as_array()
            .ok_or("a combined condition is not a list")?
            .iter()
            .map(|t| parse_condition(t, block))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(if key == "AND" {
            Condition::And(terms)
        } else {
            Condition::Or(terms)
        });
    }
    if map.is_empty() {
        return Err("an empty condition".into());
    }
    let mut all = Vec::with_capacity(map.len());
    for (name, value) in map {
        let (index, property) = block
            .property(name)
            .ok_or_else(|| format!("unknown property '{name}'"))?;
        // Numbers and booleans are read as their text.
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            other => return Err(format!("the value of '{name}' is {other}")),
        };
        // Any of the terms separated by '|' may hold; '!' negates one.
        let mut terms = Vec::new();
        for term in text.split('|') {
            let (negated, value) = match term.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, term),
            };
            let value_index = property
                .index_of(value)
                .ok_or_else(|| format!("unknown value '{value}' for property '{name}'"))?;
            terms.push((negated, value_index));
        }
        let passes = (0..property.values.len())
            .map(|i| terms.iter().any(|&(negated, v)| (i == v) != negated))
            .collect();
        all.push(Condition::Property(index, passes));
    }
    Ok(if all.len() == 1 {
        all.remove(0)
    } else {
        Condition::And(all)
    })
}

impl Condition {
    fn holds(&self, block: &BlockInfo, state: u32) -> bool {
        match self {
            Condition::And(terms) => terms.iter().all(|t| t.holds(block, state)),
            Condition::Or(terms) => terms.iter().any(|t| t.holds(block, state)),
            Condition::Property(index, passes) => passes[block.value_index(state, *index)],
        }
    }
}

/// A key of `variants`: `property=value` pairs separated by commas, all of which a
/// state must have (`VariantSelector.predicate`).
fn parse_selector(key: &str, block: &BlockInfo) -> Result<Vec<(usize, usize)>, String> {
    let mut wanted = Vec::new();
    for pair in key.split(',') {
        let (name, value) = match pair.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (pair, None),
        };
        match (block.property(name), value) {
            (Some((index, property)), Some(value)) => {
                let v = property
                    .index_of(value)
                    .ok_or_else(|| format!("unknown value '{value}' for property '{name}'"))?;
                wanted.retain(|(i, _)| *i != index);
                wanted.push((index, v));
            }
            _ if name.is_empty() => {}
            _ => return Err(format!("unknown blockstate property '{name}'")),
        }
    }
    Ok(wanted)
}

/// `java.util.Random`'s generator, as the game's single-threaded random source runs it.
#[derive(Debug, Clone, Copy)]
struct JavaRandom(i64);

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> JavaRandom {
        JavaRandom((seed ^ Self::MULTIPLIER) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    fn next_long(&mut self) -> i64 {
        let upper = i64::from(self.next(32));
        let lower = i64::from(self.next(32));
        (upper << 32).wrapping_add(lower)
    }
}

/// Every block state's baked model.
#[derive(Debug, Clone)]
pub struct BlockModels {
    parts: Vec<ModelPart>,
    choices: Vec<Choice>,
    /// Choices of multipart states, in the order of their conditions.
    multipart: Vec<u32>,
    /// By state id.
    states: Vec<StateModel>,
    /// What went wrong on the way, for the log: files that could not be read, models
    /// and textures that are missing.
    pub warnings: Vec<String>,
}

struct Baker<'a> {
    files: &'a dyn ModelFiles,
    atlas: &'a Atlas,
    raw: HashMap<Id, Rc<RawModel>>,
    missing: Rc<RawModel>,
    baked: HashMap<(Id, ModelRotation), u32>,
    parts: Vec<ModelPart>,
    warnings: Vec<String>,
}

impl Baker<'_> {
    fn warn(&mut self, text: String) {
        // Enough to see what is wrong with a pack, not a line per state.
        if self.warnings.len() < 200 {
            self.warnings.push(text);
        }
    }

    fn raw(&mut self, id: &Id) -> Rc<RawModel> {
        if let Some(m) = self.raw.get(id) {
            return m.clone();
        }
        let model = match self.files.model(id).map(|text| parse_model(&text)) {
            Some(Ok(m)) => Rc::new(m),
            Some(Err(e)) => {
                self.warn(format!("model {id}: {e}"));
                self.missing.clone()
            }
            None => {
                self.warn(format!("model {id} is missing"));
                self.missing.clone()
            }
        };
        self.raw.insert(id.clone(), model.clone());
        model
    }

    /// A model and its parents, the model first. A parent that leads back to one of
    /// its children is replaced by the missing model.
    fn chain(&mut self, id: &Id) -> Vec<Rc<RawModel>> {
        let mut chain = vec![self.raw(id)];
        let mut seen = vec![id.clone()];
        while let Some(parent) = chain.last().and_then(|m| m.parent.clone()) {
            if seen.contains(&parent) {
                self.warn(format!("model {id}: its parents form a loop at {parent}"));
                chain.push(self.missing.clone());
                break;
            }
            chain.push(self.raw(&parent));
            seen.push(parent);
        }
        chain
    }

    /// Bakes a model under a rotation, once.
    fn part(&mut self, model: &Id, rotation: ModelRotation) -> u32 {
        let key = (model.clone(), rotation);
        if let Some(&i) = self.baked.get(&key) {
            return i;
        }
        let chain = self.chain(model);
        // Texture slots: a child's entry replaces its parents' (`TextureSlots.Resolver`),
        // then references are followed until nothing changes.
        let mut resolved: HashMap<&str, (&Id, bool)> = HashMap::new();
        let mut unresolved: Vec<(&str, &str)> = Vec::new();
        for m in chain.iter().rev() {
            for (slot, contents) in &m.textures {
                unresolved.retain(|(s, _)| s != slot);
                resolved.remove(slot.as_str());
                match contents {
                    Slot::Material {
                        sprite,
                        force_translucent,
                    } => {
                        resolved.insert(slot, (sprite, *force_translucent));
                    }
                    Slot::Reference(target) => unresolved.push((slot, target)),
                }
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            unresolved.retain(|(slot, target)| match resolved.get(target).copied() {
                Some(material) => {
                    resolved.insert(slot, material);
                    changed = true;
                    false
                }
                None => true,
            });
        }
        let atlas = self.atlas;
        let mut missing_textures = Vec::new();
        let groups = {
            let missing = std::cell::RefCell::new(&mut missing_textures);
            let material = |slot: &str| {
                let slot = slot.strip_prefix('#').unwrap_or(slot);
                match resolved.get(slot) {
                    Some(&(id, force_translucent)) => Material {
                        atlas,
                        sprite: atlas.sprite(id).unwrap_or_else(|| {
                            missing.borrow_mut().push(id.to_string());
                            atlas.missing()
                        }),
                        force_translucent,
                    },
                    None => {
                        missing.borrow_mut().push(format!("#{slot}"));
                        Material {
                            atlas,
                            sprite: atlas.missing(),
                            force_translucent: false,
                        }
                    }
                }
            };
            match chain.iter().find_map(|m| m.elements.as_deref()) {
                Some(elements) => bake_elements(elements, rotation, &material),
                None => Default::default(),
            }
        };
        let ambient_occlusion = chain
            .iter()
            .find_map(|m| m.ambient_occlusion)
            .unwrap_or(true);
        missing_textures.sort();
        missing_textures.dedup();
        if !missing_textures.is_empty() {
            self.warn(format!(
                "model {model}: no texture for {}",
                missing_textures.join(", ")
            ));
        }
        let index = self.parts.len() as u32;
        self.parts
            .push(ModelPart::from_groups(groups, ambient_occlusion));
        self.baked.insert(key, index);
        index
    }

    fn choice(&mut self, variants: &[Variant], weighted: bool) -> Choice {
        let entries: Vec<(u32, u32)> = variants
            .iter()
            .map(|v| (v.weight, self.part(&v.model, v.rotation)))
            .collect();
        Choice {
            total: entries.iter().map(|e| e.0).sum(),
            entries,
            weighted,
        }
    }
}

impl BlockModels {
    /// Bakes every block state's model from `files` with the sprites of the block
    /// `atlas`. A state no definition covers, or whose definition or model is broken,
    /// is drawn from the missing model.
    pub fn bake(files: &dyn ModelFiles, atlas: &Atlas) -> BlockModels {
        let missing = Rc::new(missing_model());
        let mut baker = Baker {
            files,
            atlas,
            raw: HashMap::new(),
            missing: missing.clone(),
            baked: HashMap::new(),
            parts: Vec::new(),
            warnings: Vec::new(),
        };
        let missing_id = Id::minecraft("builtin/missing").expect("a valid id");
        baker.raw.insert(missing_id.clone(), missing);
        let missing_part = baker.part(&missing_id, ModelRotation::default());
        let mut choices = vec![Choice {
            entries: vec![(1, missing_part)],
            total: 1,
            weighted: false,
        }];
        let mut multipart = Vec::new();
        let mut states = vec![StateModel::Simple(0); STATE_COUNT];
        let mut first = 0;
        while let Some(state) = BlockState::from_id(first) {
            let block = state.block();
            let info = block.info();
            first = info.states().end;
            let Some(id) = Id::parse(block.name()) else {
                continue;
            };
            for (pack, text) in files.blockstates(&id).iter().enumerate() {
                if let Err(e) = define(
                    &mut baker,
                    info,
                    text,
                    &mut choices,
                    &mut multipart,
                    &mut states,
                ) {
                    baker.warn(format!("blockstates of {id} (pack {pack}): {e}"));
                }
            }
        }
        BlockModels {
            parts: baker.parts,
            choices,
            multipart,
            states,
            warnings: baker.warnings,
        }
    }

    /// How many distinct parts were baked.
    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// The parts a block state is drawn from at a position with `seed` (its
    /// `getSeed`), appended to `out` in drawing order
    /// (`BlockStateModel.collectParts` with the renderer's random source set to the
    /// seed).
    pub fn parts<'a>(&'a self, state: u32, seed: i64, out: &mut Vec<&'a ModelPart>) {
        let mut random = JavaRandom::new(seed);
        let pick = |choice: u32, random: &mut JavaRandom| {
            let c = &self.choices[choice as usize];
            if !c.weighted {
                return &self.parts[c.entries[0].1 as usize];
            }
            let mut selection = random.next_int(c.total as i32) as u32;
            for &(weight, part) in &c.entries {
                if selection < weight {
                    return &self.parts[part as usize];
                }
                selection -= weight;
            }
            &self.parts[c.entries[0].1 as usize]
        };
        match self.states.get(state as usize) {
            None => {}
            Some(StateModel::Simple(choice)) => out.push(pick(*choice, &mut random)),
            Some(StateModel::Multipart(start, len)) => {
                // Every part of a multipart block draws from the same seed.
                let seed = random.next_long();
                for &choice in &self.multipart[*start as usize..(*start + *len) as usize] {
                    let mut random = JavaRandom::new(seed);
                    out.push(pick(choice, &mut random));
                }
            }
        }
    }

    /// Whether any part a state can be drawn from has a quad.
    pub fn is_empty(&self, state: u32) -> bool {
        let choices: &[u32] = match self.states.get(state as usize) {
            None => return true,
            Some(StateModel::Simple(c)) => std::slice::from_ref(c),
            Some(StateModel::Multipart(start, len)) => {
                &self.multipart[*start as usize..(*start + *len) as usize]
            }
        };
        choices.iter().all(|c| {
            self.choices[*c as usize]
                .entries
                .iter()
                .all(|e| self.parts[e.1 as usize].quads.is_empty())
        })
    }
}

/// Applies one pack's definition of a block's states: `variants` for the states their
/// keys select, `multipart` for every state they leave
/// (`BlockStateModelDispatcher.instantiate`).
fn define(
    baker: &mut Baker<'_>,
    block: &'static BlockInfo,
    text: &str,
    choices: &mut Vec<Choice>,
    multipart: &mut Vec<u32>,
    states: &mut [StateModel],
) -> Result<(), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let (variants, parts) = (v.get("variants"), v.get("multipart"));
    if variants.is_none() && parts.is_none() {
        return Err("neither 'variants' nor 'multipart' found".into());
    }
    let mut covered = vec![false; block.state_count as usize];
    if let Some(variants) = variants {
        let map = variants.as_object().ok_or("variants is not an object")?;
        if map.is_empty() {
            return Err("variants is empty".into());
        }
        for (key, value) in map {
            let (list, weighted) = parse_variants(value)?;
            let wanted = match parse_selector(key, block) {
                Ok(w) => w,
                Err(e) => {
                    baker.warn(format!("variant '{key}': {e}"));
                    continue;
                }
            };
            let choice = choices.len() as u32;
            choices.push(baker.choice(&list, weighted));
            for state in block.states() {
                if wanted
                    .iter()
                    .all(|&(property, value)| block.value_index(state, property) == value)
                {
                    states[state as usize] = StateModel::Simple(choice);
                    covered[(state - block.first_state) as usize] = true;
                }
            }
        }
    }
    if let Some(parts) = parts {
        let list = parts.as_array().ok_or("multipart is not a list")?;
        if list.is_empty() {
            return Err("multipart is empty".into());
        }
        let mut selectors = Vec::with_capacity(list.len());
        for part in list {
            let condition = part
                .get("when")
                .map(|w| parse_condition(w, block))
                .transpose()?;
            let (variants, weighted) =
                parse_variants(part.get("apply").ok_or("a part without 'apply'")?)?;
            let choice = choices.len() as u32;
            choices.push(baker.choice(&variants, weighted));
            selectors.push((condition, choice));
        }
        for state in block.states() {
            if covered[(state - block.first_state) as usize] {
                continue;
            }
            let start = multipart.len() as u32;
            for (condition, choice) in &selectors {
                if condition.as_ref().is_none_or(|c| c.holds(block, state)) {
                    multipart.push(*choice);
                }
            }
            states[state as usize] = StateModel::Multipart(start, multipart.len() as u32 - start);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ruthenium_data::v26_3::Block;

    use super::*;
    use crate::Image;
    use crate::atlas::{ATLASES, missing_id};

    /// A 64 x 32 atlas: the missing sprite at (0, 0), an opaque 16 x 16 sprite `a` at
    /// (16, 0), `holes` at (32, 0) whose left half is fully transparent, and `glass`
    /// at (48, 0) whose top row is half transparent; each without padding.
    fn atlas() -> Atlas {
        let mut level = Image::new(64, 32);
        for y in 0..16 {
            for x in 0..64 {
                let alpha = match x {
                    32..40 => 0,
                    48.. if y == 0 => 128,
                    _ => 255,
                };
                level.set(x, y, alpha << 24 | 0x80_8080);
            }
        }
        let sprite = |x: u32, transparent, translucent| Sprite {
            x,
            y: 0,
            padding: 0,
            width: 16,
            height: 16,
            transparency: Transparency {
                transparent,
                translucent,
            },
            blur: false,
            clamp: false,
            gui: None,
            animated: None,
        };
        let id = |path: &str| Id::minecraft(path).unwrap();
        Atlas {
            def: ATLASES[1],
            width: 64,
            height: 32,
            max_level: 0,
            levels: vec![level],
            sprites: BTreeMap::from([
                (missing_id(), sprite(0, false, false)),
                (id("block/a"), sprite(16, false, false)),
                (id("block/holes"), sprite(32, true, false)),
                (id("block/glass"), sprite(48, false, true)),
            ]),
            animated: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[derive(Default)]
    struct Files {
        models: HashMap<String, String>,
        states: HashMap<String, Vec<String>>,
    }

    impl ModelFiles for Files {
        fn model(&self, id: &Id) -> Option<String> {
            self.models.get(&id.to_string()).cloned()
        }

        fn blockstates(&self, block: &Id) -> Vec<String> {
            self.states
                .get(&block.to_string())
                .cloned()
                .unwrap_or_default()
        }
    }

    const CUBE: &str = r##"{
        "textures": {"all": "block/a", "side": "#all"},
        "elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {
            "down": {"texture": "#side", "cullface": "down"},
            "up": {"texture": "#side", "cullface": "up", "tintindex": 2},
            "north": {"texture": "#side", "cullface": "north"},
            "south": {"texture": "#side", "cullface": "south"},
            "west": {"texture": "#side", "cullface": "west"},
            "east": {"texture": "#side", "cullface": "east"}}}]}"##;

    /// A lopsided box with default texture coordinates: nothing about it is symmetric,
    /// so every rotation of it is told apart.
    const BOX: &str = r##"{
        "textures": {"t": "block/a"},
        "elements": [{"from": [1, 0, 3], "to": [12, 7, 16], "faces": {
            "down": {"texture": "#t", "cullface": "down"},
            "up": {"texture": "#t"},
            "north": {"texture": "#t"},
            "south": {"texture": "#t", "cullface": "south"},
            "west": {"texture": "#t"},
            "east": {"texture": "#t"}}}]}"##;

    fn elements(text: &str) -> Vec<Element> {
        parse_model(text).unwrap().elements.unwrap()
    }

    fn bake(text: &str, rotation: ModelRotation) -> ModelPart {
        let atlas = atlas();
        let id = Id::minecraft("block/a").unwrap();
        let material = |_: &str| Material {
            atlas: &atlas,
            sprite: atlas.sprite(&id).unwrap(),
            force_translucent: false,
        };
        ModelPart::from_groups(bake_elements(&elements(text), rotation, &material), true)
    }

    /// The texture coordinates (in sixteenths of the sprite `a`) a face without `uv`
    /// shows at a point of the block, by the direction it faces: what `default_uvs`
    /// gives each corner.
    fn world_uv(direction: u8, p: Vec3) -> [f32; 2] {
        let (x, y, z) = (p[0] * 16.0, p[1] * 16.0, p[2] * 16.0);
        match direction {
            0 => [x, 16.0 - z],
            1 => [x, z],
            2 => [16.0 - x, 16.0 - y],
            3 => [x, 16.0 - y],
            4 => [z, 16.0 - y],
            _ => [16.0 - z, 16.0 - y],
        }
    }

    fn sprite_uv(q: &BakedQuad, i: usize) -> [f32; 2] {
        [q.uvs[i][0] - 16.0, q.uvs[i][1]]
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn a_cube_bakes_six_culled_faces_in_the_bakery_vertex_order() {
        let part = bake(CUBE, ModelRotation::default());
        assert!(part.unculled().is_empty());
        assert_eq!(part.all().len(), 6);
        for d in 0..6 {
            let quads = part.culled(d);
            assert_eq!(quads.len(), 1, "{}", FACE_NAMES[d]);
            let q = &quads[0];
            assert_eq!(usize::from(q.direction), d);
            assert_eq!(q.layer, Layer::Solid);
            assert_eq!(q.tint_index, if d == 1 { 2 } else { -1 });
            for i in 0..4 {
                let pick = FACE_VERTICES[d][i];
                assert_eq!(q.positions[i], pick.map(|max| if max { 1.0 } else { 0.0 }));
                assert!(close(sprite_uv(q, i), world_uv(d as u8, q.positions[i])));
            }
            // Counter-clockwise seen from outside: the normal of the corners is the
            // face's direction.
            assert_eq!(facing(&q.positions), Some(d as u8));
        }
        // The up face starts at its north-west corner and goes south first.
        let up = &part.culled(1)[0];
        assert_eq!(up.positions[0], [0.0, 1.0, 0.0]);
        assert_eq!(up.positions[1], [0.0, 1.0, 1.0]);
        assert_eq!(up.uvs[0], [16.0, 0.0]);
        assert_eq!(up.uvs[2], [32.0, 16.0]);
    }

    #[test]
    fn y_90_turns_north_to_east_and_x_90_turns_up_to_north() {
        let turned = |x, y, z, v| {
            apply(
                &ModelRotation {
                    x,
                    y,
                    z,
                    uv_lock: false,
                }
                .matrix(),
                v,
            )
        };
        assert_eq!(turned(0, 1, 0, NORMALS[2]), NORMALS[5]);
        assert_eq!(turned(0, 2, 0, NORMALS[2]), NORMALS[3]);
        assert_eq!(turned(0, 3, 0, NORMALS[2]), NORMALS[4]);
        assert_eq!(turned(1, 0, 0, NORMALS[1]), NORMALS[2]);
        assert_eq!(turned(1, 0, 0, NORMALS[0]), NORMALS[3]);
        // x first, then y: an upright log with x 90 and y 90 lies along x.
        assert_eq!(turned(1, 1, 0, NORMALS[1]), NORMALS[5]);
        // z 90 turns up to east.
        assert_eq!(turned(0, 0, 1, NORMALS[1]), NORMALS[5]);
        // Culled faces follow: the box's south face is culled with east after y 270.
        let part = bake(
            BOX,
            ModelRotation {
                y: 3,
                ..ModelRotation::default()
            },
        );
        assert_eq!(part.culled(5).len(), 1);
        assert_eq!(part.culled(5)[0].direction, 5);
        assert_eq!(part.culled(0).len(), 1);
        assert_eq!(part.unculled().len(), 4);
    }

    #[test]
    fn every_rotation_keeps_geometry_winding_and_texture() {
        let plain = bake(BOX, ModelRotation::default());
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let rotation = ModelRotation {
                        x,
                        y,
                        z,
                        uv_lock: false,
                    };
                    let m = rotation.matrix();
                    let part = bake(BOX, rotation);
                    assert_eq!(part.all().len(), 6);
                    for q in part.all() {
                        // Still facing outwards, and starting at its face's first corner.
                        assert_eq!(facing(&q.positions), Some(q.direction));
                        // It is one of the unrotated quads, turned: every corner keeps
                        // its texture coordinates.
                        let back = transpose(&m);
                        let original = plain
                            .all()
                            .iter()
                            .find(|o| {
                                nearest(apply(&back, NORMALS[usize::from(q.direction)]))
                                    == o.direction
                            })
                            .unwrap();
                        for i in 0..4 {
                            let p = add(apply(&back, sub(q.positions[i], [0.5; 3])), [0.5; 3]);
                            let j = (0..4)
                                .find(|j| {
                                    (0..3).all(|a| (original.positions[*j][a] - p[a]).abs() < 1e-5)
                                })
                                .unwrap_or_else(|| panic!("{rotation:?}: no corner at {p:?}"));
                            assert!(
                                close(q.uvs[i], original.uvs[j]),
                                "{rotation:?}: {:?} is not {:?}",
                                q.uvs[i],
                                original.uvs[j]
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn uv_lock_keeps_the_texture_where_it_is_in_the_world() {
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let rotation = ModelRotation {
                        x,
                        y,
                        z,
                        uv_lock: true,
                    };
                    for q in bake(BOX, rotation).all() {
                        for i in 0..4 {
                            let (got, wanted) =
                                (sprite_uv(q, i), world_uv(q.direction, q.positions[i]));
                            assert!(
                                close(got, wanted),
                                "{rotation:?}, {}: {got:?} is not {wanted:?}",
                                FACE_NAMES[usize::from(q.direction)]
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_face_rotation_turns_the_texture_and_not_the_corners() {
        let model = r##"{"textures": {"t": "block/a"}, "elements": [{"from": [0, 0, 0],
            "to": [16, 16, 16], "faces": {"up": {"texture": "#t", "uv": [0, 0, 8, 4],
            "rotation": 90}}}]}"##;
        let part = bake(model, ModelRotation::default());
        let q = &part.unculled()[0];
        assert_eq!(q.positions[0], [0.0, 1.0, 0.0]);
        // Vertex 0 shows the corner vertex 1 would: left, bottom.
        assert_eq!(sprite_uv(q, 0), [0.0, 4.0]);
        assert_eq!(sprite_uv(q, 1), [8.0, 4.0]);
        assert_eq!(sprite_uv(q, 2), [8.0, 0.0]);
        assert_eq!(sprite_uv(q, 3), [0.0, 0.0]);
    }

    #[test]
    fn an_element_rotation_turns_about_its_origin_and_rescales() {
        // A plane through the block's middle, turned 45 degrees about y: one half of a
        // cross. Rescaled, it reaches the block's corners.
        let cross = |rescale: bool| {
            format!(
                r##"{{"textures": {{"t": "block/a"}}, "elements": [{{"from": [0, 0, 8],
                "to": [16, 16, 8], "rotation": {{"origin": [8, 8, 8], "axis": "y",
                "angle": 45, "rescale": {rescale}}}, "faces": {{
                "north": {{"texture": "#t"}}, "south": {{"texture": "#t"}}}}}}]}}"##
            )
        };
        let part = bake(&cross(true), ModelRotation::default());
        assert_eq!(part.unculled().len(), 2);
        for q in part.unculled() {
            for p in q.positions {
                assert!((p[0] - 0.5).abs() > 0.4999 && (p[0] - 0.5).abs() < 0.5001);
                assert!(((p[0] - 0.5).abs() - (p[2] - 0.5).abs()).abs() < 1e-6);
            }
        }
        let part = bake(&cross(false), ModelRotation::default());
        let reach = part.unculled()[0].positions[0][0] - 0.5;
        assert!((reach.abs() - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // The first faces a direction between two: the first of them in order wins.
        // A flat element draws only the faces across it.
        let flat = r##"{"textures": {"t": "block/a"}, "elements": [{"from": [0, 0, 8],
            "to": [16, 16, 8], "faces": {"north": {"texture": "#t"},
            "up": {"texture": "#t"}, "east": {"texture": "#t"}}}]}"##;
        assert_eq!(bake(flat, ModelRotation::default()).all().len(), 1);
    }

    #[test]
    fn the_layer_comes_from_the_texels_a_face_shows() {
        let atlas = atlas();
        let bake_with = |sprite: &str, uv: &str, force: bool| {
            let id = Id::minecraft(sprite).unwrap();
            let material = |_: &str| Material {
                atlas: &atlas,
                sprite: atlas.sprite(&id).unwrap(),
                force_translucent: force,
            };
            let model = format!(
                r##"{{"elements": [{{"from": [0, 0, 0], "to": [16, 16, 16], "faces":
                {{"up": {{"texture": "#t"{uv}}}}}}}]}}"##
            );
            bake_elements(&elements(&model), ModelRotation::default(), &material)[0][0].layer
        };
        assert_eq!(bake_with("block/a", "", false), Layer::Solid);
        assert_eq!(bake_with("block/a", "", true), Layer::Translucent);
        assert_eq!(bake_with("block/holes", "", false), Layer::Cutout);
        // The right half of the sprite has no hole, and a mirrored face shows the same
        // texels.
        let right = r#", "uv": [8, 0, 16, 16]"#;
        assert_eq!(bake_with("block/holes", right, false), Layer::Solid);
        let mirrored = r#", "uv": [16, 0, 8, 16]"#;
        assert_eq!(bake_with("block/holes", mirrored, false), Layer::Solid);
        let touching = r#", "uv": [7.5, 0, 16, 16]"#;
        assert_eq!(bake_with("block/holes", touching, false), Layer::Cutout);
        assert_eq!(bake_with("block/glass", "", false), Layer::Translucent);
        let below = r#", "uv": [0, 1, 16, 16]"#;
        assert_eq!(bake_with("block/glass", below, false), Layer::Solid);
    }

    #[test]
    fn parents_give_elements_and_children_override_textures() {
        let mut files = Files::default();
        files
            .models
            .insert("minecraft:block/cube".into(), CUBE.into());
        files.models.insert(
            "minecraft:block/child".into(),
            r##"{"parent": "block/cube", "ambientocclusion": false,
                "textures": {"all": "block/holes"}}"##
                .into(),
        );
        files.models.insert(
            "minecraft:block/loop".into(),
            r#"{"parent": "block/loop"}"#.into(),
        );
        files.models.insert(
            "minecraft:block/unset".into(),
            r##"{"parent": "block/cube", "textures": {"all": "#nowhere"}}"##.into(),
        );
        let atlas = atlas();
        let mut baker = Baker {
            files: &files,
            atlas: &atlas,
            raw: HashMap::new(),
            missing: Rc::new(missing_model()),
            baked: HashMap::new(),
            parts: Vec::new(),
            warnings: Vec::new(),
        };
        let id = |path: &str| Id::minecraft(path).unwrap();
        let child = baker.part(&id("block/child"), ModelRotation::default());
        assert_eq!(
            child,
            baker.part(&id("block/child"), ModelRotation::default())
        );
        let part = &baker.parts[child as usize];
        assert!(!part.ambient_occlusion);
        // The parent's "side" refers to "all", which the child set to the sprite at 32.
        assert_eq!(part.all().len(), 6);
        assert!(part.all().iter().all(|q| q.uvs[0][0] >= 32.0));
        assert!(part.all().iter().all(|q| q.layer == Layer::Cutout));
        assert!(baker.warnings.is_empty(), "{:?}", baker.warnings);
        // A missing file, a loop and an unset slot all show the missing sprite on a cube.
        for broken in ["block/absent", "block/loop", "block/unset"] {
            let i = baker.part(&id(broken), ModelRotation::default());
            let part = &baker.parts[i as usize];
            assert_eq!(part.all().len(), 6, "{broken}");
            assert!(part.all().iter().all(|q| q.uvs[0][0] <= 16.0), "{broken}");
            assert!(part.ambient_occlusion);
        }
        assert_eq!(baker.warnings.len(), 3, "{:?}", baker.warnings);
    }

    #[test]
    fn variants_and_multipart_select_by_state() {
        let fence = Block::from_name("minecraft:oak_fence").unwrap().info();
        let state = |values: &[(&str, &str)]| {
            values.iter().fold(fence.default_state, |s, (p, v)| {
                fence.with_value(s, p, v).unwrap()
            })
        };
        let holds = |json: &str, s: u32| {
            parse_condition(&serde_json::from_str(json).unwrap(), fence)
                .unwrap()
                .holds(fence, s)
        };
        let north = state(&[("north", "true")]);
        let plain = state(&[]);
        assert!(holds(r#"{"north": "true"}"#, north));
        assert!(!holds(r#"{"north": "true"}"#, plain));
        assert!(holds(r#"{"north": true}"#, north));
        assert!(holds(r#"{"north": "!true"}"#, plain));
        assert!(holds(r#"{"north": "true|false"}"#, plain));
        assert!(!holds(r#"{"north": "true", "east": "true"}"#, north));
        assert!(holds(
            r#"{"OR": [{"north": "true"}, {"east": "true"}]}"#,
            north
        ));
        assert!(!holds(
            r#"{"AND": [{"north": "true"}, {"east": "true"}]}"#,
            north
        ));
        let condition = |json: &str| parse_condition(&serde_json::from_str(json).unwrap(), fence);
        assert!(condition(r#"{"north": "maybe"}"#).is_err());
        assert!(condition(r#"{"up": "true"}"#).is_err());
        assert!(condition("{}").is_err());

        let selects = |key: &str, s: u32| {
            parse_selector(key, fence)
                .unwrap()
                .iter()
                .all(|&(p, v)| fence.value_index(s, p) == v)
        };
        assert!(selects("", plain));
        assert!(selects("north=true", north));
        assert!(!selects("north=true,east=true", north));
        assert!(parse_selector("normal", fence).is_err());
        assert!(parse_selector("north=maybe", fence).is_err());
    }

    #[test]
    fn states_get_their_parts_and_weighted_ones_follow_the_seed() {
        let mut files = Files::default();
        files
            .models
            .insert("minecraft:block/cube".into(), CUBE.into());
        files
            .models
            .insert("minecraft:block/box".into(), BOX.into());
        files.states.insert(
            "minecraft:stone".into(),
            vec![
                r#"{"variants": {"": [{"model": "block/cube"},
                {"model": "block/box", "weight": 3}]}}"#
                    .into(),
            ],
        );
        files.states.insert(
            "minecraft:oak_fence".into(),
            vec![
                r#"{"multipart": [{"apply": {"model": "block/cube"}},
                {"when": {"north": "true"}, "apply": {"model": "block/box", "uvlock": true}},
                {"when": {"east": "true"}, "apply": {"model": "block/box", "y": 90}}]}"#
                    .into(),
            ],
        );
        // A later pack's definition replaces an earlier one's, state by state.
        files.states.insert(
            "minecraft:oak_slab".into(),
            vec![
                r#"{"variants": {"": {"model": "block/cube"}}}"#.into(),
                r#"{"variants": {"type=top": {"model": "block/box"}}}"#.into(),
            ],
        );
        let atlas = atlas();
        let models = BlockModels::bake(&files, &atlas);
        assert!(models.warnings.is_empty(), "{:?}", models.warnings);
        let quads = |state: u32, seed: i64| {
            let mut parts = Vec::new();
            models.parts(state, seed, &mut parts);
            parts.iter().map(|p| p.all().len()).collect::<Vec<_>>()
        };
        // The cube has six culled faces; the box four unculled ones.
        let is_cube = |state: u32, seed: i64| {
            let mut parts = Vec::new();
            models.parts(state, seed, &mut parts);
            parts[0].unculled().is_empty()
        };
        let stone = Block::from_name("minecraft:stone")
            .unwrap()
            .info()
            .default_state;
        let cubes = (0..4000)
            .filter(|seed| is_cube(stone, *seed * 7919))
            .count();
        assert!((900..1100).contains(&cubes), "{cubes} of 4000");
        assert_eq!(is_cube(stone, 42), is_cube(stone, 42));

        let fence = Block::from_name("minecraft:oak_fence").unwrap().info();
        let both = fence
            .with_value(fence.default_state, "north", "true")
            .and_then(|s| fence.with_value(s, "east", "true"))
            .unwrap();
        assert_eq!(quads(fence.default_state, 1), vec![6]);
        assert_eq!(quads(both, 1), vec![6, 6, 6]);

        let slab = Block::from_name("minecraft:oak_slab").unwrap().info();
        let top = slab.with_value(slab.default_state, "type", "top").unwrap();
        let bottom = slab
            .with_value(slab.default_state, "type", "bottom")
            .unwrap();
        assert!(!is_cube(top, 0));
        assert!(is_cube(bottom, 0));

        // A block without a definition shows the missing cube; so does a state of an
        // unknown id's nothing.
        let dirt = Block::from_name("minecraft:dirt")
            .unwrap()
            .info()
            .default_state;
        assert_eq!(quads(dirt, 0), vec![6]);
        assert!(!models.is_empty(dirt));
        assert_eq!(quads(u32::MAX, 0), Vec::<usize>::new());
        assert!(models.is_empty(u32::MAX));
    }

    #[test]
    fn the_random_source_is_javas() {
        // What a JVM prints for java.util.Random: seed 42, nextInt(10) four times; seed
        // 42, nextLong(); seed 0, nextInt(16); seed 7, nextInt(7), nextInt(1000),
        // nextInt(3).
        let mut r = JavaRandom::new(42);
        assert_eq!(
            [
                r.next_int(10),
                r.next_int(10),
                r.next_int(10),
                r.next_int(10)
            ],
            [0, 3, 8, 4]
        );
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_long(), -5_025_562_857_975_149_833);
        assert_eq!(JavaRandom::new(0).next_int(16), 11);
        let mut r = JavaRandom::new(7);
        assert_eq!(
            [r.next_int(7), r.next_int(1000), r.next_int(3)],
            [3, 164, 0]
        );
    }

    #[test]
    fn broken_files_are_errors() {
        assert!(parse_model("[]").is_err());
        assert!(
            parse_model(
                r#"{"elements": [{"from": [0, 0, 0], "to": [33, 1, 1],
            "faces": {"up": {"texture": "t"}}}]}"#
            )
            .is_err()
        );
        assert!(
            parse_model(
                r#"{"elements": [{"from": [0, 0, 0], "to": [1, 1, 1],
            "faces": {}}]}"#
            )
            .is_err()
        );
        assert!(
            parse_model(
                r#"{"elements": [{"from": [0, 0, 0], "to": [1, 1, 1],
            "faces": {"top": {"texture": "t"}}}]}"#
            )
            .is_err()
        );
        assert!(
            parse_model(
                r#"{"elements": [{"from": [0, 0, 0], "to": [1, 1, 1],
            "faces": {"up": {"texture": "t", "rotation": 45}}}]}"#
            )
            .is_err()
        );
        assert!(
            parse_model(
                r#"{"elements": [{"from": [0, 0, 0], "to": [1, 1, 1],
            "light_emission": 16, "faces": {"up": {"texture": "t"}}}]}"#
            )
            .is_err()
        );
        assert!(parse_variant(&serde_json::json!({"model": "a", "y": 45})).is_err());
        assert!(parse_variant(&serde_json::json!({"model": "a", "weight": 0})).is_err());
        assert_eq!(
            parse_variant(&serde_json::json!({"model": "a", "y": -90}))
                .unwrap()
                .rotation
                .y,
            3
        );
        // An unknown cull face is no cull face.
        let m = parse_model(
            r#"{"elements": [{"from": [0, 0, 0], "to": [1, 1, 1],
            "faces": {"up": {"texture": "t", "cullface": "bottom"}}}]}"#,
        )
        .unwrap();
        assert_eq!(m.elements.unwrap()[0].faces[1].as_ref().unwrap().cull, None);
    }
}
