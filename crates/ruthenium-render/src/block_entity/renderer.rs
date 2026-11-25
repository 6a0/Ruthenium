//! Draws block entities: instances of model parts, gathered each frame from the sections in
//! view, a draw for all instances of one part in one layer; the text of signs, a draw for
//! all glyphs; and players, from the same parts as their heads, with their shadows under
//! them and their names over them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ruthenium_assets::Font;
use ruthenium_assets::item::{Drawn, Held, ItemMesh, ItemModels, ItemTransform, Special, Tint};
use ruthenium_data::v26_3::registries::DataComponentType;
use ruthenium_gpu::{
    Arrival, BufferPool, Error, FrameData, Gpu, Pass, PoolConfig, RangeHandle, TextureDesc,
    TextureId, TextureTable, UploadRing, UploadTicket, VertexAttribute, VertexInput, vk,
};
use ruthenium_protocol::nbt;
use ruthenium_protocol::profile::ResolvableProfile;
use ruthenium_protocol::text::{Style, StyledText};
use ruthenium_shaders::interface::{GLINT, GLYPH, MODEL_PART, SHADOW, SIGN};
use ruthenium_sim::avatar::{AvatarRecord, DEFAULT_SKINS, SkinSource};
use ruthenium_sim::environment::CardinalLight;
use ruthenium_sim::item::{Components, EquipmentSlot, Stack, Trim};
use ruthenium_sim::scene::{Frame, SectionKey, Terrain};
use ruthenium_sim::scoreboard::{NameRecord, NameText};
use ruthenium_ui::Quad;
use ruthenium_ui::hud::{HudItem, HudText};

use super::affine::Affine;
use super::avatar::{self, Figure, Moment};
use super::equipment::{self, EquipmentSet};
use super::hand::{self, Hands, Shown};
use super::model::{Model, ModelSet, PartDraw, Pose, Skull};
use super::nametag;
use super::place::{self, Chest, DYE_COLOURS, DYES, Kind};
use super::shadow::{self, Ground, Piece};
use super::sign::{self, GLYPH_WORDS, GlyphPages, SignText};
use super::skins::{LoadedSkin, Look, Skins, default_path};
use crate::atmosphere::Fog;
use crate::pipelines::PipelineId;
use crate::sky::SkyImage;
use crate::terrain::table::BlockTable;
use crate::terrain::world::write_world;
use crate::terrain::{CameraPose, View};

/// Bytes of a vertex: position (blocks) and texture coordinate as floats, and the normal.
pub const VERTEX_BYTES: usize = 24;

/// The layout of the models' vertices: `model.vert`'s inputs.
pub const MODEL_VERTEX: VertexInput = VertexInput {
    stride: VERTEX_BYTES as u32,
    attributes: &[
        VertexAttribute {
            offset: 0,
            format: vk::Format::R32G32B32_SFLOAT,
        },
        VertexAttribute {
            offset: 12,
            format: vk::Format::R32G32_SFLOAT,
        },
        VertexAttribute {
            offset: 20,
            format: vk::Format::R8G8B8A8_SNORM,
        },
    ],
};

/// Where the models' push constants are, in bytes: four words the vertex stage reads, then
/// the fog as the terrain's fragments read it. `model_frag.glsl` declares the same block.
pub(crate) const PUSH_FOG_COLOUR: usize = 16;
pub(crate) const PUSH_FOG_ENVIRONMENTAL: usize = 32;
pub(crate) const PUSH_FOG_RENDER_DISTANCE: usize = 40;
pub(crate) const MODEL_PUSH_BYTES: usize = 48;

/// Block entities as far as this from the camera are drawn, in blocks.
const VIEW_DISTANCE: f64 = 64.0;
/// How far a section has to have faded in for its block entities to be drawn.
pub(crate) const MIN_VISIBILITY: f32 = 0.3;
/// Skulls are drawn this much nearer to the camera than they are.
const TOWARDS_CAMERA: f32 = 1.0 - 1.0 / 4096.0;
/// Patterns of one banner that are drawn.
const MAX_PATTERNS: usize = 16;
/// Banners whose patterns, and signs whose text, are kept between frames.
const KEPT_PATTERNS: usize = 4096;
const KEPT_SIGNS: usize = 4096;
const KEPT_HEADS: usize = 4096;
const KEPT_NAMES: usize = 4096;
/// How far a player's shadow reaches from its middle, for a player of the usual size, and
/// how strong it is (`AvatarRenderer`'s shadow radius, `EntityRenderer.shadowStrength`).
const PLAYER_SHADOW: f32 = 0.5;
const SHADOW_STRENGTH: f32 = 1.0;
/// Pieces of shadows drawn in a frame at most.
const MAX_SHADOW_PIECES: usize = 1 << 16;
/// The bit of a text's light that says it is drawn in none (`sign_vert.glsl`).
const UNLIT: u32 = 1 << 31;
/// How near the player has to be for glowing text to have its outline, in blocks.
const OUTLINE_DISTANCE: f64 = 16.0;
/// Bytes of the frame's data left for what is drawn after block entities.
const RESERVED: u64 = 512 << 10;
/// Bytes of it glyphs take at most.
const GLYPH_BYTES: u64 = 2 << 20;

/// A colour that leaves the texture as it is.
const WHITE: u32 = u32::MAX;
/// How strong the glint is and how fast it moves: the game's own measures of its Glint
/// Strength and Glint Speed options.
const GLINT_STRENGTH: f32 = 0.75;
const GLINT_SPEED: f64 = 0.5;
/// The glints: which of the two pictures each is drawn from (armour's or items'), and how
/// large it lies over the pictures of armour, of items' own models (whose pictures are
/// small parts of an atlas) and of the models the game has for a shield or a trident
/// (`ARMOR_ENTITY_GLINT_TEXTURING`, `GLINT_TEXTURING`, `ENTITY_GLINT_TEXTURING`).
const GLINTS: [(usize, f32); 3] = [(0, 0.16), (1, 8.0), (1, 0.5)];
/// Trims whose pictures are kept.
const KEPT_TRIMS: usize = 1024;
/// Bytes of the one page the items' models are copied into as they are first held.
const ITEM_PAGE: u64 = 4 << 20;
/// The picture of a trident's own model, below `textures/entity`.
const TRIDENT: &str = "trident/trident";

/// A trim's picture and the palette it is coloured by.
type TrimKey = (String, Option<String>);
/// Where a trim and an equipment asset are, and whether it is on the legs.
type TrimPlace = (usize, usize, bool);

/// What glints over a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glint {
    None,
    Armour,
    Item,
    Entity,
}

/// An item's model on its way to the GPU.
#[derive(Debug, Clone, Copy)]
enum ItemRange {
    /// Asked for, and not placed yet.
    Wanted,
    /// Placed, and not copied yet.
    Placed(RangeHandle),
    Copied(RangeHandle, UploadTicket),
    Ready(RangeHandle),
    /// There was no room for it.
    Failed,
}

/// An item's model as vertices: four for a quad, as [`MODEL_VERTEX`] has them.
fn item_vertices(mesh: &ItemMesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(mesh.quads.len() * 4 * VERTEX_BYTES);
    for quad in &mesh.quads {
        let normal = ruthenium_assets::model::NORMALS[usize::from(quad.direction.min(5))]
            .map(|c| (c * 127.0) as i8 as u8);
        for (position, uv) in quad.positions.iter().zip(quad.uvs) {
            for c in position.iter().chain(&uv) {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&normal);
            out.push(0);
        }
    }
    out
}

/// Where an item's model is from the hand that holds it (`ItemTransform.apply`): moved,
/// turned and scaled as the model says for the hand, a left hand's the other way round,
/// about the model's middle.
fn place_item(hand: Affine, transform: Option<ItemTransform>, left: bool) -> Affine {
    let Some(t) = transform else {
        return hand.translate([-0.5; 3]);
    };
    let way = if left { -1.0 } else { 1.0 };
    let degrees = std::f32::consts::PI / 180.0;
    hand.translate([way * t.translation[0], t.translation[1], t.translation[2]])
        .rotate_x(t.rotation[0] * degrees)
        .rotate_y(way * t.rotation[1] * degrees)
        .rotate_z(way * t.rotation[2] * degrees)
        .scale(t.scale)
        .translate([-0.5; 3])
}

/// The colour of a model's quads of one tint, for a stack: 0xRRGGBB.
fn tint_colour(tints: &[Tint], index: i32, c: &Components) -> u32 {
    match usize::try_from(index).ok().and_then(|i| tints.get(i)) {
        None => 0xFF_FFFF,
        Some(Tint::Constant(rgb) | Tint::Default(rgb)) => *rgb,
        Some(Tint::Dye(default)) => c.dyed_color.unwrap_or(*default),
        Some(Tint::Potion(default)) => c.potion_color.unwrap_or(*default),
    }
}

/// Where the glint's picture is read for a part's texture coordinates, at a time
/// (`TextureTransform.setupGlintTexturing`): moved along, turned by a tenth of a quarter
/// turn, and scaled.
fn glint_rows(millis: u64, scale: f32) -> ([f32; 3], [f32; 3]) {
    let moved = (millis as f64 * GLINT_SPEED * 8.0) as i64;
    let along = (moved % 110_000) as f32 / 110_000.0;
    let up = (moved % 30_000) as f32 / 30_000.0;
    let (sin, cos) = 0.174_532_92_f32.sin_cos();
    (
        [cos * scale, -sin * scale, -along],
        [sin * scale, cos * scale, up],
    )
}
/// What an invisible player is tinted with for those who see it.
const INVISIBLE: u32 = 0x26FF_FFFF;

/// What block entities are drawn from: the models, and the pictures by their path below
/// `textures/entity` without the suffix.
#[derive(Debug)]
pub struct BlockEntityAssets {
    pub models: ModelSet,
    pub textures: Vec<(String, SkyImage)>,
    /// What signs and names are written in.
    pub font: Font,
    pub lang: HashMap<String, String>,
    /// The picture of an entity's shadow (`textures/misc/shadow.png`).
    pub shadow: Option<SkyImage>,
    /// What worn things look like; their pictures are among `textures`, below the folders
    /// [`equipment`] names.
    pub equipment: EquipmentSet,
    /// The pictures of armour trims, by their path below `textures` without the suffix,
    /// each with the palette its colours are of (a path below `textures/palettes`).
    pub trims: Vec<(String, SkyImage, Option<String>)>,
    /// The palettes, by their path below `textures/palettes` without the suffix.
    pub palettes: Vec<(String, SkyImage)>,
    /// The glint over enchanted armour, and over enchanted items
    /// (`textures/misc/enchanted_glint_armor.png`, `enchanted_glint_item.png`).
    pub glint: [Option<SkyImage>; 2],
    /// What held things are drawn from; without it they are not drawn.
    pub items: Option<ItemAssets>,
}

/// The items' models, and the first level of the atlas their pictures are in.
#[derive(Debug)]
pub struct ItemAssets {
    pub models: ItemModels,
    pub atlas: SkyImage,
}

/// The folder of the patterns on shields below `textures/entity`: all its pictures are
/// wanted.
pub const SHIELD_DIR: &str = "shield";

/// The folder of the banners' patterns below `textures/entity`.
pub const BANNER_DIR: &str = "banner";

/// The pictures block entities need besides the banners' patterns (which are all those in
/// [`BANNER_DIR`]), by their path below `textures/entity`.
pub fn texture_paths() -> Vec<String> {
    let mut out = Vec::new();
    for chest in CHESTS {
        for christmas in [false, true] {
            let name = place::chest_texture(chest, christmas);
            // An ender chest has no halves.
            let halves: &[&str] = if chest == Chest::Ender {
                &[""]
            } else {
                &HALVES
            };
            for half in halves {
                let path = format!("chest/{name}{half}");
                if !out.contains(&path) {
                    out.push(path);
                }
            }
        }
    }
    out.push("shulker/shulker".to_owned());
    out.extend(DYES.map(|d| format!("shulker/shulker_{d}")));
    out.extend(SKULLS.map(|s| skull_texture(s).to_owned()));
    out.push(TRIDENT.to_owned());
    for i in 0..DEFAULT_SKINS {
        let path = default_path(i);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

const CHESTS: [Chest; 7] = [
    Chest::Normal,
    Chest::Trapped,
    Chest::Ender,
    Chest::Copper(0),
    Chest::Copper(1),
    Chest::Copper(2),
    Chest::Copper(3),
];
const HALVES: [&str; 3] = ["", "_left", "_right"];
const SKULLS: [Skull; 7] = [
    Skull::Skeleton,
    Skull::WitherSkeleton,
    Skull::Player,
    Skull::Zombie,
    Skull::Creeper,
    Skull::Piglin,
    Skull::Dragon,
];

fn skull_texture(skull: Skull) -> &'static str {
    match skull {
        Skull::Skeleton => "skeleton/skeleton",
        Skull::WitherSkeleton => "skeleton/wither_skeleton",
        Skull::Player => "player/slim/steve",
        Skull::Zombie => "zombie/zombie",
        Skull::Creeper => "creeper/creeper",
        Skull::Piglin => "piglin/piglin",
        Skull::Dragon => "enderdragon/dragon",
    }
}

/// The layer a skin is drawn in.
fn skin_layer(look: &Look) -> u8 {
    if look.blended { SKIN_SORTED } else { SKIN }
}

/// Whether a model of this kind is drawn nearer than it is: a plain skull. (A head with a
/// skin of its own is where it is.)
fn is_nearer(kind: Kind, layer: u8) -> bool {
    matches!(kind, Kind::Skull { .. }) && layer == CUTOUT_FACES
}

/// Gives what is drawn from back to front its place among the others, each one's second
/// layer over its first.
fn order(sorted: &mut [(f32, std::ops::Range<usize>)], instances: &mut [Instance]) {
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (n, (_, range)) in sorted.iter().enumerate() {
        for i in &mut instances[range.clone()] {
            i.batch.1 += n as u32 * 2;
        }
    }
}

/// The models' vertices, four per quad.
fn vertices(models: &ModelSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(models.quads().len() * 4 * VERTEX_BYTES);
    for face in models.quads() {
        let normal = face.normal.map(|c| (c * 127.0).round() as i8 as u8);
        for [x, y, z, u, v] in face.corners {
            for c in [x / 16.0, y / 16.0, z / 16.0, u, v] {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&normal);
            out.push(0);
        }
    }
    out
}

/// Whether it is 24 to 26 December, by the world's clock: chests wear their Christmas
/// pictures then. (The game goes by the machine's time zone.)
fn is_christmas(now: SystemTime) -> bool {
    let days = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64);
    // Days since 1 March of a year 0, in which the leap day is the year's last.
    let day_of_era = (days + 719_468).rem_euclid(146_097);
    let year = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year + year / 4 - year / 100);
    // 24 December is 298 days after 1 March.
    (298..=300).contains(&day_of_year)
}

