//! Biome colours: what grass, leaves and the like are multiplied by where they stand, as
//! 26.3's client works it out (`ClientLevel.calculateBlockTint`).
//!
//! A biome gives four colours: grass, foliage, dry foliage and water. Its effects may
//! name them; where they don't, the first three come from a colour map by the biome's
//! temperature and downfall. A block takes the mean of its colour over the columns
//! around it (Biome Blend), each column's biome picked as the game picks a block's: the
//! nearest of the eight quarter-chunk cells around it, their corners jittered by the
//! level's hashed seed.

use ruthenium_sim::biome::{biome_info, zoomed_cell};
use ruthenium_sim::environment::BiomeLook;
use ruthenium_sim::scene::Neighbourhood;
use serde_json::Value;

/// Which of a biome's colours a tint layer takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BiomeColour {
    Grass,
    Foliage,
    DryFoliage,
    Water,
}

/// The game's three colour maps (`textures/colormap/*.png`) as 0xAARRGGBB pixels, row
/// after row; empty where the game's files have none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColourMaps {
    pub grass: Vec<u32>,
    pub foliage: Vec<u32>,
    pub dry_foliage: Vec<u32>,
}

/// What the game takes where a map has no pixel for a climate (`GrassColor`,
/// `FoliageColor`, `DryFoliageColor`).
const GRASS_DEFAULT: u32 = -65281_i32 as u32;
const FOLIAGE_DEFAULT: u32 = -12012264_i32 as u32;
const DRY_FOLIAGE_DEFAULT: u32 = -10732494_i32 as u32;

/// A swamp's two grass colours (`BiomeSpecialEffects.GrassColorModifier.SWAMP`).
const SWAMP_DARK: u32 = -11766212_i32 as u32;
const SWAMP_LIGHT: u32 = -9801671_i32 as u32;

/// `ColorMapColorUtil.get`: the pixel of a map for a climate.
fn map_colour(pixels: &[u32], temperature: f32, downfall: f32, default: u32) -> u32 {
    let temperature = f64::from(temperature.clamp(0.0, 1.0));
    let rain = f64::from(downfall.clamp(0.0, 1.0)) * temperature;
    let x = ((1.0 - temperature) * 255.0) as i32;
    let y = ((1.0 - rain) * 255.0) as i32;
    pixels
        .get((y << 8 | x) as usize)
        .copied()
        .unwrap_or(default)
}

/// A colour of a biome's effects (`ExtraCodecs.STRING_RGB_COLOR`): `#RRGGBB`, opaque; a
/// number, as it is; or three parts from 0 to 1, opaque.
fn effect_colour(v: &Value) -> Option<u32> {
    if let Some(s) = v.as_str() {
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
        return Some(0xFF00_0000 | u32::from_str_radix(hex, 16).ok()?);
    }
    if let Some(n) = v.as_i64() {
        return Some(n as u32);
    }
    let parts = v.as_array().filter(|a| a.len() == 3)?;
    let mut c = 0xFF;
    for p in parts {
        c = c << 8 | ((p.as_f64()? as f32 * 255.0).floor() as i32 & 0xFF) as u32;
    }
    Some(c)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BiomeTint {
    grass: u32,
    /// Its grass is one of two colours by a noise, whatever `grass` says.
    swamp: bool,
    foliage: u32,
    dry_foliage: u32,
    water: u32,
}

/// Every biome's four colours, by the biome's network id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BiomeTints {
    biomes: Vec<BiomeTint>,
    /// The biome of blocks in no loaded chunk.
    plains: Option<usize>,
}

impl BiomeTints {
    pub fn new(maps: &ColourMaps, biomes: &[BiomeLook]) -> BiomeTints {
        let tints = biomes
            .iter()
            .map(|b| {
                let named = |name: &str| b.effects.get(name).and_then(effect_colour);
                let mapped =
                    |map: &[u32], default| map_colour(map, b.temperature, b.downfall, default);
                let modifier = b
                    .effects
                    .get("grass_color_modifier")
                    .and_then(Value::as_str);
                let mut grass =
                    named("grass_color").unwrap_or_else(|| mapped(&maps.grass, GRASS_DEFAULT));
                if modifier == Some("dark_forest") {
                    grass = 0xFF00_0000 | ((grass & 0xFE_FEFE) + 0x28_340A) >> 1;
                }
                BiomeTint {
                    grass,
                    swamp: modifier == Some("swamp"),
                    foliage: named("foliage_color")
                        .unwrap_or_else(|| mapped(&maps.foliage, FOLIAGE_DEFAULT)),
                    dry_foliage: named("dry_foliage_color")
                        .unwrap_or_else(|| mapped(&maps.dry_foliage, DRY_FOLIAGE_DEFAULT)),
                    // The game reads no biome without one.
                    water: named("water_color").unwrap_or(u32::MAX),
                }
            })
            .collect();
        BiomeTints {
            biomes: tints,
            plains: biomes.iter().position(|b| b.id == "minecraft:plains"),
        }
    }

