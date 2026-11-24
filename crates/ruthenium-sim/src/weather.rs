//! Rain as the client sees it: the levels the server's game events set, and where it
//! falls (`Level.precipitationAt`): a column open to the sky (full sky light: no block
//! above that dampens light or closes the face between it and the next), with nothing
//! above in the motion-blocking heightmap, in a biome warm enough for rain.
//!
//! Only movement reads it: a riptide trident needs water or rain.

use crate::biome::Precipitation;
use crate::blocks::{BlockTags, Blocks};
use crate::world::Level;

/// `ClientboundGameEventPacket` events that set the weather.
pub const START_RAINING: u8 = 1;
pub const STOP_RAINING: u8 = 2;
pub const RAIN_LEVEL_CHANGE: u8 = 7;
pub const THUNDER_LEVEL_CHANGE: u8 = 8;

/// A level's rain and thunder (`Level.rainLevel`, `thunderLevel`; the client sets
/// the previous tick's values with them, so the levels never blend).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Weather {
    pub rain: f32,
    pub thunder: f32,
    /// `canHaveWeather`: a dimension with sky light, no ceiling, not the End.
    pub possible: bool,
}

impl Weather {
    /// For a dimension (`Level.canHaveWeather`).
    pub fn new(has_sky_light: bool, has_ceiling: bool, dimension: &str) -> Weather {
        Weather {
            possible: has_sky_light && !has_ceiling && dimension != "minecraft:the_end",
            ..Weather::default()
        }
    }

    /// `handleGameEvent`'s weather events; `false` for any other event.
    pub fn game_event(&mut self, event: u8, param: f32) -> bool {
        match event {
            // Starting sets the level to 0 and stopping to 1: the level changes that
            // follow in the same tick put them right.
            START_RAINING => self.rain = 0.0,
            STOP_RAINING => self.rain = 1.0,
            RAIN_LEVEL_CHANGE => self.rain = clamp(param),
            THUNDER_LEVEL_CHANGE => self.thunder = clamp(param),
            _ => return false,
        }
        true
    }

    /// `isRaining`: above 0.2 (the float level against the double 0.2).
    pub fn is_raining(&self) -> bool {
        self.possible && f64::from(self.rain) > 0.2
    }

    /// `isThundering`: thunder times rain above 0.9.
    pub fn is_thundering(&self) -> bool {
        self.possible && f64::from(self.thunder * self.rain) > 0.9
    }
}

/// `Mth.clamp` to 0..1 (NaN stays NaN).
fn clamp(v: f32) -> f32 {
    if v < 0.0 { 0.0 } else { v.min(1.0) }
}

/// The blocks and tags weather reads.
#[derive(Debug, Clone, Copy)]
pub struct Sky<'a> {
    pub level: &'a Level,
    pub blocks: &'a Blocks,
    pub tags: &'a BlockTags,
}

