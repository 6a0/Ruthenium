//! Draws block entities: instances of model parts, gathered each frame from the sections in
//! view, a draw for all instances of one part in one layer; the text of signs, a draw for
//! all glyphs; and players, from the same parts as their heads.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ruthenium_assets::Font;
use ruthenium_gpu::{
    Arrival, BufferPool, Error, FrameData, Gpu, Pass, PoolConfig, RangeHandle, TextureDesc,
    TextureId, TextureTable, UploadRing, UploadTicket, VertexAttribute, VertexInput, vk,
};
use ruthenium_protocol::nbt;
use ruthenium_protocol::profile::ResolvableProfile;
use ruthenium_shaders::interface::{GLYPH, MODEL_PART, SIGN};
use ruthenium_sim::avatar::{DEFAULT_SKINS, SkinSource};
use ruthenium_sim::environment::CardinalLight;
use ruthenium_sim::scene::{Frame, SectionKey, Terrain};

use super::affine::Affine;
use super::avatar::{self, Figure, Moment};
use super::model::{Model, ModelSet, PartDraw, Pose, Skull};
use super::place::{self, Chest, DYE_COLOURS, DYES, Kind};
use super::sign::{self, GLYPH_WORDS, GlyphPages, SignText};
use super::skins::{LoadedSkin, Look, Skins, default_path};
use crate::atmosphere::Fog;
use crate::pipelines::PipelineId;
use crate::sky::SkyImage;
use crate::terrain::View;
use crate::terrain::table::BlockTable;
use crate::terrain::world::write_world;

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
/// How near the player has to be for glowing text to have its outline, in blocks.
const OUTLINE_DISTANCE: f64 = 16.0;
/// Bytes of the frame's data left for what is drawn after block entities.
const RESERVED: u64 = 512 << 10;
/// Bytes of it glyphs take at most.
const GLYPH_BYTES: u64 = 2 << 20;

/// A colour that leaves the texture as it is.
const WHITE: u32 = u32::MAX;
/// What an invisible player is tinted with for those who see it.
const INVISIBLE: u32 = 0x26FF_FFFF;

