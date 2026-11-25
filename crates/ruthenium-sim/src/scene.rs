//! What the game thread publishes for drawing, and how a reader takes it.
//!
//! The game thread is the only writer of the level. After each of its frames it
//! publishes two things here ([`ScenePublisher`]):
//!
//! - **The terrain**: for every chunk that changed, its [`Column`] as it is now. A
//!   column is a list of shared planes, so publishing one copies no block or light
//!   data, and a plane nobody changed keeps its identity from one publication to the
//!   next (a light update leaves the block plane's 8 KiB array where it was). The cost
//!   of a publication follows what changed, not what is loaded.
//! - **The frame**: the player, the other entities, the clocks and the environment as
//!   the frame's last tick left them, with the number of the last input the game thread
//!   had taken in by then.
//!
//! A reader ([`Scene`], any thread) always takes the latest of both and never waits for
//! the game thread: if it falls behind, it skips to the newest.
//!
//! **Latest state and ordered changes are separate.** The terrain says what is. Beside
//! it runs a bounded queue of *dirty keys*: which sections (or whole chunks) changed
//! and how ([`dirty`]). Keys coalesce: a section changed twice is in the queue once,
//! with both reasons. When more keys arrive than the queue holds, it is emptied and a
//! [`Rescan`] is left in their place: the reader then compares what it has against the
//! terrain's stamps, which are the authority. So a reader that misses any number of
//! notifications still finds every chunk that left (no geometry outlives its chunk) and
//! every plane that is newer than what it used.
//!
//! **Identity.** Each level has an *epoch* (a new one for every dimension the player
//! enters), each loaded chunk a *generation* (the stamp of its load: a chunk that left
//! and came back at the same place has another), each plane the stamp of its last
//! change. A mesh job pins the chunks around its section ([`Scene::pin`]), which gives
//! it planes that never change under it and a [`Stamp`] of everything it may read,
//! missing neighbours included; [`Scene::is_current`] says whether a result made from
//! them still describes the level.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::environment::Environment;
use crate::player::Pose;
use crate::world::{BlockPlane, Column, Level, LightPlane};

/// Why a section (or a chunk) is in the dirty queue. Flags add up while a key waits.
pub mod dirty {
    /// The section's block states changed.
    pub const BLOCKS: u8 = 1;
    /// Its sky or block light changed.
    pub const LIGHT: u8 = 1 << 1;
    /// Its biomes changed.
    pub const BIOMES: u8 = 1 << 2;
    /// The data of a block entity in it changed, or a block entity went.
    pub const BLOCK_ENTITIES: u8 = 1 << 3;
    /// (A chunk.) It was loaded, or replaced by a new load.
    pub const LOADED: u8 = 1 << 4;
    /// (A chunk.) It was unloaded. With [`LOADED`] too, both happened since the key
    /// was queued; the terrain says which came last.
    pub const UNLOADED: u8 = 1 << 5;
    /// Something next to it changed: a block or light within its reach, or a
    /// neighbouring chunk that came or went.
    pub const NEIGHBOUR: u8 = 1 << 6;
    /// The change is the player's own (a predicted action, or the server correcting
    /// one): draw it before anything else.
    pub const URGENT: u8 = 1 << 7;
}

/// A section by its section coordinates, or a whole chunk when `y` is
/// [`WHOLE_COLUMN`].
pub type SectionKey = [i32; 3];

/// The `y` of a [`SectionKey`] that stands for a whole chunk.
pub const WHOLE_COLUMN: i32 = i32::MIN;

/// What changed in a level between two publications (`Level::take_changes`).
#[derive(Debug, Clone)]
pub struct Changes {
    columns: HashSet<(i32, i32)>,
    dirty: HashMap<SectionKey, u8>,
    capacity: usize,
    overflowed: bool,
}

impl Changes {
    pub fn new(capacity: usize) -> Changes {
        Changes {
            columns: HashSet::new(),
            dirty: HashMap::new(),
            capacity: capacity.max(1),
            overflowed: false,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.dirty.is_empty() && !self.overflowed
    }

    /// The chunks whose published copy is out of date (changed, loaded or unloaded).
    pub fn columns(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.columns.iter().copied()
    }

    /// The dirty keys and their flags, in no order.
    pub fn dirty(&self) -> impl Iterator<Item = (SectionKey, u8)> + '_ {
        self.dirty.iter().map(|(k, f)| (*k, *f))
    }

    /// More keys were noted than fit: they were dropped, and everything has to be
    /// compared again.
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub(crate) fn column_changed(&mut self, x: i32, z: i32) {
        self.columns.insert((x, z));
    }

    /// Whether a chunk is noted as a whole: its sections need no keys of their own.
    pub(crate) fn column_marked(&self, x: i32, z: i32) -> bool {
        self.overflowed || self.dirty.contains_key(&[x, WHOLE_COLUMN, z])
    }

    pub(crate) fn mark_column(&mut self, x: i32, z: i32, flags: u8) {
        self.mark_section([x, WHOLE_COLUMN, z], flags);
    }

    pub(crate) fn mark_section(&mut self, key: SectionKey, flags: u8) {
        if self.overflowed {
            return;
        }
        if let Some(f) = self.dirty.get_mut(&key) {
            *f |= flags;
        } else if self.dirty.len() >= self.capacity {
            self.overflowed = true;
            self.dirty.clear();
        } else {
            self.dirty.insert(key, flags);
        }
    }
}

/// What a level's sections are counted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dims {
    /// The section y of the level's lowest section.
    pub min_section: i32,
    /// How many sections a chunk has.
    pub sections: i32,
    pub has_skylight: bool,
    /// The level's hashed seed, which jitters the borders between biomes
    /// (`BiomeManager`).
    pub biome_seed: i64,
    /// A flat world, as the server says: where the horizon is and how soon the void
    /// darkens depend on it.
    pub flat: bool,
}

impl Dims {
    pub fn of(level: &Level) -> Dims {
        Dims {
            min_section: level.min_y() >> 4,
            sections: level.height() >> 4,
            has_skylight: level.has_skylight(),
            biome_seed: level.biomes.seed,
            flat: level.flat,
        }
    }
}

/// The published level: every loaded chunk as of the last publication.
#[derive(Debug, Default)]
pub struct Terrain {
    epoch: u64,
    dims: Dims,
    columns: HashMap<(i32, i32), Arc<Column>>,
    stamp: u64,
}

impl Terrain {
    /// Which level this is: a new number for every level the session enters.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The biome id of a 4 x 4 x 4 cell (its coordinates are blocks divided by four,
    /// before the client's jitter of cell borders), its y clamped into the level;
    /// `None` where no chunk is published.
    pub fn biome_cell(&self, cell: [i32; 3]) -> Option<u16> {
        let c = self.column(cell[0] >> 2, cell[2] >> 2)?;
        let cells = (self.dims.sections << 2).max(1);
        let y = (cell[1] - (self.dims.min_section << 2)).clamp(0, cells - 1);
        let s = c.sections().get((y >> 2) as usize)?;
        Some(s.biomes.get(cell[0] & 3, y & 3, cell[2] & 3))
    }

    pub fn dims(&self) -> Dims {
        self.dims
    }