/// What the last frame drew.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockEntityStats {
    /// Sections with block entities, and block entities drawn.
    pub sections: u32,
    pub drawn: u32,
    /// Their parts, and the draws that drew them.
    pub parts: u32,
    pub draws: u32,
    /// Players and mannequins drawn.
    pub players: u32,
    /// Parts of the player's own hands and of what they hold.
    pub hand_parts: u32,
    /// Parts of the items a GUI shows.
    pub gui_parts: u32,
    /// Sides of signs drawn, and their glyphs.
    pub texts: u32,
    pub glyphs: u32,
    /// Lines over heads drawn, and the pieces of shadows.
    pub names: u32,
    pub shadows: u32,
    /// Parts and glyphs there was no room for, in all frames.
    pub overflowed: u64,
}

/// One part to draw: its record, and what it is sorted into draws by.
struct Instance {
    /// The layer; its place in it, where that matters; then the part's first quad.
    batch: (u8, u32, u32),
    quads: u32,
    transform: Affine,
    light: u32,
    colour: u32,
    texture: TextureId,
    hurt: bool,
    glint: Glint,
    /// Where its vertices are: 0 for the models', else the number of an item's model
    /// and one.
    source: u32,
}

/// The layers, in the order they are drawn in: patterns lie over the cloth they are on.
const SOLID: u8 = 0;
const CUTOUT: u8 = 1;
const CUTOUT_FACES: u8 = 2;
const PATTERN: u8 = 3;
/// Skins: those whose texels are whole or clear, in any order; those with others, from back
/// to front; and the invisible.
const SKIN: u8 = 4;
const SKIN_SORTED: u8 = 5;
const SKIN_INVISIBLE: u8 = 6;
/// What of held things is seen through, from back to front.
const ITEM_SORTED: u8 = 7;
const LAYERS: [PipelineId; 8] = [
    PipelineId::ModelSolid,
    PipelineId::ModelCutout,
    PipelineId::ModelCutoutFaces,
    PipelineId::ModelPattern,
    PipelineId::ModelTranslucent,
    PipelineId::ModelTranslucent,
    PipelineId::ModelTranslucentFront,
    PipelineId::ModelTranslucentFront,
];

/// A banner's patterns as last read from its data: their textures and colours.
struct Patterns {
    tag: Arc<nbt::Tag>,
    layers: Vec<(TextureId, u32)>,
}

/// A player's head as last read from its data: whose it is.
struct Head {
    /// As a sign's.
    stamp: (u64, u64),
    tag: Option<Arc<nbt::Tag>>,
    owner: Option<Arc<SkinSource>>,
}

/// One side of a sign, laid out.
struct Side {
    text: SignText,
    /// Records as [`sign::records`] makes them, and the newest picture they need.
    glyphs: Vec<[u32; GLYPH_WORDS]>,
    newest: u64,
    obfuscated: bool,
}

/// A sign's text as last read from its data.
struct Sign {
    /// The column's load and the last change to its block entities.
    stamp: (u64, u64),
    tag: Option<Arc<nbt::Tag>>,
    /// The front and the back.
    sides: [Option<Box<Side>>; 2],
}

/// One side of a sign to draw.
struct SignRun {
    pos: [i32; 3],
    back: bool,
    outlined: bool,
    outline: u32,
    transform: Affine,
    light: u32,
    away: f64,
}

/// One line over a head as last laid out.
#[derive(Default)]
struct NameGlyphs {
    line: Vec<(char, Style)>,
    /// Records as [`sign::records`] makes them: the glyphs whole, and what lies behind
    /// them with the glyphs seen through.
    solid: Vec<[u32; GLYPH_WORDS]>,
    sheer: Vec<[u32; GLYPH_WORDS]>,
    /// The newest picture they need.
    newest: u64,
    obfuscated: bool,
}

/// What is written over an entity's head as last read.
struct Named {
    text: Arc<NameText>,
    offset: i32,
    /// The name, and what stands below it.
    lines: [Option<NameGlyphs>; 2],
}

/// One line over a head to draw.
struct NameRun {
    id: i32,
    /// 0: the name; 1: what stands below it.
    line: usize,
    transform: Affine,
    /// Block and sky light, 0 to 15.
    light: (u8, u8),
    /// Not seen through walls.
    discrete: bool,
    away: f32,
}

/// One piece of a shadow to draw.
struct ShadowRun {
    /// Where its entity stands, from the camera.
    origin: [f32; 3],
    radius: f32,
    piece: Piece,
}

/// How the player's view moves with it (the game's View Bobbing and Damage Tilt options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewOptions {
    pub bobbing: bool,
    /// 0 to 1.
    pub damage_tilt: f32,
}

impl Default for ViewOptions {
    fn default() -> ViewOptions {
        ViewOptions {
            bobbing: true,
            damage_tilt: 1.0,
        }
    }
}

impl ViewOptions {
    /// `m` with the view's tilt and bob done first (`bobHurt`, then `bobView`).
    pub(crate) fn bob(
        &self,
        m: Affine,
        r: &ruthenium_sim::first_person::FirstPersonRecord,
        partial: f32,
    ) -> Affine {
        let m = hand::bob_hurt(m, r, partial, self.damage_tilt);
        if self.bobbing {
            hand::bob_view(m, r, partial)
        } else {
            m
        }
    }
}

/// What [`BlockEntityRenderer::gather_players`] takes of the frame besides the players.
pub(crate) struct PlayerView<'a> {
    pub camera: &'a CameraPose,
    pub view: &'a View,
    pub view_distance: i32,
    pub partial: f32,
    /// What the sky's light lacks to its full strength now (`getSkyDarken`).
    pub sky_darken: i32,
}

/// How a stack is held, as its item's definition asks.
#[derive(Debug, Clone, Copy)]
struct HeldHow {
    /// In a left hand.
    left: bool,
    /// Seen by the one who holds it.
    first_person: bool,
    /// Not held at all: in a slot of a GUI.
    gui: bool,
    left_handed: bool,
    /// The hand uses it, for so many ticks, with so many left.
    using: bool,
    use_ticks: i32,
    use_remaining: i32,
}

/// Which lights shade a model (`model.vert`'s `lights`): the level's usual two, the
/// Nether's, and those of an item in a GUI, a flat one's and a block's.
const LIGHTS_USUAL: u32 = 1;
const LIGHTS_NETHER: u32 = 2;
const LIGHTS_GUI_FLAT: u32 = 3;
const LIGHTS_GUI_SIDE: u32 = 4;

/// The light of what a GUI shows: the brightest of both kinds.
const GUI_LIGHT: (u8, u8) = (15, 15);

/// A text's shadow: a quarter as bright (`ARGB.scaleRGB(color, 0.25)`).
fn shadow_colour(color: u32) -> u32 {
    let quarter = |shift: u32| ((color >> shift & 0xFF) as f32 * 0.25) as u32;
    color & 0xFF00_0000 | quarter(16) << 16 | quarter(8) << 8 | quarter(0)
}

/// What is held is drawn with: the atlases its pictures are in.
#[derive(Clone, Copy)]
struct Atlases {
    items: Option<TextureId>,
    blocks: Option<TextureId>,
}

/// One section's block entities: where in the section, and their states.
pub(crate) type SectionEntities = [([u8; 3], u16)];

/// See the module docs.
pub(crate) struct BlockEntityRenderer {
    pool: BufferPool,
    range: RangeHandle,
    /// The vertices until they are staged, and their upload after.
    bytes: Vec<u8>,
    upload: Option<UploadTicket>,
    ready: bool,
    models: ModelSet,
    textures: HashMap<String, TextureId>,
    chests: HashMap<(Chest, bool, usize), TextureId>,
    shulkers: Vec<Option<TextureId>>,
    skulls: Vec<Option<TextureId>>,
    /// The resting parts of a state in its block's space.
    resting: HashMap<u32, Vec<PartDraw>>,
    patterns: HashMap<[i32; 3], Patterns>,
    font: Font,
    lang: HashMap<String, String>,
    pages: GlyphPages,
    signs: HashMap<[i32; 3], Sign>,
    heads: HashMap<[i32; 3], Head>,
    skins: Skins,
    names: HashMap<i32, Named>,
    name_runs: Vec<NameRun>,
    /// What [`Self::draw`] leaves for [`Self::draw_over`]: the names seen through walls,
    /// as their draw's push constants and how many glyphs there are.
    over: Option<([u8; MODEL_PUSH_BYTES], u32)>,
    equipment: EquipmentSet,
    /// The pictures of trims and the palettes, as the pack has them.
    trim_bases: HashMap<String, (SkyImage, Option<String>)>,
    palettes: HashMap<String, Vec<u8>>,
    /// The trims' pictures in their palettes, as far as they were asked for: nothing for
    /// one that is asked for and not made yet, or cannot be.
    trim_textures: HashMap<TrimKey, Option<TextureId>>,
    trims_wanted: Vec<TrimKey>,
    /// The pictures of each equipment asset's layers, on the body and on the legs, once
    /// it has been worn.
    worn_pictures: [HashMap<String, Vec<Option<TextureId>>>; 2],
    /// Which picture a trim has on an armour (by where the two are, and whether on the
    /// legs), if it has one. The trim is kept, so that nothing else comes to be there.
    trim_keys: HashMap<TrimPlace, (Arc<Trim>, Option<TrimKey>)>,
    /// What a worn thing is drawn with, while it is being added.
    worn: Vec<(TextureId, u32)>,
    glint_pictures: [Option<TextureId>; 2],
    started: std::time::Instant,
    items: Option<ItemModels>,
    item_atlas: Option<TextureId>,
    /// The items' atlas until its upload has been started.
    item_atlas_waiting: Option<SkyImage>,
    /// The vertices of the items' models that have been held, and where each is.
    item_pool: BufferPool,
    item_ranges: HashMap<u32, ItemRange>,
    items_wanted: Vec<u32>,
    shadow_picture: Option<TextureId>,
    show_shadows: bool,
    shadows: Vec<ShadowRun>,
    pieces: Vec<Piece>,
    /// What is drawn from back to front: how far it is, and where its instances are.
    sorted: Vec<(f32, std::ops::Range<usize>)>,
    runs: Vec<SignRun>,
    /// State of the numbers obfuscated text is picked by.
    random: u64,
    instances: Vec<Instance>,
    /// The player's own hands, which are drawn over the whole world: their parts, and
    /// what of them is drawn from back to front.
    hand: Vec<Instance>,
    hand_sorted: Vec<(f32, std::ops::Range<usize>)>,
    /// The items a GUI shows, drawn over everything with a view and lights of their own:
    /// the flat ones, and those lit from the side as blocks are.
    gui: [Vec<Instance>; 2],
    parts: Vec<PartDraw>,
    christmas: bool,
    stats: BlockEntityStats,
}

impl std::fmt::Debug for BlockEntityRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockEntityRenderer")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// 0xRRGGBB as a record's colour: red in the low byte, opaque.
pub(super) fn record_colour(rgb: u32) -> u32 {
    0xFF00_0000 | (rgb & 0xFF) << 16 | rgb & 0xFF00 | rgb >> 16 & 0xFF
}

/// A block's light as a record's, from its levels.
fn record_light(block: u8, sky: u8) -> u32 {
    u32::from(block) << 4 | u32::from(sky) << 12
}

/// The patterns a banner's data lists: the pattern's id (or the picture it names itself)
/// and the dye.
fn listed_patterns(tag: &nbt::Tag) -> impl Iterator<Item = (&str, u8)> {
    tag.as_compound()
        .map_or(&[][..], |c| c.list("patterns"))
        .iter()
        .filter_map(|layer| {
            let layer = layer.as_compound()?;
            let pattern = layer
                .string("pattern")
                .or_else(|| layer.get("pattern")?.as_compound()?.string("asset_id"))?;
            Some((pattern, place::dye(layer.string("color")?)?))
        })
        .take(MAX_PATTERNS)
}

/// The picture of a pattern id, below `textures/entity`.
fn pattern_texture(id: &str) -> String {
    let path = id.strip_prefix("minecraft:").unwrap_or(id);
    format!("{BANNER_DIR}/{path}")
}

