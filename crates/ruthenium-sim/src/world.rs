//! The blocks of the level the player is in: chunk sections as the server sends them,
//! and the collision shapes around a box in the order the game visits them
//! (`BlockCollisions`).
//!
//! The level also keeps what only drawing reads: each section's light as the server
//! sends it, the motion-blocking heightmap, and the data of block entities. A chunk is a
//! [`Column`] of planes (block states, biomes, sky light, block light), each behind an
//! `Arc`: a snapshot of the level ([`crate::scene`]) shares them, and the level copies a
//! plane only when it changes one that a snapshot still holds. Every change stamps the
//! plane it touched with a number that only grows, so a reader can tell which planes of
//! a section are newer than the ones it used.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::nbt;
use ruthenium_protocol::packets::play::{ChunkBlockEntity, LIGHT_BYTES, LightData};
use ruthenium_protocol::wire::{Reader, WireError};

use crate::biome::Climate;
use crate::block_entity::{BlockEntities, BlockEntityShape, Kind, kind_of};
use crate::blocks::Blocks;
use crate::direction::Direction;
use crate::scene::{Changes, dirty};
use crate::shape::{Aabb, EPSILON, Placed};

/// Bits per entry of the global block-state palette (`ceillog2` of the state count).
pub fn global_bits(count: usize) -> u32 {
    usize::BITS - (count.max(1) - 1).leading_zeros()
}

/// `Heightmap.Types.MOTION_BLOCKING`'s id in a chunk packet.
const HEIGHTMAP_MOTION_BLOCKING: i32 = 4;

/// A section's block states: one state for all of it, or one per block (x fastest,
/// then z, then y).
#[derive(Debug, Clone)]
pub enum BlockPlane {
    Uniform(u16),
    Dense(Arc<[u16; 4096]>),
}