    /// The level's stamp when it was last published: no published plane is newer.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    pub fn column(&self, x: i32, z: i32) -> Option<&Arc<Column>> {
        self.columns.get(&(x, z))
    }

    pub fn len(&self) -> usize {
        self.columns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Every loaded chunk, in no order: what a reader compares itself against after a
    /// [`Rescan`].
    pub fn columns(&self) -> impl Iterator<Item = ((i32, i32), &Arc<Column>)> + '_ {
        self.columns.iter().map(|(k, c)| (*k, c))
    }

    /// Counts of what is published, for logs and the debug text.
    pub fn summary(&self) -> TerrainSummary {
        let mut s = TerrainSummary {
            chunks: self.columns.len(),
            ..TerrainSummary::default()
        };
        let count = |planes: &mut [usize; 3], p: &LightPlane| {
            planes[match p {
                LightPlane::Dense(_) | LightPlane::Full => 0,
                LightPlane::Dark => 1,
                LightPlane::Absent => 2,
            }] += 1;
        };
        for c in self.columns.values() {
            s.dense_sections += c
                .sections()
                .iter()
                .filter(|p| matches!(p.blocks, BlockPlane::Dense(_)))
                .count();
            for l in c.light() {
                count(&mut s.sky, &l.sky);
                count(&mut s.block, &l.block);
            }
            s.with_heights += usize::from(c.heights().is_some());
            s.block_entities += c.block_entities().len();
        }
        s
    }

    /// The block state, sky light and block light at a block position, where its chunk
    /// is published.
    pub fn probe(&self, pos: [i32; 3]) -> Option<Probe> {
        let c = self.column(pos[0] >> 4, pos[2] >> 4)?;
        let i = (pos[1] >> 4) - self.dims.min_section;
        let (x, y, z) = (pos[0] & 15, pos[1] & 15, pos[2] & 15);
        let block = usize::try_from(i)
            .ok()
            .and_then(|i| c.sections().get(i))
            .map_or(0, |s| s.blocks.get(x, y, z));
        let light = usize::try_from(i + 1).ok().filter(|l| *l < c.light().len());
        Some(Probe {
            block,
            sky_light: match light {
                Some(l) => c.sky_light(self.dims.has_skylight, l, x, y, z),
                None if self.dims.has_skylight => 15,
                None => 0,
            },
            block_light: light.map_or(0, |l| c.block_light(l, x, y, z)),
        })
    }

    fn dep(&self, section: SectionKey) -> Dep {
        self.dep_in(self.columns.get(&(section[0], section[2])), section[1])
    }

    /// The state of the section at `y` of a chunk (none: no chunk is loaded there).
    fn dep_in(&self, column: Option<&Arc<Column>>, y: i32) -> Dep {
        let Some(c) = column else {
            return Dep::default();
        };
        let i = y - self.dims.min_section;
        let planes = usize::try_from(i).ok().and_then(|i| c.sections().get(i));
        Dep {
            generation: c.generation(),
            blocks: planes.map_or(0, |p| p.blocks_rev),
            biomes: planes.map_or(0, |p| p.biomes_rev),
            // The light sections reach one section past the level's both ends.
            light: usize::try_from(i + 1)
                .ok()
                .and_then(|l| c.light().get(l))
                .map_or(0, |l| l.rev),
        }
    }

    fn stamp_of(&self, section: SectionKey) -> Stamp {
        let mut deps = [Dep::default(); 27];
        for (i, d) in deps.iter_mut().enumerate() {
            let (dx, dz, dy) = (i as i32 % 3 - 1, i as i32 / 3 % 3 - 1, i as i32 / 9 - 1);
            *d = self.dep([section[0] + dx, section[1] + dy, section[2] + dz]);
        }
        Stamp {
            epoch: self.epoch,
            section,
            deps,
        }
    }
}

/// What is at one block position ([`Terrain::probe`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub block: u16,
    pub sky_light: u8,
    pub block_light: u8,
}

/// Counts of what a terrain holds ([`Terrain::summary`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerrainSummary {
    pub chunks: usize,
    /// Sections with a state per block (the others are of one state).
    pub dense_sections: usize,
    /// Light sections with sky light as an array or all lit, all dark, and holding
    /// none.
    pub sky: [usize; 3],
    /// The same for block light.
    pub block: [usize; 3],
    /// Chunks with motion-blocking heights.
    pub with_heights: usize,
    /// Block entities with data from the server.
    pub block_entities: usize,
}

impl std::fmt::Display for TerrainSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} chunks ({} with heights), {} dense sections, sky light {}/{}/{} and block light {}/{}/{} sections (arrays/dark/not sent), {} block entities",
            self.chunks,
            self.with_heights,
            self.dense_sections,
            self.sky[0],
            self.sky[1],
            self.sky[2],
            self.block[0],
            self.block[1],
            self.block[2],
            self.block_entities
        )
    }
}

/// What one section's data was when it was read: its chunk's generation (0 where no
/// chunk was loaded) and the stamps of its planes (0 outside the level's height).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dep {
    pub generation: u64,
    pub blocks: u64,
    pub light: u64,
    pub biomes: u64,
}

/// The states of all sections of the 3 × 3 chunks around one chunk, as published at
/// one moment ([`Scene::around`]): for judging the stamps of that chunk's sections
/// without a look-up for each of the 27 sections each of them depends on.
#[derive(Debug, Clone, Default)]
pub struct Around {
    epoch: u64,
    at: (i32, i32),
    /// The y of the first layer.
    lowest: i32,
    /// Bottom up, each layer's nine chunks at `(dx + 1) + 3 * (dz + 1)`.
    layers: Vec<[Dep; 9]>,
}

impl Around {
    /// [`Scene::is_current_where_loaded`] for a stamp of a section of this chunk, as
    /// of the moment the states were read.
    pub fn is_current_where_loaded(&self, stamp: &Stamp) -> bool {
        if stamp.epoch != self.epoch || (stamp.section[0], stamp.section[2]) != self.at {
            return false;
        }
        stamp.deps.iter().enumerate().all(|(i, was)| {
            let layer = stamp.section[1] + i as i32 / 9 - 1 - self.lowest;
            // A section's stamp reaches one layer past the level's both ends at most.
            usize::try_from(layer)
                .ok()
                .and_then(|l| self.layers.get(l))
                .is_some_and(|l| {
                    let now = l[i % 9];
                    now == *was || (now.generation == 0 && was.generation != 0)
                })
        })
    }
}

/// Everything a result made from a [`Neighbourhood`] depends on: the level, the
/// section, and the state of the 27 sections around and including it (index
/// `(dx + 1) + 3 * (dz + 1) + 9 * (dy + 1)`), the missing ones too, so that a neighbour
/// arriving later shows as a difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub epoch: u64,
    pub section: SectionKey,
    pub deps: [Dep; 27],
}

impl Stamp {
    /// The centre section's own state.
    pub fn centre(&self) -> Dep {
        self.deps[13]
    }
}

/// A section and everything around it that meshing it may read, pinned: the planes are
/// the ones published when it was taken and never change, whatever the game thread does
/// meanwhile.
#[derive(Debug, Clone)]
pub struct Neighbourhood {
    dims: Dims,
    /// The 3×3 chunks, index `(dx + 1) + 3 * (dz + 1)`.
    columns: [Option<Arc<Column>>; 9],
    stamp: Stamp,
}

/// Blocks along one side of the scratch [`Neighbourhood::fill_blocks`] fills: a section
/// and one block around it.
pub const HALO_SIDE: usize = 18;