impl BlockEntityRenderer {
    /// Creates the models' buffer and the pictures' textures. Not on a frame's path.
    pub fn new(
        gpu: &Arc<Gpu>,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        assets: BlockEntityAssets,
    ) -> Result<BlockEntityRenderer, Error> {
        let bytes = vertices(&assets.models);
        let page = (bytes.len() as u64).next_power_of_two().max(1 << 16);
        let mut pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: page,
                budget_bytes: page,
                low_watermark: 0,
                high_watermark: page,
                release_after: u32::MAX,
                granule: 8,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER,
        );
        pool.grow_now()?;
        let range = pool
            .alloc(bytes.len() as u64, 8)
            .map_err(|e| Error::Unsupported(format!("no room for the models: {e:?}")))?;
        let mut ids = HashMap::new();
        for (path, i) in &assets.textures {
            if i.rgba.is_empty() || i.rgba.len() != i.width as usize * i.height as usize * 4 {
                continue;
            }
            if let Ok(id) = textures.create_with(
                uploads,
                TextureDesc::rgba8(i.width, i.height),
                &[&i.rgba],
                Arrival::NextFrame,
            ) {
                ids.insert(path.clone(), id);
            }
        }
        let mut chests = HashMap::new();
        for chest in CHESTS {
            for christmas in [false, true] {
                for (i, half) in HALVES.into_iter().enumerate() {
                    let name = place::chest_texture(chest, christmas);
                    if let Some(id) = ids.get(&format!("chest/{name}{half}")) {
                        chests.insert((chest, christmas, i), *id);
                    }
                }
            }
        }
        let shulkers = std::iter::once("shulker/shulker".to_owned())
            .chain(DYES.map(|d| format!("shulker/shulker_{d}")))
            .map(|p| ids.get(&p).copied())
            .collect();
        let skulls = SKULLS.map(|s| ids.get(skull_texture(s)).copied()).to_vec();
        let mut picture = |i: Option<SkyImage>| {
            i.filter(|i| {
                !i.rgba.is_empty() && i.rgba.len() == i.width as usize * i.height as usize * 4
            })
            .and_then(|i| {
                textures
                    .create_with(
                        uploads,
                        TextureDesc::rgba8(i.width, i.height),
                        &[&i.rgba],
                        Arrival::NextFrame,
                    )
                    .ok()
            })
        };
        let shadow_picture = picture(assets.shadow);
        let glint_pictures = assets.glint.map(&mut picture);
        // (Their atlas is too large to stage at once: it is streamed, from `stage` on.)
        let (items, item_atlas_waiting) = match assets.items {
            Some(i) => (Some(i.models), Some(i.atlas)),
            None => (None, None),
        };
        let mut item_pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: ITEM_PAGE,
                budget_bytes: ITEM_PAGE,
                low_watermark: 0,
                high_watermark: ITEM_PAGE,
                release_after: u32::MAX,
                granule: 8,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER,
        );
        if items.is_some() {
            item_pool.grow_now()?;
        }
        Ok(BlockEntityRenderer {
            pool,
            range,
            bytes,
            upload: None,
            ready: false,
            models: assets.models,
            chests,
            shulkers,
            skulls,
            resting: HashMap::new(),
            patterns: HashMap::new(),
            font: assets.font,
            lang: assets.lang,
            pages: GlyphPages::default(),
            signs: HashMap::new(),
            heads: HashMap::new(),
            skins: Skins::new(std::array::from_fn(|i| ids.get(&default_path(i)).copied())),
            names: HashMap::new(),
            name_runs: Vec::new(),
            over: None,
            equipment: assets.equipment,
            trim_bases: assets
                .trims
                .into_iter()
                .map(|(path, image, palette)| (path, (image, palette)))
                .collect(),
            palettes: assets
                .palettes
                .into_iter()
                .map(|(path, image)| (path, image.rgba))
                .collect(),
            trim_textures: HashMap::new(),
            trims_wanted: Vec::new(),
            worn_pictures: [HashMap::new(), HashMap::new()],
            trim_keys: HashMap::new(),
            worn: Vec::new(),
            glint_pictures,
            started: std::time::Instant::now(),
            items,
            item_atlas: None,
            item_atlas_waiting,
            item_pool,
            item_ranges: HashMap::new(),
            items_wanted: Vec::new(),
            shadow_picture,
            show_shadows: true,
            shadows: Vec::new(),
            pieces: Vec::new(),
            sorted: Vec::new(),
            runs: Vec::new(),
            random: 0x9E37_79B9_7F4A_7C15,
            instances: Vec::new(),
            hand: Vec::new(),
            hand_sorted: Vec::new(),
            gui: [Vec::new(), Vec::new()],
            parts: Vec::new(),
            textures: ids,
            christmas: is_christmas(SystemTime::now()),
            stats: BlockEntityStats::default(),
        })
    }

    pub fn stats(&self) -> BlockEntityStats {
        self.stats
    }

    /// Whether entities cast shadows (the game's Entity Shadows option).
    pub fn set_shadows(&mut self, on: bool) {
        self.show_shadows = on;
    }

    /// Stages the models, until that has worked once, and the glyphs that wait for it:
    /// before the frame's `UploadRing::record`.
    pub fn stage(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) {
        self.pages.stage(textures, uploads);
        self.skins.stage(textures, uploads);
        for key in std::mem::take(&mut self.trims_wanted) {
            let made = self.trim_picture(&key).and_then(|(width, height, rgba)| {
                textures
                    .create_with(
                        uploads,
                        TextureDesc::rgba8(width, height),
                        &[&rgba],
                        Arrival::NextFrame,
                    )
                    .ok()
            });
            self.trim_textures.insert(key, made);
        }
        // The items' atlas, once the ring has a stream to spare for it.
        if let Some(atlas) = self.item_atlas_waiting.take() {
            let whole = atlas.rgba.len() == atlas.width as usize * atlas.height as usize * 4;
            let started = whole
                .then(|| {
                    textures
                        .create(TextureDesc::rgba8(atlas.width, atlas.height))
                        .ok()
                })
                .flatten()
                .and_then(
                    |id| match textures.stream(uploads, id, vec![atlas.rgba.clone()]) {
                        Ok(()) => Some(id),
                        Err(_) => {
                            textures.destroy(id);
                            None
                        }
                    },
                );
            match started {
                Some(id) => self.item_atlas = Some(id),
                None if whole => self.item_atlas_waiting = Some(atlas),
                None => {}
            }
        }
        // The models of what is newly held, as far as there is room and the ring takes
        // them; one it does not take waits for the next frame.
        for mesh in std::mem::take(&mut self.items_wanted) {
            let Some(model) = self.items.as_ref().and_then(|i| i.mesh(mesh)) else {
                continue;
            };
            let bytes = item_vertices(model);
            let placed = match self.item_ranges.get(&mesh) {
                Some(ItemRange::Placed(handle)) => Some(*handle),
                _ => self.item_pool.alloc(bytes.len() as u64, 8).ok(),
            };
            let Some(handle) = placed else {
                self.item_ranges.insert(mesh, ItemRange::Failed);
                self.stats.overflowed += 1;
                continue;
            };
            let copied = self
                .item_pool
                .resolve(handle)
                .and_then(|r| uploads.upload_buffer(r.buffer, r.offset, &bytes).ok());
            match copied {
                Some(ticket) => {
                    self.item_ranges
                        .insert(mesh, ItemRange::Copied(handle, ticket));
                }
                None => {
                    self.item_ranges.insert(mesh, ItemRange::Placed(handle));
                    self.items_wanted.push(mesh);
                }
            }
        }
        if self.upload.is_none()
            && let Some(range) = self.pool.resolve(self.range)
        {
            // Full for now: the next frame tries again.
            self.upload = uploads
                .upload_buffer(range.buffer, range.offset, &self.bytes)
                .ok();
            if self.upload.is_some() {
                self.bytes = Vec::new();
            }
        }
    }

    /// Notes whether the models can be read: after the frame's `UploadRing::record`.
    pub fn note_ready(&mut self, uploads: &UploadRing) {
        if !self.ready {
            self.ready = self.upload.is_some_and(|t| uploads.is_ready(t));
        }
        for range in self.item_ranges.values_mut() {
            if let ItemRange::Copied(handle, ticket) = *range
                && uploads.is_ready(ticket)
            {
                *range = ItemRange::Ready(handle);
            }
        }
    }

    /// The skins that are wanted and have not been asked for.
    pub fn take_skin_requests(&mut self) -> Vec<Arc<SkinSource>> {
        self.skins.take_requests()
    }

    /// The answer to one of [`Self::take_skin_requests`].
    pub fn skin_found(&mut self, source: &Arc<SkinSource>, skin: LoadedSkin) {
        self.skins.found(source, skin);
    }

    /// Starts a frame's gathering.
    pub fn begin(&mut self) {
        self.instances.clear();
        self.sorted.clear();
        self.hand.clear();
        self.hand_sorted.clear();
        self.gui.iter_mut().for_each(Vec::clear);
        self.skins.begin();
        self.runs.clear();
        self.name_runs.clear();
        self.shadows.clear();
        self.over = None;
        self.stats = BlockEntityStats {
            overflowed: self.stats.overflowed,
            ..BlockEntityStats::default()
        };
    }

    /// Adds the block entities of a section in view, as the level has them now: one whose
    /// block has changed since the section was meshed is left out.
    #[allow(clippy::too_many_arguments)]
    pub fn gather(
        &mut self,
        key: SectionKey,
        list: &SectionEntities,
        terrain: &Terrain,
        table: &BlockTable,
        frame: &Frame,
        partial: f32,
        camera: [f64; 3],
        view: &View,
        textures: &TextureTable,
        mut marks: Option<&mut crate::block_marks::BlockMarks>,
    ) {
        self.stats.sections += 1;
        for &(at, state) in list {
            let pos: [i32; 3] = std::array::from_fn(|i| (key[i] << 4) + i32::from(at[i]));
            let away: f64 = (0..3)
                .map(|i| (f64::from(pos[i]) + 0.5 - camera[i]).powi(2))
                .sum();
            if away >= VIEW_DISTANCE * VIEW_DISTANCE {
                continue;
            }
            // Its model reaches a block beyond its own at most, a banner two up.
            let from = |by: f32| -> [f32; 3] {
                std::array::from_fn(|i| (pos[i] - view.block[i]) as f32 + by)
            };
            if !view.sees(from(-1.0), from(3.0)) {
                continue;
            }
            let state = u32::from(state);
            let Some(probe) = terrain.probe(pos).filter(|p| u32::from(p.block) == state) else {
                continue;
            };
            let Some(kind) = Kind::of(state) else {
                continue;
            };
            let lit = |p: ruthenium_sim::scene::Probe| {
                let info = table.state(u32::from(p.block));
                if info.flags & crate::terrain::table::StateInfo::EMISSIVE != 0 {
                    (15, 15)
                } else {
                    (p.block_light.max(info.light_emission), p.sky_light)
                }
            };
            let lid = |at: [i32; 3]| {
                frame
                    .lids
                    .binary_search_by_key(&at, |l| l.pos)
                    .map_or(0.0, |i| {
                        let l = frame.lids[i];
                        l.old + partial * (l.now - l.old)
                    })
            };
            let (mut block, mut sky) = lit(probe);
            let mut open = match kind {
                Kind::Chest(_) | Kind::Shulker(_) => lid(pos),
                _ => 0.0,
            };
            // A double chest opens and is lit as one.
            if let Kind::Chest(_) = kind
                && let Some(towards) = place::other_half(state)
            {
                let other = towards.step(pos);
                if let Some(p) = terrain
                    .probe(other)
                    .filter(|p| place::is_other_half(state, u32::from(p.block)))
                {
                    let (b, s) = lit(p);
                    (block, sky) = (block.max(b), sky.max(s));
                    open = open.max(lid(other));
                }
            }
            let origin: [f32; 3] = std::array::from_fn(|i| {
                ((f64::from(pos[i]) - f64::from(view.block[i])) as f32) + view.offset[i]
            });
            let place = Placed {
                origin,
                light: record_light(block, sky),
            };
            if let Kind::Sign { hanging, wall } = kind {
                self.add_sign(pos, state, [hanging, wall], terrain, frame, place, away);
                self.stats.drawn += 1;
                continue;
            }
            let usable = |id: Option<TextureId>| id.filter(|t| textures.is_ready(*t));
            let first = self.instances.len();
            match kind {
                Kind::Chest(chest) => {
                    let (model, half) = place::chest_half(state);
                    let half = HALVES.iter().position(|h| *h == half).unwrap_or(0);
                    let texture = self.chests.get(&(chest, self.christmas, half)).copied();
                    let Some(texture) = usable(texture) else {
                        continue;
                    };
                    let pose = if open > 0.0 {
                        Pose::Chest(open)
                    } else {
                        Pose::Rest
                    };
                    self.add(kind, state, model, pose, place, CUTOUT, texture, WHITE);
                }
                Kind::Shulker(dye) => {
                    let i = dye.map_or(0, |d| usize::from(d) + 1);
                    let Some(texture) = usable(self.shulkers[i]) else {
                        continue;
                    };
                    self.add(
                        kind,
                        state,
                        Model::Shulker,
                        Pose::Shulker(open),
                        place,
                        CUTOUT_FACES,
                        texture,
                        WHITE,
                    );
                }
                Kind::Skull { skull, .. } => {
                    let model = Model::Skull(skull);
                    let still = Pose::Skull(0.0);
                    let (layer, texture) = if skull == Skull::Player
                        && let Some(owner) = self.owner(terrain, pos)
                    {
                        let Some(look) = self.skins.look(&owner, textures) else {
                            continue;
                        };
                        (skin_layer(&look), look.body)
                    } else {
                        let i = SKULLS.iter().position(|s| *s == skull).unwrap_or(0);
                        let Some(texture) = usable(self.skulls[i]) else {
                            continue;
                        };
                        (CUTOUT_FACES, texture)
                    };
                    self.add(kind, state, model, still, place, layer, texture, WHITE);
                    self.sort(first, layer);
                }
                Kind::Sign { .. } => {}
                Kind::Banner { dye, wall } => {
                    let (pole, flag) = if wall {
                        (Model::WallBanner, Model::WallFlag)
                    } else {
                        (Model::StandingBanner, Model::StandingFlag)
                    };
                    let picture = |r: &Self, path: &str| usable(r.textures.get(path).copied());
                    let Some(cloth) = picture(self, "banner/banner_base") else {
                        continue;
                    };
                    let wave = Pose::Flag(place::banner_phase(pos, frame.game_time, partial));
                    self.add(kind, state, pole, Pose::Rest, place, SOLID, cloth, WHITE);
                    self.add(kind, state, flag, wave, place, SOLID, cloth, WHITE);
                    let ground = record_colour(DYE_COLOURS[usize::from(dye)]);
                    if let Some(base) = picture(self, "banner/base") {
                        self.add(kind, state, flag, wave, place, PATTERN, base, ground);
                    }
                    for i in 0.. {
                        let Some((texture, colour)) = self.pattern(terrain, pos, i) else {
                            break;
                        };
                        if textures.is_ready(texture) {
                            self.add(kind, state, flag, wave, place, PATTERN, texture, colour);
                        }
                    }
                }
            }
            if let Some((_, stage)) = frame.breaking.iter().find(|(p, _)| *p == pos)
                && let Some(marks) = marks.as_deref_mut()
            {
                // Each physical part casts one decal. The pattern layers repeat the banner cloth.
                let parts = self.instances[first..]
                    .iter()
                    .filter(|part| part.batch.0 != PATTERN)
                    .map(|part| {
                        let (layer, _, first_quad) = part.batch;
                        let faces = &self.models.quads()
                            [first_quad as usize..(first_quad + part.quads) as usize];
                        (layer >= SKIN, part.transform, faces)
                    });
                marks.add_model_block(*stage, parts, origin, view);
            }
            self.stats.drawn += 1;
        }
    }

    /// Adds the players and mannequins that are in view and in a section that is shown,
    /// with their shadows and their names.
    #[allow(clippy::too_many_arguments)]
    pub fn gather_players(
        &mut self,
        frame: &Frame,
        terrain: &Terrain,
        table: &BlockTable,
        seen: &PlayerView<'_>,
        textures: &TextureTable,
        block_atlas: TextureId,
        shown: impl Fn([i32; 3]) -> bool,
    ) {
        let (view, partial, camera) = (seen.view, seen.partial, seen.camera.position);
        let atlases = Atlases {
            items: self.item_atlas.filter(|t| textures.is_ready(*t)),
            blocks: Some(block_atlas).filter(|t| textures.is_ready(*t)),
        };
        let axes = seen.camera.axes();
        let shadow_picture = self
            .shadow_picture
            .filter(|t| self.show_shadows && textures.is_ready(*t));
        // `Entity.getViewScale`.
        let scale = (f64::from(seen.view_distance) / 8.0).clamp(1.0, 2.5);
        for e in &frame.entities {
            let Some(r) = e.avatar.as_deref() else {
                continue;
            };
            // The game may be frozen: then only players go on.
            let goes_on = matches!(*r.skin, SkinSource::Player { .. });
            let partial = if frame.frozen && !goes_on {
                1.0
            } else {
                partial
            };
            let at = Moment { partial, camera };
            let here = avatar::position(e, partial);
            let [width, height] = r.size.map(f64::from);
            let reach = (width + height + width) / 3.0 * 10.0 * 64.0 * scale;
            let away: f64 = (0..3).map(|i| (e.position[i] - camera[i]).powi(2)).sum();
            // (What is not a number is left out too.)
            if away >= reach * reach || away.is_nan() {
                continue;
            }
            let low = [here[0] - width / 2.0, here[1], here[2] - width / 2.0];
            let size = [width, height, width];
            let corner = |by: f64, high: f64| -> [f32; 3] {
                std::array::from_fn(|i| {
                    (low[i] + size[i] * high + by - f64::from(view.block[i])) as f32
                })
            };
            if !view.sees(corner(-0.5, 0.0), corner(0.5, 1.0))
                || !shown(e.position.map(|c| c.floor() as i32))
            {
                continue;
            }
            let Some(look) = self.skins.look(&r.skin, textures) else {
                continue;
            };
            let eyes = [here[0], here[1] + f64::from(r.eye_height), here[2]];
            let light = terrain
                .probe(eyes.map(|c| c.floor() as i32))
                .map_or((0, 15), |p| (p.block_light, p.sky_light));
            let levels = (if r.on_fire { 15 } else { light.0 }, light.1);
            let light = record_light(levels.0, levels.1);
            let figure = Figure::new(e, r, at);
            let (layer, colour) = if r.invisible {
                (SKIN_INVISIBLE, INVISIBLE)
            } else {
                (skin_layer(&look), WHITE)
            };
            let model = if look.slim {
                Model::PlayerSlim
            } else {
                Model::Player
            };
            // One that is not seen is not drawn, but for what it wears and holds.
            let first = self.instances.len();
            self.parts.clear();
            if !r.hidden {
                self.models
                    .draw_by(model, &mut self.parts, |name, m| figure.place(name, m));
            }
            for p in &self.parts {
                self.instances.push(Instance {
                    batch: (layer, u32::from(p.over), p.first_quad),
                    quads: p.quads,
                    transform: figure.transform.then_inner(p.transform),
                    light,
                    colour,
                    texture: look.body,
                    hurt: r.hurt,
                    glint: Glint::None,
                    source: 0,
                });
            }
            self.sort(first, layer);
            if !r.spectator {
                for (place, stack) in r.worn.iter().enumerate() {
                    self.add_armour(&figure, look.slim, place, stack, light, textures);
                }
                for off_hand in [false, true] {
                    self.add_held(&figure, r, look.slim, off_hand, light, atlases, textures);
                }
            }
            if let Some(cape) = look.cape
                && !r.invisible
                && figure.wears_cape()
            {
                self.parts.clear();
                self.models
                    .draw_by(Model::Cape, &mut self.parts, |name, m| {
                        figure.place_cape(name, m)
                    });
                let place = figure.cape_transform();
                for p in &self.parts {
                    self.instances.push(Instance {
                        batch: (SOLID, 0, p.first_quad),
                        quads: p.quads,
                        transform: place.then_inner(p.transform),
                        light,
                        colour: WHITE,
                        texture: cape,
                        // A cape does not redden.
                        hurt: false,
                        glint: Glint::None,
                        source: 0,
                    });
                }
            }
            self.stats.players += 1;
            let from: [f32; 3] = std::array::from_fn(|i| (here[i] - camera[i]) as f32);
            // The invisible cast none, whoever sees them.
            if shadow_picture.is_some() && !r.invisible {
                let radius = PLAYER_SHADOW * r.scale;
                let ground = |pos: [i32; 3]| {
                    let below = terrain.probe([pos[0], pos[1] - 1, pos[2]])?;
                    let bounds = table.shadow_ground(u32::from(below.block))?;
                    let over = terrain.probe(pos)?;
                    let sky = i32::from(over.sky_light) - seen.sky_darken;
                    Some(Ground {
                        bounds,
                        light: sky.max(i32::from(over.block_light)),
                    })
                };
                self.pieces.clear();
                shadow::pieces(
                    here,
                    radius,
                    shadow::power(away, SHADOW_STRENGTH),
                    frame.environment.ambient_light,
                    ground,
                    &mut self.pieces,
                );
                let room = MAX_SHADOW_PIECES.saturating_sub(self.shadows.len());
                self.stats.overflowed += self.pieces.len().saturating_sub(room) as u64;
                self.shadows
                    .extend(self.pieces.iter().take(room).map(|&piece| ShadowRun {
                        origin: from,
                        radius: radius.min(32.0),
                        piece,
                    }));
            }
            if let Some(name) = &e.name {
                // Its top, lowered with it where it crouches.
                let mut top = from;
                top[1] += r.size[1];
                if e.pose == ruthenium_sim::entity::pose::CROUCHING {
                    top[1] += r.scale * -2.0 / 16.0;
                }
                self.add_name(e.id, name, top, away, levels, axes);
            }
        }
    }

    /// Gathers the player's own hands as its eyes see them, for [`Self::draw_hand`]
    /// (`GameRenderer.renderItemInHand`): the bare arm or what each hand holds, moved
    /// with the view's bob and tilt as `options` say.
    pub fn gather_hand(
        &mut self,
        frame: &Frame,
        terrain: &Terrain,
        seen: &PlayerView<'_>,
        textures: &TextureTable,
        block_atlas: TextureId,
        options: ViewOptions,
    ) {
        /// `GameType.SPECTATOR`.
        const SPECTATOR: i32 = 3;
        let r = &frame.player.first_person;
        if r.sleeping || frame.player.game_type == SPECTATOR {
            return;
        }
        let Some(look) = self.skins.look(&r.skin, textures) else {
            return;
        };
        let atlases = Atlases {
            items: self.item_atlas.filter(|t| textures.is_ready(*t)),
            blocks: Some(block_atlas).filter(|t| textures.is_ready(*t)),
        };
        // In the light at the eyes, as the player itself is drawn.
        let eyes = seen.camera.position.map(|c| c.floor() as i32);
        let (block, sky) = terrain
            .probe(eyes)
            .map_or((0, 15), |p| (p.block_light, p.sky_light));
        let light = record_light(if r.on_fire { 15 } else { block }, sky);
        // From the view's own space (x to the right, y up, ahead at negative z) to
        // blocks around the eyes.
        let [forward, right, up] = seen.camera.axes();
        let from_view = Affine(std::array::from_fn(|i| [right[i], up[i], -forward[i], 0.0]));
        let m = options.bob(from_view, r, seen.partial);
        let hands = Hands {
            r,
            partial: seen.partial,
            x_rot: seen.camera.x_rot,
            y_rot: seen.camera.y_rot,
        };
        let m = hands.trailing(m);
        // The hands are gathered as everything else is, into a list of their own.
        std::mem::swap(&mut self.instances, &mut self.hand);
        std::mem::swap(&mut self.sorted, &mut self.hand_sorted);
        for off_hand in [false, true] {
            let drawn = if off_hand {
                r.hands.off()
            } else {
                r.hands.main()
            };
            if !drawn {
                continue;
            }
            let stack = &r.held[usize::from(off_hand)];
            let swap_scale = match (&self.items, stack.c.item_model.as_deref()) {
                (Some(items), Some(id)) if !stack.is_empty() => items.swap_scale(id),
                _ => 1.0,
            };
            match hands.hand(m, off_hand, swap_scale) {
                Shown::Nothing => {}
                Shown::Arm(at) => {
                    self.add_arm(at, hands.is_right(off_hand), &look, r.parts, light);
                }
                Shown::Item(at) => {
                    let how = HeldHow {
                        left: !hands.is_right(off_hand),
                        first_person: true,
                        gui: false,
                        left_handed: r.left_handed,
                        using: r.using == Some(off_hand),
                        use_ticks: r.use_duration[usize::from(off_hand)] - r.use_remaining,
                        use_remaining: r.use_remaining,
                    };
                    let _ = self.add_held_at(stack, at, how, light, atlases, textures);
                }
            }
        }
        std::mem::swap(&mut self.instances, &mut self.hand);
        std::mem::swap(&mut self.sorted, &mut self.hand_sorted);
    }

    /// Adds the player's own bare arm at `at` (`AvatarRenderer.renderHand`): the arm of
    /// its model at rest, turned a little outwards, with its sleeve if that is shown.
    fn add_arm(&mut self, at: Affine, right: bool, look: &Look, shown: u8, light: u32) {
        let model = if look.slim {
            Model::PlayerSlim
        } else {
            Model::Player
        };
        let (arm, sleeve, bit, turn) = if right {
            ("right_arm", "right_sleeve", avatar::part::RIGHT_SLEEVE, 0.1)
        } else {
            ("left_arm", "left_sleeve", avatar::part::LEFT_SLEEVE, -0.1)
        };
        self.parts.clear();
        self.models.draw_by(model, &mut self.parts, |name, m| {
            if name == arm {
                m.rotation[2] = turn;
                true
            } else if name == sleeve {
                shown & bit != 0
            } else {
                // The model's root carries the arm; nothing else of it is drawn.
                name.is_empty()
            }
        });
        let layer = skin_layer(look);
        let first = self.instances.len();
        for p in &self.parts {
            self.instances.push(Instance {
                batch: (layer, u32::from(p.over), p.first_quad),
                quads: p.quads,
                transform: at.then_inner(p.transform),
                light,
                colour: WHITE,
                texture: look.body,
                hurt: false,
                glint: Glint::None,
                source: 0,
            });
        }
        self.sort(first, layer);
    }

    /// Adds what is written over the head of the entity `id`, whose top is at `top` from
    /// the camera and whose place is `away` blocks from it, squared: its name, and what
    /// stands below it for those who are near.
    fn add_name(
        &mut self,
        id: i32,
        name: &NameRecord,
        top: [f32; 3],
        away: f64,
        light: (u8, u8),
        axes: [[f32; 3]; 3],
    ) {
        let nearer_than = |blocks: f64| away < blocks * blocks;
        if !nearer_than(f64::from(name.distance))
            || name.discrete && !nearer_than(nametag::DISCRETE_DISTANCE)
        {
            return;
        }
        let known = self
            .names
            .get(&id)
            .is_some_and(|n| Arc::ptr_eq(&n.text, &name.text) && n.offset == name.offset);
        if !known {
            if self.names.len() >= KEPT_NAMES {
                self.names.clear();
            }
            let lang = &self.lang;
            let line = |text: &nbt::Tag| NameGlyphs {
                line: StyledText::from_nbt_in(text, lang)
                    .spans
                    .iter()
                    .flat_map(|s| s.text.chars().map(|c| (c, s.style)))
                    .collect(),
                ..NameGlyphs::default()
            };
            let mut named = Named {
                text: name.text.clone(),
                offset: name.offset,
                lines: [
                    Some(line(&name.text.name)),
                    name.text.below.as_ref().map(line),
                ],
            };
            for glyphs in named.lines.iter_mut().flatten() {
                lay_out_name(
                    glyphs,
                    name.offset,
                    &self.font,
                    &mut self.pages,
                    &mut self.random,
                );
            }
            self.names.insert(id, named);
        }
        let Some(named) = self.names.get_mut(&id) else {
            return;
        };
        let below = named.lines[1].is_some() && nearer_than(f64::from(name.below_distance));
        // The name stands a line higher over what is below it.
        let lines = [(0, if below { nametag::LINE } else { 0.0 }), (1, 0.0)];
        for (line, lift) in lines.into_iter().take(if below { 2 } else { 1 }) {
            let Some(glyphs) = &mut named.lines[line] else {
                continue;
            };
            if glyphs.obfuscated {
                lay_out_name(
                    glyphs,
                    name.offset,
                    &self.font,
                    &mut self.pages,
                    &mut self.random,
                );
            }
            if !self.pages.arrived(glyphs.newest) {
                continue;
            }
            self.name_runs.push(NameRun {
                id,
                line,
                transform: nametag::transform(top, lift, axes),
                light,
                discrete: name.discrete,
                away: away as f32,
            });
        }
    }

    /// Notes that the instances from `first` on are drawn from back to front with what
    /// else is, if their layer is.
    fn sort(&mut self, first: usize, layer: u8) {
        if matches!(layer, SKIN_SORTED | SKIN_INVISIBLE | ITEM_SORTED)
            && let Some(i) = self.instances.get(first)
        {
            let away = i.transform.0.iter().map(|row| row[3] * row[3]).sum();
            self.sorted.push((away, first..self.instances.len()));
        }
    }

    /// Whose head the one at `pos` is.
    fn owner(&mut self, terrain: &Terrain, pos: [i32; 3]) -> Option<Arc<SkinSource>> {
        let column = terrain.column(pos[0] >> 4, pos[2] >> 4)?;
        let stamp = (column.generation(), column.block_entities_rev());
        if self.heads.get(&pos).is_none_or(|h| h.stamp != stamp) {
            let tag = column
                .block_entities()
                .iter()
                .find(|e| e.pos == pos)
                .and_then(|e| e.tag.clone());
            let same = |h: &Head| match (&h.tag, &tag) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            match self.heads.get_mut(&pos) {
                Some(h) if same(h) => h.stamp = stamp,
                _ => {
                    if self.heads.len() >= KEPT_HEADS {
                        self.heads.clear();
                    }
                    let owner = tag
                        .as_deref()
                        .and_then(|t| t.as_compound()?.get("profile"))
                        .and_then(ResolvableProfile::from_nbt)
                        .map(|p| Arc::new(SkinSource::Profile(p)));
                    self.heads.insert(pos, Head { stamp, tag, owner });
                }
            }
        }
        self.heads.get(&pos)?.owner.clone()
    }

    /// Adds the sides of a sign that have text and face the camera.
    #[allow(clippy::too_many_arguments)]
    fn add_sign(
        &mut self,
        pos: [i32; 3],
        state: u32,
        [hanging, wall]: [bool; 2],
        terrain: &Terrain,
        frame: &Frame,
        place: Placed,
        away: f64,
    ) {
        let Some(column) = terrain.column(pos[0] >> 4, pos[2] >> 4) else {
            return;
        };
        let stamp = (column.generation(), column.block_entities_rev());
        if self.signs.get(&pos).is_none_or(|s| s.stamp != stamp) {
            let tag = column
                .block_entities()
                .iter()
                .find(|e| e.pos == pos)
                .and_then(|e| e.tag.clone());
            let same = |s: &Sign| match (&s.tag, &tag) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            match self.signs.get_mut(&pos) {
                // Another block entity of the chunk changed.
                Some(s) if same(s) && s.stamp.0 == stamp.0 => s.stamp = stamp,
                _ => {
                    if self.signs.len() >= KEPT_SIGNS {
                        self.signs.clear();
                    }
                    let sides = ["front_text", "back_text"].map(|key| {
                        let text = SignText::read(tag.as_ref()?, key, &self.lang)?;
                        Some(Box::new(Side {
                            text,
                            glyphs: Vec::new(),
                            newest: 0,
                            obfuscated: false,
                        }))
                    });
                    let mut sign = Sign { stamp, tag, sides };
                    for side in sign.sides.iter_mut().flatten() {
                        lay_out(side, hanging, &self.font, &mut self.pages, &mut self.random);
                    }
                    self.signs.insert(pos, sign);
                }
            }
        }
        let Some(sign) = self.signs.get_mut(&pos) else {
            return;
        };
        for (i, side) in sign.sides.iter_mut().enumerate() {
            let Some(side) = side else {
                continue;
            };
            let mut transform = place::sign_text(state, hanging, wall, i == 0);
            for (row, origin) in transform.0.iter_mut().zip(place.origin) {
                row[3] += origin;
            }
            if !faces_camera(&transform) {
                continue;
            }
            if side.obfuscated {
                lay_out(side, hanging, &self.font, &mut self.pages, &mut self.random);
            }
            if !self.pages.arrived(side.newest) {
                continue;
            }
            let near: f64 = (0..3)
                .map(|i| (f64::from(pos[i]) + 0.5 - frame.player.position[i]).powi(2))
                .sum();
            self.runs.push(SignRun {
                pos,
                back: i == 1,
                outline: record_colour(side.text.outline()),
                outlined: side
                    .text
                    .outlined(near < OUTLINE_DISTANCE * OUTLINE_DISTANCE),
                transform,
                light: if side.text.glowing {
                    record_light(15, 15)
                } else {
                    place.light
                },
                away,
            });
        }
    }

    /// The `i`th pattern of the banner at `pos`.
    fn pattern(&mut self, terrain: &Terrain, pos: [i32; 3], i: usize) -> Option<(TextureId, u32)> {
        if i == 0 {
            let tag = terrain
                .column(pos[0] >> 4, pos[2] >> 4)?
                .block_entities()
                .iter()
                .find(|e| e.pos == pos)?
                .tag
                .as_ref()?;
            if !self
                .patterns
                .get(&pos)
                .is_some_and(|p| Arc::ptr_eq(&p.tag, tag))
            {
                if self.patterns.len() >= KEPT_PATTERNS {
                    self.patterns.clear();
                }
                let layers = listed_patterns(tag)
                    .filter_map(|(id, dye)| {
                        let texture = *self.textures.get(&pattern_texture(id))?;
                        Some((texture, record_colour(DYE_COLOURS[usize::from(dye)])))
                    })
                    .collect();
                let tag = tag.clone();
                self.patterns.insert(pos, Patterns { tag, layers });
            }
        }
        self.patterns.get(&pos)?.layers.get(i).copied()
    }

    /// Adds a model's parts.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        kind: Kind,
        state: u32,
        model: Model,
        pose: Pose,
        place: Placed,
        layer: u8,
        texture: TextureId,
        colour: u32,
    ) {
        let outer = place::transform(kind, state);
        self.parts.clear();
        // What does not move is worked out once for each state. (No state has two models
        // that do not.)
        let still = match pose {
            Pose::Rest | Pose::Skull(_) => true,
            Pose::Shulker(open) => open == 0.0,
            Pose::Chest(_) | Pose::Flag(_) => false,
        };
        if still {
            let (models, parts) = (&self.models, &mut self.parts);
            let kept = self.resting.entry(state).or_insert_with(|| {
                let mut kept = Vec::new();
                models.draw(model, pose, &mut kept);
                for p in &mut kept {
                    p.transform = outer.then_inner(p.transform);
                }
                kept
            });
            parts.extend_from_slice(kept);
        } else {
            self.models.draw(model, pose, &mut self.parts);
            for p in &mut self.parts {
                p.transform = outer.then_inner(p.transform);
            }
        }
        // (A head with a skin of its own is where it is.)
        let nearer = is_nearer(kind, layer);
        for p in &self.parts {
            let mut transform = p.transform;
            for (row, origin) in transform.0.iter_mut().zip(place.origin) {
                row[3] += origin;
                if nearer {
                    *row = row.map(|c| c * TOWARDS_CAMERA);
                }
            }
            self.instances.push(Instance {
                batch: (layer, u32::from(p.over), p.first_quad),
                quads: p.quads,
                transform,
                light: place.light,
                colour,
                texture,
                hurt: false,
                glint: Glint::None,
                source: 0,
            });
        }
    }

    /// Adds what a figure wears in one of its four places, if that is armour for the
    /// place and has a look (`HumanoidArmorLayer`, `EquipmentLayerRenderer`): its layers in
    /// their order, each in its colour, and its trim over them.
    fn add_armour(
        &mut self,
        figure: &Figure,
        slim: bool,
        place: usize,
        stack: &Stack,
        light: u32,
        textures: &TextureTable,
    ) {
        const SLOTS: [EquipmentSlot; 4] = [
            EquipmentSlot::Head,
            EquipmentSlot::Chest,
            EquipmentSlot::Legs,
            EquipmentSlot::Feet,
        ];
        let c = &stack.c;
        if stack.is_empty() || c.equippable.is_none_or(|e| e.slot != SLOTS[place]) {
            return;
        }
        let Some(asset) = c.equipment_asset.as_deref() else {
            return;
        };
        let Some(worn) = self.equipment.get(asset) else {
            return;
        };
        let leggings = SLOTS[place] == EquipmentSlot::Legs;
        let (layers, dir) = if leggings {
            (&worn.leggings, equipment::LEGGINGS_DIR)
        } else {
            (&worn.humanoid, equipment::HUMANOID_DIR)
        };
        if layers.is_empty() {
            return;
        }
        // Its layers' pictures are looked up once.
        let pictures = &mut self.worn_pictures[usize::from(leggings)];
        if !pictures.contains_key(asset) {
            let found = layers
                .iter()
                .map(|l| self.textures.get(&format!("{dir}/{}", l.texture)).copied())
                .collect();
            pictures.insert(asset.to_owned(), found);
        }
        let pictures = &pictures[asset];
        // What it is drawn with, in order: each layer's picture and colour, then the trim.
        let mut drawn = std::mem::take(&mut self.worn);
        drawn.clear();
        for (layer, picture) in layers.iter().zip(pictures) {
            let Some(colour) = layer.colour(c.dyed_color) else {
                continue;
            };
            if let Some(texture) = picture.filter(|t| textures.is_ready(*t)) {
                drawn.push((texture, record_colour(colour)));
            }
        }
        if let Some(trim) = &c.trim {
            let at = (
                Arc::as_ptr(trim) as usize,
                std::ptr::from_ref(worn) as usize,
                leggings,
            );
            if !self.trim_keys.contains_key(&at) {
                if self.trim_keys.len() >= KEPT_TRIMS {
                    self.trim_keys.clear();
                }
                let (picture, palette) = worn.trim_picture(trim);
                let key = equipment::trim_path(&picture, leggings).map(|base| (base, palette));
                self.trim_keys.insert(at, (Arc::clone(trim), key));
            }
            if let Some(key) = &self.trim_keys[&at].1 {
                match self.trim_textures.get(key) {
                    Some(Some(texture)) if textures.is_ready(*texture) => {
                        drawn.push((*texture, WHITE));
                    }
                    Some(_) => {}
                    None => {
                        if self.trim_textures.len() >= KEPT_TRIMS {
                            self.trim_textures.clear();
                        }
                        self.trim_textures.insert(key.clone(), None);
                        self.trims_wanted.push(key.clone());
                    }
                }
            }
        }
        // Enchanted, its first layer glints; with a trim, all of it does.
        let glints = c.glints() && self.glint_pictures[0].is_some_and(|t| textures.is_ready(t));
        let trimmed = c.trim.is_some();
        self.parts.clear();
        let model = Model::Armour {
            slim,
            piece: place as u8,
        };
        self.models
            .draw_by(model, &mut self.parts, |name, m| figure.place_worn(name, m));
        for (n, (texture, colour)) in drawn.iter().copied().enumerate() {
            let glint = if glints && (trimmed || n == 0) {
                Glint::Armour
            } else {
                Glint::None
            };
            for p in &self.parts {
                // Worn things are drawn a little nearer than they are.
                let mut transform = figure.transform.then_inner(p.transform);
                for row in &mut transform.0 {
                    *row = row.map(|c| c * TOWARDS_CAMERA);
                }
                self.instances.push(Instance {
                    batch: (CUTOUT_FACES, 0, p.first_quad),
                    quads: p.quads,
                    transform,
                    light,
                    colour,
                    texture,
                    // What is worn does not redden.
                    hurt: false,
                    glint,
                    source: 0,
                });
            }
        }
        self.worn = drawn;
    }

    /// Adds what a figure holds in its main hand or its other one (`ItemInHandLayer`):
    /// what the stack's item definition draws for it, placed in the hand.
    #[allow(clippy::too_many_arguments)]
    fn add_held(
        &mut self,
        figure: &Figure,
        r: &AvatarRecord,
        slim: bool,
        off_hand: bool,
        light: u32,
        atlases: Atlases,
        textures: &TextureTable,
    ) {
        let stack = &r.held[usize::from(off_hand)];
        if stack.is_empty() {
            return;
        }
        // The main hand is the right one, unless it is the left.
        let left = off_hand != r.left_handed;
        // Where the hand is: on the arm, turned to hold (a thin arm half a texel
        // further out).
        let arm = if left { "left_arm" } else { "right_arm" };
        let model = if slim {
            Model::PlayerSlim
        } else {
            Model::Player
        };
        let on_arm = self.models.part_transform(model, arm, |name, m| {
            figure.place_worn(name, m);
            if slim && name == arm {
                m.position[0] += if left { -0.5 } else { 0.5 };
            }
        });
        let Some(on_arm) = on_arm else {
            return;
        };
        let side = if left { -1.0 } else { 1.0 };
        let hand = figure
            .transform
            .then_inner(on_arm)
            .rotate_x(-std::f32::consts::FRAC_PI_2)
            .rotate_y(std::f32::consts::PI)
            .translate([side / 16.0, 0.125, -0.625]);
        let how = HeldHow {
            left,
            first_person: false,
            gui: false,
            left_handed: r.left_handed,
            using: r.using.is_some_and(|(_, off)| off == off_hand),
            use_ticks: r.using.map_or(0, |u| u.0),
            use_remaining: r.use_remaining,
        };
        let _ = self.add_held_at(stack, hand, how, light, atlases, textures);
    }

    /// Adds a stack's models where a hand at `hand` holds it, as its item's definition
    /// says for a stack held so. Returns how a GUI lights the first of them: from the
    /// side, as a block, or not; nothing for a stack that has no model.
    fn add_held_at(
        &mut self,
        stack: &Stack,
        hand: Affine,
        how: HeldHow,
        light: u32,
        atlases: Atlases,
        textures: &TextureTable,
    ) -> Option<bool> {
        let c = &stack.c;
        let id = c.item_model.as_deref().filter(|_| !stack.is_empty())?;
        // (Taken out for as long as its leaves are read.)
        let items = self.items.take()?;
        let mut side_lit = None;
        let left = how.left;
        let has_component = |name: &str, _ignore_default: bool| {
            stack.patch.added.iter().any(|(kind, _)| {
                u32::try_from(*kind)
                    .ok()
                    .and_then(DataComponentType::from_id)
                    .is_some_and(|k| k.name() == name)
            })
        };
        let held = Held {
            context: match (how.first_person, left) {
                _ if how.gui => "gui",
                (false, true) => "thirdperson_lefthand",
                (false, false) => "thirdperson_righthand",
                (true, true) => "firstperson_lefthand",
                (true, false) => "firstperson_righthand",
            },
            left_handed: how.left_handed,
            using: how.using,
            use_ticks: how.use_ticks,
            use_remaining: how.use_remaining,
            damage: c
                .damageable()
                .then(|| (c.damage.unwrap_or(0), c.max_damage.unwrap_or(0))),
            count: (stack.count, c.max_stack_size()),
            charge: match (c.charged, c.charged_rocket) {
                (false, _) => "none",
                (true, true) => "rocket",
                (true, false) => "arrow",
            },
            charge_ticks: c.crossbow_charge_ticks as f32,
            trim_material: c
                .trim
                .as_deref()
                .map(|t| &*t.material)
                .filter(|m| !m.is_empty()),
            block_state: c.block_state.as_deref().map_or(&[][..], Vec::as_slice),
            has_component: &has_component,
        };
        {
            let glints = c.glints();
            items.resolve_each(id, &held, &mut |leaf| {
                let (mesh, transformation) = match leaf {
                    Drawn::Model {
                        mesh,
                        transformation,
                        ..
                    } => (mesh, transformation),
                    Drawn::Special {
                        base,
                        transformation,
                        ..
                    } => (base, transformation),
                };
                let Some(model) = items.mesh(mesh) else {
                    return;
                };
                side_lit.get_or_insert(model.gui_side_lit);
                let place = if how.gui {
                    model.gui
                } else if how.first_person {
                    model.first_person[usize::from(left)]
                } else {
                    model.hands[usize::from(left)]
                };
                let placed = place_item(hand, place, left).then_inner(Affine(*transformation));
                match leaf {
                    Drawn::Model { tints, .. } => self.add_item(
                        mesh,
                        model,
                        placed,
                        |tint| record_colour(tint_colour(tints, tint, c)),
                        light,
                        glints,
                        atlases,
                        textures,
                    ),
                    Drawn::Special { special, .. } => {
                        self.add_special(special, placed, c, light, glints, textures);
                    }
                }
            });
        }
        self.items = Some(items);
        side_lit
    }

    /// Adds an item's model where `placed` puts it: its quads by the atlas they are of
    /// and the tint that colours them, once its vertices are there.
    #[allow(clippy::too_many_arguments)]
    fn add_item(
        &mut self,
        mesh: u32,
        model: &ItemMesh,
        placed: Affine,
        colour: impl Fn(i32) -> u32,
        light: u32,
        glints: bool,
        atlases: Atlases,
        textures: &TextureTable,
    ) {
        if model.quads.is_empty() {
            return;
        }
        match self.item_ranges.get(&mesh) {
            Some(ItemRange::Ready(_)) => {}
            Some(_) => return,
            None => {
                self.item_ranges.insert(mesh, ItemRange::Wanted);
                self.items_wanted.push(mesh);
                return;
            }
        }
        let glint = if glints && self.glint_pictures[1].is_some_and(|t| textures.is_ready(t)) {
            Glint::Item
        } else {
            Glint::None
        };
        // What is seen through last, so that it can be put in its place among the rest.
        for translucent in [false, true] {
            let first = self.instances.len();
            for run in model.runs.iter().filter(|r| r.translucent == translucent) {
                let atlas = if run.block_atlas {
                    atlases.blocks
                } else {
                    atlases.items
                };
                let Some(texture) = atlas else {
                    continue;
                };
                self.instances.push(Instance {
                    batch: (if translucent { ITEM_SORTED } else { CUTOUT }, 0, run.first),
                    quads: run.count,
                    transform: placed,
                    light,
                    colour: colour(run.tint_index),
                    texture,
                    hurt: false,
                    glint,
                    source: mesh + 1,
                });
            }
            if translucent {
                self.sort(first, ITEM_SORTED);
            }
        }
    }

    /// Adds a shield or a trident where `placed` puts it, from the game's own model of
    /// it (`ShieldSpecialRenderer`, `TridentSpecialRenderer`).
    fn add_special(
        &mut self,
        special: Special,
        placed: Affine,
        c: &Components,
        light: u32,
        glints: bool,
        textures: &TextureTable,
    ) {
        let patterns = c.banner_patterns.as_deref().unwrap_or(&[]);
        let patterned = !patterns.is_empty() || c.base_color.is_some();
        let (model, base) = match special {
            Special::Shield if patterned => (Model::Shield, "shield/shield_base"),
            Special::Shield => (Model::Shield, "shield/shield_base_nopattern"),
            Special::Trident => (Model::Trident, TRIDENT),
            Special::Other => return,
        };
        let usable = |r: &Self, path: &str| {
            r.textures
                .get(path)
                .copied()
                .filter(|t| textures.is_ready(*t))
        };
        let Some(texture) = usable(self, base) else {
            return;
        };
        // A shield with patterns does not glint here: its glint is a draw of its own.
        let glint =
            if glints && !patterned && self.glint_pictures[1].is_some_and(|t| textures.is_ready(t))
            {
                Glint::Entity
            } else {
                Glint::None
            };
        // Its ground, then its patterns over it in their order.
        let mut drawn = [(SOLID, texture, WHITE, glint); 2 + MAX_PATTERNS];
        let mut count = 1;
        let mut push = |entry| {
            drawn[count] = entry;
            count += 1;
        };
        if special == Special::Shield && patterned {
            let dyed = |dye: u8| record_colour(DYE_COLOURS[usize::from(dye.min(15))]);
            if let Some(ground) = usable(self, "shield/base") {
                push((
                    PATTERN,
                    ground,
                    dyed(c.base_color.unwrap_or(0)),
                    Glint::None,
                ));
            }
            for (asset, dye) in patterns.iter().take(MAX_PATTERNS) {
                let path = asset.strip_prefix("minecraft:").unwrap_or(asset);
                if let Some(pattern) = usable(self, &format!("{SHIELD_DIR}/{path}")) {
                    push((PATTERN, pattern, dyed(*dye), Glint::None));
                }
            }
        }
        self.parts.clear();
        self.models.draw(model, Pose::Rest, &mut self.parts);
        for (layer, texture, colour, glint) in drawn.into_iter().take(count) {
            for p in &self.parts {
                self.instances.push(Instance {
                    batch: (layer, 0, p.first_quad),
                    quads: p.quads,
                    transform: placed.then_inner(p.transform),
                    light,
                    colour,
                    texture,
                    hurt: false,
                    glint,
                    source: 0,
                });
            }
        }
    }

    /// A trim's picture in its palette: its size and texels.
    fn trim_picture(&self, key: &(String, Option<String>)) -> Option<(u32, u32, Vec<u8>)> {
        let (image, base) = self.trim_bases.get(&key.0)?;
        let mapped = key
            .1
            .as_deref()
            .and_then(equipment::palette_path)
            .and_then(|target| {
                Some((
                    self.palettes.get(base.as_deref()?)?,
                    self.palettes.get(target)?,
                ))
            })
            .map_or_else(
                || image.rgba.clone(),
                |(base, target)| equipment::map_palette(&image.rgba, base, target),
            );
        Some((image.width, image.height, mapped))
    }

    /// Draws what was gathered, into `pass`, which has a depth attachment. `indices` are
    /// the shared quad indices.
    ///
    /// # Safety
    /// `pass` is of the frame being recorded, whose `UploadRing::record` and
    /// [`Self::note_ready`] have run; `indices` and the light map can be read.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
        cardinal: CardinalLight,
        lightmap: TextureId,
        indices: (vk::Buffer, u64),
    ) {
        profiling::scope!("block_entities_draw");
        let Some(range) = self.pool.resolve(self.range).filter(|_| self.ready) else {
            return;
        };
        // SAFETY: the caller's.
        unsafe {
            self.draw_models(
                pass,
                frame_data,
                view,
                fog,
                match cardinal {
                    CardinalLight::Default => LIGHTS_USUAL,
                    CardinalLight::Nether => LIGHTS_NETHER,
                },
                lightmap,
                indices,
                (range.buffer, range.offset),
            )
        };
        // Over the boards the mesher drew.
        // SAFETY: the caller's.
        unsafe { self.draw_signs(pass, frame_data, view, fog, lightmap) };
        // SAFETY: the caller's.
        unsafe { self.draw_names(pass, frame_data, view, fog, lightmap) };
    }

    /// Draws the player's own hands, as [`Self::gather_hand`] left them, into `pass`,
    /// whose depths have been cleared: `view` is the hands' own, with the field of view
    /// they are always seen with.
    ///
    /// # Safety
    /// As [`Self::draw`].
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw_hand(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
        cardinal: CardinalLight,
        lightmap: TextureId,
        indices: (vk::Buffer, u64),
    ) {
        if self.hand.is_empty() {
            return;
        }
        let Some(range) = self.pool.resolve(self.range).filter(|_| self.ready) else {
            return;
        };
        // Drawn as the world's models are, in their place; no shadow is among them.
        std::mem::swap(&mut self.instances, &mut self.hand);
        std::mem::swap(&mut self.sorted, &mut self.hand_sorted);
        let shadows = std::mem::take(&mut self.shadows);
        let parts = self.stats.parts;
        // SAFETY: the caller's.
        unsafe {
            self.draw_models(
                pass,
                frame_data,
                view,
                fog,
                match cardinal {
                    CardinalLight::Default => LIGHTS_USUAL,
                    CardinalLight::Nether => LIGHTS_NETHER,
                },
                lightmap,
                indices,
                (range.buffer, range.offset),
            )
        };
        self.stats.hand_parts = std::mem::replace(&mut self.stats.parts, parts);
        self.shadows = shadows;
        std::mem::swap(&mut self.instances, &mut self.hand);
        std::mem::swap(&mut self.sorted, &mut self.hand_sorted);
    }

    /// The game's font, as the signs are written in it.
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// Adds the items a GUI shows (`GuiGraphicsExtractor.item`, `GuiItemAtlas.drawToSlot`):
    /// each of `items` holds `slots`' stack of its number in a square of 16 GUI units,
    /// which are `scale` pixels each. `using` is the slot whose stack is being used, for
    /// so many ticks with so many left.
    pub fn gather_gui_items(
        &mut self,
        items: &[HudItem],
        slots: &[Stack],
        scale: f32,
        using: Option<(usize, i32, i32)>,
        block_atlas: TextureId,
        textures: &TextureTable,
    ) {
        let atlases = Atlases {
            items: self.item_atlas.filter(|t| textures.is_ready(*t)),
            blocks: Some(block_atlas).filter(|t| textures.is_ready(*t)),
        };
        let light = record_light(GUI_LIGHT.0, GUI_LIGHT.1);
        let side = 16.0 * scale;
        let sorted = std::mem::take(&mut self.sorted);
        let mut gathered = std::mem::take(&mut self.instances);
        for item in items {
            let Some(stack) = slots.get(item.slot) else {
                continue;
            };
            // Pixels from the target's corner: the slot's middle, the model a slot wide
            // and upright, towards the eyes.
            let mut at = Affine::IDENTITY;
            if let Some([wide, tall]) = item.squeeze {
                let about = [
                    (item.x + 8) as f32 * scale,
                    (item.y + 12) as f32 * scale,
                    0.0,
                ];
                at = at
                    .translate(about)
                    .scale([wide, tall, 1.0])
                    .translate(about.map(|c| -c));
            }
            let at = at
                .translate([
                    (item.x + 8) as f32 * scale,
                    (item.y + 8) as f32 * scale,
                    0.0,
                ])
                .scale([side, -side, side]);
            let used = using.filter(|u| u.0 == item.slot);
            let how = HeldHow {
                left: false,
                first_person: false,
                gui: true,
                left_handed: false,
                using: used.is_some(),
                use_ticks: used.map_or(0, |u| u.1),
                use_remaining: used.map_or(0, |u| u.2),
            };
            // How its first model is lit, all of it is.
            if let Some(side_lit) = self.add_held_at(stack, at, how, light, atlases, textures) {
                self.gui[usize::from(side_lit)].append(&mut self.instances);
            }
            self.instances.clear();
            self.sorted.clear();
        }
        std::mem::swap(&mut self.instances, &mut gathered);
        self.sorted = sorted;
    }

    /// Draws the items a GUI shows, as [`Self::gather_gui_items`] left them, into
    /// `pass`, whose depths have been cleared, on a target of `size` pixels.
    ///
    /// # Safety
    /// As [`Self::draw`].
    pub unsafe fn draw_gui_items(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        size: [f32; 2],
        lightmap: TextureId,
        indices: (vk::Buffer, u64),
    ) {
        if self.gui.iter().all(Vec::is_empty) {
            return;
        }
        let Some(range) = self.pool.resolve(self.range).filter(|_| self.ready) else {
            return;
        };
        let view = View::gui(size);
        let sorted = std::mem::take(&mut self.sorted);
        let shadows = std::mem::take(&mut self.shadows);
        let parts = self.stats.parts;
        let mut drawn = 0;
        for (list, lights) in [LIGHTS_GUI_FLAT, LIGHTS_GUI_SIDE].into_iter().enumerate() {
            std::mem::swap(&mut self.instances, &mut self.gui[list]);
            // SAFETY: the caller's.
            unsafe {
                self.draw_models(
                    pass,
                    frame_data,
                    &view,
                    &Fog::NONE,
                    lights,
                    lightmap,
                    indices,
                    (range.buffer, range.offset),
                )
            };
            drawn += std::mem::replace(&mut self.stats.parts, 0);
            std::mem::swap(&mut self.instances, &mut self.gui[list]);
        }
        self.stats.gui_parts = drawn;
        self.stats.parts = parts;
        self.shadows = shadows;
        self.sorted = sorted;
    }

    /// Lines of text as a GUI's quads, in the game's font: every line's shadows, then
    /// its glyphs. A glyph whose picture has not arrived yet is left out.
    pub fn gui_text(&mut self, texts: &[HudText], out: &mut Vec<Quad>) {
        for text in texts {
            for shadow in [true, false] {
                if shadow && !text.shadow {
                    continue;
                }
                let mut x = text.x as f32;
                for c in text.text.chars() {
                    let glyph = self.font.glyph(c);
                    let pen = x;
                    x += glyph.advance(false);
                    let Some(picture) = glyph.picture else {
                        continue;
                    };
                    let Some((texture, uv)) = self.pages.picture(sign::Ink::Char(c), &self.font)
                    else {
                        continue;
                    };
                    let (by, color) = if shadow {
                        (glyph.shadow_offset, shadow_colour(text.color))
                    } else {
                        (0.0, text.color)
                    };
                    let y = text.y as f32;
                    out.push(Quad {
                        rect: [
                            pen + by,
                            y + picture.top + by,
                            pen + picture.right + by,
                            y + picture.bottom + by,
                        ],
                        uv,
                        color,
                        texture,
                    });
                }
            }
        }
    }

    /// Draws what is seen through everything else, into `pass`: the names over heads,
    /// once the rest of the world is drawn.
    ///
    /// # Safety
    /// As [`Self::draw`], which has run for this frame with the same `pass`.
    pub unsafe fn draw_over(&mut self, pass: &mut Pass<'_>) {
        if let Some((push, glyphs)) = self.over.take() {
            // SAFETY: the push constants are sign_vert.glsl's and point at the records
            // `draw_names` wrote into this frame's data, whose pages have arrived.
            unsafe { pass.draw(PipelineId::NameTagSheer.index(), &push, 6, glyphs) };
            self.stats.draws += 1;
        }
    }

    /// # Safety
    /// As [`Self::draw`].
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_models(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
        lights: u32,
        lightmap: TextureId,
        indices: (vk::Buffer, u64),
        vertices: (vk::Buffer, u64),
    ) {
        if self.instances.is_empty() {
            return;
        }
        // From back to front, each one's second layer over its first.
        order(&mut self.sorted, &mut self.instances);
        // Stable: the patterns of a banner stay in their order.
        self.instances
            .sort_by_key(|i| (i.batch.0, i.batch.1, i.source, i.batch.2));
        let Some((world, matrix)) = write_world(frame_data, view, [1.0; 2], lightmap) else {
            self.stats.overflowed += self.instances.len() as u64;
            return;
        };
        let room = (frame_data.capacity() - frame_data.used()).saturating_sub(RESERVED)
            / u64::from(MODEL_PART.bytes());
        let count = self.instances.len().min(room as usize);
        self.stats.overflowed += (self.instances.len() - count) as u64;
        if count == 0 {
            return;
        }
        // The glints' records first: the parts they lie over name them.
        let mut glints = [0u32; 3];
        if self.instances[..count]
            .iter()
            .any(|i| i.glint != Glint::None)
        {
            let millis = self.started.elapsed().as_millis() as u64;
            for (n, (of, scale)) in GLINTS.into_iter().enumerate() {
                let Some(picture) = self.glint_pictures[of] else {
                    continue;
                };
                if let Some((at, bytes)) = frame_data.alloc(GLINT.words(), 4) {
                    write_glint(bytes, picture, glint_rows(millis, scale));
                    glints[n] = at;
                }
            }
        }
        let Some((first, bytes)) = frame_data.alloc(count as u32 * MODEL_PART.words(), 4) else {
            self.stats.overflowed += count as u64;
            return;
        };
        for (i, record) in self.instances[..count]
            .iter()
            .zip(bytes.as_chunks_mut::<PART_BYTES>().0)
        {
            write_part(record, i, glints);
        }
        let push = model_push([world, matrix, first, lights], fog);
        self.stats.parts = count as u32;
        // Shadows lie under what is seen through: they are drawn before the skins.
        let shadows = self.write_shadows(frame_data, [world, matrix], fog);
        let before_skins = self.instances[..count].partition_point(|i| i.batch.0 < SKIN);
        let mut shadows_at = shadows.map(|s| (before_skins, s));
        let mut pushed = false;
        let mut at = 0;
        // SAFETY: the push constants are model.vert's and the records they point at were
        // written just above; the vertices' copy can be read (`ready`), every draw reads
        // a part's own quads, and no part has more than the shared indices reach; the
        // textures are of the table and have arrived (`gather`).
        unsafe {
            pass.bind_index_buffer(indices.0, indices.1);
            // The vertices bound: none yet.
            let mut bound = u32::MAX;
            while at < count {
                if let Some((_, (shadow_push, pieces))) = shadows_at.take_if(|s| s.0 == at) {
                    pass.draw(PipelineId::Shadow.index(), &shadow_push, 6, pieces);
                    self.stats.draws += 1;
                    pushed = false;
                }
                let Instance {
                    batch,
                    quads,
                    source,
                    ..
                } = self.instances[at];
                let run = self.instances[at..count]
                    .iter()
                    .take_while(|i| i.batch == batch && i.source == source)
                    .count();
                if source != bound {
                    // An item's model is ready, or it was not gathered.
                    let from = match source.checked_sub(1) {
                        None => Some(vertices),
                        Some(mesh) => match self.item_ranges.get(&mesh) {
                            Some(ItemRange::Ready(handle)) => self
                                .item_pool
                                .resolve(*handle)
                                .map(|r| (r.buffer, r.offset)),
                            _ => None,
                        },
                    };
                    let Some((buffer, offset)) = from else {
                        at += run;
                        continue;
                    };
                    pass.bind_vertex_buffer(buffer, offset);
                    bound = source;
                }
                pass.draw_indexed_instances(
                    LAYERS[usize::from(batch.0)].index(),
                    if pushed { &[] } else { &push },
                    quads * 6,
                    0,
                    (batch.2 * 4) as i32,
                    at as u32,
                    run as u32,
                );
                pushed = true;
                self.stats.draws += 1;
                at += run;
            }
            // (Nothing but what is solid was drawn.)
            if let Some((_, (shadow_push, pieces))) = shadows_at {
                pass.draw(PipelineId::Shadow.index(), &shadow_push, 6, pieces);
                self.stats.draws += 1;
            }
        }
    }

    /// Writes the frame's pieces of shadows; their draw's push constants and how many
    /// there are, if any.
    fn write_shadows(
        &mut self,
        frame_data: &mut FrameData,
        [world, matrix]: [u32; 2],
        fog: &Fog,
    ) -> Option<([u8; MODEL_PUSH_BYTES], u32)> {
        let picture = self.shadow_picture?;
        let room = (frame_data.capacity() - frame_data.used()).saturating_sub(RESERVED)
            / u64::from(SHADOW.bytes());
        let count = self.shadows.len().min(room as usize);
        self.stats.overflowed += (self.shadows.len() - count) as u64;
        if count == 0 {
            return None;
        }
        let (first, bytes) = frame_data.alloc(count as u32 * SHADOW.words(), 4)?;
        for (s, record) in self
            .shadows
            .iter()
            .zip(bytes.as_chunks_mut::<SHADOW_BYTES>().0)
        {
            write_shadow(record, s);
        }
        self.stats.shadows = count as u32;
        let push = model_push([world, matrix, first, picture.index()], fog);
        Some((push, count as u32))
    }

    /// Draws the names that were gathered, but for those seen through walls, which are
    /// left for [`Self::draw_over`]: where they are not behind something, those are drawn
    /// whole here too.
    ///
    /// # Safety
    /// As [`Self::draw`].
    unsafe fn draw_names(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
        lightmap: TextureId,
    ) {
        if self.name_runs.is_empty() {
            return;
        }
        // From back to front: what is nearer lies over what is farther.
        self.name_runs.sort_by(|a, b| b.away.total_cmp(&a.away));
        let glyphs_of = |names: &HashMap<i32, Named>, run: &NameRun| {
            names
                .get(&run.id)
                .and_then(|n| n.lines[run.line].as_ref())
                .map_or((0, 0), |g| (g.solid.len() as u64, g.sheer.len() as u64))
        };
        // The nearest are drawn when not all can be: a name takes two records and its
        // glyphs twice at most.
        let room = (frame_data.capacity() - frame_data.used())
            .saturating_sub(RESERVED)
            .min(GLYPH_BYTES);
        let mut taken = 0;
        let mut kept = 0;
        for run in self.name_runs.iter().rev() {
            let (solid, sheer) = glyphs_of(&self.names, run);
            taken += 2 * u64::from(SIGN.bytes()) + (solid + sheer) * u64::from(GLYPH.bytes());
            if taken > room {
                break;
            }
            kept += 1;
        }
        self.stats.overflowed += (self.name_runs.len() - kept) as u64;
        self.name_runs.drain(..self.name_runs.len() - kept);
        if kept == 0 {
            return;
        }
        // In the first draw a name seen through walls is whole, another as it is; the
        // second draws the first kind again, seen through.
        let (mut here, mut over) = (0u64, 0u64);
        let mut seen_through = 0;
        for run in &self.name_runs {
            let (solid, sheer) = glyphs_of(&self.names, run);
            if run.discrete {
                here += sheer;
            } else {
                here += solid;
                over += sheer;
                seen_through += 1;
            }
        }
        let Some((world, matrix)) = write_world(frame_data, view, [1.0; 2], lightmap) else {
            return;
        };
        let records = (kept + seen_through) as u32;
        let Some((signs, bytes)) = frame_data.alloc(records * SIGN.words(), 4) else {
            return;
        };
        let mut sign_records = bytes.as_chunks_mut::<SIGN_BYTES>().0.iter_mut();
        for run in &self.name_runs {
            let (block, sky) = run.light;
            let light = if run.discrete {
                record_light(block, sky)
            } else {
                record_light(block.max(nametag::EMISSION), sky.max(nametag::EMISSION))
            };
            if let Some(record) = sign_records.next() {
                write_sign_record(record, &run.transform, light, 0);
            }
        }
        for run in self.name_runs.iter().filter(|r| !r.discrete) {
            if let Some(record) = sign_records.next() {
                write_sign_record(record, &run.transform, UNLIT, 0);
            }
        }
        let Some((first, bytes)) = frame_data.alloc((here + over) as u32 * GLYPH.words(), 4) else {
            return;
        };
        let textures = self.pages.textures();
        let mut out = bytes.as_chunks_mut::<GLYPH_RECORD_BYTES>().0.iter_mut();
        let page =
            |glyph: &[u32; GLYPH_WORDS]| textures[glyph[GLYPH.offset("texture") as usize] as usize];
        for (n, run) in self.name_runs.iter().enumerate() {
            let Some(g) = self
                .names
                .get(&run.id)
                .and_then(|n| n.lines[run.line].as_ref())
            else {
                continue;
            };
            let glyphs = if run.discrete { &g.sheer } else { &g.solid };
            for (glyph, record) in glyphs.iter().zip(&mut out) {
                write_glyph(record, glyph, page(glyph), n as u32);
            }
        }
        let seen_through = self.name_runs.iter().filter(|r| !r.discrete);
        for (number, run) in (kept as u32..).zip(seen_through) {
            let Some(g) = self
                .names
                .get(&run.id)
                .and_then(|n| n.lines[run.line].as_ref())
            else {
                continue;
            };
            for (glyph, record) in g.sheer.iter().zip(&mut out) {
                write_glyph(record, glyph, page(glyph), number);
            }
        }
        self.stats.names = kept as u32;
        self.stats.glyphs += (here + over) as u32;
        if here > 0 {
            let push = model_push([world, matrix, first, signs], fog);
            // SAFETY: the push constants are sign_vert.glsl's, the records they point at
            // were written just above, and the pages they name have arrived
            // (`add_name`).
            unsafe { pass.draw(PipelineId::NameTag.index(), &push, 6, here as u32) };
            self.stats.draws += 1;
        }
        if over > 0 {
            // No fog lies over what is seen through everything.
            let clear = Fog {
                colour: [0.0; 4],
                ..*fog
            };
            let first = first + here as u32 * GLYPH.words();
            self.over = Some((
                model_push([world, matrix, first, signs], &clear),
                over as u32,
            ));
        }
    }

    /// Draws the text that was gathered: the outlines, then the text over them.
    ///
    /// # Safety
    /// As [`Self::draw`].
    unsafe fn draw_signs(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        view: &View,
        fog: &Fog,
        lightmap: TextureId,
    ) {
        if self.runs.is_empty() {
            return;
        }
        // The nearest are drawn when not all can be.
        self.runs.sort_by(|a, b| a.away.total_cmp(&b.away));
        let record = u64::from(GLYPH.bytes());
        let room = (frame_data.capacity() - frame_data.used())
            .saturating_sub(RESERVED)
            .saturating_sub(self.runs.len() as u64 * u64::from(SIGN.bytes()))
            .min(GLYPH_BYTES)
            / record;
        let Some((world, matrix)) = write_world(frame_data, view, [1.0; 2], lightmap) else {
            return;
        };
        let (mut kept, mut glyphs) = (0, 0u64);
        for run in &self.runs {
            let count = self.side(run).map_or(0, |s| s.glyphs.len()) as u64;
            if glyphs + count > room {
                break;
            }
            glyphs += count;
            kept += 1;
        }
        self.stats.overflowed += (self.runs.len() - kept) as u64;
        self.runs.truncate(kept);
        if glyphs == 0 {
            return;
        }
        // Those with an outline first: their records are drawn twice.
        self.runs.sort_by_key(|r| !r.outlined);
        let outlined: usize = self
            .runs
            .iter()
            .take_while(|r| r.outlined)
            .filter_map(|r| Some(self.side(r)?.glyphs.len()))
            .sum();
        let words = GLYPH.words();
        let Some((signs, bytes)) = frame_data.alloc(kept as u32 * SIGN.words(), 4) else {
            return;
        };
        for (run, record) in self.runs.iter().zip(bytes.as_chunks_mut::<SIGN_BYTES>().0) {
            write_sign(record, run);
        }
        let Some((first, bytes)) = frame_data.alloc(glyphs as u32 * words, 4) else {
            return;
        };
        let textures = self.pages.textures();
        let mut records = bytes.as_chunks_mut::<GLYPH_RECORD_BYTES>().0.iter_mut();
        for (n, run) in self.runs.iter().enumerate() {
            let side = self.side(run).map_or(&[][..], |s| &s.glyphs);
            for (glyph, out) in side.iter().zip(&mut records) {
                let page = glyph[GLYPH.offset("texture") as usize] as usize;
                write_glyph(out, glyph, textures[page], n as u32);
            }
        }
        self.stats.texts = kept as u32;
        self.stats.glyphs = glyphs as u32;
        // The text lies over the outlines: it is drawn nearer.
        for (pipeline, instances) in [
            (PipelineId::SignOutline, outlined as u32 * 8),
            (PipelineId::SignText, glyphs as u32),
        ] {
            if instances == 0 {
                continue;
            }
            let push = model_push([world, matrix, first, signs], fog);
            // SAFETY: the push constants are sign_vert.glsl's, the records they point at
            // were written just above (an outline's glyph eight times over, the first
            // `outlined` of them), and the pages they name have arrived (`add_sign`).
            unsafe { pass.draw(pipeline.index(), &push, 6, instances) };
            self.stats.draws += 1;
        }
    }

    fn side(&self, run: &SignRun) -> Option<&Side> {
        self.signs.get(&run.pos)?.sides[usize::from(run.back)].as_deref()
    }
}