/// What block entities are drawn from: the models, and the pictures by their path below
/// `textures/entity` without the suffix.
#[derive(Debug)]
pub struct BlockEntityAssets {
    pub models: ModelSet,
    pub textures: Vec<(String, SkyImage)>,
    /// What signs are written in.
    pub font: Font,
    pub lang: HashMap<String, String>,
}

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
    /// Sides of signs drawn, and their glyphs.
    pub texts: u32,
    pub glyphs: u32,
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
const LAYERS: [PipelineId; 7] = [
    PipelineId::ModelSolid,
    PipelineId::ModelCutout,
    PipelineId::ModelCutoutFaces,
    PipelineId::ModelPattern,
    PipelineId::ModelTranslucent,
    PipelineId::ModelTranslucent,
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
    /// What is drawn from back to front: how far it is, and where its instances are.
    sorted: Vec<(f32, std::ops::Range<usize>)>,
    runs: Vec<SignRun>,
    /// State of the numbers obfuscated text is picked by.
    random: u64,
    instances: Vec<Instance>,
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
            sorted: Vec::new(),
            runs: Vec::new(),
            random: 0x9E37_79B9_7F4A_7C15,
            instances: Vec::new(),
            parts: Vec::new(),
            textures: ids,
            christmas: is_christmas(SystemTime::now()),
            stats: BlockEntityStats::default(),
        })
    }

    pub fn stats(&self) -> BlockEntityStats {
        self.stats
    }

    /// Stages the models, until that has worked once, and the glyphs that wait for it:
    /// before the frame's `UploadRing::record`.
    pub fn stage(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) {
        self.pages.stage(textures, uploads);
        self.skins.stage(textures, uploads);
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
        self.skins.begin();
        self.runs.clear();
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
                    if skull == Skull::Player
                        && let Some(owner) = self.owner(terrain, pos)
                    {
                        let Some(look) = self.skins.look(&owner, textures) else {
                            continue;
                        };
                        let first = self.instances.len();
                        let layer = skin_layer(&look);
                        self.add(kind, state, model, still, place, layer, look.body, WHITE);
                        self.sort(first, layer);
                        self.stats.drawn += 1;
                        continue;
                    }
                    let i = SKULLS.iter().position(|s| *s == skull).unwrap_or(0);
                    let Some(texture) = usable(self.skulls[i]) else {
                        continue;
                    };
                    self.add(
                        kind,
                        state,
                        model,
                        still,
                        place,
                        CUTOUT_FACES,
                        texture,
                        WHITE,
                    );
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
            self.stats.drawn += 1;
        }
    }

    /// Adds the players and mannequins that are in view and in a section that is shown.
    #[allow(clippy::too_many_arguments)]
    pub fn gather_players(
        &mut self,
        frame: &Frame,
        terrain: &Terrain,
        view_distance: i32,
        partial: f32,
        camera: [f64; 3],
        view: &View,
        textures: &TextureTable,
        shown: impl Fn([i32; 3]) -> bool,
    ) {
        // `Entity.getViewScale`.
        let scale = (f64::from(view_distance) / 8.0).clamp(1.0, 2.5);
        for e in &frame.entities {
            let Some(r) = e.avatar.as_deref().filter(|r| !r.hidden) else {
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
            let light = record_light(if r.on_fire { 15 } else { light.0 }, light.1);
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
            let first = self.instances.len();
            self.parts.clear();
            self.models
                .draw_by(model, &mut self.parts, |name, m| figure.place(name, m));
            for p in &self.parts {
                self.instances.push(Instance {
                    batch: (layer, u32::from(p.over), p.first_quad),
                    quads: p.quads,
                    transform: figure.transform.then_inner(p.transform),
                    light,
                    colour,
                    texture: look.body,
                    hurt: r.hurt,
                });
            }
            self.sort(first, layer);
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
                    });
                }
            }
            self.stats.players += 1;
        }
    }

    /// Notes that the instances from `first` on are drawn from back to front with what
    /// else is, if their layer is.
    fn sort(&mut self, first: usize, layer: u8) {
        if matches!(layer, SKIN_SORTED | SKIN_INVISIBLE)
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
            });
        }
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
                cardinal,
                lightmap,
                indices,
                (range.buffer, range.offset),
            )
        };
        // Over the boards the mesher drew.
        // SAFETY: the caller's.
        unsafe { self.draw_signs(pass, frame_data, view, fog, lightmap) };
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
        cardinal: CardinalLight,
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
        self.instances.sort_by_key(|i| i.batch);
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
        let Some((first, bytes)) = frame_data.alloc(count as u32 * MODEL_PART.words(), 4) else {
            self.stats.overflowed += count as u64;
            return;
        };
        for (i, record) in self.instances[..count]
            .iter()
            .zip(bytes.as_chunks_mut::<PART_BYTES>().0)
        {
            write_part(record, i);
        }
        let lights = match cardinal {
            CardinalLight::Default => 1,
            CardinalLight::Nether => 2,
        };
        let push = model_push([world, matrix, first, lights], fog);
        self.stats.parts = count as u32;
        let mut pushed = false;
        let mut at = 0;
        // SAFETY: the push constants are model.vert's and the records they point at were
        // written just above; the vertices' copy can be read (`ready`), every draw reads
        // a part's own quads, and no part has more than the shared indices reach; the
        // textures are of the table and have arrived (`gather`).
        unsafe {
            pass.bind_vertex_buffer(vertices.0, vertices.1);
            pass.bind_index_buffer(indices.0, indices.1);
            while at < count {
                let Instance { batch, quads, .. } = self.instances[at];
                let run = self.instances[at..count]
                    .iter()
                    .take_while(|i| i.batch == batch)
                    .count();
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
    for (i, c) in run.transform.0.as_flattened().iter().enumerate() {
        let at = SIGN.offset("row_x") as usize + i;
        bytes[at * 4..at * 4 + 4].copy_from_slice(&c.to_ne_bytes());
    }
    for (field, word) in [("light", run.light), ("outline", run.outline)] {
        let at = SIGN.offset(field) as usize * 4;
        bytes[at..at + 4].copy_from_slice(&word.to_ne_bytes());
    }
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

fn write_part(bytes: &mut [u8], i: &Instance) {
    let mut put = |word: usize, value: [u8; 4]| {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value);
    };
    let rows = MODEL_PART.offset("row_x") as usize;
    for (n, c) in i.transform.0.as_flattened().iter().enumerate() {
        put(rows + n, c.to_ne_bytes());
    }
    put(
        MODEL_PART.offset("overlay") as usize,
        u32::from(i.hurt).to_ne_bytes(),
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
fn model_push(words: [u32; 4], fog: &Fog) -> [u8; MODEL_PUSH_BYTES] {
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
        };
        let mut bytes = [0xAAu8; PART_BYTES];
        write_part(&mut bytes, &i);
        let word = |n: usize| u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap());
        for n in 0..12 {
            assert_eq!(f32::from_bits(word(n)), n as f32);
        }
        assert_eq!((word(12), word(13)), (0x70F0, 0xFF12_3456));
        assert_eq!(word(14), TextureId::MISSING.index());
        assert_eq!(word(15), 1);
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
    fn the_seen_through_are_drawn_from_back_to_front() {
        let part = |over: u32| Instance {
            batch: (SKIN_SORTED, over, 7 * over),
            quads: 6,
            transform: Affine::IDENTITY,
            light: 0,
            colour: WHITE,
            texture: TextureId::MISSING,
            hurt: false,
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
        // 22 chests, 17 shulker boxes, 7 skulls, 18 skins (one of which a skull has).
        assert_eq!(paths.len(), 22 + 17 + 7 + 17);
    }
}