impl Neighbourhood {
    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }

    pub fn section(&self) -> SectionKey {
        self.stamp.section
    }

    pub fn dims(&self) -> Dims {
        self.dims
    }

    /// The chunk at an offset of -1, 0 or 1 chunks from the centre's, if it was loaded.
    pub fn column(&self, dx: i32, dz: i32) -> Option<&Arc<Column>> {
        self.columns
            .get(usize::try_from((dx + 1) + 3 * (dz + 1)).ok()?)?
            .as_ref()
    }

    /// The chunk and the section index (from the level's lowest section; it may lie
    /// outside the level) of a block given relative to the centre section's corner,
    /// each coordinate from -16 to 31.
    fn locate(&self, x: i32, y: i32, z: i32) -> (Option<&Arc<Column>>, i32) {
        let index = self.stamp.section[1] - self.dims.min_section + (y >> 4);
        (self.column(x >> 4, z >> 4), index)
    }

    /// The block state at a position relative to the centre section's corner (each
    /// coordinate from -16 to 31): air where no chunk was loaded or outside the level.
    pub fn block(&self, x: i32, y: i32, z: i32) -> u16 {
        let (c, i) = self.locate(x, y, z);
        c.zip(usize::try_from(i).ok())
            .and_then(|(c, i)| c.sections().get(i))
            .map_or(0, |s| s.blocks.get(x & 15, y & 15, z & 15))
    }

    /// The block light there; dark where no chunk was loaded.
    pub fn block_light(&self, x: i32, y: i32, z: i32) -> u8 {
        let (c, i) = self.locate(x, y, z);
        match (c, usize::try_from(i + 1)) {
            (Some(c), Ok(l)) => c.block_light(l, x & 15, y & 15, z & 15),
            _ => 0,
        }
    }

    /// The sky light there; full light where no chunk was loaded or beyond the light
    /// sections, dark in a dimension without sky light.
    pub fn sky_light(&self, x: i32, y: i32, z: i32) -> u8 {
        if !self.dims.has_skylight {
            return 0;
        }
        let (c, i) = self.locate(x, y, z);
        match (c, usize::try_from(i + 1)) {
            (Some(c), Ok(l)) if l < c.light().len() => c.sky_light(true, l, x & 15, y & 15, z & 15),
            _ => 15,
        }
    }

    /// The biome id of the 4×4×4 cell a block is in (before the client's jitter of
    /// cell borders), its y clamped into the level; `None` where no chunk was loaded.
    pub fn biome(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let (c, i) = self.locate(x, y, z);
        let c = c?;
        let top = self.dims.sections - 1;
        let (i, cy) = if i < 0 {
            (0, 0)
        } else if i > top {
            (top, 3)
        } else {
            (i, (y & 15) >> 2)
        };
        let s = c.sections().get(usize::try_from(i).ok()?)?;
        Some(s.biomes.get((x & 15) >> 2, cy, (z & 15) >> 2))
    }

    /// Copies the centre section's block states and one block around it into `out`,
    /// in the planes' order (x fastest, then z, then y; index
    /// `(x + 1) + 18 * (z + 1) + 324 * (y + 1)`), so that a mesher reads its neighbours
    /// from one array in sequence.
    pub fn fill_blocks(&self, out: &mut [u16; HALO_SIDE * HALO_SIDE * HALO_SIDE]) {
        let base = self.stamp.section[1] - self.dims.min_section;
        let mut i = 0;
        for y in -1..17_i32 {
            let section = usize::try_from(base + (y >> 4)).ok();
            for z in -1..17_i32 {
                for x in -1..17_i32 {
                    out[i] = self
                        .column(x >> 4, z >> 4)
                        .zip(section)
                        .and_then(|(c, s)| c.sections().get(s))
                        .map_or(0, |s| s.blocks.get(x & 15, y & 15, z & 15));
                    i += 1;
                }
            }
        }
    }
}

/// The dirty queue was emptied (it overflowed, or a new level began): every section
/// published up to `stamp` in level `epoch` may have changed, come or gone without a
/// key. Keys queued after it are complete again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rescan {
    pub epoch: u64,
    pub stamp: u64,
}

/// What [`Scene::drain_dirty`] took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Drained {
    /// The level the keys are of.
    pub epoch: u64,
    /// Compare everything first: the keys alone don't say all that changed.
    pub rescan: Option<Rescan>,
}

#[derive(Debug)]
struct Queue {
    keys: HashMap<SectionKey, u8>,
    capacity: usize,
    rescan: Option<Rescan>,
    stats: QueueStats,
}

/// The dirty queue's counters since the scene was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    /// The most keys that ever waited.
    pub high_water: usize,
    /// How often it overflowed into a rescan.
    pub overflows: u64,
    /// Keys taken in (after coalescing within one publication).
    pub queued: u64,
    /// Keys that only added flags to one already waiting.
    pub coalesced: u64,
}

impl Queue {
    fn overflow(&mut self, epoch: u64, stamp: u64) {
        self.keys.clear();
        self.rescan = Some(Rescan { epoch, stamp });
        self.stats.overflows += 1;
    }
}

/// An entity as drawing needs it: who it is and where it was and is.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRecord {
    pub id: i32,
    pub uuid: u128,
    /// The entity type's registry id.
    pub entity_type: i32,
    pub position: [f64; 3],
    /// Where it was as its last tick began.
    pub old_position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub on_ground: bool,
    /// The `Pose` id the server gave it.
    pub pose: i32,
    pub baby: bool,
    /// The entity it rides.
    pub vehicle: Option<i32>,
    /// A player's or mannequin's pose and skin.
    pub avatar: Option<Box<crate::avatar::AvatarRecord>>,
    /// What is written over its head, if the player is shown it.
    pub name: Option<crate::scoreboard::NameRecord>,
}

/// The local player as drawing and the HUD need it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub entity_id: i32,
    pub position: [f64; 3],
    /// Where the player was as its last tick began (`xo`...).
    pub old_position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub old_rot: (f32, f32),
    pub eye_height: f32,
    /// The eyes are in water, as of the last tick, and how well they see there
    /// (`getWaterVision`, 0 to 1).
    pub eye_in_water: bool,
    pub water_vision: f32,
    pub pose: Pose,
    pub on_ground: bool,
    pub sprinting: bool,
    pub flying: bool,
    pub game_type: i32,
    pub health: f32,
    pub food: i32,
    pub selected_slot: i32,
    /// The attack's charge, 0 to 1.
    pub attack_strength: f32,
    /// The block being broken and how far, 0 to 1.
    pub breaking: Option<([i32; 3], f32)>,
    /// The effects on the player: (registry id, amplifier, ticks left or -1).
    pub effects: Vec<(i32, i32, i32)>,
    /// The entity the player rides.
    pub vehicle: Option<i32>,
    /// Counts the times the player was put somewhere instead of moving there (a
    /// teleport, a respawn): when it differs from the last frame's, nothing may be
    /// interpolated from the old position.
    pub placements: u64,
}

/// A world clock as of the frame (`ClockNetworkState`, counted on by the ticks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSample {
    /// The clock's id in the `world_clock` registry.
    pub clock: i32,
    pub total_ticks: i64,
    pub partial_tick: f32,
    pub rate: f32,
}

/// One frame of the game thread, as its last tick left things.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The level it is of ([`Terrain::epoch`]).
    pub epoch: u64,
    /// Counts publications: each frame's is higher than the one before.
    pub sequence: u64,
    /// The last input frame the game thread had taken in ([`crate::session::Session::frame`]
    /// numbers them). Turns of the camera made after it are not in this frame yet.
    pub input_sequence: u64,
    /// When that input was handed to the game thread, to measure how old the frame's
    /// input is.
    pub input_at: Option<Instant>,
    pub published: Instant,
    /// Game ticks run in this level so far.
    pub ticks: u64,
    /// When the current tick began on the monotonic clock, and how long a tick lasts:
    /// what [`Frame::partial_tick`] interpolates with.
    pub tick_start: Instant,
    pub ms_per_tick: f32,
    /// The server froze the game (`/tick freeze`).
    pub frozen: bool,
    /// The level's game time.
    pub game_time: i64,
    pub clocks: Vec<ClockSample>,
    /// Rain and thunder, 0 to 1 (the client keeps no earlier value to blend from).
    pub rain: f32,
    pub thunder: f32,
    pub dimension: Arc<str>,
    pub environment: Arc<Environment>,
    pub player: PlayerState,
    /// The lids of chests and shulker boxes that are not shut, by position.
    pub lids: Vec<crate::block_entity::LidRecord>,
    pub entities: Vec<EntityRecord>,
    /// Entities left out because [`MAX_ENTITIES`] were already in.
    pub entities_dropped: u32,
    /// The level's stamp when the frame was made.
    pub terrain_stamp: u64,
}