    /// A biome's colours; plains' for a block in no loaded chunk.
    fn of(&self, biome: Option<u16>) -> Option<&BiomeTint> {
        biome
            .map(usize::from)
            .or(self.plains)
            .and_then(|i| self.biomes.get(i))
    }
}

/// The widest Biome Blend: 15 x 15 columns.
pub const MAX_BLEND_RADIUS: u8 = 7;
/// Columns along a side of what a section's blocks can blend over.
const SIDE: usize = 16 + 2 * MAX_BLEND_RADIUS as usize;
const COLUMNS: usize = SIDE * SIDE;

fn column(x: i32, z: i32) -> usize {
    let r = i32::from(MAX_BLEND_RADIUS);
    ((x + r) + SIDE as i32 * (z + r)) as usize
}

/// A biome id that is not known yet, and the one of a block in no loaded chunk.
const UNKNOWN: u16 = u16::MAX;
const NO_CHUNK: u16 = u16::MAX - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Spread {
    #[default]
    Unknown,
    /// Every cell a column's biome could be picked from holds this biome.
    One(Option<u16>),
    Mixed,
}

#[derive(Debug, Clone)]
struct Layer {
    kind: BiomeColour,
    y: i32,
    colours: [u32; 256],
    known: [u64; 4],
}

/// One section's biome colours as far as its mesh has asked for them. A mesher keeps one
/// and clears it for every section.
#[derive(Debug, Default)]
pub(crate) struct TintScratch {
    spread: Spread,
    /// Where one biome is all there is: its colours by kind, blended.
    flat: [Option<u32>; 4],
    /// The biomes of the columns of a layer of blocks, where biomes meet.
    ids: Vec<(i32, Box<[u16; COLUMNS]>)>,
    /// Which of its two colours a swamp's grass has in each column: 0 not known yet,
    /// 1 the light one, 2 the dark one.
    swamp: Vec<u8>,
    layers: Vec<Layer>,
}

impl TintScratch {
    pub fn clear(&mut self) {
        self.spread = Spread::Unknown;
        self.flat = [None; 4];
        self.ids.clear();
        self.swamp.clear();
        self.layers.clear();
    }

    /// Whether one biome is all that the columns around the section's blocks can be in:
    /// then nothing needs the jitter.
    fn spread(&mut self, n: &Neighbourhood, radius: i32) -> Spread {
        if self.spread == Spread::Unknown {
            // A block's biome is one of the cells around it, from the one two blocks
            // before it. Tints are read from one below the section to one above it.
            let cells = |low: i32, high: i32| ((low - 2) >> 2)..=((high - 2) >> 2) + 1;
            let first = n.biome(0, 0, 0);
            let one = cells(-1, 16).all(|y| {
                cells(-radius, 15 + radius).all(|z| {
                    cells(-radius, 15 + radius).all(|x| n.biome(x * 4, y * 4, z * 4) == first)
                })
            });
            self.spread = if one {
                Spread::One(first)
            } else {
                Spread::Mixed
            };
        }
        self.spread
    }

    /// The biome of a block, given from the section's corner (`BiomeManager.getBiome`).
    fn biome(&mut self, n: &Neighbourhood, x: i32, y: i32, z: i32) -> Option<u16> {
        let layer = match self.ids.iter().position(|(at, _)| *at == y) {
            Some(i) => i,
            None => {
                self.ids.push((y, Box::new([UNKNOWN; COLUMNS])));
                self.ids.len() - 1
            }
        };
        let id = &mut self.ids[layer].1[column(x, z)];
        if *id == UNKNOWN {
            let origin = n.section().map(|s| s << 4);
            let cell = zoomed_cell(
                n.dims().biome_seed,
                [origin[0] + x, origin[1] + y, origin[2] + z],
            );
            *id = n
                .biome(
                    (cell[0] << 2) - origin[0],
                    (cell[1] << 2) - origin[1],
                    (cell[2] << 2) - origin[2],
                )
                .unwrap_or(NO_CHUNK);
        }
        (*id != NO_CHUNK).then_some(*id)
    }

