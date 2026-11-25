//! Rain and snow: columns of falling drops or flakes around the camera, as 26.3's
//! `WeatherEffectRenderer` extracts and draws them.
//!
//! Every column of blocks within the Weather Radius whose biome has rain or snow at the
//! camera's height gets a quad from the top of its motion-blocking blocks (or the
//! radius below the camera) to the radius above the camera, turned to face the camera's
//! column. Its picture scrolls down with the game's time, at a speed (rain) or with a
//! drift (snow) that a generator seeded by the column's place gives, and fades with the
//! rain level and with distance. Which columns fall and how is worked out again only
//! when the camera enters another block or the level changes; the frame only scrolls
//! and places them.

use std::sync::Arc;

use ruthenium_gpu::{FrameData, Pass, TextureId, TextureTable};
use ruthenium_javamath::random::JavaRandom;
use ruthenium_shaders::interface::WEATHER;
use ruthenium_sim::biome::{Climate, Precipitation, zoomed_cell};
use ruthenium_sim::environment::Environment;
use ruthenium_sim::scene::Terrain;

use crate::atmosphere::Fog;
use crate::pipelines::PipelineId;

/// The Weather Radius option's range, in blocks (`options.weatherRadius`).
pub const MIN_RADIUS: u32 = 3;
pub const MAX_RADIUS: u32 = 10;

/// What falls in a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fall {
    Rain,
    Snow,
}

/// What a column's generator gave: a rain column's speed, or a snow column's two
/// drifts (a double and a Gaussian each).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Draws {
    Rain { speed: f32 },
    Snow { u: (f64, f64), v: (f64, f64) },
}

/// A column the weather falls in (`WeatherEffectRenderer.ColumnInstance`, less what
/// changes with time).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub x: i32,
    pub z: i32,
    pub bottom: i32,
    pub top: i32,
    /// The light at the column's foot or the camera's height, whichever is higher: block
    /// light in bits 4 to 7, sky light in bits 20 to 23 (`LightCoordsUtil.pack`).
    pub light: u32,
    pub draws: Draws,
}

/// The seed of a column's generator.
pub fn column_seed(x: i32, z: i32) -> i32 {
    let a = x
        .wrapping_mul(x)
        .wrapping_mul(3121)
        .wrapping_add(x.wrapping_mul(45_238_971));
    let b = z
        .wrapping_mul(z)
        .wrapping_mul(418_711)
        .wrapping_add(z.wrapping_mul(13761));
    a ^ b
}

/// How many ticks a rain column's picture is ahead of the others'.
fn tick_offset(x: i32, z: i32) -> i32 {
    x.wrapping_mul(x)
        .wrapping_mul(3121)
        .wrapping_add(x.wrapping_mul(45_238_971))
        .wrapping_add(z.wrapping_mul(z).wrapping_mul(418_711))
        .wrapping_add(z.wrapping_mul(13761))
        & 0xFF
}

/// `SingleThreadedRandomSource`: `java.util.Random`'s generator, its doubles and its
/// Gaussians (`MarsagliaPolarGaussian`).
struct Generator {
    random: JavaRandom,
    next_gaussian: Option<f64>,
}

impl Generator {
    fn new(seed: i32) -> Generator {
        Generator {
            random: JavaRandom::new(i64::from(seed)),
            next_gaussian: None,
        }
    }