/// The most entities a frame carries.
pub const MAX_ENTITIES: usize = 4096;

impl Frame {
    /// An empty frame, for the publisher to fill.
    pub fn new(now: Instant, environment: Arc<Environment>, dimension: Arc<str>) -> Frame {
        Frame {
            epoch: 0,
            sequence: 0,
            input_sequence: 0,
            input_at: None,
            published: now,
            ticks: 0,
            tick_start: now,
            ms_per_tick: crate::clock::DEFAULT_MS_PER_TICK,
            frozen: false,
            game_time: 0,
            clocks: Vec::new(),
            rain: 0.0,
            thunder: 0.0,
            dimension,
            environment,
            player: PlayerState {
                entity_id: 0,
                position: [0.0; 3],
                old_position: [0.0; 3],
                y_rot: 0.0,
                x_rot: 0.0,
                old_rot: (0.0, 0.0),
                eye_height: 0.0,
                eye_in_water: false,
                water_vision: 0.0,
                pose: Pose::Standing,
                on_ground: false,
                sprinting: false,
                flying: false,
                game_type: 0,
                health: 0.0,
                food: 0,
                selected_slot: 0,
                attack_strength: 0.0,
                breaking: None,
                effects: Vec::new(),
                vehicle: None,
                placements: 0,
            },
            lids: Vec::new(),
            entities: Vec::new(),
            entities_dropped: 0,
            terrain_stamp: 0,
        }
    }

    /// How far the current tick has come at `now`, 0 to 1 (vanilla's partial tick):
    /// positions are drawn that far from a tick's old values to its new ones.
    pub fn partial_tick(&self, now: Instant) -> f32 {
        if self.frozen {
            return 1.0;
        }
        let ms = now.saturating_duration_since(self.tick_start).as_secs_f32() * 1000.0;
        (ms / self.ms_per_tick).clamp(0.0, 1.0)
    }
}

#[derive(Debug)]
struct Shared {
    terrain: RwLock<Terrain>,
    queue: Mutex<Queue>,
    frame: Mutex<Option<Arc<Frame>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The reader's side of what the game thread publishes. Cheap to clone; every clone
/// reads the same scene.
#[derive(Debug, Clone)]
pub struct Scene {
    shared: Arc<Shared>,
}

/// Dirty keys the queue holds before it overflows into a [`Rescan`]: more than the
/// sections of a 12-chunk view distance (25 × 25 chunks of 24 sections).
pub const DEFAULT_DIRTY_CAPACITY: usize = 1 << 15;

impl Scene {
    /// A scene and its publisher, with a dirty queue of `capacity` keys.
    pub fn new(capacity: usize) -> (ScenePublisher, Scene) {
        let scene = Scene {
            shared: Arc::new(Shared {
                terrain: RwLock::new(Terrain::default()),
                queue: Mutex::new(Queue {
                    keys: HashMap::new(),
                    capacity: capacity.max(1),
                    rescan: None,
                    stats: QueueStats::default(),
                }),
                frame: Mutex::new(None),
            }),
        };
        (
            ScenePublisher {
                scene: scene.clone(),
                epoch: 0,
                sequence: 0,
                spare: Vec::new(),
                stats: PublishStats::default(),
            },
            scene,
        )
    }

    /// The latest frame (`None` outside a level). Never waits for the game thread.
    pub fn frame(&self) -> Option<Arc<Frame>> {
        lock(&self.shared.frame).clone()
    }

    fn terrain(&self) -> std::sync::RwLockReadGuard<'_, Terrain> {
        self.shared
            .terrain
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads the published terrain. The game thread can't publish while `f` runs, so
    /// keep it short: take what is needed (columns are `Arc`s) and go.
    pub fn with_terrain<R>(&self, f: impl FnOnce(&Terrain) -> R) -> R {
        f(&self.terrain())
    }

    /// Pins a section with the chunks around it, as published now. `None` when its
    /// chunk isn't loaded or the section lies outside the level's height.
    pub fn pin(&self, section: SectionKey) -> Option<Neighbourhood> {
        let t = self.terrain();
        let i = section[1] - t.dims.min_section;
        if !(0..t.dims.sections).contains(&i) || t.column(section[0], section[2]).is_none() {
            return None;
        }
        let columns = std::array::from_fn(|c| {
            let (dx, dz) = (c as i32 % 3 - 1, c as i32 / 3 - 1);
            t.column(section[0] + dx, section[2] + dz).cloned()
        });
        Some(Neighbourhood {
            dims: t.dims,
            columns,
            stamp: t.stamp_of(section),
        })
    }

    /// Whether everything `stamp` was made from is still what is published: the same
    /// level, the same chunks around the section (none came, left or was replaced), no
    /// plane of the 27 sections changed. A result whose stamp isn't current is stale.
    pub fn is_current(&self, stamp: &Stamp) -> bool {
        let t = self.terrain();
        t.epoch == stamp.epoch && t.stamp_of(stamp.section) == *stamp
    }

    /// Whether everything `stamp` was made from that is still there is what is
    /// published: as [`Self::is_current`], but for chunks around the section that have
    /// left since. What was read of a chunk that is gone cannot be wrong until the chunk
    /// is back, and then it is another load of it, which shows. The game does not build
    /// a section again either when a chunk next to it leaves.
    pub fn is_current_where_loaded(&self, stamp: &Stamp) -> bool {
        let t = self.terrain();
        t.epoch == stamp.epoch
            && t.stamp_of(stamp.section)
                .deps
                .iter()
                .zip(&stamp.deps)
                .all(|(now, was)| now == was || (now.generation == 0 && was.generation != 0))
    }

    /// Reads the state of every section of the 3 × 3 chunks around chunk `x`, `z` into
    /// `out`, under one lock: what [`Around::is_current_where_loaded`] compares the
    /// stamps of that chunk's sections with.
    pub fn around(&self, x: i32, z: i32, out: &mut Around) {
        let t = self.terrain();
        out.epoch = t.epoch;
        out.at = (x, z);
        // One layer below the lowest section and one above the highest: what the
        // lowest and the highest sections' stamps reach.
        let lowest = t.dims.min_section - 1;
        out.lowest = lowest;
        let columns: [Option<&Arc<Column>>; 9] =
            std::array::from_fn(|c| t.column(x + c as i32 % 3 - 1, z + c as i32 / 3 - 1));
        out.layers.clear();
        out.layers.extend(
            (0..t.dims.sections + 2)
                .map(|layer| std::array::from_fn(|c| t.dep_in(columns[c], lowest + layer))),
        );
    }

    /// Moves every waiting dirty key into `out` (appended, in no order) and says
    /// whether a rescan is due first. The keys are of the level `epoch`; a reader still
    /// holding another level's data drops that data.
    pub fn drain_dirty(&self, out: &mut Vec<(SectionKey, u8)>) -> Drained {
        // The epoch is read under the queue's lock: a new level empties the queue
        // under it too, so the keys taken belong to the epoch returned.
        let mut q = lock(&self.shared.queue);
        let epoch = self.terrain().epoch;
        out.extend(q.keys.drain());
        Drained {
            epoch,
            rescan: q.rescan.take(),
        }
    }

