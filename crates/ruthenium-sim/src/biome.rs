//! Biomes as the client reads them for weather: the climate each biome declares, the
//! temperature at a block (a biome's own, lowered with height above the snow line and
//! patched by the frozen modifier's noise), what falls there when it rains, and which
//! biome a block is in (`BiomeManager.getBiome`'s jittered pick among the eight nearest
//! quarter-chunk cells).
//!
//! The temperature noises are fixed: 2D simplex noise seeded by `LegacyRandomSource`
//! with the seeds vanilla gives them (`Biome.java:59-72`), so every client computes the
//! same values.

use std::sync::OnceLock;

use ruthenium_javamath::mth;

/// What a biome declares about its weather (`Biome.ClimateSettings`, less downfall).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Climate {
    pub has_precipitation: bool,
    pub temperature: f32,
    /// `temperature_modifier: frozen` (frozen oceans: warmer patches by noise).
    pub frozen: bool,
}

impl Climate {
    /// Plains' (`data/minecraft/worldgen/biome/plains.json`), the client's biome for
    /// blocks in no loaded chunk.
    pub const PLAINS: Climate = Climate {
        has_precipitation: true,
        temperature: 0.8,
        frozen: false,
    };

    /// From a biome's JSON or NBT fields: `has_precipitation`, `temperature` and the
    /// optional `temperature_modifier`.
    pub fn new(has_precipitation: bool, temperature: f32, modifier: Option<&str>) -> Climate {
        Climate {
            has_precipitation,
            temperature,
            frozen: modifier == Some("frozen"),
        }
    }

    /// A biome's climate from its data-pack JSON.
    pub fn from_json(v: &serde_json::Value) -> Option<Climate> {
        Some(Climate::new(
            v.get("has_precipitation")?.as_bool()?,
            v.get("temperature")?.as_f64()? as f32,
            v.get("temperature_modifier").and_then(|m| m.as_str()),
        ))
    }

    /// `Biome.getHeightAdjustedTemperature` (what `getTemperature` caches): the
    /// modifier's temperature, less 0.05 per 40 blocks (with noise) above 17 over sea
    /// level.
    pub fn temperature_at(&self, pos: [i32; 3], sea_level: i32) -> f32 {
        let own = if self.frozen {
            frozen_temperature(pos, self.temperature)
        } else {
            self.temperature
        };
        let snow_line = sea_level + 17;
        if pos[1] <= snow_line {
            return own;
        }
        let jitter = noises().temperature.sample(
            f64::from(pos[0] as f32 / 8.0),
            f64::from(pos[2] as f32 / 8.0),
        ) * 8.0;
        own - (jitter + pos[1] as f32 - snow_line as f32) * 0.05 / 40.0
    }

    /// `Biome.getPrecipitationAt`: none where the biome has none, snow below 0.15,
    /// else rain.
    pub fn precipitation_at(&self, pos: [i32; 3], sea_level: i32) -> Precipitation {
        if !self.has_precipitation {
            Precipitation::None
        } else if self.temperature_at(pos, sea_level) >= 0.15 {
            Precipitation::Rain
        } else {
            Precipitation::Snow
        }
    }
}

/// `Biome.Precipitation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

/// `TemperatureModifier.FROZEN`: patches (by two noises) where the biome reads 0.2
/// instead of its own temperature.
fn frozen_temperature(pos: [i32; 3], own: f32) -> f32 {
    let n = noises();
    let (x, z) = (f64::from(pos[0]), f64::from(pos[2]));
    let large = f64::from(n.frozen.sample(x * 0.05, z * 0.05) * 7.0);
    let edge = f64::from(n.biome_info.sample(x * 0.2, z * 0.2));
    if large + edge < 0.3 && f64::from(n.biome_info.sample(x * 0.09, z * 0.09)) < 0.8 {
        return 0.2;
    }
    own
}