/// Whether a text with this transform is seen from its front, with the camera at the
/// origin.
fn faces_camera(transform: &Affine) -> bool {
    transform.0.iter().map(|row| -row[3] * row[2]).sum::<f32>() > 0.0
}

/// Lays out a side's text and makes its records.
fn lay_out(side: &mut Side, hanging: bool, font: &Font, pages: &mut GlyphPages, random: &mut u64) {
    let mut pick = |below: usize| {
        // xorshift64
        *random ^= *random << 13;
        *random ^= *random >> 7;
        *random ^= *random << 17;
        (*random % below as u64) as usize
    };
    let layout = sign::layout(&side.text, hanging, font, &mut pick);
    // What changes every frame does not wait for its pictures.
    (side.glyphs, side.newest) = sign::records(&layout.text, pages, font, layout.obfuscated);
    side.obfuscated = layout.obfuscated;
}

/// Lays out a line over a head and makes its records.
fn lay_out_name(
    glyphs: &mut NameGlyphs,
    offset: i32,
    font: &Font,
    pages: &mut GlyphPages,
    random: &mut u64,
) {
    let mut pick = |below: usize| {
        // xorshift64
        *random ^= *random << 13;
        *random ^= *random >> 7;
        *random ^= *random << 17;
        (*random % below as u64) as usize
    };
    let laid = nametag::layout(&glyphs.line, offset, font, &mut pick);
    // What changes every frame does not wait for its pictures.
    let (solid, a) = sign::records(&laid.solid, pages, font, laid.obfuscated);
    let (sheer, b) = sign::records(&laid.sheer, pages, font, laid.obfuscated);
    (glyphs.solid, glyphs.sheer) = (solid, sheer);
    glyphs.newest = a.max(b);
    glyphs.obfuscated = laid.obfuscated;
}