    fn float(&mut self) -> f32 {
        self.random.next(24) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    fn double(&mut self) -> f64 {
        let upper = i64::from(self.random.next(26));
        let lower = i64::from(self.random.next(27));
        ((upper << 27) + lower) as f64 * 1.110_223_024_625_156_5e-16
    }

    fn gaussian(&mut self) -> f64 {
        if let Some(g) = self.next_gaussian.take() {
            return g;
        }
        loop {
            let x = 2.0 * self.double() - 1.0;
            let y = 2.0 * self.double() - 1.0;
            let r = x * x + y * y;
            if r < 1.0 && r != 0.0 {
                let multiplier = (-2.0 * r.ln() / r).sqrt();
                self.next_gaussian = Some(y * multiplier);
                return x * multiplier;
            }
        }
    }
}

/// What a column's generator gives it.
pub fn draws(fall: Fall, x: i32, z: i32) -> Draws {
    let mut g = Generator::new(column_seed(x, z));
    match fall {
        Fall::Rain => Draws::Rain {
            speed: 3.0 + g.float(),
        },
        Fall::Snow => {
            let u = (g.double(), g.gaussian());
            let v = (g.double(), g.gaussian());
            Draws::Snow { u, v }
        }
    }
}

/// What the level says of a column (`ClientLevel`'s heightmap, `getPrecipitationAt`
/// and `LightCoordsUtil.getLightCoords`).
pub trait Ground {
    /// `getHeight(MOTION_BLOCKING)`: the y above the column's highest block that blocks
    /// motion or holds a fluid.
    fn height(&self, x: i32, z: i32) -> i32;
    /// What falls at a block: nothing where no chunk is loaded or the biome has no
    /// precipitation.
    fn fall(&self, pos: [i32; 3]) -> Option<Fall>;
    /// Block light and sky light at a block.
    fn light(&self, pos: [i32; 3]) -> (u8, u8);
}

/// The columns around the camera's block `camera` out to `radius`, rain and snow
/// apart, each in the game's order (`WeatherEffectRenderer.extractRenderState`). The
/// camera's own column is left out: the game's quad for it has no width.
pub fn gather(
    ground: &impl Ground,
    camera: [i32; 3],
    radius: i32,
    rain: &mut Vec<Column>,
    snow: &mut Vec<Column>,
) {
    rain.clear();
    snow.clear();
    let [cx, cy, cz] = camera;
    for z in cz - radius..=cz + radius {
        for x in cx - radius..=cx + radius {
            if x == cx && z == cz {
                continue;
            }
            let height = ground.height(x, z);
            let bottom = (cy - radius).max(height);
            let top = (cy + radius).max(height);
            if top == bottom {
                continue;
            }
            let Some(fall) = ground.fall([x, cy, z]) else {
                continue;
            };
            let (block, sky) = ground.light([x, cy.max(height), z]);
            let column = Column {
                x,
                z,
                bottom,
                top,
                light: u32::from(block) << 4 | u32::from(sky) << 20,
                draws: draws(fall, x, z),
            };
            match fall {
                Fall::Rain => rain.push(column),
                Fall::Snow => snow.push(column),
            }
        }
    }
}

/// The half-widths of the quads, by a column's place from the camera's (32 x 32 around
/// it): each faces the camera's column.
#[derive(Debug, Clone)]
pub struct ColumnSizes {
    x: [f32; 1024],
    z: [f32; 1024],
}

impl ColumnSizes {
    pub fn new() -> ColumnSizes {
        let mut sizes = ColumnSizes {
            x: [0.0; 1024],
            z: [0.0; 1024],
        };
        for z in 0..32 {
            for x in 0..32 {
                let dx = (x - 16) as f32;
                let dz = (z - 16) as f32;
                let distance = (dx * dx + dz * dz).sqrt();
                sizes.x[(z * 32 + x) as usize] = -dz / distance;
                sizes.z[(z * 32 + x) as usize] = dx / distance;
            }
        }
        sizes
    }
}

impl Default for ColumnSizes {
    fn default() -> ColumnSizes {
        ColumnSizes::new()
    }
}

/// One column's quad as the shader draws it: its corners go top of the first side, top
/// of the other, bottom of the other, bottom of the first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub start: [f32; 2],
    pub end: [f32; 2],
    /// Bottom and top.
    pub y: [f32; 2],
    pub u: [f32; 2],
    /// At the top and at the bottom.
    pub v: [f32; 2],
    pub alpha: u8,
    /// `LightCoordsUtil`'s packing.
    pub light: u32,
}

/// What a frame's weather is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    pub camera: [f64; 3],
    pub radius: i32,
    /// The rain level, 0 to 1.
    pub intensity: f32,
    pub game_time: i64,
    pub partial: f32,
}

