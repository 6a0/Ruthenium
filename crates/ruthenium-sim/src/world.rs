//! The blocks of the level the player is in: chunk sections as the server sends them,
//! and the collision shapes around a box in the order the game visits them
//! (`BlockCollisions`).

use std::collections::HashMap;

use ruthenium_protocol::wire::{Reader, WireError};

use crate::block_entity::{BlockEntities, BlockEntityShape, Kind, kind_of};
use crate::blocks::Blocks;
use crate::direction::Direction;
use crate::shape::{Aabb, EPSILON, Placed};

/// Bits per entry of the global block-state palette (`ceillog2` of the state count).
pub fn global_bits(count: usize) -> u32 {
    usize::BITS - (count.max(1) - 1).leading_zeros()
}

#[derive(Debug, Clone)]
enum Section {
    Single(u16),
    Mixed(Box<[u16; 4096]>),
}

impl Section {
    fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        match self {
            Section::Single(s) => *s,
            Section::Mixed(a) => a[((y << 8) | (z << 4) | x) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, state: u16) {
        if let Section::Single(s) = *self {
            if s == state {
                return;
            }
            *self = Section::Mixed(Box::new([s; 4096]));
        }
        if let Section::Mixed(a) = self {
            a[((y << 8) | (z << 4) | x) as usize] = state;
        }
    }
}

#[derive(Debug, Clone)]
struct Chunk {
    sections: Vec<Section>,
}

/// The level's blocks.
#[derive(Debug, Clone)]
pub struct Level {
    min_y: i32,
    height: i32,
    /// The dimension's `gameplay/fast_lava` attribute (the nether's lava flows and
    /// pushes faster).
    fast_lava: bool,
    chunks: HashMap<(i32, i32), Chunk>,
    /// The moving pistons and shulker boxes, and their tickers.
    pub block_entities: BlockEntities,
    /// `PistonMovingBlockEntity.NOCLIP`: while a piston moves an entity, the way it
    /// goes (blocks moving that way don't stop it).
    pub noclip: Option<Direction>,
    /// When set, every `setBlock` call the client level makes outside a predicted
    /// action (position, state, flags, update limit), in order: tests compare them with
    /// vanilla's.
    pub set_log: Option<Vec<crate::block::level::SetCall>>,
}

impl Level {
    pub fn new(min_y: i32, height: i32) -> Level {
        Level {
            min_y,
            height,
            fast_lava: false,
            chunks: HashMap::new(),
            block_entities: BlockEntities::default(),
            noclip: None,
            set_log: None,
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

    pub fn has_chunk(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x, z))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn forget_chunk(&mut self, x: i32, z: i32) {
        self.chunks.remove(&(x, z));
        self.block_entities.clear_chunk(x, z);
    }

    /// An empty chunk of air (for tests and building levels by hand).
    pub fn add_empty_chunk(&mut self, x: i32, z: i32) {
        let n = (self.height / 16) as usize;
        self.chunks.insert(
            (x, z),
            Chunk {
                sections: vec![Section::Single(0); n],
            },
        );
    }

    /// Reads a chunk's sections (`LevelChunkSection.read`, one per 16 blocks of the
    /// level's height): block count, fluid count, block states, biomes.
    /// `biome_bits` is the biome registry's global palette width.
    pub fn load_chunk(
        &mut self,
        x: i32,
        z: i32,
        data: &[u8],
        biome_bits: u32,
    ) -> Result<(), WireError> {
        let n = (self.height / 16) as usize;
        let state_bits = global_bits(ruthenium_data::v26_3::blocks::STATE_COUNT);
        let mut r = Reader::new(data);
        let mut sections = Vec::with_capacity(n);
        for _ in 0..n {
            r.bytes(4)?; // non-empty block count, fluid count
            sections.push(read_states(&mut r, state_bits)?);
            skip_biomes(&mut r, biome_bits)?;
        }
        self.chunks.insert((x, z), Chunk { sections });
        // `replaceWithPacketData`: the chunk's block entities go, and the ones the
        // packet lists are made again (`getBlockEntity` with IMMEDIATE). The server lists
        // every shulker box's, so a box is all it takes.
        self.block_entities.clear_chunk(x, z);
        for (i, section) in self.chunks[&(x, z)].sections.iter().enumerate() {
            let y0 = self.min_y + i as i32 * 16;
            let mut found = Vec::new();
            match section {
                Section::Single(s) => {
                    if kind_of(u32::from(*s)) == Some(Kind::ShulkerBox) {
                        found.extend((0..4096).map(|c| (c, *s)));
                    }
                }
                Section::Mixed(a) => found.extend(
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
        Ok(())
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
        u32::from(c.sections[s].get(x & 15, (y - self.min_y) & 15, z & 15))
    }

    /// Sets a block (a block update, or building a level by hand). No effect where no
    /// chunk is loaded or outside the level.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u32) {
        if y < self.min_y || y >= self.min_y + self.height {
            return;
        }
        let min_y = self.min_y;
        let old = self.block(x, y, z);
        if let Some(c) = self.chunks.get_mut(&(x >> 4, z >> 4)) {
            let s = ((y - min_y) >> 4) as usize;
            c.sections[s].set(x & 15, (y - min_y) & 15, z & 15, state as u16);
            if old != state {
                self.block_entities.on_block_changed([x, y, z], old, state);
            }
        }
    }

    /// Sets a block as `LevelChunk.setBlockState` does: the state it replaced, or `None`
    /// when nothing changed (the same state, no chunk loaded there, or outside the
    /// level).
    pub fn replace_block(&mut self, x: i32, y: i32, z: i32, state: u32) -> Option<u32> {
        if y < self.min_y || y >= self.min_y + self.height || !self.has_chunk(x >> 4, z >> 4) {
            return None;
        }
        let old = self.block(x, y, z);
        if old == state {
            return None;
        }
        self.set_block(x, y, z, state);
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
                        _ => match blocks.collision_for(state, [x, y, z], who) {
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

/// A block-state paletted container, expanded.
fn read_states(r: &mut Reader<'_>, global: u32) -> Result<Section, WireError> {
    let bits = u32::from(r.u8()?);
    // `Strategy.createForBlockStates`: 0 single, 1-4 linear at 4 bits, 5-8 hashmap,
    // more: the global palette at its own width in memory.
    let (stored, palette) = match bits {
        0 => {
            let v = r.varint()?;
            return Ok(Section::Single(v as u16));
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
    let mut out = Box::new([0u16; 4096]);
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
    Ok(Section::Mixed(out))
}

/// Skips a biome paletted container (`Strategy.createForBiomes`: 0 single, 1-3
/// linear, more: global).
fn skip_biomes(r: &mut Reader<'_>, global: u32) -> Result<(), WireError> {
    let bits = u32::from(r.u8()?);
    let stored = match bits {
        0 => {
            r.varint()?;
            return Ok(());
        }
        1..=3 => {
            let n = r.count(64)?;
            for _ in 0..n {
                r.varint()?;
            }
            bits
        }
        _ => global,
    };
    let per_long = 64 / stored;
    r.bytes(64_u32.div_ceil(per_long) as usize * 8)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