/// Where a block entity is from the camera, and its light.
#[derive(Clone, Copy)]
struct Placed {
    origin: [f32; 3],
    light: u32,
}

/// Bytes of a sign's and a glyph's record.
const SIGN_BYTES: usize = SIGN.bytes() as usize;
const GLYPH_RECORD_BYTES: usize = GLYPH.bytes() as usize;

fn write_sign(bytes: &mut [u8; SIGN_BYTES], run: &SignRun) {
    write_sign_record(bytes, &run.transform, run.light, run.outline);
}

/// A text's record: where it is, its light and the colour of what surrounds its glyphs.
fn write_sign_record(bytes: &mut [u8; SIGN_BYTES], transform: &Affine, light: u32, outline: u32) {
    for (i, c) in transform.0.as_flattened().iter().enumerate() {
        let at = SIGN.offset("row_x") as usize + i;
        bytes[at * 4..at * 4 + 4].copy_from_slice(&c.to_ne_bytes());
    }
    for (field, word) in [("light", light), ("outline", outline)] {
        let at = SIGN.offset(field) as usize * 4;
        bytes[at..at + 4].copy_from_slice(&word.to_ne_bytes());
    }
}

/// Bytes of the record of a shadow's piece.
const SHADOW_BYTES: usize = SHADOW.bytes() as usize;