/// A column's quad this frame (`createRainColumnInstance` or
/// `createSnowColumnInstance`, then `prepareInstances`).
pub fn quad(c: &Column, m: &Moment, sizes: &ColumnSizes) -> Quad {
    let wrapped = (m.game_time & 131_071) as i32;
    let (u_offset, v_offset, light, max_alpha) = match c.draws {
        Draws::Rain { speed } => {
            let ticks = wrapped.wrapping_add(tick_offset(c.x, c.z));
            let offset = -(ticks as f32 + m.partial) / 32.0 * speed;
            (0.0, offset % 32.0, c.light, 1.0)
        }
        Draws::Snow { u, v } => {
            let time = wrapped as f32 + m.partial;
            let u = (u.0 + f64::from(time * 0.01 * u.1 as f32)) as f32;
            let v = (v.0 + f64::from(time * v.1 as f32 * 0.001)) as f32;
            let v_offset = -((m.game_time & 511) as f32 + m.partial) / 512.0;
            let block = (c.light >> 4 & 15) * 3 + 15;
            let sky = (c.light >> 20 & 15) * 3 + 15;
            (u, v_offset + v, (block / 4) << 4 | (sky / 4) << 20, 0.8)
        }
    };
    let [cx, cy, cz] = m.camera;
    let rel_x = (f64::from(c.x) + 0.5 - cx) as f32;
    let rel_z = (f64::from(c.z) + 0.5 - cz) as f32;
    let distance = rel_x * rel_x + rel_z * rel_z;
    let radius = (m.radius * m.radius) as f32;
    let t = (distance / radius).min(1.0);
    let alpha = (max_alpha + t * (0.5 - max_alpha)) * m.intensity;
    let index = (c.z - cz.floor() as i32 + 16) * 32 + c.x - cx.floor() as i32 + 16;
    let index = index.clamp(0, 1023) as usize;
    let (half_x, half_z) = (sizes.x[index] / 2.0, sizes.z[index] / 2.0);
    Quad {
        start: [rel_x - half_x, rel_z - half_z],
        end: [rel_x + half_x, rel_z + half_z],
        y: [
            (f64::from(c.bottom) - cy) as f32,
            (f64::from(c.top) - cy) as f32,
        ],
        u: [u_offset + 0.0, u_offset + 1.0],
        v: [
            c.bottom as f32 * 0.25 + v_offset,
            c.top as f32 * 0.25 + v_offset,
        ],
        alpha: ((alpha * 255.0).floor() as i32 & 0xFF) as u8,
        light,
    }
}

/// The published level, as the weather reads it.
#[derive(Debug)]
pub struct TerrainGround<'a> {
    pub terrain: &'a Terrain,
    pub environment: &'a Environment,
    /// The biome of a chunk that is not loaded.
    pub plains: u16,
}

impl Ground for TerrainGround<'_> {
    fn height(&self, x: i32, z: i32) -> i32 {
        let bottom = self.terrain.dims().min_section << 4;
        self.terrain
            .column(x >> 4, z >> 4)
            .and_then(|c| c.heights())
            .map_or(bottom, |h| {
                i32::from(h[((x & 15) | (z & 15) << 4) as usize])
            })
    }

    fn fall(&self, pos: [i32; 3]) -> Option<Fall> {
        self.terrain.column(pos[0] >> 4, pos[2] >> 4)?;
        let dims = self.terrain.dims();
        let id = self
            .terrain
            .biome_cell(zoomed_cell(dims.biome_seed, pos))
            .unwrap_or(self.plains);
        let biome = self.environment.biomes.get(usize::from(id))?;
        let climate = Climate::new(
            biome.has_precipitation,
            biome.temperature,
            biome.frozen.then_some("frozen"),
        );
        match climate.precipitation_at(pos, dims.sea_level) {
            Precipitation::None => None,
            Precipitation::Rain => Some(Fall::Rain),
            Precipitation::Snow => Some(Fall::Snow),
        }
    }

    fn light(&self, pos: [i32; 3]) -> (u8, u8) {
        self.terrain
            .probe(pos)
            .map_or((0, 15), |p| (p.block_light, p.sky_light))
    }
}

/// What the columns were gathered for: when any of it changes they are gathered again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gathered {
    camera: [i32; 3],
    radius: i32,
    epoch: u64,
    stamp: u64,
    environment: usize,
}

/// The weather's pictures and its columns from one frame to the next.
#[derive(Debug)]
pub(crate) struct WeatherRenderer {
    rain_texture: Option<TextureId>,
    snow_texture: Option<TextureId>,
    sizes: Box<ColumnSizes>,
    rain: Vec<Column>,
    snow: Vec<Column>,
    gathered: Option<Gathered>,
    moment: Option<Moment>,
}

impl WeatherRenderer {
    pub fn new(
        rain_texture: Option<TextureId>,
        snow_texture: Option<TextureId>,
    ) -> WeatherRenderer {
        WeatherRenderer {
            rain_texture,
            snow_texture,
            sizes: Box::new(ColumnSizes::new()),
            rain: Vec::new(),
            snow: Vec::new(),
            gathered: None,
            moment: None,
        }
    }