    /// Whether a swamp's grass is the dark one in a column.
    fn swamp_is_dark(&mut self, n: &Neighbourhood, x: i32, z: i32) -> bool {
        if self.swamp.is_empty() {
            self.swamp.resize(COLUMNS, 0);
        }
        let known = &mut self.swamp[column(x, z)];
        if *known == 0 {
            let section = n.section();
            let (wx, wz) = (
                f64::from((section[0] << 4) + x),
                f64::from((section[2] << 4) + z),
            );
            let ground = f64::from(biome_info(wx * 0.0225, wz * 0.0225));
            *known = if ground < -0.1 { 2 } else { 1 };
        }
        *known == 2
    }

    /// A biome's colour of a kind in a column.
    fn of_biome(
        &mut self,
        n: &Neighbourhood,
        tint: &BiomeTint,
        kind: BiomeColour,
        x: i32,
        z: i32,
    ) -> u32 {
        match kind {
            BiomeColour::Grass if tint.swamp => {
                if self.swamp_is_dark(n, x, z) {
                    SWAMP_DARK
                } else {
                    SWAMP_LIGHT
                }
            }
            BiomeColour::Grass => tint.grass,
            BiomeColour::Foliage => tint.foliage,
            BiomeColour::DryFoliage => tint.dry_foliage,
            BiomeColour::Water => tint.water,
        }
    }