impl BlockPlane {
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        match self {
            BlockPlane::Uniform(s) => *s,
            BlockPlane::Dense(a) => a[((y << 8) | (z << 4) | x) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, state: u16) {
        if let BlockPlane::Uniform(s) = *self {
            if s == state {
                return;
            }
            *self = BlockPlane::Dense(Arc::new([s; 4096]));
        }
        if let BlockPlane::Dense(a) = self {
            Arc::make_mut(a)[((y << 8) | (z << 4) | x) as usize] = state;
        }
    }

    /// Whether two planes are the same storage (or the same single state): nothing was
    /// copied between them.
    pub fn shares(&self, other: &BlockPlane) -> bool {
        match (self, other) {
            (BlockPlane::Uniform(a), BlockPlane::Uniform(b)) => a == b,
            (BlockPlane::Dense(a), BlockPlane::Dense(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// A section's biomes: one per 4×4×4 cell (`PalettedContainer` for biomes, x fastest,
/// then z, then y).
#[derive(Debug, Clone)]
pub enum BiomePlane {
    Uniform(u16),
    Dense(Arc<[u16; 64]>),
}

impl BiomePlane {
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        match self {
            BiomePlane::Uniform(b) => *b,
            BiomePlane::Dense(a) => a[((y << 4) | (z << 2) | x) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, biome: u16) -> bool {
        if self.get(x, y, z) == biome {
            return false;
        }
        if let BiomePlane::Uniform(b) = *self {
            *self = BiomePlane::Dense(Arc::new([b; 64]));
        }
        if let BiomePlane::Dense(a) = self {
            Arc::make_mut(a)[((y << 4) | (z << 2) | x) as usize] = biome;
        }
        true
    }

    pub fn shares(&self, other: &BiomePlane) -> bool {
        match (self, other) {
            (BiomePlane::Uniform(a), BiomePlane::Uniform(b)) => a == b,
            (BiomePlane::Dense(a), BiomePlane::Dense(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One light section's sky or block light (`DataLayer`): a nibble per block. The
/// server sends an array, says the section is all dark, or says nothing about it.
#[derive(Debug, Clone, Default)]
pub enum LightPlane {
    /// The server sent nothing for the section.
    #[default]
    Absent,
    /// All dark (an empty `DataLayer`).
    Dark,
    Dense(Arc<[u8; LIGHT_BYTES]>),
}

impl LightPlane {
    /// The light of a block of the section, `None` where the server sent nothing.
    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        match self {
            LightPlane::Absent => None,
            LightPlane::Dark => Some(0),
            LightPlane::Dense(a) => {
                let i = ((y << 8) | (z << 4) | x) as usize;
                Some(a[i >> 1] >> ((i & 1) * 4) & 15)
            }
        }
    }

    pub fn shares(&self, other: &LightPlane) -> bool {
        match (self, other) {
            (LightPlane::Absent, LightPlane::Absent) | (LightPlane::Dark, LightPlane::Dark) => true,
            (LightPlane::Dense(a), LightPlane::Dense(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One section's blocks and biomes, each with the stamp of its last change.
#[derive(Debug, Clone)]
pub struct SectionPlanes {
    pub blocks: BlockPlane,
    pub biomes: BiomePlane,
    pub blocks_rev: u64,
    pub biomes_rev: u64,
}

/// One light section's sky and block light, with the stamp of the last change to
/// either.
#[derive(Debug, Clone, Default)]
pub struct LightPlanes {
    pub sky: LightPlane,
    pub block: LightPlane,
    pub rev: u64,
}

/// What the server sent about a block entity, for drawing it: a sign's text, a banner's
/// patterns, a skull's profile. How a block entity looks otherwise follows from its
/// block's state.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockEntityRecord {
    pub pos: [i32; 3],
    /// The block entity type's registry id.
    pub kind: i32,
    pub tag: Option<Arc<nbt::Tag>>,
}

/// One chunk: its sections from the level's bottom up, its light sections (one more
/// below and one more above than it has sections), its motion-blocking heights and its
/// block entities' data.
#[derive(Debug, Clone)]
pub struct Column {
    generation: u64,
    sections: Vec<SectionPlanes>,
    light: Vec<LightPlanes>,
    heights: Option<Arc<[i16; 256]>>,
    block_entities: Arc<Vec<BlockEntityRecord>>,
    block_entities_rev: u64,
}

impl Column {
    /// The stamp of the load that made this chunk. A chunk unloaded and loaded again at
    /// the same place has another one, so nothing made from the old chunk passes for the
    /// new one's.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The sections, from the level's lowest.
    pub fn sections(&self) -> &[SectionPlanes] {
        &self.sections
    }

    /// The light sections: index 0 is the one below the level's lowest section, so
    /// section `i`'s light is at `i + 1`.
    pub fn light(&self) -> &[LightPlanes] {
        &self.light
    }

    /// The light of section `i`.
    pub fn section_light(&self, i: usize) -> &LightPlanes {
        &self.light[i + 1]
    }

    /// For each column of blocks (index `x + 16 * z`), the y above its highest block
    /// that blocks motion or holds a fluid (`Heightmap.Types.MOTION_BLOCKING`), the
    /// level's bottom where there is none. `None` when the level can't keep it: the
    /// server sent none and the tags to make one are not known.
    pub fn heights(&self) -> Option<&[i16; 256]> {
        self.heights.as_deref()
    }

    pub fn block_entities(&self) -> &[BlockEntityRecord] {
        &self.block_entities
    }

    /// The stamp of the last change to the block entities' data.
    pub fn block_entities_rev(&self) -> u64 {
        self.block_entities_rev
    }

    /// The block light at a block of light section `light` (`BlockLightSectionStorage`:
    /// dark where there is no data).
    pub fn block_light(&self, light: usize, x: i32, y: i32, z: i32) -> u8 {
        self.light
            .get(light)
            .and_then(|l| l.block.get(x, y, z))
            .unwrap_or(0)
    }

    /// The sky light at a block of light section `light`
    /// (`SkyLightSectionStorage.getLightValue`): the section's own data; without any,
    /// what the bottom layer of the next section above with data has in that column of
    /// blocks; full light above all data. Dark in a dimension without sky light.
    pub fn sky_light(&self, has_skylight: bool, light: usize, x: i32, y: i32, z: i32) -> u8 {
        if !has_skylight {
            return 0;
        }
        match self.light.get(light).and_then(|l| l.sky.get(x, y, z)) {
            Some(v) => v,
            None => self
                .light
                .iter()
                .skip(light + 1)
                .find_map(|l| l.sky.get(x, 0, z))
                .unwrap_or(15),
        }
    }
}

/// A chunk as its packet carries it, for [`Level::load_chunk_packet`].
#[derive(Debug, Clone, Copy)]
pub struct ChunkPacket<'a> {
    pub heightmaps: &'a [(i32, Vec<i64>)],
    pub data: &'a [u8],
    pub block_entities: &'a [ChunkBlockEntity],
    pub light: &'a LightData,
}

/// What the level knows of biomes: the climate of each biome by network id, the
/// hashed seed that jitters which biome a block is in, the sea level (the snow line is
/// 17 above it), and the biome empty chunks are made of.
#[derive(Debug, Clone)]
pub struct BiomeInfo {
    pub climates: std::sync::Arc<Vec<Climate>>,
    pub seed: i64,
    pub sea_level: i32,
    pub empty_chunk_biome: u16,
}

impl Default for BiomeInfo {
    fn default() -> BiomeInfo {
        BiomeInfo {
            climates: std::sync::Arc::new(Vec::new()),
            seed: 0,
            sea_level: 63,
            empty_chunk_biome: 0,
        }
    }
}

/// The level's blocks.
#[derive(Debug, Clone)]
pub struct Level {
    min_y: i32,
    height: i32,
    /// The dimension's `gameplay/fast_lava` attribute (the nether's lava flows and
    /// pushes faster).
    fast_lava: bool,
    /// Whether the dimension has sky light (its type's `has_skylight`).
    has_skylight: bool,
    chunks: HashMap<(i32, i32), Arc<Column>>,
    /// The last stamp given out: each change takes the next.
    stamp: u64,
    /// For each block state, whether it counts in the motion-blocking heightmap (a bit
    /// per state id). Without it, heights come only from chunk packets and are dropped
    /// when a block changes.
    motion_blocking: Option<Arc<Vec<u64>>>,
    /// What changed since the level was last published, when something publishes it.
    changes: Option<Box<Changes>>,
    /// The moving pistons and shulker boxes, and their tickers.
    pub block_entities: BlockEntities,
    /// `PistonMovingBlockEntity.NOCLIP`: while a piston moves an entity, the way it
    /// goes (blocks moving that way don't stop it).
    pub noclip: Option<Direction>,
    /// When set, every `setBlock` call the client level makes outside a predicted
    /// action (position, state, flags, update limit), in order: tests compare them with
    /// vanilla's.
    pub set_log: Option<Vec<crate::block::level::SetCall>>,
    /// The level's world border (`ClientLevel.getWorldBorder`).
    pub border: crate::border::WorldBorder,
    /// The rain and thunder the server says there are.
    pub weather: crate::weather::Weather,
    pub biomes: BiomeInfo,
}

impl Level {
    pub fn new(min_y: i32, height: i32) -> Level {
        Level {
            min_y,
            height,
            fast_lava: false,
            has_skylight: true,
            chunks: HashMap::new(),
            stamp: 0,
            motion_blocking: None,
            changes: None,
            block_entities: BlockEntities::default(),
            noclip: None,
            set_log: None,
            border: crate::border::WorldBorder::default(),
            weather: crate::weather::Weather::default(),
            biomes: BiomeInfo::default(),
        }
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn fast_lava(&self) -> bool {
        self.fast_lava
    }

    pub fn set_fast_lava(&mut self, fast: bool) {
        self.fast_lava = fast;
    }

    pub fn has_skylight(&self) -> bool {
        self.has_skylight
    }

    pub fn set_has_skylight(&mut self, has: bool) {
        self.has_skylight = has;
    }

    /// Which block states count in the motion-blocking heightmap: a bit per state id
    /// (the `blocks_motion_in_heightmap` tag, or a fluid). Chunks loaded from then on
    /// without a heightmap of their own get one by a scan, and block changes keep the
    /// heights up to date.
    pub fn set_motion_blocking(&mut self, states: Arc<Vec<u64>>) {
        self.motion_blocking = Some(states);
    }

    pub fn has_chunk(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x, z))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// A chunk as it is now. The planes are shared: holding it costs no copy until the
    /// level changes one of them.
    pub fn column(&self, x: i32, z: i32) -> Option<&Arc<Column>> {
        self.chunks.get(&(x, z))
    }

    /// The loaded chunks' positions, in no order.
    pub fn chunk_positions(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.chunks.keys().copied()
    }

    /// The last stamp a change took: every plane's stamp is at most this.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// Starts noting what changes, for [`crate::scene::Scene::publish_level`]. `capacity`
    /// bounds the sections noted between two publications; past it the note is that
    /// everything has to be looked at again.
    pub fn track_changes(&mut self, capacity: usize) {
        let mut changes = Changes::new(capacity);
        // What is loaded already has never been published.
        for &(x, z) in self.chunks.keys() {
            changes.column_changed(x, z);
            changes.mark_column(x, z, dirty::LOADED);
        }
        self.changes = Some(Box::new(changes));
    }

    /// What changed since the last call (none without [`Level::track_changes`]).
    pub fn take_changes(&mut self) -> Option<Changes> {
        let c = self.changes.as_mut()?;
        let capacity = c.capacity();
        Some(std::mem::replace(c, Changes::new(capacity)))
    }

    fn next_stamp(&mut self) -> u64 {
        self.stamp += 1;
        self.stamp
    }

    /// The lowest section's y and the number of sections.
    fn section_range(&self) -> (i32, i32) {
        (self.min_y >> 4, self.height >> 4)
    }

    /// Notes that a chunk came, went or was replaced: its own sections, and the
    /// neighbouring chunks' whose faces towards it have to be looked at again.
    fn note_column(&mut self, x: i32, z: i32, flags: u8) {
        let Some(c) = &mut self.changes else {
            return;
        };
        c.column_changed(x, z);
        c.mark_column(x, z, flags);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if (dx, dz) != (0, 0) {
                    c.mark_column(x + dx, z + dz, dirty::NEIGHBOUR);
                }
            }
        }
    }

    /// Notes a change inside section `section` (its y an index from the level's lowest
    /// section, which a light section outside the level's height falls outside of):
    /// that section with `flags`, and with `halo` the sections around it, or with
    /// `at` (a block of it) only those the block touches (`setBlockDirty`: every section
    /// within one block).
    fn note_section(&mut self, x: i32, index: i32, z: i32, flags: u8, at: Option<[i32; 3]>) {
        let (min_section, count) = self.section_range();
        let Some(c) = &mut self.changes else {
            return;
        };
        c.column_changed(x, z);
        let urgent = flags & dirty::URGENT;
        for dy in -1..=1 {
            let i = index + dy;
            if i < 0 || i >= count {
                continue;
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let own = (dx, dy, dz) == (0, 0, 0);
                    if let Some(p) = at
                        && !own
                    {
                        // Only the sections within one block of `p`.
                        let near = |v: i32, d: i32| match d {
                            -1 => v & 15 == 0,
                            1 => v & 15 == 15,
                            _ => true,
                        };
                        if !(near(p[0], dx) && near(p[1], dy) && near(p[2], dz)) {
                            continue;
                        }
                    }
                    let f = if own {
                        flags
                    } else {
                        dirty::NEIGHBOUR | urgent
                    };
                    c.mark_section([x + dx, min_section + i, z + dz], f);
                }
            }
        }
    }

    pub fn forget_chunk(&mut self, x: i32, z: i32) {
        if self.chunks.remove(&(x, z)).is_some() {
            self.note_column(x, z, dirty::UNLOADED);
        }
        self.block_entities.clear_chunk(x, z);
    }

    fn insert_column(&mut self, x: i32, z: i32, sections: Vec<(BlockPlane, BiomePlane)>) {
        let stamp = self.next_stamp();
        let n = sections.len();
        let column = Column {
            generation: stamp,
            sections: sections
                .into_iter()
                .map(|(blocks, biomes)| SectionPlanes {
                    blocks,
                    biomes,
                    blocks_rev: stamp,
                    biomes_rev: stamp,
                })
                .collect(),
            light: (0..n + 2)
                .map(|_| LightPlanes {
                    rev: stamp,
                    ..LightPlanes::default()
                })
                .collect(),
            heights: None,
            block_entities: Arc::new(Vec::new()),
            block_entities_rev: stamp,
        };
        self.chunks.insert((x, z), Arc::new(column));
        self.note_column(x, z, dirty::LOADED);
    }

    /// An empty chunk of air (for tests and building levels by hand).
    pub fn add_empty_chunk(&mut self, x: i32, z: i32) {
        let n = (self.height / 16) as usize;
        let biome = self.biomes.empty_chunk_biome;
        self.insert_column(
            x,
            z,
            vec![(BlockPlane::Uniform(0), BiomePlane::Uniform(biome)); n],
        );
        self.scan_heights(x, z);
    }

    /// Reads a chunk's sections (`LevelChunkSection.read`, one per 16 blocks of the
    /// level's height): block count, fluid count, block states, biomes.
    /// `biome_bits` is the biome registry's global palette width. The chunk has no
    /// light and no block entity data: [`Level::load_chunk_packet`] takes those too.
    pub fn load_chunk(
        &mut self,
        x: i32,
        z: i32,
        data: &[u8],
        biome_bits: u32,
    ) -> Result<(), WireError> {
        self.load_chunk_packet(
            x,
            z,
            ChunkPacket {
                heightmaps: &[],
                data,
                block_entities: &[],
                light: &LightData::default(),
            },
            biome_bits,
        )
    }

    /// A chunk from its packet (`replaceWithPacketData`, then the packet's light):
    /// whatever was loaded there is replaced whole.
    pub fn load_chunk_packet(
        &mut self,
        x: i32,
        z: i32,
        packet: ChunkPacket<'_>,
        biome_bits: u32,
    ) -> Result<(), WireError> {
        let n = (self.height / 16) as usize;
        let state_bits = global_bits(ruthenium_data::v26_3::blocks::STATE_COUNT);
        let mut r = Reader::new(packet.data);
        let mut sections = Vec::with_capacity(n);
        for _ in 0..n {
            r.bytes(4)?; // non-empty block count, fluid count
            let blocks = read_states(&mut r, state_bits)?;
            sections.push((blocks, read_biomes(&mut r, biome_bits)?));
        }
        let heights = packet
            .heightmaps
            .iter()
            .find(|(kind, _)| *kind == HEIGHTMAP_MOTION_BLOCKING)
            .and_then(|(_, longs)| unpack_heights(longs, self.min_y, self.height));
        self.insert_column(x, z, sections);
        match heights {
            Some(h) => {
                if let Some(c) = self.chunks.get_mut(&(x, z)) {
                    Arc::make_mut(c).heights = Some(Arc::new(h));
                }
            }
            None => self.scan_heights(x, z),
        }
        // `replaceWithPacketData`: the chunk's block entities go, and the ones the
        // packet lists are made again (`getBlockEntity` with IMMEDIATE). The server lists
        // every shulker box's, so a box is all it takes.
        self.block_entities.clear_chunk(x, z);
        for (i, section) in self.chunks[&(x, z)].sections.iter().enumerate() {
            let y0 = self.min_y + i as i32 * 16;
            let mut found = Vec::new();
            match &section.blocks {
                BlockPlane::Uniform(s) => {
                    if kind_of(u32::from(*s)) == Some(Kind::ShulkerBox) {
                        found.extend((0..4096).map(|c| (c, *s)));
                    }
                }
                BlockPlane::Dense(a) => found.extend(
                    a.iter()
                        .enumerate()
                        .filter(|(_, s)| kind_of(u32::from(**s)) == Some(Kind::ShulkerBox))
                        .map(|(c, s)| (c, *s)),
                ),
            }
            for (c, s) in found {
                let pos = [
                    x * 16 + (c & 15) as i32,
                    y0 + (c >> 8) as i32,
                    z * 16 + (c >> 4 & 15) as i32,
                ];
                self.block_entities.on_block_changed(pos, 0, u32::from(s));
            }
        }
        if !packet.block_entities.is_empty()
            && let Some(c) = self.chunks.get_mut(&(x, z))
        {
            let records = packet
                .block_entities
                .iter()
                .map(|e| BlockEntityRecord {
                    pos: [
                        x * 16 + i32::from(e.xz >> 4),
                        i32::from(e.y),
                        z * 16 + i32::from(e.xz & 15),
                    ],
                    kind: e.kind,
                    tag: e.tag.clone().map(Arc::new),
                })
                .collect();
            Arc::make_mut(c).block_entities = Arc::new(records);
        }
        self.apply_light(x, z, packet.light, false);
        Ok(())
    }

    /// New light for some of a chunk's light sections (`applyLightData`): a section in
    /// a mask takes the next array, a section in an empty mask goes dark, the others
    /// keep what they had. Nothing where no chunk is loaded.
    pub fn update_light(&mut self, x: i32, z: i32, light: &LightData) {
        self.apply_light(x, z, light, true);
    }

    fn apply_light(&mut self, x: i32, z: i32, light: &LightData, note: bool) {
        let Some(count) = self.chunks.get(&(x, z)).map(|c| c.light.len()) else {
            return;
        };
        let (mut sky, mut block) = (light.sky.iter(), light.block.iter());
        let pick = |mask: &[u64],
                    empty: &[u64],
                    arrays: &mut std::slice::Iter<'_, Arc<[u8; LIGHT_BYTES]>>,
                    i: usize| {
            if LightData::bit(mask, i) {
                // A mask bit without its array: the packet is short; the section keeps
                // what it had.
                arrays.next().map(|a| LightPlane::Dense(a.clone()))
            } else if LightData::bit(empty, i) {
                Some(LightPlane::Dark)
            } else {
                None
            }
        };
        for i in 0..count {
            let new_sky = pick(&light.sky_mask, &light.empty_sky_mask, &mut sky, i);
            let new_block = pick(&light.block_mask, &light.empty_block_mask, &mut block, i);
            if new_sky.is_none() && new_block.is_none() {
                continue;
            }
            let stamp = self.next_stamp();
            let Some(c) = self.chunks.get_mut(&(x, z)) else {
                return;
            };
            let l = &mut Arc::make_mut(c).light[i];
            if let Some(p) = new_sky {
                l.sky = p;
            }
            if let Some(p) = new_block {
                l.block = p;
            }
            l.rev = stamp;
            if note {
                // `setSectionDirtyWithNeighbors`.
                self.note_section(x, i as i32 - 1, z, dirty::LIGHT, None);
            } else if let Some(ch) = &mut self.changes {
                ch.column_changed(x, z);
            }
        }
    }

    /// A block entity's data from the server (`handleBlockEntityData`), kept for
    /// drawing; nothing where no chunk is loaded. Data for another type than the one
    /// kept there replaces it.
    pub fn set_block_entity_data(&mut self, pos: [i32; 3], kind: i32, tag: Option<nbt::Tag>) {
        let key = (pos[0] >> 4, pos[2] >> 4);
        if !self.chunks.contains_key(&key) {
            return;
        }
        let stamp = self.next_stamp();
        let Some(c) = self.chunks.get_mut(&key) else {
            return;
        };
        let c = Arc::make_mut(c);
        let record = BlockEntityRecord {
            pos,
            kind,
            tag: tag.map(Arc::new),
        };
        let list = Arc::make_mut(&mut c.block_entities);
        match list.iter_mut().find(|e| e.pos == pos) {
            Some(e) => *e = record,
            None => list.push(record),
        }
        c.block_entities_rev = stamp;
        let index = (pos[1] - self.min_y) >> 4;
        self.note_block_entity(key, index);
    }

    fn note_block_entity(&mut self, key: (i32, i32), index: i32) {
        let (min_section, count) = self.section_range();
        if let Some(ch) = &mut self.changes {
            ch.column_changed(key.0, key.1);
            if (0..count).contains(&index) {
                ch.mark_section([key.0, min_section + index, key.1], dirty::BLOCK_ENTITIES);
            }
        }
    }

    /// `handleChunksBiomes`: a loaded chunk's biomes replaced (its sections' biome
    /// containers in order); chunks not loaded are left alone.
    pub fn load_biomes(
        &mut self,
        x: i32,
        z: i32,
        data: &[u8],
        biome_bits: u32,
    ) -> Result<(), WireError> {
        if !self.chunks.contains_key(&(x, z)) {
            return Ok(());
        }
        let stamp = self.next_stamp();
        let Some(c) = self.chunks.get_mut(&(x, z)) else {
            return Ok(());
        };
        let mut r = Reader::new(data);
        for s in &mut Arc::make_mut(c).sections {
            s.biomes = read_biomes(&mut r, biome_bits)?;
            s.biomes_rev = stamp;
        }
        self.note_column(x, z, dirty::BIOMES);
        Ok(())
    }

    /// Sets the biome of every cell whose corner lies between two blocks (`fillbiome`,
    /// in loaded chunks).
    pub fn fill_biome(&mut self, from: [i32; 3], to: [i32; 3], biome: u16) {
        let lo: [i32; 3] = std::array::from_fn(|a| from[a].min(to[a]) >> 2);
        let hi: [i32; 3] = std::array::from_fn(|a| from[a].max(to[a]) >> 2);
        let (min_q, max_q) = (self.min_y >> 2, (self.min_y >> 2) + (self.height >> 2) - 1);
        let min_y = self.min_y;
        let stamp = self.next_stamp();
        let mut touched: Vec<(i32, i32)> = Vec::new();
        for qx in lo[0]..=hi[0] {
            for qz in lo[2]..=hi[2] {
                let key = (qx >> 2, qz >> 2);
                let Some(c) = self.chunks.get_mut(&key) else {
                    continue;
                };
                for qy in lo[1].max(min_q)..=hi[1].min(max_q) {
                    let s = (((qy << 2) >> 4) - (min_y >> 4)) as usize;
                    let same = c.sections[s].biomes.get(qx & 3, qy & 3, qz & 3) == biome;
                    if same {
                        continue;
                    }
                    let section = &mut Arc::make_mut(c).sections[s];
                    section.biomes.set(qx & 3, qy & 3, qz & 3, biome);
                    section.biomes_rev = stamp;
                    if !touched.contains(&key) {
                        touched.push(key);
                    }
                }
            }
        }
        for (x, z) in touched {
            self.note_column(x, z, dirty::BIOMES);
        }
    }

    /// `getNoiseBiome`: the biome of a quarter-chunk cell (its y clamped into the
    /// level); plains where no chunk is.
    pub fn noise_biome(&self, cell: [i32; 3]) -> Climate {
        let Some(c) = self.chunks.get(&(cell[0] >> 2, cell[2] >> 2)) else {
            return Climate::PLAINS;
        };
        let min_q = self.min_y >> 2;
        let qy = cell[1].clamp(min_q, min_q + (self.height >> 2) - 1);
        let s = (((qy << 2) >> 4) - (self.min_y >> 4)) as usize;
        let id = c.sections[s].biomes.get(cell[0] & 3, qy & 3, cell[2] & 3);
        self.biomes
            .climates
            .get(usize::from(id))
            .copied()
            .unwrap_or(Climate::PLAINS)
    }

    /// `getBiome`: the biome a block is in.
    pub fn biome(&self, pos: [i32; 3]) -> Climate {
        self.noise_biome(crate::biome::zoomed_cell(self.biomes.seed, pos))
    }

    /// The highest of a column's loaded sections that isn't all air, as the index of
    /// its section; `None` where no chunk is or all is air.
    pub(crate) fn top_filled_section(
        &self,
        x: i32,
        z: i32,
        is_air: impl Fn(u32) -> bool,
    ) -> Option<Option<usize>> {
        let c = self.chunks.get(&(x >> 4, z >> 4))?;
        Some(c.sections.iter().rposition(|s| match &s.blocks {
            BlockPlane::Uniform(s) => !is_air(u32::from(*s)),
            BlockPlane::Dense(_) => true,
        }))
    }

    /// The state at a block position: air outside the level or where no chunk is.
    pub fn block(&self, x: i32, y: i32, z: i32) -> u32 {
        if y < self.min_y || y >= self.min_y + self.height {
            return 0;
        }
        let Some(c) = self.chunks.get(&(x >> 4, z >> 4)) else {
            return 0;
        };
        let s = ((y - self.min_y) >> 4) as usize;
        u32::from(
            c.sections[s]
                .blocks
                .get(x & 15, (y - self.min_y) & 15, z & 15),
        )
    }

    /// The light section a block's y is in, as an index into [`Column::light`], when
    /// it is one the level keeps.
    fn light_index(&self, y: i32) -> Option<usize> {
        let i = ((y - self.min_y) >> 4) + 1;
        (0..(self.height >> 4) + 2)
            .contains(&i)
            .then_some(i as usize)
    }

    /// The sky light at a block as the server last sent it: full light outside the
    /// level's light sections and where no chunk is, as for the client's light engine.
    pub fn sky_light(&self, x: i32, y: i32, z: i32) -> u8 {
        if !self.has_skylight {
            return 0;
        }
        match (self.chunks.get(&(x >> 4, z >> 4)), self.light_index(y)) {
            (Some(c), Some(i)) => c.sky_light(true, i, x & 15, y & 15, z & 15),
            _ => 15,
        }
    }

    /// The block light at a block as the server last sent it.
    pub fn block_light(&self, x: i32, y: i32, z: i32) -> u8 {
        match (self.chunks.get(&(x >> 4, z >> 4)), self.light_index(y)) {
            (Some(c), Some(i)) => c.block_light(i, x & 15, y & 15, z & 15),
            _ => 0,
        }
    }

    /// The y above the highest motion-blocking block of a column of blocks
    /// (`getHeight(MOTION_BLOCKING, x, z)`), where the level keeps it.
    pub fn motion_blocking_top(&self, x: i32, z: i32) -> Option<i32> {
        let h = self.chunks.get(&(x >> 4, z >> 4))?.heights.as_ref()?;
        Some(i32::from(h[((x & 15) | (z & 15) << 4) as usize]))
    }

    fn counts_in_heightmap(table: &[u64], state: u32) -> bool {
        table
            .get((state / 64) as usize)
            .is_some_and(|w| w >> (state % 64) & 1 == 1)
    }

    /// Makes a chunk's heights by looking down each column of blocks, when the states
    /// that count are known.
    fn scan_heights(&mut self, x: i32, z: i32) {
        let Some(table) = self.motion_blocking.clone() else {
            return;
        };
        let (min_y, top) = (self.min_y, self.min_y + self.height);
        let Some(c) = self.chunks.get_mut(&(x, z)) else {
            return;
        };
        let mut heights = [min_y as i16; 256];
        for (i, h) in heights.iter_mut().enumerate() {
            let (bx, bz) = ((i & 15) as i32, (i >> 4) as i32);
            *h = Self::first_counting_below(c, &table, min_y, bx, top, bz) as i16;
        }
        Arc::make_mut(c).heights = Some(Arc::new(heights));
    }

    /// The y above the highest counting block below `from` in a column of blocks of a
    /// chunk, the level's bottom when there is none.
    fn first_counting_below(
        c: &Column,
        table: &[u64],
        min_y: i32,
        x: i32,
        from: i32,
        z: i32,
    ) -> i32 {
        let mut y = from - 1;
        while y >= min_y {
            let s = &c.sections[((y - min_y) >> 4) as usize].blocks;
            // A section of one state that doesn't count is skipped whole.
            if let BlockPlane::Uniform(state) = s
                && !Self::counts_in_heightmap(table, u32::from(*state))
            {
                y = (((y - min_y) >> 4) << 4) + min_y - 1;
                continue;
            }
            if Self::counts_in_heightmap(table, u32::from(s.get(x, (y - min_y) & 15, z))) {
                return y + 1;
            }
            y -= 1;
        }
        min_y
    }

    /// `Heightmap.update`: a block changed at the top of its column of blocks or above.
    fn update_height(c: &mut Column, table: Option<&[u64]>, min_y: i32, pos: [i32; 3], state: u32) {
        let Some(table) = table else {
            // Nothing says which states count: the heights can't be kept.
            c.heights = None;
            return;
        };
        let i = ((pos[0] & 15) | (pos[2] & 15) << 4) as usize;
        let Some(first_free) = c.heights.as_ref().map(|h| i32::from(h[i])) else {
            return;
        };
        let new = if pos[1] <= first_free - 2 {
            return;
        } else if Self::counts_in_heightmap(table, state) {
            if pos[1] < first_free {
                return;
            }
            pos[1] + 1
        } else if first_free - 1 == pos[1] {
            // The top block went: the next one down is the top now.
            Self::first_counting_below(c, table, min_y, pos[0] & 15, pos[1], pos[2] & 15)
        } else {
            return;
        };
        if let Some(h) = &mut c.heights {
            Arc::make_mut(h)[i] = new as i16;
        }
    }

    /// Sets a block (a block update, or building a level by hand). No effect where no
    /// chunk is loaded or outside the level.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u32) {
        self.put_block(x, y, z, state, false);
    }

    fn put_block(&mut self, x: i32, y: i32, z: i32, state: u32, urgent: bool) {
        if y < self.min_y || y >= self.min_y + self.height {
            return;
        }
        let min_y = self.min_y;
        let old = self.block(x, y, z);
        if old == state || !self.chunks.contains_key(&(x >> 4, z >> 4)) {
            return;
        }
        let stamp = self.next_stamp();
        let table = self.motion_blocking.clone();
        let Some(c) = self.chunks.get_mut(&(x >> 4, z >> 4)) else {
            return;
        };
        let c = Arc::make_mut(c);
        let s = ((y - min_y) >> 4) as usize;
        c.sections[s]
            .blocks
            .set(x & 15, (y - min_y) & 15, z & 15, state as u16);
        c.sections[s].blocks_rev = stamp;
        Self::update_height(
            c,
            table.as_deref().map(Vec::as_slice),
            min_y,
            [x, y, z],
            state,
        );
        // `LevelChunk.setBlockState`: another block there takes the block entity away.
        let mut entity_gone = false;
        if c.block_entities.iter().any(|e| e.pos == [x, y, z]) && !same_block(old, state) {
            Arc::make_mut(&mut c.block_entities).retain(|e| e.pos != [x, y, z]);
            c.block_entities_rev = stamp;
            entity_gone = true;
        }
        let flags = dirty::BLOCKS
            | if urgent { dirty::URGENT } else { 0 }
            | if entity_gone {
                dirty::BLOCK_ENTITIES
            } else {
                0
            };
        self.note_section(x >> 4, s as i32, z >> 4, flags, Some([x, y - min_y, z]));
        self.block_entities.on_block_changed([x, y, z], old, state);
    }

    /// Sets a block as `LevelChunk.setBlockState` does: the state it replaced, or `None`
    /// when nothing changed (the same state, no chunk loaded there, or outside the
    /// level). The change is the client's own (a predicted action, a piston's move, or
    /// the server putting a prediction right): it is noted as one to draw first.
    pub fn replace_block(&mut self, x: i32, y: i32, z: i32, state: u32) -> Option<u32> {
        if y < self.min_y || y >= self.min_y + self.height || !self.has_chunk(x >> 4, z >> 4) {
            return None;
        }
        let old = self.block(x, y, z);
        if old == state {
            return None;
        }
        self.put_block(x, y, z, state, true);
        Some(old)
    }

    /// The block collision shapes that `box` could meet, in the order the game's
    /// `BlockCollisions` yields them: a cursor over the box grown by one block, x
    /// fastest, then y, then z; corner cells skipped, edge cells only for shapes
    /// taller or wider than a block.
    pub fn block_collisions<'a>(&'a self, blocks: &'a Blocks, b: &Aabb) -> Vec<Placed<'a>> {
        self.block_collisions_at(blocks, b)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    }

    /// [`Level::block_collisions`] with each shape's block position.
    pub fn block_collisions_at<'a>(
        &'a self,
        blocks: &'a Blocks,
        b: &Aabb,
    ) -> Vec<([i32; 3], Placed<'a>)> {
        self.block_collisions_for(blocks, b, None, None)
    }

    /// [`Level::block_collisions_at`] for an entity (`BlockCollisions` with its
    /// collision context: scaffolding's shape depends on it). With `suffocating`, only
    /// the blocks that suffocate by those tags (`collidesWithSuffocatingBlock`).
    /// Moving pistons and shulker boxes have their block entity's shape: moving pistons
    /// count even in the cells by the box's edges, as the game's cursor lets them.
    pub fn block_collisions_for<'a>(
        &'a self,
        blocks: &'a Blocks,
        b: &Aabb,
        suffocating: Option<&crate::blocks::BlockTags>,
        who: Option<crate::blocks::Who>,
    ) -> Vec<([i32; 3], Placed<'a>)> {
        let lo = |v: f64| ruthenium_javamath::mth::floor(v - EPSILON) - 1;
        let hi = |v: f64| ruthenium_javamath::mth::floor(v + EPSILON) + 1;
        let (x0, x1) = (lo(b.min[0]), hi(b.max[0]));
        let (y0, y1) = (lo(b.min[1]), hi(b.max[1]));
        let (z0, z1) = (lo(b.min[2]), hi(b.max[2]));
        let mut out: Vec<([i32; 3], Placed<'a>)> = Vec::new();
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let face = i32::from(x == x0 || x == x1)
                        + i32::from(y == y0 || y == y1)
                        + i32::from(z == z0 || z == z1);
                    if face == 3 {
                        continue;
                    }
                    let state = self.block(x, y, z);
                    if state == 0 {
                        continue;
                    }
                    let kind = kind_of(state);
                    if suffocating
                        .is_some_and(|tags| !self.suffocating(blocks, state, [x, y, z], tags))
                    {
                        continue;
                    }
                    if face == 1 && !blocks.large_shape(state) {
                        continue;
                    }
                    // Face type 2 cells count only for moving pistons.
                    if face == 2 && kind != Some(Kind::MovingPiston) {
                        continue;
                    }
                    let at = [f64::from(x), f64::from(y), f64::from(z)];
                    let placed = match kind.map(|k| {
                        (
                            k,
                            self.block_entities
                                .collision_shape([x, y, z], k, self.noclip),
                        )
                    }) {
                        Some((_, BlockEntityShape::Shape(shape))) => Placed {
                            shape,
                            offset: None,
                            pos: at,
                        },
                        Some((_, BlockEntityShape::Empty))
                        | Some((Kind::MovingPiston, BlockEntityShape::Missing)) => continue,
                        _ => match blocks
                            .collision_for(state, [x, y, z], who)
                            .or_else(|| self.lava_surface(blocks, state, [x, y, z], who))
                        {
                            Some(p) => p,
                            None => continue,
                        },
                    };
                    // `shape == Shapes.block()`: a block entity's full cube is that shape too.
                    let identity = if kind.is_some() {
                        placed.shape.is_block()
                    } else {
                        blocks.is_full_block(state) && placed.shape.is_block()
                    };
                    if identity {
                        let cell = Aabb {
                            min: [f64::from(x), f64::from(y), f64::from(z)],
                            max: [f64::from(x) + 1.0, f64::from(y) + 1.0, f64::from(z) + 1.0],
                        };
                        if b.intersects(&cell) {
                            out.push(([x, y, z], placed));
                        }
                    } else if placed.overlaps(b) {
                        out.push(([x, y, z], placed));
                    }
                }
            }
        }
        out
    }

    /// `LiquidBlock.getCollisionShape` for an entity that stands on lava (a strider):
    /// a lava source with no lava above is solid to its lower half for one above that.
    fn lava_surface<'a>(
        &self,
        blocks: &'a Blocks,
        state: u32,
        pos: [i32; 3],
        who: Option<crate::blocks::Who>,
    ) -> Option<Placed<'a>> {
        let who = who.filter(|w| w.stands_on_lava)?;
        if blocks.name(state) != "minecraft:lava" {
            return None;
        }
        let source =
            ruthenium_data::v26_3::blocks::BlockState::from_id(state)?.value("level") == Some("0");
        let above = self.block(pos[0], pos[1] + 1, pos[2]);
        if !source || blocks.name(above) == "minecraft:lava" || !who.is_above(pos[1], 0.5) {
            return None;
        }
        Some(Placed {
            shape: blocks.lava_surface(),
            offset: None,
            pos: [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
        })
    }
}

impl Level {
    /// `isSuffocating`: an open shulker box doesn't (`NOT_CLOSED_SHULKER`); the rest as
    /// [`Blocks::suffocating`].
    fn suffocating(
        &self,
        blocks: &Blocks,
        state: u32,
        pos: [i32; 3],
        tags: &crate::blocks::BlockTags,
    ) -> bool {
        if let Some(s) = self.block_entities.shulker(pos)
            && kind_of(state) == Some(Kind::ShulkerBox)
        {
            return s.is_closed();
        }
        blocks.suffocating(state, tags)
    }
}

/// Whether two states are of one block.
fn same_block(a: u32, b: u32) -> bool {
    use ruthenium_data::v26_3::blocks::BlockState;
    match (BlockState::from_id(a), BlockState::from_id(b)) {
        (Some(a), Some(b)) => a.block().name() == b.block().name(),
        _ => a == b,
    }
}

/// A heightmap's heights from its packed longs (`SimpleBitStorage` of 256 entries of
/// `ceillog2(height + 1)` bits, none spanning two longs): each is the count of blocks
/// from the level's bottom up to the top of the column of blocks.
fn unpack_heights(longs: &[i64], min_y: i32, height: i32) -> Option<[i16; 256]> {
    let bits = global_bits(height as usize + 1).max(1);
    let per_long = (64 / bits) as usize;
    if longs.len() < 256_usize.div_ceil(per_long) {
        return None;
    }
    let mask = (1u64 << bits) - 1;
    let mut out = [0i16; 256];
    for (i, h) in out.iter_mut().enumerate() {
        let v = (longs[i / per_long] as u64 >> ((i % per_long) as u32 * bits)) & mask;
        *h = (min_y + (v as i32).min(height)) as i16;
    }
    Some(out)
}

/// A block-state paletted container, expanded.
fn read_states(r: &mut Reader<'_>, global: u32) -> Result<BlockPlane, WireError> {
    let bits = u32::from(r.u8()?);
    // `Strategy.createForBlockStates`: 0 single, 1-4 linear at 4 bits, 5-8 hashmap,
    // more: the global palette at its own width in memory.
    let (stored, palette) = match bits {
        0 => {
            let v = r.varint()?;
            return Ok(BlockPlane::Uniform(v as u16));
        }
        1..=8 => {
            let n = r.count(4096)?;
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                p.push(r.varint()? as u16);
            }
            (bits.max(4), Some(p))
        }
        _ => (global, None),
    };
    let per_long = 64 / stored;
    let longs = 4096_u32.div_ceil(per_long) as usize;
    let raw = r.bytes(longs * 8)?;
    let mask = (1u64 << stored) - 1;
    let mut out = [0u16; 4096];
    for (i, v) in out.iter_mut().enumerate() {
        let cell = i / per_long as usize;
        let word = u64::from_be_bytes(raw[cell * 8..cell * 8 + 8].try_into().unwrap());
        let shift = (i - cell * per_long as usize) as u32 * stored;
        let index = (word >> shift & mask) as usize;
        *v = match &palette {
            Some(p) => *p.get(index).ok_or(WireError::Invalid("palette index"))?,
            None => index as u16,
        };
    }
    Ok(BlockPlane::Dense(Arc::new(out)))
}

/// Reads a biome paletted container (`Strategy.createForBiomes`: 0 single, 1-3
/// linear, more: global).
fn read_biomes(r: &mut Reader<'_>, global: u32) -> Result<BiomePlane, WireError> {
    let bits = u32::from(r.u8()?);
    let (stored, palette) = match bits {
        0 => {
            let id = r.varint()?;
            return Ok(BiomePlane::Uniform(
                u16::try_from(id).map_err(|_| WireError::Invalid("biome id"))?,
            ));
        }
        1..=3 => {
            let n = r.count(64)?;
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                p.push(u16::try_from(r.varint()?).map_err(|_| WireError::Invalid("biome id"))?);
            }
            (bits, Some(p))
        }
        _ => (global.max(1), None),
    };
    let per_long = 64 / stored;
    let raw = r.bytes(64_u32.div_ceil(per_long) as usize * 8)?;
    let mask = (1u64 << stored) - 1;
    let mut out = [0u16; 64];
    for (i, v) in out.iter_mut().enumerate() {
        let cell = i / per_long as usize;
        let word = u64::from_be_bytes(raw[cell * 8..cell * 8 + 8].try_into().unwrap());
        let index = (word >> ((i - cell * per_long as usize) as u32 * stored) & mask) as usize;
        *v = match &palette {
            Some(p) => *p
                .get(index)
                .ok_or(WireError::Invalid("biome palette index"))?,
            None => index as u16,
        };
    }
    Ok(BiomePlane::Dense(Arc::new(out)))
}

/// Chunk and light data for tests, here and in the modules that publish levels.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use ruthenium_protocol::wire;

    /// A chunk's section data: `sections` sections, each all of the state `fill` gives
    /// for its index, in biome 0.
    pub fn chunk_data(sections: usize, fill: impl Fn(usize) -> u16) -> Vec<u8> {
        let mut data = Vec::new();
        for i in 0..sections {
            data.extend([0, 0, 0, 0, 0]);
            wire::write_varint(&mut data, i32::from(fill(i)));
            data.extend([0, 0]);
        }
        data
    }

    /// Light for the light sections listed: `Some(v)` an array with every nibble `v`,
    /// `None` the section marked dark.
    pub fn light(sky: &[(usize, Option<u8>)], block: &[(usize, Option<u8>)]) -> LightData {
        let mut l = LightData::default();
        let mut put = |list: &[(usize, Option<u8>)], block: bool| {
            let mut list = list.to_vec();
            list.sort_by_key(|(i, _)| *i);
            for (i, v) in list {
                let (mask, empty, arrays) = if block {
                    (&mut l.block_mask, &mut l.empty_block_mask, &mut l.block)
                } else {
                    (&mut l.sky_mask, &mut l.empty_sky_mask, &mut l.sky)
                };
                let set = |m: &mut Vec<u64>| {
                    if m.len() <= i / 64 {
                        m.resize(i / 64 + 1, 0);
                    }
                    m[i / 64] |= 1 << (i % 64);
                };
                match v {
                    Some(v) => {
                        set(mask);
                        arrays.push(Arc::new([v | v << 4; LIGHT_BYTES]));
                    }
                    None => set(empty),
                }
            }
        };
        put(sky, false);
        put(block, true);
        l
    }

    /// A motion-blocking table in which exactly `states` count.
    pub fn counting(states: &[u32]) -> Arc<Vec<u64>> {
        let mut bits = vec![0u64; ruthenium_data::v26_3::blocks::STATE_COUNT.div_ceil(64)];
        for s in states {
            bits[(s / 64) as usize] |= 1 << (s % 64);
        }
        Arc::new(bits)
    }

    /// A level 64 blocks high from y -16 (4 sections, 6 light sections).
    pub fn small_level() -> Level {
        Level::new(-16, 64)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;

    const STONE: u32 = 1;

    #[test]
    fn light_is_kept_as_sent_dark_or_absent() {
        let mut level = small_level();
        let data = chunk_data(4, |i| if i == 0 { 1 } else { 0 });
        // Light sections: 0 below the level, 1..=4 the sections, 5 above. Sky: an
        // array of 9 at section index 1 (light 2), dark at light 1, nothing else.
        // Block: an array of 5 in the section below the level's bottom.
        let l = light(&[(2, Some(9)), (1, None)], &[(0, Some(5))]);
        let packet = ChunkPacket {
            heightmaps: &[],
            data: &data,
            block_entities: &[],
            light: &l,
        };
        level.load_chunk_packet(0, 0, packet, 6).unwrap();
        let c = level.column(0, 0).unwrap().clone();
        assert_eq!(c.light().len(), 6);
        assert!(matches!(c.light()[1].sky, LightPlane::Dark));
        assert!(matches!(c.light()[3].sky, LightPlane::Absent));
        // Section 0 (y -16..0) is dark, section 1 (y 0..16) has 9.
        assert_eq!(level.sky_light(3, -10, 3), 0);
        assert_eq!(level.sky_light(3, 5, 3), 9);
        // Above all data: full light. Below the dark section, the boundary section has
        // no data and reads the bottom layer of the next one with data: dark.
        assert_eq!(level.sky_light(3, 20, 3), 15);
        assert_eq!(level.sky_light(3, -20, 3), 0);
        assert_eq!(c.sky_light(true, 0, 3, 5, 3), 0);
        // Outside the light sections and where no chunk is: full light, no block light.
        assert_eq!(level.sky_light(3, 500, 3), 15);
        assert_eq!(level.sky_light(40, 5, 3), 15);
        assert_eq!(level.block_light(40, 5, 3), 0);
        // Block light: the boundary section below has 5, the others none.
        assert_eq!(level.block_light(3, -17, 3), 5);
        assert_eq!(level.block_light(3, -16, 3), 0);
        assert_eq!(c.section_light(0).rev, c.light()[1].rev);

        // An update names light sections 1 (now an array) and 2 (now dark) for sky;
        // the block light of section 0 isn't named and stays.
        let before = level.stamp();
        level.update_light(0, 0, &light(&[(1, Some(3)), (2, None)], &[]));
        assert_eq!(level.sky_light(3, -10, 3), 3);
        assert_eq!(level.sky_light(3, 5, 3), 0);
        assert_eq!(level.block_light(3, -17, 3), 5);
        let after = level.column(0, 0).unwrap();
        assert!(after.light()[1].rev > before && after.light()[2].rev > before);
        assert_eq!(after.light()[0].rev, c.light()[0].rev);
        assert!(after.light()[0].block.shares(&c.light()[0].block));
        // The copy taken before the update still reads the old light.
        assert_eq!(c.sky_light(true, 2, 3, 5, 3), 9);
        // Light for a chunk that isn't loaded goes nowhere.
        level.update_light(7, 7, &light(&[(1, Some(3))], &[]));
        assert!(!level.has_chunk(7, 7));

        // A dimension without sky light is dark whatever was sent.
        level.set_has_skylight(false);
        assert_eq!(level.sky_light(3, -10, 3), 0);
        assert_eq!(level.sky_light(3, 500, 3), 0);
    }

    #[test]
    fn a_nibble_per_block_in_the_data_layers_order() {
        let mut bytes = [0u8; LIGHT_BYTES];
        // Block (x 1, y 2, z 3): index 2 << 8 | 3 << 4 | 1 = 561, the high nibble of
        // byte 280; its neighbour at x 0 is the low nibble.
        bytes[280] = 0xC4;
        let p = LightPlane::Dense(Arc::new(bytes));
        assert_eq!(p.get(1, 2, 3), Some(12));
        assert_eq!(p.get(0, 2, 3), Some(4));
        assert_eq!(p.get(2, 2, 3), Some(0));
        assert_eq!(LightPlane::Dark.get(1, 2, 3), Some(0));
        assert_eq!(LightPlane::Absent.get(1, 2, 3), None);
    }

    #[test]
    fn heights_come_from_the_packet_or_a_scan_and_follow_block_changes() {
        // From the packet: 64 blocks high needs 7 bits, 9 entries a long.
        let mut longs = vec![0i64; 29];
        let mut put = |i: usize, v: u64| longs[i / 9] |= (v << ((i % 9) * 7)) as i64;
        put(0, 16); // x 0, z 0: 16 blocks from the bottom
        put(5 + 16 * 2, 64); // x 5, z 2: to the top
        let data = chunk_data(4, |i| if i == 0 { 1 } else { 0 });
        let mut level = small_level();
        let l = LightData::default();
        let packet = ChunkPacket {
            heightmaps: &[(1, vec![0; 29]), (4, longs)],
            data: &data,
            block_entities: &[],
            light: &l,
        };
        level.load_chunk_packet(0, 0, packet, 6).unwrap();
        assert_eq!(level.motion_blocking_top(0, 0), Some(0));
        assert_eq!(level.motion_blocking_top(5, 2), Some(48));
        assert_eq!(level.motion_blocking_top(1, 0), Some(-16));
        assert_eq!(level.motion_blocking_top(99, 0), None);
        // A block change with no table to judge it by: the heights are dropped.
        level.set_block(0, 0, 0, STONE);
        assert_eq!(level.motion_blocking_top(0, 0), None);

        // With the table, a chunk without a heightmap gets one by a scan (the bottom
        // section is stone: the top is y 0), and changes keep it.
        let mut level = small_level();
        level.set_motion_blocking(counting(&[STONE]));
        level.load_chunk(0, 0, &data, 6).unwrap();
        assert_eq!(level.motion_blocking_top(3, 3), Some(0));
        // A counting block higher up raises it; one that doesn't count (state 9)
        // changes nothing; a block under the top neither.
        level.set_block(3, 20, 3, STONE);
        assert_eq!(level.motion_blocking_top(3, 3), Some(21));
        level.set_block(3, 30, 3, 9);
        level.set_block(3, 10, 3, STONE);
        assert_eq!(level.motion_blocking_top(3, 3), Some(21));
        // The top block going: the next one down is found (y 10, then the stone).
        level.set_block(3, 20, 3, 0);
        assert_eq!(level.motion_blocking_top(3, 3), Some(11));
        level.set_block(3, 10, 3, 9);
        assert_eq!(level.motion_blocking_top(3, 3), Some(0));
        // Down to nothing.
        for y in -16..0 {
            level.set_block(3, y, 3, 0);
        }
        assert_eq!(level.motion_blocking_top(3, 3), Some(-16));
        assert_eq!(level.motion_blocking_top(4, 3), Some(0));
        // It agrees with a scan of every column of blocks.
        let scanned = |level: &Level, x: i32, z: i32| {
            (-16..48)
                .rev()
                .find(|&y| level.block(x, y, z) == STONE)
                .map_or(-16, |y| y + 1)
        };
        for (x, z) in [(3, 3), (4, 3), (0, 0), (15, 15)] {
            assert_eq!(level.motion_blocking_top(x, z), Some(scanned(&level, x, z)));
        }
        // An empty chunk made by hand starts at the bottom.
        level.add_empty_chunk(1, 0);
        assert_eq!(level.motion_blocking_top(16, 0), Some(-16));
    }

    #[test]
    fn block_entity_data_is_kept_until_another_block_takes_its_place() {
        use ruthenium_protocol::nbt::{Compound, Tag};
        let tag = |text: &str| {
            let mut c = Compound::new();
            c.put("text", Tag::String(text.into()));
            Tag::Compound(c)
        };
        let chest = Block::from_name("minecraft:chest").unwrap().default_state();
        let chest_west = chest.with("facing", "west").unwrap().id();
        let chest = chest.id();
        let mut level = small_level();
        let data = chunk_data(4, |_| 0);
        let listed = [
            ChunkBlockEntity {
                xz: 0x21,
                y: 5,
                kind: 7,
                tag: Some(tag("a")),
            },
            ChunkBlockEntity {
                xz: 0xF0,
                y: -3,
                kind: 2,
                tag: None,
            },
        ];
        let l = LightData::default();
        let packet = ChunkPacket {
            heightmaps: &[],
            data: &data,
            block_entities: &listed,
            light: &l,
        };
        level.load_chunk_packet(2, -1, packet, 6).unwrap();
        let c = level.column(2, -1).unwrap().clone();
        assert_eq!(c.block_entities().len(), 2);
        // x in the high nibble, z in the low, within chunk (2, -1).
        assert_eq!(c.block_entities()[0].pos, [34, 5, -15]);
        assert_eq!(c.block_entities()[1].pos, [47, -3, -16]);
        assert_eq!(c.block_entities()[0].tag.as_deref(), Some(&tag("a")));
        // New data replaces the record; data for a place without one adds it.
        level.set_block(40, 0, -8, chest);
        level.set_block_entity_data([34, 5, -15], 7, Some(tag("b")));
        level.set_block_entity_data([40, 0, -8], 2, Some(tag("c")));
        let now = level.column(2, -1).unwrap().clone();
        assert_eq!(now.block_entities().len(), 3);
        assert_eq!(now.block_entities()[0].tag.as_deref(), Some(&tag("b")));
        assert!(now.block_entities_rev() > c.block_entities_rev());
        // The copy from before still has the old data.
        assert_eq!(c.block_entities()[0].tag.as_deref(), Some(&tag("a")));
        // Another state of the same block keeps the record; another block drops it.
        level.set_block(40, 0, -8, chest_west);
        assert_eq!(level.column(2, -1).unwrap().block_entities().len(), 3);
        level.set_block(40, 0, -8, STONE);
        let gone = level.column(2, -1).unwrap();
        assert_eq!(gone.block_entities().len(), 2);
        assert!(gone.block_entities().iter().all(|e| e.pos != [40, 0, -8]));
        // Data for a chunk that isn't loaded goes nowhere; a reload starts over.
        level.set_block_entity_data([400, 0, 0], 7, None);
        level.load_chunk(2, -1, &data, 6).unwrap();
        assert!(level.column(2, -1).unwrap().block_entities().is_empty());
    }

    #[test]
    fn a_reloaded_chunk_is_another_generation_and_planes_are_shared_until_changed() {
        let mut level = small_level();
        let data = chunk_data(4, |i| if i == 0 { 1 } else { 0 });
        level.load_chunk(0, 0, &data, 6).unwrap();
        level.set_block(1, 1, 1, STONE);
        let first = level.column(0, 0).unwrap().clone();
        // A held copy doesn't see later changes, and only the changed plane is copied.
        level.set_block(1, 2, 1, STONE);
        let second = level.column(0, 0).unwrap().clone();
        assert_eq!(first.sections()[1].blocks.get(1, 2, 1), 0);
        assert_eq!(second.sections()[1].blocks.get(1, 2, 1), 1);
        assert!(
            !first.sections()[1]
                .blocks
                .shares(&second.sections()[1].blocks)
        );
        assert!(
            first.sections()[0]
                .blocks
                .shares(&second.sections()[0].blocks)
        );
        assert!(
            first.sections()[1]
                .biomes
                .shares(&second.sections()[1].biomes)
        );
        assert!(second.sections()[1].blocks_rev > first.sections()[1].blocks_rev);
        assert_eq!(
            second.sections()[0].blocks_rev,
            first.sections()[0].blocks_rev
        );
        assert_eq!(second.generation(), first.generation());
        // Setting the state a block already has changes nothing, stamps included.
        let stamp = level.stamp();
        level.set_block(1, 2, 1, STONE);
        assert_eq!(level.replace_block(1, 2, 1, STONE), None);
        assert_eq!(level.stamp(), stamp);
        // Unloaded and loaded again: same content, another generation.
        level.forget_chunk(0, 0);
        level.load_chunk(0, 0, &data, 6).unwrap();
        let third = level.column(0, 0).unwrap();
        assert!(third.generation() > second.generation());
        assert_eq!(level.block(1, 1, 1), 0);
        assert_eq!(level.chunk_positions().collect::<Vec<_>>(), [(0, 0)]);
    }

    use super::*;
    use ruthenium_data::v26_3::Block;
    use ruthenium_protocol::wire;

    #[test]
    fn decodes_single_and_paletted_sections() {
        let mut data = Vec::new();
        // Section 0: all stone (1).
        data.extend([0, 0, 0, 0, 0]);
        wire::write_varint(&mut data, 1);
        data.extend([0, 0]); // biomes: single 0
        // Section 1: palette [air, 9] at 4 bits, block (1, 2, 3) is 9.
        data.extend([0, 0, 0, 0, 4]);
        wire::write_varint(&mut data, 2);
        wire::write_varint(&mut data, 0);
        wire::write_varint(&mut data, 9);
        let mut longs = vec![0u64; 256];
        let i = (2 << 8) | (3 << 4) | 1;
        longs[i / 16] |= 1 << ((i % 16) * 4);
        for l in longs {
            data.extend(l.to_be_bytes());
        }
        data.extend([0, 0]);
        let mut level = Level::new(-64, 32);
        level.load_chunk(2, -1, &data, 7).unwrap();
        assert_eq!(level.block(32, -64, -16), 1);
        assert_eq!(level.block(47, -60, -1), 1);
        assert_eq!(level.block(33, -64 + 16 + 2, -16 + 3), 9);
        assert_eq!(level.block(32, -64 + 16 + 2, -16 + 3), 0);
        assert_eq!(level.block(32, -100, -16), 0);
        assert_eq!(global_bits(35723), 16);
        assert_eq!(global_bits(65), 7);
        assert_eq!(global_bits(64), 6);
    }
}