    /// Works out what falls around the camera this frame: nothing without rain.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        terrain: &Terrain,
        environment: &Arc<Environment>,
        stamp: u64,
        moment: Moment,
    ) {
        if moment.intensity <= 0.0 {
            self.moment = None;
            return;
        }
        let camera = moment.camera.map(|c| c.floor() as i32);
        let key = Gathered {
            camera,
            radius: moment.radius,
            epoch: terrain.epoch(),
            stamp,
            environment: Arc::as_ptr(environment) as usize,
        };
        if self.gathered != Some(key) {
            let plains = environment
                .biomes
                .iter()
                .position(|b| b.id == "minecraft:plains")
                .unwrap_or(0) as u16;
            let ground = TerrainGround {
                terrain,
                environment,
                plains,
            };
            gather(
                &ground,
                camera,
                moment.radius,
                &mut self.rain,
                &mut self.snow,
            );
            self.gathered = Some(key);
        }
        self.moment = Some(moment);
    }

    /// Draws the frame's rain, then its snow, into `pass`, which has a depth attachment:
    /// after the clouds. `matrix` and `world` are the word offsets of the frame's
    /// WORLD_MATRIX and WORLD records (its light map). A picture that has not arrived
    /// yet is left out.
    ///
    /// # Safety
    /// `pass` is of the frame being recorded, and the records were written this frame.
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        records: (u32, u32),
        textures: &TextureTable,
        fog: &Fog,
    ) {
        let Some(m) = self.moment else {
            return;
        };
        for (columns, texture) in [
            (&self.rain, self.rain_texture),
            (&self.snow, self.snow_texture),
        ] {
            let Some(texture) = texture.filter(|t| !columns.is_empty() && textures.is_ready(*t))
            else {
                continue;
            };
            let Some((first, bytes)) = frame_data.alloc(columns.len() as u32 * WEATHER.words(), 4)
            else {
                return;
            };
            let (records_bytes, _) = bytes.as_chunks_mut::<RECORD_BYTES>();
            for (c, out) in columns.iter().zip(records_bytes) {
                write_quad(out, &quad(c, &m, &self.sizes));
            }
            let push = weather_push(records, first, texture, fog);
            // SAFETY: the records were written just above; the rest is the caller's.
            unsafe { pass.draw(PipelineId::Weather.index(), &push, 6, columns.len() as u32) };
        }
    }
}

/// Bytes of a WEATHER record.
const RECORD_BYTES: usize = WEATHER.words() as usize * 4;

/// Writes one WEATHER record.
fn write_quad(out: &mut [u8; RECORD_BYTES], q: &Quad) {
    let words = [
        q.start[0].to_bits(),
        q.start[1].to_bits(),
        q.end[0].to_bits(),
        q.end[1].to_bits(),
        q.y[0].to_bits(),
        q.y[1].to_bits(),
        q.u[0].to_bits(),
        q.u[1].to_bits(),
        q.v[0].to_bits(),
        q.v[1].to_bits(),
        u32::from(q.alpha),
        q.light,
    ];
    let (out, _) = out.as_chunks_mut::<4>();
    for (w, o) in words.iter().zip(out) {
        *o = w.to_ne_bytes();
    }
}

/// Bytes of `weather.vert`'s and `weather.frag`'s push constants.
const WEATHER_PUSH_BYTES: usize = 52;