    /// The colour of a kind at a block, given from the section's corner (x and z inside
    /// the section, y from one below it to one above): 0xAARRGGBB, the mean over the
    /// `2 * radius + 1` columns each way around it. `None` while no biome's colours are
    /// known.
    pub fn colour(
        &mut self,
        n: &Neighbourhood,
        tints: &BiomeTints,
        radius: u8,
        kind: BiomeColour,
        [x, y, z]: [i32; 3],
    ) -> Option<u32> {
        if tints.biomes.is_empty() {
            return None;
        }
        let radius = i32::from(radius.min(MAX_BLEND_RADIUS));
        // The mean of one colour is that colour, less its alpha.
        let blended = |c: u32| if radius == 0 { c } else { 0xFF00_0000 | c };
        let spread = self.spread(n, radius);
        if let Spread::One(biome) = spread {
            let tint = tints.of(biome)?;
            if !(tint.swamp && kind == BiomeColour::Grass) {
                let slot = &mut self.flat[kind as usize];
                if slot.is_none() {
                    *slot = Some(blended(match kind {
                        BiomeColour::Grass => tint.grass,
                        BiomeColour::Foliage => tint.foliage,
                        BiomeColour::DryFoliage => tint.dry_foliage,
                        BiomeColour::Water => tint.water,
                    }));
                }
                return *slot;
            }
        }
        let layer = match self.layers.iter().position(|l| l.kind == kind && l.y == y) {
            Some(i) => i,
            None => {
                self.layers.push(Layer {
                    kind,
                    y,
                    colours: [0; 256],
                    known: [0; 4],
                });
                self.layers.len() - 1
            }
        };
        let at = (x | z << 4) as usize;
        if self.layers[layer].known[at >> 6] & 1 << (at & 63) != 0 {
            return Some(self.layers[layer].colours[at]);
        }
        let one = |scratch: &mut TintScratch, x: i32, z: i32| -> Option<u32> {
            let biome = match spread {
                Spread::One(biome) => biome,
                _ => scratch.biome(n, x, y, z),
            };
            Some(scratch.of_biome(n, tints.of(biome)?, kind, x, z))
        };
        let colour = if radius == 0 {
            one(self, x, z)?
        } else {
            let (mut red, mut green, mut blue) = (0, 0, 0);
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let c = one(self, x + dx, z + dz)?;
                    red += c >> 16 & 0xFF;
                    green += c >> 8 & 0xFF;
                    blue += c & 0xFF;
                }
            }
            let count = ((2 * radius + 1) * (2 * radius + 1)) as u32;
            0xFF00_0000 | (red / count) << 16 | (green / count) << 8 | (blue / count)
        };
        let l = &mut self.layers[layer];
        l.colours[at] = colour;
        l.known[at >> 6] |= 1 << (at & 63);
        Some(colour)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use ruthenium_sim::scene::{Dims, Scene};
    use ruthenium_sim::world::Level;

    use super::super::testing::World;
    use super::*;

    fn biome(id: &str, temperature: f32, downfall: f32, effects: Value) -> BiomeLook {
        BiomeLook {
            id: id.into(),
            temperature,
            downfall,
            has_precipitation: true,
            frozen: false,
            attributes: Value::Null,
            effects,
        }
    }

    /// Maps whose pixels say where they are: the grass map's are their index, the
    /// others' their index with a mark in the top byte.
    fn maps() -> ColourMaps {
        ColourMaps {
            grass: (0..65536).collect(),
            foliage: (0..65536).map(|i| 0x0100_0000 | i).collect(),
            dry_foliage: (0..65536).map(|i| 0x0200_0000 | i).collect(),
        }
    }

    /// Four made-up biomes: plains by the maps, one that names its colours, a dark
    /// forest and a swamp.
    pub(crate) fn tints() -> BiomeTints {
        BiomeTints::new(
            &ColourMaps {
                grass: vec![0xFF79_C05A; 65536],
                foliage: vec![0xFF59_AE30; 65536],
                dry_foliage: vec![0xFFA0_7D3A; 65536],
            },
            &[
                biome(
                    "minecraft:plains",
                    0.8,
                    0.4,
                    json!({"water_color": "#3f76e4"}),
                ),
                biome(
                    "minecraft:badlands",
                    2.0,
                    0.0,
                    json!({"water_color": "#3f76e4", "grass_color": "#90814d",
                           "foliage_color": "#9e814d", "dry_foliage_color": 8_224_125}),
                ),
                biome(
                    "minecraft:dark_forest",
                    0.7,
                    0.8,
                    json!({"water_color": "#3f76e4", "grass_color_modifier": "dark_forest"}),
                ),
                biome(
                    "minecraft:swamp",
                    0.8,
                    0.9,
                    json!({"water_color": "#617b64", "grass_color_modifier": "swamp",
                           "foliage_color": "#6a7039"}),
                ),
            ],
        )
    }

    /// The test world with a seed for the jitter.
    pub(crate) fn world(seed: i64, chunks: &[(i32, i32)]) -> World {
        let mut level = Level::new(-16, 64);
        level.biomes.seed = seed;
        level.track_changes(4096);
        let (mut publisher, scene) = Scene::new(4096);
        publisher.begin_level(Dims::of(&level));
        let mut world = World {
            level,
            publisher,
            scene,
        };
        for &(x, z) in chunks {
            world.load(x, z);
        }
        world
    }

    const AROUND: [(i32, i32); 9] = [
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
    fn a_map_is_read_by_temperature_and_by_downfall_times_temperature() {
        let map: Vec<u32> = (0..65536).collect();
        // Plains: 0.8 and 0.4 as floats. 0.2 short of the warm edge is column 50, and
        // 0.32 of rain row 173.
        assert_eq!(map_colour(&map, 0.8, 0.4, 7), 173 << 8 | 50);
        assert_eq!(map_colour(&map, 0.5, 1.0, 7), 127 << 8 | 127);
        // Both are kept within 0 and 1.
        assert_eq!(map_colour(&map, 2.0, 1.5, 7), 0);
        assert_eq!(map_colour(&map, -1.0, 0.5, 7), 255 << 8 | 255);
        assert_eq!(map_colour(&map, 1.0, 0.0, 7), 255 << 8);
        // A map too small for the place gives the game's stand-in.
        assert_eq!(map_colour(&map[..256], 0.8, 0.4, 7), 7);
        assert_eq!(map_colour(&[], 0.8, 0.4, 7), 7);
    }

    #[test]
    fn an_effects_colour_is_text_a_number_or_three_parts() {
        assert_eq!(effect_colour(&json!("#617b64")), Some(0xFF61_7B64));
        // A number is taken as it is, without alpha.
        assert_eq!(effect_colour(&json!(4_159_204)), Some(0x003F_76E4));
        assert_eq!(effect_colour(&json!(-1)), Some(u32::MAX));
        assert_eq!(effect_colour(&json!([1.0, 0.5, 0.0])), Some(0xFFFF_7F00));
        for bad in [
            json!("617b64"),
            json!("#12"),
            json!([1.0, 0.5]),
            json!(null),
        ] {
            assert_eq!(effect_colour(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_biome_names_its_colours_or_takes_them_from_the_maps() {
        let t = BiomeTints::new(
            &maps(),
            &[
                biome(
                    "minecraft:plains",
                    0.8,
                    0.4,
                    json!({"water_color": "#3f76e4"}),
                ),
                biome(
                    "minecraft:badlands",
                    2.0,
                    0.0,
                    json!({"water_color": "#3f76e4", "grass_color": "#90814d",
                           "foliage_color": "#9e814d", "dry_foliage_color": 8_224_125}),
                ),
            ],
        );
        let at = 173 << 8 | 50;
        assert_eq!(
            t.biomes[0],
            BiomeTint {
                grass: at,
                swamp: false,
                foliage: 0x0100_0000 | at,
                dry_foliage: 0x0200_0000 | at,
                water: 0xFF3F_76E4,
            }
        );
        assert_eq!(
            t.biomes[1],
            BiomeTint {
                grass: 0xFF90_814D,
                swamp: false,
                foliage: 0xFF9E_814D,
                dry_foliage: 8_224_125,
                water: 0xFF3F_76E4,
            }
        );
        // Plains stand in for a block in no chunk, and for a biome nobody knows.
        assert_eq!(t.of(None), Some(&t.biomes[0]));
        assert_eq!(t.of(Some(1)), Some(&t.biomes[1]));
        assert_eq!(t.of(Some(9)), None);
        assert_eq!(BiomeTints::default().of(None), None);
    }

    #[test]
    fn a_dark_forest_darkens_its_grass_and_a_swamp_has_two() {
        let t = tints();
        // Halfway between the map's colour, less each channel's last bit, and 0x28340A.
        assert_eq!(
            t.biomes[2].grass,
            0xFF00_0000 | (0x78_C05A + 0x28_340A) >> 1
        );
        assert_eq!(t.biomes[2].grass, 0xFF50_7A32);
        assert!(t.biomes[3].swamp && !t.biomes[2].swamp);

        let mut seen = [false; 2];
        for chunk in 0..12 {
            let mut world = world(1, &[(chunk, 0)]);
            world
                .level
                .fill_biome([chunk * 16, -16, 0], [chunk * 16 + 15, 47, 15], 3);
            let n = world.pin([chunk, 0, 0]);
            let mut scratch = TintScratch::default();
            // Away from the chunk's edges, where a block's cell can be the next chunk's.
            for (x, z) in [(2, 2), (13, 4), (7, 9), (4, 13)] {
                let got = scratch.colour(&n, &t, 0, BiomeColour::Grass, [x, 5, z]);
                let ground = biome_info(f64::from(chunk * 16 + x) * 0.0225, f64::from(z) * 0.0225);
                let dark = f64::from(ground) < -0.1;
                assert_eq!(got, Some(if dark { SWAMP_DARK } else { SWAMP_LIGHT }));
                seen[usize::from(dark)] = true;
            }
            // Its other colours are as any biome's.
            assert_eq!(
                scratch.colour(&n, &t, 2, BiomeColour::Foliage, [8, 5, 8]),
                Some(0xFF6A_7039)
            );
        }
        assert_eq!(seen, [true; 2], "both colours were met");
        assert_eq!((SWAMP_DARK, SWAMP_LIGHT), (0xFF4C_763C, 0xFF6A_7039));
    }

    #[test]
    fn where_one_biome_is_all_around_its_colour_is_the_blocks() {
        let mut world = world(77, &AROUND);
        for &(x, z) in &AROUND {
            world
                .level
                .fill_biome([x * 16, -16, z * 16], [x * 16 + 15, 47, z * 16 + 15], 1);
        }
        let n = world.pin([0, 0, 0]);
        let t = tints();
        let mut scratch = TintScratch::default();
        for at in [[0, 0, 0], [15, 16, 15], [8, -1, 3]] {
            for radius in [0, 1, 2, 7] {
                assert_eq!(
                    scratch.colour(&n, &t, radius, BiomeColour::Grass, at),
                    Some(0xFF90_814D)
                );
            }
            // Without a blend the colour is as the biome gives it; a mean is opaque.
            assert_eq!(
                scratch.colour(&n, &t, 0, BiomeColour::DryFoliage, at),
                Some(8_224_125)
            );
            scratch.clear();
            assert_eq!(
                scratch.colour(&n, &t, 2, BiomeColour::DryFoliage, at),
                Some(0xFF00_0000 | 8_224_125)
            );
            scratch.clear();
        }
        // No biome's colours are known: nothing to say.
        assert_eq!(
            scratch.colour(&n, &BiomeTints::default(), 2, BiomeColour::Grass, [0; 3]),
            None
        );
    }

    #[test]
    fn where_biomes_meet_a_colour_is_the_mean_over_the_columns_around() {
        let seed = 0x1234_5678_9ABC_DEF0_u64 as i64;
        let mut world = world(seed, &AROUND);
        // Badlands east of x 8 in the middle chunk and beyond, a dark forest south of
        // z 12; plains elsewhere.
        world.level.fill_biome([8, -16, -16], [31, 47, 31], 1);
        world.level.fill_biome([-16, -16, 12], [31, 47, 31], 2);
        let n = world.pin([0, 0, 0]);
        let t = tints();
        // The biome of a block as the game picks it, from the cells as they were filled.
        let picked = |x: i32, y: i32, z: i32| {
            let cell = zoomed_cell(seed, [x, y, z]);
            if cell[2] >= 3 {
                2
            } else if cell[0] >= 2 {
                1
            } else {
                0
            }
        };
        let grass = |b: usize| t.biomes[b].grass;
        let mut scratch = TintScratch::default();
        let mut met = [false; 3];
        for y in [-1, 0, 7, 16] {
            for z in 0..16 {
                for x in 0..16 {
                    met[picked(x, y, z)] = true;
                    assert_eq!(
                        scratch.colour(&n, &t, 0, BiomeColour::Grass, [x, y, z]),
                        Some(grass(picked(x, y, z))),
                        "at {x} {y} {z}"
                    );
                }
            }
        }
        assert_eq!(met, [true; 3], "all three biomes are in the section");
        // The jitter moves the border: it is not the straight line of the cells.
        assert!((0..16).any(|z| picked(7, 0, z) == 1) || (0..16).any(|z| picked(8, 0, z) == 0));
        for radius in [1, 2, 7] {
            scratch.clear();
            for (x, y, z) in [(0, 0, 0), (7, 0, 3), (8, 5, 15), (15, 16, 15), (9, -1, 12)] {
                let mut sum = [0u32; 3];
                for dz in -radius..=radius {
                    for dx in -radius..=radius {
                        let c = grass(picked(x + dx, y, z + dz));
                        for (s, shift) in sum.iter_mut().zip([16, 8, 0]) {
                            *s += c >> shift & 0xFF;
                        }
                    }
                }
                let count = ((2 * radius + 1) * (2 * radius + 1)) as u32;
                let mean =
                    0xFF00_0000 | (sum[0] / count) << 16 | (sum[1] / count) << 8 | (sum[2] / count);
                assert_eq!(
                    scratch.colour(&n, &t, radius as u8, BiomeColour::Grass, [x, y, z]),
                    Some(mean),
                    "radius {radius} at {x} {y} {z}"
                );
                // And again, from what it kept.
                assert_eq!(
                    scratch.colour(&n, &t, radius as u8, BiomeColour::Grass, [x, y, z]),
                    Some(mean)
                );
            }
        }
    }

    #[test]
    fn a_block_in_no_chunk_is_in_plains() {
        // Only the middle chunk is loaded, all badlands: a blend over its edge takes
        // plains' colour for the columns outside.
        let mut world = world(5, &[(0, 0)]);
        world.level.fill_biome([0, -16, 0], [15, 47, 15], 1);
        let n = world.pin([0, 0, 0]);
        let t = tints();
        let mut scratch = TintScratch::default();
        let inside = scratch.colour(&n, &t, 2, BiomeColour::Grass, [8, 5, 8]);
        assert_eq!(inside, Some(0xFF90_814D));
        let edge = scratch
            .colour(&n, &t, 2, BiomeColour::Grass, [0, 5, 8])
            .unwrap();
        assert_ne!(edge, 0xFF90_814D);
        // Between the two colours, channel by channel.
        for shift in [16, 8, 0] {
            let (c, a, b) = (
                edge >> shift & 0xFF,
                0xFF90_814D_u32 >> shift & 0xFF,
                0xFF79_C05A_u32 >> shift & 0xFF,
            );
            assert!(c > a.min(b) && c < a.max(b));
        }
    }
    #[test]
    fn clearing_forgets_the_last_section() {
        let seed = 31;
        let mut world = world(seed, &AROUND);
        for &(x, z) in &AROUND {
            world
                .level
                .fill_biome([x * 16, -16, z * 16], [x * 16 + 15, 47, z * 16 + 15], 1);
        }
        let badlands = world.pin([0, 0, 0]);
        let t = tints();
        let mut scratch = TintScratch::default();
        let first = scratch.colour(&badlands, &t, 2, BiomeColour::Grass, [3, 3, 3]);
        assert_eq!(first, Some(0xFF90_814D));
        // The same place when a swamp has come up to it, and when it is all dark
        // forest: each asked of a scratch that held the section before.
        world.level.fill_biome([-16, -16, -16], [7, 47, 31], 3);
        let mixed = world.pin([0, 0, 0]);
        scratch.clear();
        let second = scratch.colour(&mixed, &t, 2, BiomeColour::Grass, [3, 3, 3]);
        for at in [[3, 3, 3], [9, 3, 2], [12, 3, 9], [1, 3, 14]] {
            scratch.colour(&mixed, &t, 1, BiomeColour::Foliage, at);
        }
        assert_eq!(
            second,
            TintScratch::default().colour(&mixed, &t, 2, BiomeColour::Grass, [3, 3, 3])
        );
        assert_ne!(second, first);
        for &(x, z) in &AROUND {
            world
                .level
                .fill_biome([x * 16, -16, z * 16], [x * 16 + 15, 47, z * 16 + 15], 2);
        }
        let forest = world.pin([0, 0, 0]);
        scratch.clear();
        assert_eq!(
            scratch.colour(&forest, &t, 2, BiomeColour::Grass, [3, 3, 3]),
            Some(0xFF50_7A32)
        );
        scratch.clear();
        assert_eq!(
            scratch.colour(&mixed, &t, 2, BiomeColour::Grass, [3, 3, 3]),
            second
        );
        // Another meeting of biomes in the same place: plains in the north.
        world.level.fill_biome([-16, -16, -16], [31, 47, 5], 0);
        let other = world.pin([0, 0, 0]);
        scratch.clear();
        let mut differs = false;
        for at in [[3, 3, 3], [9, 3, 2], [12, 3, 9], [1, 3, 14]] {
            let got = scratch.colour(&other, &t, 1, BiomeColour::Foliage, at);
            assert_eq!(
                got,
                TintScratch::default().colour(&other, &t, 1, BiomeColour::Foliage, at),
                "at {at:?}"
            );
            differs |=
                got != TintScratch::default().colour(&mixed, &t, 1, BiomeColour::Foliage, at);
        }
        assert!(differs);
    }
    #[test]
    fn one_biome_in_the_section_is_not_one_biome_around_it() {
        let seed = 7;
        let t = tints();
        let (badlands, plains) = (t.biomes[1].grass, t.biomes[0].grass);
        // Badlands, but plains from x 20 on: beyond the section, within the widest blend.
        let mut east = world(seed, &AROUND);
        east.level.fill_biome([-16, -16, -16], [19, 47, 31], 1);
        let n = east.pin([0, 0, 0]);
        let mut scratch = TintScratch::default();
        assert_eq!(
            scratch.colour(&n, &t, 2, BiomeColour::Grass, [15, 5, 8]),
            Some(badlands)
        );
        scratch.clear();
        let wide = scratch.colour(&n, &t, 7, BiomeColour::Grass, [15, 5, 8]);
        assert!(wide.is_some() && wide != Some(badlands));

        // Badlands, but plains below the section and above it: blocks at the
        // section's bottom and top can be in those cells.
        let mut layers = world(seed, &AROUND);
        layers.level.fill_biome([-16, 0, -16], [31, 15, 31], 1);
        let n = layers.pin([0, 0, 0]);
        let mut scratch = TintScratch::default();
        for (y, beyond) in [(0, -1), (15, 4)] {
            let mut met = [false; 2];
            for z in 0..16 {
                for x in 0..16 {
                    let outside = zoomed_cell(seed, [x, y, z])[1] == beyond;
                    met[usize::from(outside)] = true;
                    assert_eq!(
                        scratch.colour(&n, &t, 0, BiomeColour::Grass, [x, y, z]),
                        Some(if outside { plains } else { badlands }),
                        "at {x} {y} {z}"
                    );
                }
            }
            assert_eq!(met, [true; 2], "y {y} has blocks in both cells");
        }
    }
}
