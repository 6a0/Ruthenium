//! Draws block entities: instances of model parts, gathered each frame from the sections in
//! view, a draw for all instances of one part in one layer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ruthenium_gpu::{
    Arrival, BufferPool, Error, FrameData, Gpu, Pass, PoolConfig, RangeHandle, TextureDesc,
    TextureId, TextureTable, UploadRing, UploadTicket, VertexAttribute, VertexInput, vk,
};
use ruthenium_protocol::nbt;
use ruthenium_shaders::interface::MODEL_PART;
use ruthenium_sim::environment::CardinalLight;
use ruthenium_sim::scene::{Frame, SectionKey, Terrain};

use super::affine::Affine;
use super::model::{Model, ModelSet, PartDraw, Pose, Skull};
use super::place::{self, Chest, DYE_COLOURS, DYES, Kind};
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
/// Banners whose patterns are kept between frames.
const KEPT_PATTERNS: usize = 4096;

/// A colour that leaves the texture as it is.
const WHITE: u32 = u32::MAX;

/// What block entities are drawn from: the models, and the pictures by their path below
/// `textures/entity` without the suffix.
#[derive(Debug)]
pub struct BlockEntityAssets {
    pub models: ModelSet,
    pub textures: Vec<(String, SkyImage)>,
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
    /// Parts there was no room for, in all frames.
    pub overflowed: u64,
}

/// One part to draw: its record, and what it is sorted into draws by.
struct Instance {
    /// The layer, then the part's first quad.
    batch: (u8, u32),
    quads: u32,
    transform: Affine,
    light: u32,
    colour: u32,
    texture: TextureId,
}

/// The layers, in the order they are drawn in: patterns lie over the cloth they are on.
const SOLID: u8 = 0;
const CUTOUT: u8 = 1;
const CUTOUT_FACES: u8 = 2;
const PATTERN: u8 = 3;
const LAYERS: [PipelineId; 4] = [
    PipelineId::ModelSolid,
    PipelineId::ModelCutout,
    PipelineId::ModelCutoutFaces,
    PipelineId::ModelPattern,
];

/// A banner's patterns as last read from its data: their textures and colours.
struct Patterns {
    tag: Arc<nbt::Tag>,
    layers: Vec<(TextureId, u32)>,
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
fn record_colour(rgb: u32) -> u32 {
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
            textures: ids,
            chests,
            shulkers,
            skulls,
            resting: HashMap::new(),
            patterns: HashMap::new(),
            instances: Vec::new(),
            parts: Vec::new(),
            christmas: is_christmas(SystemTime::now()),
            stats: BlockEntityStats::default(),
        })
    }

    pub fn stats(&self) -> BlockEntityStats {
        self.stats
    }

    /// Stages the models, until that has worked once: before the frame's
    /// `UploadRing::record`.
    pub fn stage(&mut self, uploads: &mut UploadRing) {
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

    /// Starts a frame's gathering.
    pub fn begin(&mut self) {
        self.instances.clear();
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
                    let i = SKULLS.iter().position(|s| *s == skull).unwrap_or(0);
                    let Some(texture) = usable(self.skulls[i]) else {
                        continue;
                    };
                    self.add(
                        kind,
                        state,
                        Model::Skull(skull),
                        Pose::Skull(0.0),
                        place,
                        CUTOUT_FACES,
                        texture,
                        WHITE,
                    );
                }
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
        let nearer = matches!(kind, Kind::Skull { .. });
        for p in &self.parts {
            let mut transform = p.transform;
            for (row, origin) in transform.0.iter_mut().zip(place.origin) {
                row[3] += origin;
                if nearer {
                    *row = row.map(|c| c * TOWARDS_CAMERA);
                }
            }
            self.instances.push(Instance {
                batch: (layer, p.first_quad),
                quads: p.quads,
                transform,
                light: place.light,
                colour,
                texture,
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
        if self.instances.is_empty() {
            return;
        }
        // Stable: the patterns of a banner stay in their order.
        self.instances.sort_by_key(|i| i.batch);
        let Some((world, matrix)) = write_world(frame_data, view, [1.0; 2], lightmap) else {
            self.stats.overflowed += self.instances.len() as u64;
            return;
        };
        let room = (frame_data.capacity() - frame_data.used()) / u64::from(MODEL_PART.bytes());
        let count = self.instances.len().min(room.saturating_sub(1) as usize);
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
        let push = model_push(world, matrix, first, fog, cardinal);
        self.stats.parts = count as u32;
        let mut pushed = false;
        let mut at = 0;
        // SAFETY: the push constants are model.vert's and the records they point at were
        // written just above; the vertices' copy can be read (`ready`), every draw reads
        // a part's own quads, and no part has more than the shared indices reach; the
        // textures are of the table and have arrived (`gather`).
        unsafe {
            pass.bind_vertex_buffer(range.buffer, range.offset);
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
                    (batch.1 * 4) as i32,
                    at as u32,
                    run as u32,
                );
                pushed = true;
                self.stats.draws += 1;
                at += run;
            }
        }
    }
}

/// Where a block entity is from the camera, and its light.
#[derive(Clone, Copy)]
struct Placed {
    origin: [f32; 3],
    light: u32,
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
    put(MODEL_PART.offset("light") as usize, i.light.to_ne_bytes());
    put(MODEL_PART.offset("colour") as usize, i.colour.to_ne_bytes());
    put(
        MODEL_PART.offset("texture") as usize,
        i.texture.index().to_ne_bytes(),
    );
}

/// The models' push constants for a frame's draws.
fn model_push(
    world: u32,
    matrix: u32,
    parts: u32,
    fog: &Fog,
    cardinal: CardinalLight,
) -> [u8; MODEL_PUSH_BYTES] {
    let mut push = [0u8; MODEL_PUSH_BYTES];
    let mut put = |at: usize, word: [u8; 4]| push[at..at + 4].copy_from_slice(&word);
    let lights: u32 = match cardinal {
        CardinalLight::Default => 1,
        CardinalLight::Nether => 2,
    };
    for (i, word) in [world, matrix, parts, lights].into_iter().enumerate() {
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
            batch: (SOLID, 0),
            quads: 1,
            transform: Affine(std::array::from_fn(|r| {
                std::array::from_fn(|c| (r * 4 + c) as f32)
            })),
            light: 0x70F0,
            colour: 0xFF12_3456,
            texture: TextureId::MISSING,
        };
        let mut bytes = [0xAAu8; PART_BYTES];
        write_part(&mut bytes, &i);
        let word = |n: usize| u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap());
        for n in 0..12 {
            assert_eq!(f32::from_bits(word(n)), n as f32);
        }
        assert_eq!((word(12), word(13)), (0x70F0, 0xFF12_3456));
        assert_eq!(word(14), TextureId::MISSING.index());
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
        // 22 chests, 17 shulker boxes, 7 skulls.
        assert_eq!(paths.len(), 22 + 17 + 7);
    }
}