struct Noises {
    temperature: Simplex,
    frozen: Layers,
    biome_info: Simplex,
}

fn noises() -> &'static Noises {
    static NOISES: OnceLock<Noises> = OnceLock::new();
    NOISES.get_or_init(|| {
        let mut random = LegacyRandom::new(3456);
        // Three octaves from one random, each half the frequency and twice the weight
        // of the last (`Biome.java:61-70`).
        let frozen = [(1.0, 0.14285715), (0.5, 0.2857143), (0.25, 0.5714286)]
            .into_iter()
            .map(|(frequency, amplitude)| (Simplex::new(&mut random), frequency, amplitude))
            .collect();
        Noises {
            temperature: Simplex::new(&mut LegacyRandom::new(1234)),
            frozen: Layers(frozen),
            biome_info: Simplex::new(&mut LegacyRandom::new(2345)),
        }
    })
}

/// `java.util.Random`'s generator (`LegacyRandomSource`).
struct LegacyRandom(i64);

impl LegacyRandom {
    const MULTIPLIER: i64 = 0x5DEECE66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> LegacyRandom {
        LegacyRandom((seed ^ Self::MULTIPLIER) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = self.0.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    /// `nextInt(bound)`: a power of two takes the high bits; else samples that would
    /// bias the remainder are drawn again.
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let r = sample % bound;
            if sample.wrapping_sub(r).wrapping_add(bound - 1) >= 0 {
                return r;
            }
        }
    }

    fn next_double(&mut self) -> f64 {
        let high = i64::from(self.next(26));
        let low = i64::from(self.next(27));
        ((high << 27) + low) as f64 * 1.1102230246251565E-16
    }
}

/// Weighted octaves summed in floats (`NoiseStack.get`).
struct Layers(Vec<(Simplex, f64, f32)>);

impl Layers {
    fn sample(&self, x: f64, z: f64) -> f32 {
        self.0
            .iter()
            .fold(0.0f32, |sum, (noise, frequency, amplitude)| {
                sum + amplitude * noise.sample(x * frequency, z * frequency)
            })
    }
}

/// 2D simplex noise over a shuffled permutation (`SimplexNoise` with its offsets
/// discarded: they are drawn, then multiplied by 0).
struct Simplex {
    perm: [u8; 256],
}

/// The gradients (`GradientNoise.GRADIENT`; only the first 12 are ever picked).
const GRADIENTS: [[i8; 3]; 12] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
];

impl Simplex {
    fn new(random: &mut LegacyRandom) -> Simplex {
        for _ in 0..3 {
            random.next_double();
        }
        let mut perm: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let j = i + random.next_int(256 - i as i32) as usize;
            perm.swap(i, j);
        }
        Simplex { perm }
    }

    fn hash(&self, v: i32) -> i32 {
        i32::from(self.perm[(v & 0xFF) as usize])
    }

    /// One corner's share: `(r² - d²)⁴` times the gradient's dot with the offset.
    fn corner(gradient: i32, x: f64, z: f64) -> f64 {
        let falloff = 0.5 - x * x - z * z - 0.0 * 0.0;
        if falloff < 0.0 {
            return 0.0;
        }
        let g = GRADIENTS[gradient as usize];
        let f2 = falloff * falloff;
        f2 * f2 * (f64::from(g[0]) * x + f64::from(g[1]) * z + f64::from(g[2]) * 0.0)
    }

    fn sample(&self, x: f64, z: f64) -> f32 {
        let sqrt3 = 3.0f64.sqrt();
        let skew = 0.5 * (sqrt3 - 1.0);
        let unskew = (3.0 - sqrt3) / 6.0;
        let (x, z) = (x + 0.0, z + 0.0);
        let s = (x + z) * skew;
        let (i, j) = (mth::floor(x + s), mth::floor(z + s));
        let t = f64::from(i.wrapping_add(j)) * unskew;
        let (x0, z0) = (x - (f64::from(i) - t), z - (f64::from(j) - t));
        let (di, dj) = if x0 > z0 { (1, 0) } else { (0, 1) };
        let corners = [
            (0, 0, x0, z0),
            (
                di,
                dj,
                x0 - f64::from(di) + unskew,
                z0 - f64::from(dj) + unskew,
            ),
            (1, 1, x0 - 1.0 + 2.0 * unskew, z0 - 1.0 + 2.0 * unskew),
        ];
        let (ii, jj) = (i & 0xFF, j & 0xFF);
        let [n0, n1, n2] = corners
            .map(|(a, b, cx, cz)| Self::corner(self.hash(ii + a + self.hash(jj + b)) % 12, cx, cz));
        (70.0 * (n0 + n1 + n2)) as f32
    }
}