    pub fn queue_stats(&self) -> QueueStats {
        lock(&self.shared.queue).stats
    }
}

/// What publishing has cost so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PublishStats {
    /// Publications of terrain that had something to publish.
    pub terrain_publications: u64,
    /// Chunks whose published copy was replaced or removed.
    pub columns_published: u64,
    pub frames: u64,
    /// Frames that reused an earlier frame's storage instead of allocating.
    pub frames_reused: u64,
}

/// The game thread's side: the only writer.
#[derive(Debug)]
pub struct ScenePublisher {
    scene: Scene,
    epoch: u64,
    sequence: u64,
    /// Frames published earlier, kept to reuse their storage once no reader holds them.
    spare: Vec<Arc<Frame>>,
    stats: PublishStats,
}

/// Earlier frames the publisher keeps for reuse.
const SPARE_FRAMES: usize = 4;

impl ScenePublisher {
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn stats(&self) -> PublishStats {
        self.stats
    }

    /// A new level begins (a login, or a respawn into another dimension): the terrain
    /// is emptied under a new epoch, the queue with it, and readers are told to compare
    /// everything. Returns the epoch.
    pub fn begin_level(&mut self, dims: Dims) -> u64 {
        self.epoch += 1;
        // Queue before terrain, as readers lock them: see `Scene::drain_dirty`.
        let mut q = lock(&self.scene.shared.queue);
        let mut t = self
            .scene
            .shared
            .terrain
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        t.epoch = self.epoch;
        t.dims = dims;
        t.columns.clear();
        t.stamp = 0;
        q.keys.clear();
        q.rescan = Some(Rescan {
            epoch: self.epoch,
            stamp: 0,
        });
        self.epoch
    }

    /// The session left its level (a reconfiguration, the end): nothing is published
    /// any more.
    pub fn end_level(&mut self) {
        self.begin_level(Dims::default());
        *lock(&self.scene.shared.frame) = None;
    }

    /// Publishes what changed in `level` since the last call: the changed chunks'
    /// columns replace their published copies, unloaded ones leave, and the dirty keys
    /// join the queue. Costs nothing when nothing changed.
    pub fn publish_level(&mut self, level: &mut Level) {
        // The frame's light first (`ClientLevel.update`, once per frame before
        // drawing): a chunk is never published without its light.
        level.run_light_updates();
        let Some(changes) = level.take_changes() else {
            return;
        };
        if changes.is_empty() {
            return;
        }
        let mut q = lock(&self.scene.shared.queue);
        {
            let mut t = self
                .scene
                .shared
                .terrain
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (x, z) in changes.columns() {
                match level.column(x, z) {
                    Some(c) => t.columns.insert((x, z), c.clone()),
                    None => t.columns.remove(&(x, z)),
                };
                self.stats.columns_published += 1;
            }
            t.stamp = level.stamp();
        }
        self.stats.terrain_publications += 1;
        if changes.overflowed() {
            q.overflow(self.epoch, level.stamp());
            return;
        }
        for (key, flags) in changes.dirty() {
            if let Some(f) = q.keys.get_mut(&key) {
                *f |= flags;
                q.stats.coalesced += 1;
            } else if q.keys.len() >= q.capacity {
                q.overflow(self.epoch, level.stamp());
                return;
            } else {
                q.keys.insert(key, flags);
                q.stats.queued += 1;
            }
        }
        q.stats.high_water = q.stats.high_water.max(q.keys.len());
    }

