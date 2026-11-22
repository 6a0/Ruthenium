//! The blocks of the level the player is in: chunk sections as the server sends them,
//! and the collision shapes around a box in the order the game visits them
//! (`BlockCollisions`).

use std::collections::HashMap;

use ruthenium_protocol::wire::{Reader, WireError};

use crate::blocks::Blocks;
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
    chunks: HashMap<(i32, i32), Chunk>,
}

impl Level {
    pub fn new(min_y: i32, height: i32) -> Level {
        Level {
            min_y,
            height,
            chunks: HashMap::new(),
        }
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn has_chunk(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x, z))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn forget_chunk(&mut self, x: i32, z: i32) {
        self.chunks.remove(&(x, z));
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
        if let Some(c) = self.chunks.get_mut(&(x >> 4, z >> 4)) {
            let s = ((y - min_y) >> 4) as usize;
            c.sections[s].set(x & 15, (y - min_y) & 15, z & 15, state as u16);
        }
    }

    /// The block collision shapes that `box` could meet, in the order the game's
    /// `BlockCollisions` yields them: a cursor over the box grown by one block, x
    /// fastest, then y, then z; corner cells skipped, edge cells only for shapes
    /// taller or wider than a block.
    pub fn block_collisions<'a>(&self, blocks: &'a Blocks, b: &Aabb) -> Vec<Placed<'a>> {
        self.block_collisions_at(blocks, b, false)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    }

    /// [`Level::block_collisions`] with each shape's block position;
    /// `suffocating_only` keeps only blocks that suffocate (`collidesWithSuffocatingBlock`).
    pub fn block_collisions_at<'a>(
        &self,
        blocks: &'a Blocks,
        b: &Aabb,
        suffocating_only: bool,
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
                    if suffocating_only && !blocks.suffocating(state) {
                        continue;
                    }
                    if face == 1 && !blocks.large_shape(state) {
                        continue;
                    }
                    // Face type 2 cells count only for moving pistons, which never
                    // collide with players here.
                    if face == 2 {
                        continue;
                    }
                    let Some(placed) = blocks.collision(state, [x, y, z]) else {
                        continue;
                    };
                    if blocks.is_full_block(state) {
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