impl Sky<'_> {
    /// `Level.isRainingAt`.
    pub fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        self.precipitation_at(pos) == Precipitation::Rain
    }

    /// `Level.precipitationAt`.
    pub fn precipitation_at(&self, pos: [i32; 3]) -> Precipitation {
        if !self.level.weather.is_raining()
            || !self.can_see_sky(pos)
            || self.motion_blocking_height(pos[0], pos[2]) > pos[1]
        {
            return Precipitation::None;
        }
        let sea_level = self.level.biomes.sea_level;
        self.level.biome(pos).precipitation_at(pos, sea_level)
    }

    /// `canSeeSky`: full sky light, which only comes straight down. The light is the
    /// one the column's sky light sources give (`ChunkSkyLightSources`): every block
    /// from the highest occluded edge up. A column in no chunk is lit.
    pub fn can_see_sky(&self, pos: [i32; 3]) -> bool {
        pos[1] >= self.lowest_sky_source(pos[0], pos[2])
    }

    /// `ChunkSkyLightSources.getLowestSourceY` for a column: the lowest y sky light
    /// reaches unblocked, from the top of its highest section that isn't all air down
    /// to the first occluded edge; `i32::MIN` when it reaches the bottom.
    fn lowest_sky_source(&self, x: i32, z: i32) -> i32 {
        let level = self.level;
        let Some(top) = level.top_filled_section(x, z, |s| self.blocks.is_air(s)) else {
            return i32::MIN;
        };
        let Some(top) = top else {
            return i32::MIN;
        };
        let mut above = 0;
        let mut y = level.min_y() + (top as i32 + 1) * 16;
        while y > level.min_y() {
            y -= 1;
            let below = level.block(x, y, z);
            if self.blocks.sky_edge_occluded(above, below) {
                return y + 1;
            }
            above = below;
        }
        i32::MIN
    }

    /// `getHeight(MOTION_BLOCKING)`: one above the highest block in the
    /// `blocks_motion_in_heightmap` tag or with a fluid; the level's bottom where there
    /// is none or no chunk.
    pub fn motion_blocking_height(&self, x: i32, z: i32) -> i32 {
        let level = self.level;
        let bottom = level.min_y();
        if !level.has_chunk(x >> 4, z >> 4) {
            return bottom;
        }
        let blocks = |s: u32| {
            self.tags.is(s, "minecraft:blocks_motion_in_heightmap")
                || !self.blocks.fluid(s).is_empty()
        };
        (bottom..bottom + level.height())
            .rev()
            .find(|&y| blocks(level.block(x, y, z)))
            .map_or(bottom, |y| y + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_prepare::exporters::{BlockPhysics, ShapeData, StatePhysics};

    #[test]
    fn rain_starts_above_a_fifth() {
        let mut w = Weather::new(true, false, "minecraft:overworld");
        let mut level = 0.0f32;
        for _ in 0..19 {
            level += 0.01;
            assert!(w.game_event(RAIN_LEVEL_CHANGE, level));
        }
        // 0.19000001 is above 0.19, and not above 0.2.
        assert!(!w.is_raining());
        level += 0.01;
        w.game_event(RAIN_LEVEL_CHANGE, level);
        assert!(w.is_raining());
        // Starting sets it to 0, stopping to 1; levels clamp to 0..1.
        w.game_event(START_RAINING, 0.0);
        assert_eq!(w.rain, 0.0);
        w.game_event(STOP_RAINING, 0.0);
        assert_eq!(w.rain, 1.0);
        w.game_event(RAIN_LEVEL_CHANGE, 3.0);
        assert_eq!(w.rain, 1.0);
        w.game_event(THUNDER_LEVEL_CHANGE, -1.0);
        assert_eq!(w.thunder, 0.0);
        assert!(!w.is_thundering());
        w.game_event(THUNDER_LEVEL_CHANGE, 1.0);
        assert!(w.is_thundering());
        assert!(!w.game_event(3, 0.0));
    }

    #[test]
    fn no_weather_without_a_sky_or_in_the_end() {
        for (sky, ceiling, dimension, possible) in [
            (true, false, "minecraft:overworld", true),
            (false, false, "minecraft:the_nether", false),
            (true, true, "x:roofed", false),
            (true, false, "minecraft:the_end", false),
        ] {
            let mut w = Weather::new(sky, ceiling, dimension);
            w.game_event(RAIN_LEVEL_CHANGE, 1.0);
            assert_eq!(w.is_raining(), possible, "{dimension}");
        }
    }

    /// States 0 air, 1 stone (dampens light), 2 granite (no light effect), 3 polished
    /// granite (dampens light by 1), 4 diorite (shuts the sky out with its lower
    /// face).
    fn world() -> (Level, Blocks, BlockTags) {
        let cube = |full| ShapeData {
            coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
            full: vec![full],
        };
        let state = |light_dampening, occlusion: [u32; 2]| StatePhysics {
            shape: 0,
            flags: 0,
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            bounce: 0.0,
            fluid: 0,
            sturdy: 0,
            offset: None,
            outline: 0,
            interaction_shape: 0,
            interaction: 0,
            destroy_time: 0.0,
            light_dampening,
            light_emission: 0,
            light_shape: false,
            occlusion: [occlusion[0], occlusion[1], 0, 0, 0, 0],
        };
        let blocks = Blocks::new(&BlockPhysics {
            shapes: vec![cube(false), cube(true)],
            states: vec![
                state(0, [0, 0]),
                state(15, [1, 1]),
                state(0, [0, 0]),
                state(1, [0, 0]),
                state(0, [1, 0]),
            ],
        });
        let mut level = Level::new(-64, 384);
        level.add_empty_chunk(0, 0);
        level.weather = Weather::new(true, false, "minecraft:overworld");
        level.weather.rain = 1.0;
        level.biomes.climates = std::sync::Arc::new(vec![crate::biome::Climate::PLAINS]);
        let mut tags = BlockTags::new();
        tags.set(
            "minecraft:blocks_motion_in_heightmap",
            ["minecraft:stone", "minecraft:granite"],
        );
        (level, blocks, tags)
    }

    #[test]
    fn rain_falls_to_the_open_sky() {
        let at = |level: &Level, blocks: &Blocks, tags: &BlockTags, y: i32| {
            Sky {
                level,
                blocks,
                tags,
            }
            .is_raining_at([4, y, 4])
        };
        let (mut level, blocks, tags) = world();
        for state in 0..5 {
            level.set_block(4, 70, 4, 0);
            level.set_block(4, 71, 4, state);
            // Below and in the block; above it, the sky.
            let covered = state == 1 || state == 2 || state == 3 || state == 4;
            assert_eq!(!covered, at(&level, &blocks, &tags, 60), "state {state}");
            assert!(at(&level, &blocks, &tags, 72), "state {state}");
        }
        // A block of rain (air) over the roof's top; under the ground nothing.
        level.set_block(4, 71, 4, 0);
        assert!(at(&level, &blocks, &tags, 60));
        // 0.2 as a float is above the double 0.2: raining; 0.19 isn't.
        level.weather.rain = 0.2;
        assert!(at(&level, &blocks, &tags, 60));
        level.weather.rain = 0.19;
        assert!(!at(&level, &blocks, &tags, 60));
    }
}