    /// Publishes a frame: `fill` writes it into storage that an earlier frame used, if
    /// every reader has let go of one, so a steady stream of frames allocates nothing.
    /// `first` makes a frame when there is none to reuse.
    pub fn publish_frame(&mut self, first: impl FnOnce() -> Frame, fill: impl FnOnce(&mut Frame)) {
        let reusable = self
            .spare
            .iter()
            .position(|f| Arc::strong_count(f) == 1 && Arc::weak_count(f) == 0);
        let mut frame = match reusable {
            Some(i) => {
                self.stats.frames_reused += 1;
                self.spare.swap_remove(i)
            }
            None => Arc::new(first()),
        };
        // Sole owner either way: just made, or counted above with no other handle
        // able to appear (the spare list is this thread's own).
        if let Some(f) = Arc::get_mut(&mut frame) {
            fill(f);
            self.sequence += 1;
            f.sequence = self.sequence;
            f.epoch = self.epoch;
        }
        self.stats.frames += 1;
        let old = lock(&self.scene.shared.frame).replace(frame);
        if let Some(old) = old
            && self.spare.len() < SPARE_FRAMES
        {
            self.spare.push(old);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::testing::{chunk_data, light, small_level};

    const STONE: u32 = 1;

    /// A level of 4 sections from section y -1, tracked, with stone in every chunk's
    /// bottom section, and the scene it is published to.
    fn world(chunks: &[(i32, i32)], capacity: usize) -> (Level, ScenePublisher, Scene) {
        let mut level = small_level();
        level.track_changes(capacity);
        let (mut publisher, scene) = Scene::new(capacity);
        publisher.begin_level(Dims::of(&level));
        for &(x, z) in chunks {
            load(&mut level, x, z);
        }
        publisher.publish_level(&mut level);
        (level, publisher, scene)
    }

    fn load(level: &mut Level, x: i32, z: i32) {
        let data = chunk_data(4, |i| if i == 0 { 1 } else { 0 });
        level.load_chunk(x, z, &data, 6).unwrap();
    }

    fn drain(scene: &Scene) -> (HashMap<SectionKey, u8>, Drained) {
        let mut out = Vec::new();
        let d = scene.drain_dirty(&mut out);
        let n = out.len();
        let map: HashMap<_, _> = out.into_iter().collect();
        assert_eq!(map.len(), n, "a key was queued twice");
        (map, d)
    }

    const GRID: [(i32, i32); 9] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    #[test]
    fn a_light_update_keeps_the_block_plane_and_an_old_pin_keeps_its_light() {
        let (mut level, mut publisher, scene) = world(&GRID, 1024);
        // A dense block plane in the section to watch (section y 0: index 1).
        level.set_block(4, 4, 4, STONE);
        level.update_light(0, 0, &light(&[(2, Some(7))], &[(2, Some(2))]));
        publisher.publish_level(&mut level);
        let before = scene.pin([0, 0, 0]).unwrap();
        assert_eq!(
            (before.sky_light(4, 5, 4), before.block_light(4, 5, 4)),
            (7, 2)
        );

        // Light only.
        level.update_light(0, 0, &light(&[(2, Some(12))], &[]));
        publisher.publish_level(&mut level);
        let after = scene.pin([0, 0, 0]).unwrap();
        let (b, a) = (
            &before.column(0, 0).unwrap().sections()[1],
            &after.column(0, 0).unwrap().sections()[1],
        );
        // The 8 KiB of block states are the same allocation, not a copy.
        assert!(matches!(a.blocks, BlockPlane::Dense(_)));
        assert!(a.blocks.shares(&b.blocks));
        assert!(a.biomes.shares(&b.biomes));
        assert_eq!(a.blocks_rev, b.blocks_rev);
        let (bl, al) = (
            before.column(0, 0).unwrap().section_light(1),
            after.column(0, 0).unwrap().section_light(1),
        );
        assert!(!al.sky.shares(&bl.sky));
        // The block light wasn't named by the update: the same array still.
        assert!(matches!(al.block, LightPlane::Dense(_)) && al.block.shares(&bl.block));
        assert!(al.rev > bl.rev);
        // One consistent version of the section: only its light stamp moved.
        assert_eq!(
            after.stamp().centre().blocks,
            before.stamp().centre().blocks
        );
        assert!(after.stamp().centre().light > before.stamp().centre().light);
        // The worker that pinned the old version still reads it, whatever came after.
        level.set_block(4, 5, 4, STONE);
        publisher.publish_level(&mut level);
        assert_eq!(before.sky_light(4, 5, 4), 7);
        assert_eq!(before.block(4, 5, 4), 0);
        assert_eq!(scene.pin([0, 0, 0]).unwrap().block(4, 5, 4), 1);
        assert_eq!(scene.pin([0, 0, 0]).unwrap().sky_light(4, 5, 4), 12);
        // And its result is known to be stale.
        assert!(!scene.is_current(before.stamp()));
        assert!(!scene.is_current(after.stamp()));
        assert!(scene.is_current(scene.pin([0, 0, 0]).unwrap().stamp()));
    }

    #[test]
    fn a_stamp_covers_every_neighbour_it_read_and_the_ones_that_were_missing() {
        // The centre chunk and its west neighbour only.
        let (mut level, mut publisher, scene) = world(&[(0, 0), (-1, 0)], 1024);
        let pin = scene.pin([0, 0, 0]).unwrap();
        assert!(pin.column(-1, 0).is_some() && pin.column(1, 0).is_none());
        // Index of (dx 1, dy 0, dz 0): no chunk there.
        assert_eq!(pin.stamp().deps[2 + 3 + 9].generation, 0);
        assert!(scene.is_current(pin.stamp()));
        // A section outside its reach changes: still current.
        level.set_block(-20, 40, 4, STONE);
        publisher.publish_level(&mut level);
        assert!(scene.is_current(pin.stamp()));
        // A block in the neighbouring chunk, in a section it reads: stale.
        level.set_block(-1, 4, 4, STONE);
        publisher.publish_level(&mut level);
        assert!(!scene.is_current(pin.stamp()));
        // The missing east neighbour arrives: a pin taken before is stale.
        let pin = scene.pin([0, 0, 0]).unwrap();
        load(&mut level, 1, 0);
        publisher.publish_level(&mut level);
        assert!(!scene.is_current(pin.stamp()));
        // A neighbour leaves and comes back with the very same content: it is another
        // load, and what was made from the old one doesn't pass for the new one's.
        let pin = scene.pin([0, 0, 0]).unwrap();
        level.forget_chunk(1, 0);
        load(&mut level, 1, 0);
        publisher.publish_level(&mut level);
        assert!(!scene.is_current(pin.stamp()));
        // The centre chunk itself reloaded.
        let pin = scene.pin([0, 0, 0]).unwrap();
        level.forget_chunk(0, 0);
        publisher.publish_level(&mut level);
        assert!(scene.pin([0, 0, 0]).is_none());
        load(&mut level, 0, 0);
        publisher.publish_level(&mut level);
        assert!(!scene.is_current(pin.stamp()));
        // Sections outside the level's height can't be pinned; the boundary light
        // sections are part of the stamps of the sections next to them.
        assert!(scene.pin([0, -2, 0]).is_none() && scene.pin([0, 3, 0]).is_none());
        let bottom = scene.pin([0, -1, 0]).unwrap();
        level.update_light(0, 0, &light(&[], &[(0, Some(9))]));
        publisher.publish_level(&mut level);
        assert!(!scene.is_current(bottom.stamp()));
        assert_eq!(scene.pin([0, -1, 0]).unwrap().block_light(3, -1, 3), 9);
    }

    #[test]
    fn a_chunks_sections_are_judged_together_as_one_by_one() {
        let mut level = Level::new(-16, 64);
        level.track_changes(4096);
        let (mut publisher, scene) = Scene::new(4096);
        publisher.begin_level(Dims::of(&level));
        for z in -1..=1 {
            for x in -1..=1 {
                level.add_empty_chunk(x, z);
            }
        }
        publisher.publish_level(&mut level);
        let stamps: Vec<Stamp> = (-1..3)
            .map(|y| scene.pin([0, y, 0]).unwrap().stamp().clone())
            .collect();
        let mut around = Around::default();
        let mut judge = |scene: &Scene| -> Vec<(bool, bool)> {
            scene.around(0, 0, &mut around);
            stamps
                .iter()
                .map(|s| {
                    (
                        around.is_current_where_loaded(s),
                        scene.is_current_where_loaded(s),
                    )
                })
                .collect()
        };
        assert_eq!(judge(&scene), [(true, true); 4]);
        // A chunk next door leaves: nothing that is there has changed.
        level.forget_chunk(1, -1);
        publisher.publish_level(&mut level);
        assert_eq!(judge(&scene), [(true, true); 4]);
        assert!(!scene.is_current(&stamps[0]));
        // A block in the lowest section of a neighbour: the sections that reach it.
        level.set_block(-3, -10, 5, 1);
        publisher.publish_level(&mut level);
        // (The light it changes reaches one section further up than the block.)
        assert_eq!(
            judge(&scene),
            [(false, false), (false, false), (false, false), (true, true)]
        );
        // The chunk comes back, as another load: everything that read it is stale.
        level.add_empty_chunk(1, -1);
        publisher.publish_level(&mut level);
        assert_eq!(judge(&scene), [(false, false); 4]);
        // A stamp of another chunk's section is not this chunk's to judge.
        let other = scene.pin([1, 0, 0]).unwrap().stamp().clone();
        scene.around(0, 0, &mut around);
        assert!(!around.is_current_where_loaded(&other));
    }

    #[test]
    fn readers_cross_section_and_chunk_borders() {
        let (mut level, mut publisher, scene) = world(&GRID, 1024);
        level.set_block(-1, 0, 0, 5);
        level.set_block(16, 15, 15, 6);
        level.set_block(0, 16, 0, 7);
        level.set_block(15, -1, 31, 8);
        level.fill_biome([16, 0, 0], [19, 3, 3], 3);
        publisher.publish_level(&mut level);
        let n = scene.pin([0, 0, 0]).unwrap();
        assert_eq!(n.section(), [0, 0, 0]);
        assert_eq!(n.block(-1, 0, 0), 5);
        assert_eq!(n.block(16, 15, 15), 6);
        assert_eq!(n.block(0, 16, 0), 7);
        assert_eq!(n.block(15, -1, 31), 8);
        // The section below is stone but for the block set there.
        assert_eq!(n.block(3, -1, 3), 1);
        // Below the level and above it: air.
        let top = scene.pin([0, 2, 0]).unwrap();
        assert_eq!(top.block(3, 16, 3), 0);
        assert_eq!(scene.pin([0, -1, 0]).unwrap().block(3, -1, 3), 0);
        assert_eq!(n.biome(16, 0, 0), Some(3));
        assert_eq!(n.biome(15, 0, 0), Some(0));
        // The scratch copy agrees with the reader, block for block.
        let mut scratch = [0u16; HALO_SIDE * HALO_SIDE * HALO_SIDE];
        n.fill_blocks(&mut scratch);
        for y in -1..17 {
            for z in -1..17 {
                for x in -1..17 {
                    let i = (x + 1) + 18 * (z + 1) + 324 * (y + 1);
                    assert_eq!(scratch[i as usize], n.block(x, y, z), "{x} {y} {z}");
                }
            }
        }
        // A section that holds light and was sent none is dark until light is
        // computed; the sections above all light are lit.
        assert_eq!((n.sky_light(3, 5, 3), top.sky_light(3, 5, 3)), (0, 15));
        // A probe reads the same as a pin, by block position.
        level.update_light(0, 0, &light(&[(2, Some(11))], &[(2, Some(6)), (3, None)]));
        publisher.publish_level(&mut level);
        scene.with_terrain(|t| {
            let p = t.probe([0, 16, 0]).unwrap();
            assert_eq!((p.block, p.sky_light, p.block_light), (7, 15, 0));
            let p = t.probe([3, 5, 3]).unwrap();
            assert_eq!((p.block, p.sky_light, p.block_light), (0, 11, 6));
            assert_eq!(t.probe([3, 500, 3]).unwrap().sky_light, 15);
            assert!(t.probe([300, 5, 3]).is_none());
            let s = t.summary();
            assert_eq!((s.chunks, s.with_heights, s.block_entities), (9, 0, 0));
            // The four sections the blocks above were set in.
            assert_eq!(s.dense_sections, 4);
            // Light is held up to the section above the highest one with blocks: the
            // top light section of each of the nine chunks holds none.
            assert_eq!((s.sky[2], s.block), (9, [1, 44, 9]));
            assert!(
                s.to_string()
                    .starts_with("9 chunks (0 with heights), 4 dense")
            );
        });
        let (_, _, lonely) = world(&[(0, 0)], 64);
        assert_eq!(lonely.pin([0, 0, 0]).unwrap().sky_light(-3, 5, 3), 15);
        assert_eq!(lonely.pin([0, 0, 0]).unwrap().block(-3, 5, 3), 0);
        assert_eq!(lonely.pin([0, 0, 0]).unwrap().biome(-3, 5, 3), None);
    }

    #[test]
    fn dirty_keys_say_what_changed_and_reach_as_far_as_the_change_does() {
        let (mut level, mut publisher, scene) = world(&GRID, 4096);
        // The loads: each chunk as a whole, loaded, and next to others that loaded.
        let (keys, d) = drain(&scene);
        assert_eq!(d.epoch, 1);
        // The level's beginning left a rescan; the keys came after it.
        assert_eq!(d.rescan, Some(Rescan { epoch: 1, stamp: 0 }));
        assert_eq!(
            keys[&[0, WHOLE_COLUMN, 0]],
            dirty::LOADED | dirty::NEIGHBOUR
        );
        // A chunk nobody loaded but next to one that was.
        assert_eq!(keys[&[2, WHOLE_COLUMN, 0]], dirty::NEIGHBOUR);
        assert_eq!(keys.len(), 25);

        // The first block of a section makes the sections around it hold light: their
        // planes are new, and they and the sections around them are noted too.
        level.set_block(8, 12, 8, STONE);
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(keys[&[0, 0, 0]], dirty::BLOCKS | dirty::NEIGHBOUR);
        // The light section above (y 16 to 31) is new in the nine chunks.
        assert_eq!(keys[&[1, 1, -1]], dirty::LIGHT);
        assert_eq!(keys[&[1, 2, 1]], dirty::NEIGHBOUR);
        // No chunk, no key.
        assert!(!keys.contains_key(&[2, 1, 0]));
        level.set_block(8, 28, 8, STONE);
        publisher.publish_level(&mut level);
        drain(&scene);

        // A block in the middle of a section that has blocks: that section alone.
        level.set_block(8, 8, 8, STONE);
        publisher.publish_level(&mut level);
        let (keys, d) = drain(&scene);
        assert_eq!(d.rescan, None);
        assert_eq!(keys, HashMap::from([([0, 0, 0], dirty::BLOCKS)]));

        // The player's own change at a section's corner: the eight sections that
        // meet there, all marked to be drawn first.
        assert_eq!(level.replace_block(0, 0, 0, STONE), Some(0));
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(keys.len(), 8);
        assert_eq!(keys[&[0, 0, 0]], dirty::BLOCKS | dirty::URGENT);
        for key in [[-1, 0, 0], [0, -1, 0], [0, 0, -1], [-1, -1, -1]] {
            assert_eq!(keys[&key], dirty::NEIGHBOUR | dirty::URGENT, "{key:?}");
        }
        // At the level's bottom there is no section below to mark.
        level.set_block(8, -16, 8, 0);
        publisher.publish_level(&mut level);
        assert_eq!(
            drain(&scene).0,
            HashMap::from([([0, -1, 0], dirty::BLOCKS)])
        );

        // Light: the section and all around it, as far as the level reaches (the
        // section is the level's lowest, so 2 layers of 9).
        level.update_light(0, 0, &light(&[(1, Some(4))], &[]));
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(keys.len(), 18);
        assert_eq!(keys[&[0, -1, 0]], dirty::LIGHT);
        assert_eq!(keys[&[1, 0, -1]], dirty::NEIGHBOUR);
        // The boundary light section below the level: only its neighbours inside.
        level.update_light(0, 0, &light(&[], &[(0, Some(4))]));
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(keys.len(), 9);
        assert!(keys.values().all(|f| *f == dirty::NEIGHBOUR));

        // Several changes to one section before a drain: one key, the flags added up.
        level.set_block(8, 8, 8, 0);
        level.update_light(0, 0, &light(&[(2, Some(4))], &[]));
        level.set_block_entity_data([8, 9, 8], 3, None);
        publisher.publish_level(&mut level);
        level.set_block(8, 8, 8, STONE);
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(
            keys[&[0, 0, 0]],
            dirty::BLOCKS | dirty::LIGHT | dirty::BLOCK_ENTITIES
        );
        assert!(scene.queue_stats().coalesced > 0);

        // Biomes and unloading are a chunk's.
        level.fill_biome([0, 0, 0], [3, 3, 3], 2);
        level.forget_chunk(1, 1);
        publisher.publish_level(&mut level);
        let (keys, _) = drain(&scene);
        assert_eq!(
            keys[&[0, WHOLE_COLUMN, 0]],
            dirty::BIOMES | dirty::NEIGHBOUR
        );
        // The chunk that left is also next to the one whose biomes changed.
        assert_eq!(
            keys[&[1, WHOLE_COLUMN, 1]],
            dirty::UNLOADED | dirty::NEIGHBOUR
        );
        assert_eq!(keys[&[2, WHOLE_COLUMN, 2]], dirty::NEIGHBOUR);
        // Nothing changed: nothing published, nothing queued.
        let before = publisher.stats();
        publisher.publish_level(&mut level);
        assert_eq!(publisher.stats(), before);
        assert!(drain(&scene).0.is_empty());
    }

    /// What a reader keeps per section it has drawn: the stamp it was made from.
    type Known = HashMap<SectionKey, Dep>;

    /// A reader's recovery after a rescan: everything it has is compared with the
    /// terrain. Returns the sections to make again and drops the ones that are gone.
    fn reconcile(scene: &Scene, known: &mut Known) -> Vec<SectionKey> {
        scene.with_terrain(|t| {
            known.retain(|key, _| t.column(key[0], key[2]).is_some());
            let mut todo = Vec::new();
            for ((x, z), c) in t.columns() {
                for i in 0..c.sections().len() as i32 {
                    let key = [x, t.dims().min_section + i, z];
                    let now = t.dep(key);
                    if known.get(&key) != Some(&now) {
                        todo.push(key);
                        known.insert(key, now);
                    }
                }
            }
            todo
        })
    }

    #[test]
    fn an_overflowing_queue_becomes_a_rescan_and_no_unload_is_lost() {
        // Room for 16 keys only.
        let (mut level, mut publisher, scene) = world(&GRID, 16);
        let mut known = Known::new();
        let (_, d) = drain(&scene);
        assert!(d.rescan.is_some());
        assert_eq!(reconcile(&scene, &mut known).len(), 9 * 4);
        assert!(reconcile(&scene, &mut known).is_empty());

        // More changes than the queue holds, among them the only notice that a chunk
        // left, and a chunk that left and came back.
        for (i, &(x, z)) in GRID.iter().enumerate() {
            level.replace_block(x * 16, i as i32, z * 16, STONE);
        }
        level.forget_chunk(1, 1);
        level.forget_chunk(-1, -1);
        load(&mut level, -1, -1);
        level.update_light(0, 0, &light(&[(3, Some(4))], &[]));
        publisher.publish_level(&mut level);
        let (keys, d) = drain(&scene);
        assert!(keys.is_empty());
        let rescan = d.rescan.expect("the queue overflowed");
        assert_eq!((rescan.epoch, rescan.stamp), (1, level.stamp()));
        assert!(scene.queue_stats().overflows >= 1);

        // The reader recovers from the terrain's stamps alone.
        let todo = reconcile(&scene, &mut known);
        // No geometry of the chunk that left stays.
        assert!(known.keys().all(|k| (k[0], k[2]) != (1, 1)));
        assert_eq!(known.len(), 8 * 4);
        // The reloaded chunk is made again entirely, though its content is what it was.
        assert_eq!(todo.iter().filter(|k| (k[0], k[2]) == (-1, -1)).count(), 4);
        // Each changed section is found; an untouched one is not.
        assert!(todo.contains(&[0, 0, 0]) && todo.contains(&[0, 1, 0]));
        // The section above a section that got its first block holds light now.
        assert!(todo.contains(&[1, 1, 0]));
        assert!(!todo.contains(&[0, 2, 0]));
        assert!(reconcile(&scene, &mut known).is_empty());

        // Keys after the rescan are complete again.
        level.set_block(8, 8, 8, STONE);
        publisher.publish_level(&mut level);
        let (keys, d) = drain(&scene);
        assert_eq!(d.rescan, None);
        assert_eq!(keys, HashMap::from([([0, 0, 0], dirty::BLOCKS)]));

        // The level's own note overflowing (more sections touched in one frame than
        // it holds) ends the same way.
        for x in 0..16 {
            level.update_light(-1, 0, &light(&[(1 + x % 4, Some(x as u8))], &[]));
            level.update_light(1, 0, &light(&[(1 + x % 4, Some(x as u8))], &[]));
        }
        publisher.publish_level(&mut level);
        let (keys, d) = drain(&scene);
        assert!(keys.is_empty() && d.rescan.is_some());
        assert!(!reconcile(&scene, &mut known).is_empty());
    }

    #[test]
    fn a_new_level_is_a_new_epoch_and_voids_what_was_published() {
        let (mut level, mut publisher, scene) = world(&[(0, 0)], 64);
        let pin = scene.pin([0, 0, 0]).unwrap();
        level.set_block(8, 8, 8, STONE);
        publisher.publish_level(&mut level);
        // The player goes to another dimension: a taller level, without sky light.
        let mut nether = Level::new(0, 128);
        nether.set_has_skylight(false);
        nether.track_changes(64);
        assert_eq!(publisher.begin_level(Dims::of(&nether)), 2);
        let (keys, d) = drain(&scene);
        // The old level's keys went with it.
        assert!(keys.is_empty());
        assert_eq!(d.epoch, 2);
        assert_eq!(d.rescan, Some(Rescan { epoch: 2, stamp: 0 }));
        assert!(scene.with_terrain(|t| t.is_empty() && t.dims().sections == 8));
        assert!(scene.pin([0, 0, 0]).is_none());
        assert!(!scene.is_current(pin.stamp()));
        // Same coordinates, new level: the old result still doesn't pass.
        let data = chunk_data(8, |_| 0);
        nether.load_chunk(0, 0, &data, 6).unwrap();
        publisher.publish_level(&mut nether);
        assert!(!scene.is_current(pin.stamp()));
        let now = scene.pin([0, 0, 0]).unwrap();
        assert_eq!(now.stamp().epoch, 2);
        assert_eq!(now.sky_light(3, 3, 3), 0);
        // The old pin still reads the old level's planes.
        assert_eq!(pin.block(3, -1, 3), 1);
        // Leaving the level altogether.
        publisher.end_level();
        assert!(scene.frame().is_none() && scene.pin([0, 0, 0]).is_none());
        assert_eq!(drain(&scene).1.epoch, 3);
    }

    #[test]
    fn publishing_costs_what_changed() {
        let chunks: Vec<(i32, i32)> = (0..100).map(|i| (i % 10, i / 10)).collect();
        let (mut level, mut publisher, scene) = world(&chunks, 4096);
        assert_eq!(publisher.stats().columns_published, 100);
        assert_eq!(scene.with_terrain(Terrain::len), 100);
        // One block among a hundred chunks: one column goes out again.
        level.set_block(40, -3, 40, 2);
        publisher.publish_level(&mut level);
        assert_eq!(publisher.stats().columns_published, 101);
        // A block at a chunk's edge dirties its neighbour's section, but only the
        // chunk that changed is published again.
        level.set_block(48, -3, 40, 2);
        publisher.publish_level(&mut level);
        assert_eq!(publisher.stats().columns_published, 102);
        assert_eq!(publisher.stats().terrain_publications, 3);
        // The first block of a section: the sections around it start holding light,
        // in the eight chunks around too.
        level.set_block(40, 3, 40, STONE);
        publisher.publish_level(&mut level);
        assert_eq!(publisher.stats().columns_published, 111);
        // Every other chunk is the very column published the first time.
        let first = scene.with_terrain(|t| t.column(9, 9).cloned()).unwrap();
        assert!(Arc::ptr_eq(&first, level.column(9, 9).unwrap()));
        assert_eq!(scene.with_terrain(Terrain::stamp), level.stamp());
    }

    #[test]
    fn frames_reuse_storage_once_readers_let_go() {
        let (mut publisher, scene) = Scene::new(16);
        publisher.begin_level(Dims::default());
        let env = Arc::new(Environment::default());
        let dimension: Arc<str> = Arc::from("minecraft:overworld");
        let publish = |publisher: &mut ScenePublisher, input: u64, entities: usize| {
            let (env, dimension) = (env.clone(), dimension.clone());
            publisher.publish_frame(
                || Frame::new(Instant::now(), env, dimension),
                |f| {
                    f.input_sequence = input;
                    f.entities.clear();
                    f.entities.extend((0..entities).map(|i| EntityRecord {
                        id: i as i32,
                        uuid: 0,
                        entity_type: 0,
                        position: [0.0; 3],
                        old_position: [0.0; 3],
                        y_rot: 0.0,
                        x_rot: 0.0,
                        on_ground: false,
                        pose: 0,
                        baby: false,
                        vehicle: None,
                        avatar: None,
                        name: None,
                    }));
                },
            );
        };
        assert!(scene.frame().is_none());
        publish(&mut publisher, 1, 3);
        // A reader holds the first frame while later ones are published.
        let held = scene.frame().unwrap();
        assert_eq!((held.sequence, held.epoch, held.input_sequence), (1, 1, 1));
        for i in 2..=40 {
            publish(&mut publisher, i, 5);
        }
        // What it holds is untouched; the latest is the newest.
        assert_eq!((held.sequence, held.entities.len()), (1, 3));
        let latest = scene.frame().unwrap();
        assert_eq!((latest.sequence, latest.input_sequence), (40, 40));
        assert_eq!(latest.entities.len(), 5);
        // After the first few, every frame went into storage used before: the frame
        // the reader holds is never among them.
        let s = publisher.stats();
        assert_eq!(s.frames, 40);
        assert!(s.frames_reused >= 36, "{s:?}");
        drop(held);
        drop(latest);
        publish(&mut publisher, 41, 0);
        assert_eq!(scene.frame().unwrap().sequence, 41);
        // The partial tick runs from a tick's start to its end and stops there.
        let mut f = Frame::new(Instant::now(), Arc::default(), Arc::from("x"));
        let t0 = f.tick_start;
        assert_eq!(f.partial_tick(t0), 0.0);
        assert_eq!(
            f.partial_tick(t0 + std::time::Duration::from_millis(25)),
            0.5
        );
        assert_eq!(
            f.partial_tick(t0 + std::time::Duration::from_millis(80)),
            1.0
        );
        f.frozen = true;
        assert_eq!(f.partial_tick(t0), 1.0);
    }
}