/// `LinearCongruentialGenerator.next`.
fn lcg(r: i64, c: i64) -> i64 {
    r.wrapping_mul(
        r.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(c)
}

/// `BiomeManager.getBiome`: the quarter-chunk cell (`noise`'s coordinates) whose
/// corner, jittered by the level's hashed seed, lies nearest the block.
pub fn zoomed_cell(seed: i64, pos: [i32; 3]) -> [i32; 3] {
    let rel = pos.map(|v| v - 2);
    let parent = rel.map(|v| v >> 2);
    let frac = rel.map(|v| f64::from(v & 3) / 4.0);
    let fiddle = |r: i64| (((r >> 24).rem_euclid(1024)) as f64 / 1024.0 - 0.5) * 0.9;
    let mut best = (f64::INFINITY, parent);
    for corner in 0..8 {
        // Bit 2 picks x, bit 1 y, bit 0 z, from the low corner first.
        let up = [corner & 4 != 0, corner & 2 != 0, corner & 1 != 0];
        let cell: [i32; 3] = std::array::from_fn(|a| parent[a] + i32::from(up[a]));
        let d: [f64; 3] = std::array::from_fn(|a| if up[a] { frac[a] - 1.0 } else { frac[a] });
        let mut r = seed;
        for _ in 0..2 {
            for c in cell {
                r = lcg(r, i64::from(c));
            }
        }
        let fx = fiddle(r);
        r = lcg(r, seed);
        let fy = fiddle(r);
        r = lcg(r, seed);
        let fz = fiddle(r);
        let sq = |v: f64| v * v;
        let distance = sq(d[2] + fz) + sq(d[1] + fy) + sq(d[0] + fx);
        if best.0 > distance {
            best = (distance, cell);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values from the game's own classes (`Biome.TEMPERATURE_NOISE.get`,
    /// `FROZEN_TEMPERATURE_NOISE`, `BIOME_INFO_NOISE` and `BiomeManager`).
    #[test]
    fn noises_match_the_game() {
        let n = noises();
        for &(x, z, want) in TEMPERATURE {
            assert_eq!(
                n.temperature.sample(x, z).to_bits() as i32,
                want,
                "temperature {x} {z}"
            );
        }
        for &(x, z, want) in FROZEN {
            assert_eq!(
                n.frozen.sample(x, z).to_bits() as i32,
                want,
                "frozen {x} {z}"
            );
        }
        for &(x, z, want) in BIOME_INFO {
            assert_eq!(
                n.biome_info.sample(x, z).to_bits() as i32,
                want,
                "biome info {x} {z}"
            );
        }
        for &(seed, pos, want) in ZOOM {
            assert_eq!(zoomed_cell(seed, pos), want, "zoom {seed} {pos:?}");
        }
        for &(temperature, frozen, pos, sea_level, want) in TEMPERATURES {
            let c = Climate {
                has_precipitation: true,
                temperature,
                frozen,
            };
            assert_eq!(
                c.temperature_at(pos, sea_level).to_bits() as i32,
                want,
                "{c:?} at {pos:?}, sea level {sea_level}"
            );
        }
    }

    include!("biome_golden.rs");
}