fn weather_push(
    records: (u32, u32),
    first: u32,
    texture: TextureId,
    fog: &Fog,
) -> [u8; WEATHER_PUSH_BYTES] {
    let mut push = [0u8; WEATHER_PUSH_BYTES];
    let mut put = |at: usize, word: [u8; 4]| push[at..at + 4].copy_from_slice(&word);
    put(0, records.0.to_ne_bytes());
    put(4, records.1.to_ne_bytes());
    put(8, first.to_ne_bytes());
    put(12, 1u32.to_ne_bytes());
    for (i, c) in fog.colour.iter().enumerate() {
        put(16 + i * 4, c.to_ne_bytes());
    }
    put(32, fog.environmental[0].to_ne_bytes());
    put(36, fog.environmental[1].to_ne_bytes());
    put(40, fog.render_distance[0].to_ne_bytes());
    put(44, fog.render_distance[1].to_ne_bytes());
    put(48, texture.index().to_ne_bytes());
    push
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The weather golden's generator (a 64-bit LCG, as the tool's).
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u32) -> i32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as u32 % n) as i32
        }
    }

    /// A made-up level: each column of the 21 x 21 around the camera's has its height,
    /// what falls there and its light.
    struct Stub {
        camera: [i32; 3],
        columns: Vec<(i32, Option<Fall>, u8, u8)>,
    }

    impl Stub {
        fn at(&self, x: i32, z: i32) -> &(i32, Option<Fall>, u8, u8) {
            let (dx, dz) = (x - self.camera[0], z - self.camera[2]);
            assert!(dx.abs() <= 10 && dz.abs() <= 10, "read at {x},{z}");
            &self.columns[((dz + 10) * 21 + dx + 10) as usize]
        }
    }

    impl Ground for Stub {
        fn height(&self, x: i32, z: i32) -> i32 {
            self.at(x, z).0
        }

        fn fall(&self, pos: [i32; 3]) -> Option<Fall> {
            assert_eq!(
                pos[1], self.camera[1],
                "precipitation is read at the camera's height"
            );
            self.at(pos[0], pos[2]).1
        }

        fn light(&self, pos: [i32; 3]) -> (u8, u8) {
            let c = self.at(pos[0], pos[2]);
            (c.2, c.3)
        }
    }

    struct Fnv(u64);

    impl Fnv {
        fn mix(&mut self, word: u32) {
            for b in word.to_le_bytes() {
                self.0 ^= u64::from(b);
                self.0 = self.0.wrapping_mul(0x100_0000_01b3);
            }
        }
    }

    /// The vertices of a quad, as the game's `prepareInstances` writes them.
    fn vertices(q: &Quad, h: &mut Fnv) {
        let colour = u32::from(q.alpha) << 24 | 0xFF_FFFF;
        let corners = [
            ([q.start[0], q.y[1], q.start[1]], [q.u[0], q.v[0]]),
            ([q.end[0], q.y[1], q.end[1]], [q.u[1], q.v[0]]),
            ([q.end[0], q.y[0], q.end[1]], [q.u[1], q.v[1]]),
            ([q.start[0], q.y[0], q.start[1]], [q.u[0], q.v[1]]),
        ];
        for (p, uv) in corners {
            p.iter().for_each(|c| h.mix(c.to_bits()));
            uv.iter().for_each(|c| h.mix(c.to_bits()));
            h.mix(colour);
            h.mix(q.light);
        }
    }

    #[test]
    fn the_columns_and_quads_are_the_games() {
        let mut g = Lcg(0x5765_6174_6865_7231);
        let sizes = ColumnSizes::new();
        let (mut rain, mut snow) = (Vec::new(), Vec::new());
        for (i, want) in crate::weather_golden::CASES.iter().enumerate() {
            let cam_x = f64::from(g.below(4000) - 2000) + f64::from(g.below(1000)) / 1000.0;
            let cam_y = f64::from(g.below(300) - 60) + f64::from(g.below(1000)) / 1000.0;
            let cam_z = f64::from(g.below(4000) - 2000) + f64::from(g.below(1000)) / 1000.0;
            let camera = [cam_x, cam_y, cam_z].map(|c| c.floor() as i32);
            let radius = 3 + g.below(8);
            let intensity = if g.below(4) == 0 {
                1.0
            } else {
                g.below(1001) as f32 / 1000.0
            };
            let game_time = if g.below(4) == 0 {
                i64::from(g.below(1 << 30)) * 4096 + i64::from(g.below(4096))
            } else {
                i64::from(g.below(300_000))
            };
            let partial = g.below(1001) as f32 / 1000.0;
            let columns = (0..441)
                .map(|_| {
                    let height = camera[1] + g.below(30) - 15;
                    let fall = if g.below(5) < 2 {
                        None
                    } else if g.below(2) == 0 {
                        Some(Fall::Rain)
                    } else {
                        Some(Fall::Snow)
                    };
                    let block = g.below(16) as u8;
                    let sky = g.below(16) as u8;
                    (height, fall, block, sky)
                })
                .collect();
            let stub = Stub { camera, columns };
            gather(&stub, camera, radius, &mut rain, &mut snow);
            let m = Moment {
                camera: [cam_x, cam_y, cam_z],
                radius,
                intensity,
                game_time,
                partial,
            };
            let mut h = Fnv(0xcbf2_9ce4_8422_2325);
            for c in rain.iter().chain(&snow) {
                vertices(&quad(c, &m, &sizes), &mut h);
            }
            assert_eq!(
                (rain.len() as u32, snow.len() as u32, h.0),
                *want,
                "case {i}: camera {:?} radius {radius} rain {intensity} time {game_time} partial {partial}",
                m.camera
            );
        }
    }

    #[test]
    fn a_column_s_generator_is_seeded_by_its_place() {
        assert_eq!(column_seed(0, 0), 0);
        assert_eq!(column_seed(1, 0), 3121 + 45_238_971);
        // Overflowing as Java's ints do.
        assert_eq!(
            column_seed(100_000, 0),
            100_000i32
                .wrapping_mul(100_000)
                .wrapping_mul(3121)
                .wrapping_add(100_000i32.wrapping_mul(45_238_971))
        );
        assert_eq!(draws(Fall::Rain, 5, -7), draws(Fall::Rain, 5, -7));
        assert_ne!(draws(Fall::Rain, 5, -7), draws(Fall::Rain, 5, -8));
    }
}