fn write_shadow(bytes: &mut [u8; SHADOW_BYTES], s: &ShadowRun) {
    let mut put = |field: &str, values: &[f32]| {
        let at = SHADOW.offset(field) as usize * 4;
        for (i, v) in values.iter().enumerate() {
            bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
        }
    };
    put("origin", &s.origin);
    put("radius", &[s.radius]);
    put("box", &s.piece.edges);
    put("height", &[s.piece.height]);
    put("alpha", &[s.piece.alpha]);
}

fn write_glyph(
    bytes: &mut [u8; GLYPH_RECORD_BYTES],
    record: &[u32; GLYPH_WORDS],
    texture: u32,
    sign: u32,
) {
    const TEXTURE: usize = GLYPH.offset("texture") as usize;
    const SIGN_NUMBER: usize = GLYPH.offset("sign") as usize;
    let mut words = *record;
    words[TEXTURE] = texture;
    words[SIGN_NUMBER] = sign;
    for (out, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *out = word.to_ne_bytes();
    }
}

/// Bytes of one part's record.
const PART_BYTES: usize = MODEL_PART.bytes() as usize;

/// The record of a glint: its picture, its strength and where the picture is read.
fn write_glint(bytes: &mut [u8], picture: TextureId, (row_u, row_v): ([f32; 3], [f32; 3])) {
    let mut put = |field: &str, words: &[[u8; 4]]| {
        let at = GLINT.offset(field) as usize * 4;
        for (i, w) in words.iter().enumerate() {
            bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(w);
        }
    };
    put("texture", &[picture.index().to_ne_bytes()]);
    put("strength", &[GLINT_STRENGTH.to_ne_bytes()]);
    put("row_u", &row_u.map(f32::to_ne_bytes));
    put("row_v", &row_v.map(f32::to_ne_bytes));
}

/// A part's record; `glints` are the word offsets of the records of the glints over
/// armour, over items' own models and over the game's models of items, 0 for one that is
/// not there.
fn write_part(bytes: &mut [u8], i: &Instance, glints: [u32; 3]) {
    let mut put = |word: usize, value: [u8; 4]| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value);
    };
    let rows = MODEL_PART.offset("row_x") as usize;
    for (n, c) in i.transform.0.as_flattened().iter().enumerate() {
        put(rows + n, c.to_ne_bytes());
    }
    let glint = match i.glint {
        Glint::None => 0,
        Glint::Armour => glints[0],
        Glint::Item => glints[1],
        Glint::Entity => glints[2],
    };
    put(
        MODEL_PART.offset("overlay") as usize,
        (u32::from(i.hurt) | glint << 8).to_ne_bytes(),
    );
    put(MODEL_PART.offset("light") as usize, i.light.to_ne_bytes());
    put(MODEL_PART.offset("colour") as usize, i.colour.to_ne_bytes());
    put(
        MODEL_PART.offset("texture") as usize,
        i.texture.index().to_ne_bytes(),
    );
}

/// The push constants of a frame's models, or of its signs' text: the four words the
/// vertex stage reads, and the fog.
pub(crate) fn model_push(words: [u32; 4], fog: &Fog) -> [u8; MODEL_PUSH_BYTES] {
    let mut push = [0u8; MODEL_PUSH_BYTES];
    let mut put = |at: usize, word: [u8; 4]| push[at..at + 4].copy_from_slice(&word);
    for (i, word) in words.into_iter().enumerate() {
        put(i * 4, word.to_ne_bytes());
    }
    for (i, c) in fog.colour.iter().enumerate() {
        put(PUSH_FOG_COLOUR + i * 4, c.to_ne_bytes());
    }
    for i in 0..2 {
        put(
            PUSH_FOG_ENVIRONMENTAL + i * 4,
            fog.environmental[i].to_ne_bytes(),
        );
        put(
            PUSH_FOG_RENDER_DISTANCE + i * 4,
            fog.render_distance[i].to_ne_bytes(),
        );
    }
    push
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_texts_shadow_is_a_quarter_as_bright() {
        assert_eq!(shadow_colour(0xFFFF_FFFF), 0xFF3F_3F3F);
        assert_eq!(shadow_colour(0x80FF_8004), 0x803F_2001);
        assert_eq!(shadow_colour(0xFF00_0000), 0xFF00_0000);
    }

    #[test]
    fn colours_and_lights_are_packed_as_the_shader_unpacks_them() {
        // Orange: red first, in the low byte.
        assert_eq!(
            record_colour(0xF9801D).to_le_bytes(),
            [0xF9, 0x80, 0x1D, 0xFF]
        );
        assert_eq!(record_light(15, 7) & 0xFF, 240);
        assert_eq!(record_light(15, 7) >> 8, 112);
    }

    #[test]
    fn a_part_is_written_where_the_shader_reads_it() {
        let i = Instance {
            batch: (SOLID, 0, 0),
            quads: 1,
            transform: Affine(std::array::from_fn(|r| {
                std::array::from_fn(|c| (r * 4 + c) as f32)
            })),
            light: 0x70F0,
            colour: 0xFF12_3456,
            texture: TextureId::MISSING,
            hurt: true,
            glint: Glint::Item,
            source: 0,
        };
        let mut bytes = [0xAAu8; PART_BYTES];
        write_part(&mut bytes, &i, [5, 9, 11]);
        let word = |n: usize| u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap());
        for n in 0..12 {
            assert_eq!(f32::from_bits(word(n)), n as f32);
        }
        assert_eq!((word(12), word(13)), (0x70F0, 0xFF12_3456));
        assert_eq!(word(14), TextureId::MISSING.index());
        // Hurt, and under the glint of items.
        assert_eq!(word(15), 1 | 9 << 8);
    }

    #[test]
    fn a_sign_and_its_glyphs_are_written_where_the_shader_reads_them() {
        let run = SignRun {
            pos: [0; 3],
            back: false,
            outlined: false,
            outline: 0xFF01_0203,
            transform: Affine(std::array::from_fn(|r| {
                std::array::from_fn(|c| (r * 4 + c) as f32)
            })),
            light: 0x70F0,
            away: 0.0,
        };
        let mut bytes = [0xAAu8; SIGN_BYTES];
        write_sign(&mut bytes, &run);
        let word = |b: &[u8], n: usize| u32::from_ne_bytes(b[n * 4..n * 4 + 4].try_into().unwrap());
        for n in 0..12 {
            assert_eq!(f32::from_bits(word(&bytes, n)), n as f32);
        }
        assert_eq!((word(&bytes, 12), word(&bytes, 13)), (0x70F0, 0xFF01_0203));
        // The page's number becomes its texture's, and the sign's is put in.
        let record: [u32; GLYPH_WORDS] =
            std::array::from_fn(|i| if i == 10 { 0 } else { i as u32 });
        let mut out = [0xAAu8; GLYPH_RECORD_BYTES];
        write_glyph(&mut out, &record, 9, 77);
        for n in 0..10 {
            assert_eq!(word(&out, n), n as u32);
        }
        assert_eq!((word(&out, 10), word(&out, 12)), (9, 12));
        assert_eq!(word(&out, 11), 77);
    }

    #[test]
    fn an_item_is_placed_in_a_hand_as_its_model_says() {
        let transform = ItemTransform {
            rotation: [0.0, -90.0, 55.0],
            translation: [0.25, 0.5, 0.0],
            scale: [2.0; 3],
        };
        let hand = Affine::IDENTITY;
        // Without a say: about its middle only.
        assert_eq!(place_item(hand, None, false).point([0.5; 3]), [0.0; 3]);
        // Its middle goes where it is moved to, whatever its turn and size.
        let right = place_item(hand, Some(transform), false);
        let left = place_item(hand, Some(transform), true);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(right.point([0.5; 3]), [0.25, 0.5, 0.0]));
        assert!(close(left.point([0.5; 3]), [-0.25, 0.5, 0.0]));
        // A left hand's is the right hand's seen in a mirror: a point of the model on
        // the other side of its middle lands across.
        let (p, mirrored) = ([0.75, 0.25, 0.625], [0.25, 0.25, 0.625]);
        let (r, l) = (right.point(p), left.point(mirrored));
        assert!(close(l, [-r[0], r[1], r[2]]), "{r:?} {l:?}");
        // Turned about its middle and twice as large: a quarter turn about y takes what
        // lies along the model's x to z.
        let turned = ItemTransform {
            rotation: [0.0, -90.0, 0.0],
            ..transform
        };
        let along_x = place_item(hand, Some(turned), false).point([1.0, 0.5, 0.5]);
        assert!(close(along_x, [0.25, 0.5, 1.0]), "{along_x:?}");
    }

    #[test]
    fn tints_colour_by_the_stack() {
        let tints = [
            Tint::Constant(1),
            Tint::Dye(2),
            Tint::Potion(3),
            Tint::Default(4),
        ];
        let plain = Components::default();
        let colours = |c: &Components| [-1, 0, 1, 2, 3, 4].map(|i| tint_colour(&tints, i, c));
        assert_eq!(colours(&plain), [0xFF_FFFF, 1, 2, 3, 4, 0xFF_FFFF]);
        let dyed = Components {
            dyed_color: Some(0x12_3456),
            potion_color: Some(0x65_4321),
            ..Components::default()
        };
        assert_eq!(
            colours(&dyed),
            [0xFF_FFFF, 1, 0x12_3456, 0x65_4321, 4, 0xFF_FFFF]
        );
    }

    #[test]
    fn an_items_model_is_written_as_the_models_are() {
        let mesh = ItemMesh {
            quads: vec![ruthenium_assets::item::ItemQuad {
                positions: [
                    [0.0, 1.0, 2.0],
                    [3.0, 4.0, 5.0],
                    [6.0, 7.0, 8.0],
                    [9.0, 10.0, 11.0],
                ],
                uvs: [[0.25, 0.5], [0.75, 1.0], [0.0, 0.0], [1.0, 1.0]],
                direction: 0,
            }],
            ..ItemMesh::default()
        };
        let bytes = item_vertices(&mesh);
        assert_eq!(bytes.len(), 4 * VERTEX_BYTES);
        let second = &bytes[VERTEX_BYTES..2 * VERTEX_BYTES];
        let float = |n: usize| f32::from_le_bytes(second[n * 4..n * 4 + 4].try_into().unwrap());
        assert_eq!([0, 1, 2, 3, 4].map(float), [3.0, 4.0, 5.0, 0.75, 1.0]);
        // Facing down.
        assert_eq!(second[20..], [0, 0x81, 0, 0]);
    }

    #[test]
    fn the_glint_moves_and_is_turned_and_scaled() {
        let (u, v) = glint_rows(0, 8.0);
        let (sin, cos) = 0.174_532_92_f32.sin_cos();
        assert_eq!(u, [cos * 8.0, -sin * 8.0, 0.0]);
        assert_eq!(v, [sin * 8.0, cos * 8.0, 0.0]);
        // Four times as fast as the clock: along once in 27.5 s, up once in 7.5.
        let (u, v) = glint_rows(13_750, 1.0);
        assert_eq!(u[2], -0.5);
        assert!((v[2] - 25_000.0 / 30_000.0).abs() < 1e-6);
        // Armour's from its own picture, the other two from the items'.
        assert_eq!(GLINTS.map(|g| g.0), [0, 1, 1]);
    }

    #[test]
    fn a_glint_is_written_where_the_shader_reads_it() {
        let mut bytes = vec![0xAAu8; GLINT.bytes() as usize];
        let rows = ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]);
        write_glint(&mut bytes, TextureId::MISSING, rows);
        let word = |n: usize| u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap());
        // Its picture, how strong it is, then where it is read along and up.
        assert_eq!(word(0), TextureId::MISSING.index());
        assert_eq!(f32::from_bits(word(1)), GLINT_STRENGTH);
        let rest = [2, 3, 4, 5, 6, 7].map(|n| f32::from_bits(word(n)));
        assert_eq!(rest, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn a_shadows_piece_is_written_where_the_shader_reads_it() {
        let run = ShadowRun {
            origin: [1.0, 2.0, 3.0],
            radius: 4.0,
            piece: Piece {
                edges: [5.0, 6.0, 7.0, 8.0],
                height: 9.0,
                alpha: 10.0,
            },
        };
        let mut bytes = [0xAAu8; SHADOW_BYTES];
        write_shadow(&mut bytes, &run);
        // The origin, the radius, the edges, the height and the alpha, a word each.
        for (n, word) in bytes.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(f32::from_ne_bytes(*word), (n + 1) as f32);
        }
        assert_eq!(SHADOW_BYTES, 40);
    }

    #[test]
    fn a_name_is_written_as_a_text_in_no_light_or_in_its_own() {
        let transform = Affine(std::array::from_fn(|r| {
            std::array::from_fn(|c| (r * 4 + c) as f32)
        }));
        let mut bytes = [0xAAu8; SIGN_BYTES];
        write_sign_record(&mut bytes, &transform, UNLIT, 0);
        let word = |n: usize| u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap());
        assert_eq!((word(12), word(13)), (0x8000_0000, 0));
        // No light a block can have sets that bit.
        assert_eq!(record_light(15, 15) & UNLIT, 0);
        let glsl = include_str!("../../../ruthenium-shaders/glsl/include/sign_vert.glsl");
        assert!(glsl.contains("const uint UNLIT = 0x80000000u;"));
    }

    #[test]
    fn the_seen_through_are_drawn_from_back_to_front() {
        let part = |over: u32| Instance {
            batch: (SKIN_SORTED, over, 7 * over),
            quads: 6,
            transform: Affine::IDENTITY,
            light: 0,
            colour: WHITE,
            texture: TextureId::MISSING,
            hurt: false,
            glint: Glint::None,
            source: 0,
        };
        let mut instances = vec![part(0), part(1), part(0), part(1), part(0), part(1)];
        let mut sorted = vec![(4.0, 0..2), (9.0, 2..4), (1.0, 4..6)];
        order(&mut sorted, &mut instances);
        let places: Vec<u32> = instances.iter().map(|i| i.batch.1).collect();
        assert_eq!(places, [2, 3, 0, 1, 4, 5]);
    }

    #[test]
    fn skins_have_their_layers() {
        let look = |blended| Look {
            body: TextureId::MISSING,
            cape: None,
            slim: false,
            blended,
        };
        assert_eq!(skin_layer(&look(false)), SKIN);
        assert_eq!(skin_layer(&look(true)), SKIN_SORTED);
        let head = Kind::Skull {
            skull: Skull::Player,
            wall: false,
        };
        assert!(is_nearer(head, CUTOUT_FACES) && !is_nearer(head, SKIN));
        assert!(!is_nearer(Kind::Shulker(None), CUTOUT_FACES));
    }

    #[test]
    fn a_text_is_drawn_from_its_front_only() {
        let i = Affine::IDENTITY;
        // Facing the positive z, in front of the camera and behind it.
        assert!(!faces_camera(&i.translate([0.0, 0.0, 5.0])));
        assert!(faces_camera(&i.translate([0.0, 0.0, -5.0])));
        assert!(faces_camera(
            &i.translate([3.0, 1.0, -5.0]).scale([0.01, -0.01, 0.01])
        ));
        let back = i.translate([0.0, 0.0, -5.0]).rotate_y(std::f32::consts::PI);
        assert!(!faces_camera(&back));
    }

    #[test]
    fn patterns_are_read_from_a_banners_data() {
        let mut first = nbt::Compound::new();
        first.put("pattern", nbt::Tag::String("minecraft:creeper".into()));
        first.put("color", nbt::Tag::String("red".into()));
        let mut own = nbt::Compound::new();
        own.put("asset_id", nbt::Tag::String("minecraft:globe".into()));
        let mut second = nbt::Compound::new();
        second.put("pattern", nbt::Tag::Compound(own));
        second.put("color", nbt::Tag::String("black".into()));
        let mut broken = nbt::Compound::new();
        broken.put("pattern", nbt::Tag::String("minecraft:flow".into()));
        let mut banner = nbt::Compound::new();
        banner.put(
            "patterns",
            nbt::Tag::List(
                [first, second, broken]
                    .into_iter()
                    .map(nbt::Tag::Compound)
                    .collect(),
            ),
        );
        let tag = nbt::Tag::Compound(banner);
        let read: Vec<_> = listed_patterns(&tag).collect();
        assert_eq!(read, [("minecraft:creeper", 14), ("minecraft:globe", 15)]);
        assert_eq!(pattern_texture("minecraft:creeper"), "banner/creeper");
    }

    #[test]
    fn christmas_is_three_days_in_december() {
        let day =
            |days: u64| is_christmas(UNIX_EPOCH + std::time::Duration::from_secs(days * 86_400));
        // 23 to 27 December 2026, and 24 December of a leap year.
        let of_2026 = 20_810;
        assert_eq!(
            [0, 1, 2, 3, 4].map(|d| day(of_2026 + d)),
            [false, true, true, true, false]
        );
        assert!(day(20_081) && !day(20_080));
        assert!(!day(0));
    }

    #[test]
    fn every_chest_has_its_pictures() {
        let paths = texture_paths();
        for p in [
            "chest/normal",
            "chest/trapped_left",
            "chest/christmas_right",
            "chest/copper_oxidized_left",
            "chest/ender",
            "shulker/shulker_light_gray",
            "player/slim/steve",
        ] {
            assert!(paths.iter().any(|q| q == p), "{p}");
        }
        assert!(!paths.iter().any(|p| p == "chest/ender_left"));
        // 22 chests, 17 shulker boxes, 7 skulls, 18 skins (one of which a skull has), and
        // the trident.
        assert_eq!(paths.len(), 22 + 17 + 7 + 17 + 1);
        assert!(paths.iter().any(|p| p == TRIDENT));
    }
}
