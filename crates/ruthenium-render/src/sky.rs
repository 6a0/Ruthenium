//! The sky: the dome in the sky's colour, the glow of a sunrise or sunset, the sun, the
//! moon in its phase and the stars; the End's sky. Everything is a small mesh around
//! the camera that turns with nothing but the camera, built once, as 26.3's
//! `SkyRenderer` builds them.
//!
//! Vanilla draws the sky first, over the cleared target, and the world over it. Drawn
//! that way here it is the reference; drawn after the world's opaque layers, at the
//! depth nothing but the clear has, it comes to the same picture and the world's own
//! pixels are never shaded twice.

use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, BufferPool, Error, FrameData, Gpu, Pass, PoolConfig, RangeHandle, SamplerId,
    TextureDesc, TextureId, TextureTable, UploadRing, UploadTicket, VertexAttribute, VertexInput,
    vk,
};
use ruthenium_javamath::mth;
use ruthenium_shaders::interface::SKY;

use crate::atmosphere::fog_line;
use crate::pipelines::PipelineId;
use crate::terrain::View;

/// Bytes of one sky vertex: a position, a texture coordinate and a colour.
pub const SKY_VERTEX_BYTES: usize = 24;

/// The layout the sky pipelines read: `sky.vert`'s inputs at locations 0, 1 and 2.
pub const SKY_VERTEX: VertexInput = VertexInput {
    stride: SKY_VERTEX_BYTES as u32,
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
            format: vk::Format::R8G8B8A8_UNORM,
        },
    ],
    per_instance: false,
};

/// `sky.frag`'s kinds of draw.
pub(crate) const FOGGED: u32 = 0;
pub(crate) const COLOURED: u32 = 1;
const TEXTURED: u32 = 2;

/// How far out the dome reaches and how high above (or below) the camera its middle
/// is (`SkyRenderer.buildSkyDisc`).
const DOME_RADIUS: f32 = 512.0;
const DOME_HEIGHT: f32 = 16.0;
/// How far the dark half below the horizon is raised.
const DARK_RAISE: f32 = 12.0;
/// The distance of the sun, the moon and the stars, and half the widths of the first
/// two.
const BODY_DISTANCE: f32 = 100.0;
const SUN_SIZE: f32 = 30.0;
const MOON_SIZE: f32 = 20.0;
/// Stars tried, and the seed they are scattered from.
const STAR_TRIES: usize = 1500;
const STAR_SEED: i64 = 10842;
/// The End's sky: half the side of its box, how often its texture repeats along a
/// side, and the grey it is multiplied by.
const END_HALF: f32 = 100.0;
const END_REPEATS: f32 = 16.0;
const END_GREY: [u8; 4] = [0x28, 0x28, 0x28, 0xFF];
const WHITE: [u8; 4] = [255; 4];

/// A picture of the sky, as RGBA8.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkyImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pictures the sky is drawn from, as the game's files have them
/// (`textures/environment/**`). One that is missing is not drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkyImages {
    pub sun: Option<SkyImage>,
    /// The moon's phases, from full through waning and new to waxing.
    pub moon: [Option<SkyImage>; 8],
    pub end_sky: Option<SkyImage>,
}

/// The files of the moon's phases under `textures/environment/celestial/moon/`, in the
/// order of [`SkyImages::moon`] (`MoonPhase`).
pub const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

/// When a frame's sky is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkyOrder {
    /// After the world's opaque layers, only where they left nothing: the same
    /// picture, without shading sky that the world then covers.
    #[default]
    Late,
    /// Before the world, as vanilla draws it: what `Late` is measured against.
    First,
    /// Not at all: the cleared target shows the fog's colour.
    Off,
}

impl SkyOrder {
    pub const ALL: [SkyOrder; 3] = [SkyOrder::Late, SkyOrder::First, SkyOrder::Off];

    pub fn name(self) -> &'static str {
        match self {
            SkyOrder::Late => "late",
            SkyOrder::First => "first",
            SkyOrder::Off => "off",
        }
    }

    pub fn parse(name: &str) -> Option<SkyOrder> {
        SkyOrder::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// What of the sky a frame shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sky {
    /// Nothing: a dimension without one, or a camera that cannot see it.
    None,
    Overworld(OverworldSky),
    End,
}

/// The sky over a dimension with a sun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverworldSky {
    pub colour: [f32; 3],
    /// The glow over the horizon where the sun rises or sets; its alpha is its
    /// strength.
    pub sunrise: [f32; 4],
    /// In radians, from straight overhead.
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub star_angle: f32,
    /// 0 (full) to 7.
    pub moon_phase: u8,
    /// How much of the sun and the moon the rain leaves.
    pub rain_brightness: f32,
    pub star_brightness: f32,
    /// The camera is below the horizon: the lower half of the sky is black.
    pub dark_disc: bool,
}

/// A column-major 4 x 4 matrix.
type Matrix = [f32; 16];

const IDENTITY: Matrix = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `a` after `b`: the matrix that does `b` first.
fn after(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|i| {
        let (column, row) = (i / 4, i % 4);
        (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum()
    })
}

/// A turn about one axis (0: x, 1: y, 2: z) by `radians`, counter-clockwise seen from
/// the axis's far end.
fn turn(axis: usize, radians: f32) -> Matrix {
    let (s, c) = radians.sin_cos();
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut m = IDENTITY;
    m[u * 4 + u] = c;
    m[u * 4 + v] = s;
    m[v * 4 + u] = -s;
    m[v * 4 + v] = c;
    m
}

fn moved(by: [f32; 3]) -> Matrix {
    let mut m = IDENTITY;
    m[12..15].copy_from_slice(&by);
    m
}

fn scaled(by: [f32; 3]) -> Matrix {
    let mut m = IDENTITY;
    (m[0], m[5], m[10]) = (by[0], by[1], by[2]);
    m
}

/// The product of `steps`, the last done first: as a stack of transforms is read.
fn chain(steps: &[Matrix]) -> Matrix {
    steps.iter().fold(IDENTITY, |m, s| after(&m, s))
}

pub(crate) fn vertex(out: &mut Vec<u8>, position: [f32; 3], uv: [f32; 2], colour: [u8; 4]) {
    for c in position {
        out.extend_from_slice(&c.to_le_bytes());
    }
    for c in uv {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&colour);
}

/// A run of vertices in the sky's buffer: its first vertex and how many.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Piece {
    first: u32,
    count: u32,
}

/// Appends triangles and says where they went.
fn piece(out: &mut Vec<u8>, build: impl FnOnce(&mut Vec<u8>)) -> Piece {
    let first = (out.len() / SKY_VERTEX_BYTES) as u32;
    build(out);
    Piece {
        first,
        count: (out.len() / SKY_VERTEX_BYTES) as u32 - first,
    }
}

/// A fan around its first corner, as the triangles it stands for.
fn fan(out: &mut Vec<u8>, corners: &[([f32; 3], [u8; 4])]) {
    for pair in corners[1..].windows(2) {
        for (position, colour) in [corners[0], pair[0], pair[1]] {
            vertex(out, position, [0.0; 2], colour);
        }
    }
}

/// Four corners as two triangles that share the diagonal from the first to the third.
fn quad(out: &mut Vec<u8>, corners: [([f32; 3], [f32; 2]); 4], colour: [u8; 4]) {
    for i in [0, 1, 2, 2, 3, 0] {
        vertex(out, corners[i].0, corners[i].1, colour);
    }
}

/// The dome over the camera (`up`) or the disc under it: a middle and eight corners
/// on a circle, all at one height. The lower one goes round the other way, to face
/// the camera as well.
fn dome(out: &mut Vec<u8>, up: bool) {
    let y = if up { DOME_HEIGHT } else { -DOME_HEIGHT };
    let side = if up { DOME_RADIUS } else { -DOME_RADIUS };
    let mut corners = vec![([0.0, y, 0.0], WHITE)];
    for degrees in (-180..=180).step_by(45) {
        let a = f64::from(degrees as f32 * 0.017_453_292);
        corners.push(([side * mth::cos(a), y, DOME_RADIUS * mth::sin(a)], WHITE));
    }
    fan(out, &corners);
}

/// The glow of a sunrise: a cone from an opaque middle 100 blocks out to a transparent
/// rim (`buildSunriseFan`).
fn sunrise(out: &mut Vec<u8>) {
    let mut corners = vec![([0.0, 100.0, 0.0], WHITE)];
    for i in 0..=16 {
        let a = f64::from(i as f32 * std::f32::consts::TAU / 16.0);
        let (s, c) = (mth::sin(a), mth::cos(a));
        corners.push(([s * 120.0, c * 120.0, -c * 40.0], [255, 255, 255, 0]));
    }
    fan(out, &corners);
}

/// A flat square of side 2 around the origin, facing up and down, showing its whole
/// picture; the moon's is turned half round (`buildCelestialQuad`, `buildMoonPhases`).
fn body(out: &mut Vec<u8>, moon: bool) {
    let corners = [
        [-1.0, 0.0, -1.0],
        [1.0, 0.0, -1.0],
        [1.0, 0.0, 1.0],
        [-1.0, 0.0, 1.0],
    ];
    let uvs = if moon {
        [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]]
    } else {
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
    };
    quad(out, std::array::from_fn(|i| (corners[i], uvs[i])), WHITE);
}

/// `java.util.Random`'s numbers, as the game's single-threaded random source draws them.
struct Random(i64);

impl Random {
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> Random {
        Random((seed ^ 0x5_DEEC_E66D) & Random::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5_DEEC_E66D).wrapping_add(0xB)) & Random::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    fn float(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5e-8
    }

    fn double(&mut self) -> f64 {
        let upper = i64::from(self.next(26));
        let lower = i64::from(self.next(27));
        ((upper << 27) + lower) as f64 * 1.110_223_024_625_156_5e-16
    }
}

/// The corners of every star, four each (`buildStars`): small squares 100 blocks out
/// in directions drawn evenly from inside a ball, each facing the middle and turned
/// about its own direction by a random angle.
pub(crate) fn star_corners() -> Vec<[f32; 3]> {
    let mut random = Random::new(STAR_SEED);
    let mut out = Vec::with_capacity(STAR_TRIES * 4);
    for _ in 0..STAR_TRIES {
        let p: [f32; 3] = std::array::from_fn(|_| random.float() * 2.0 - 1.0);
        let size = 0.15 + random.float() * 0.1;
        let length_squared = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        // Outside the ball or too near its middle to have a direction: no star, and
        // no further draw.
        if length_squared <= 0.010_000_001 || length_squared >= 1.0 {
            continue;
        }
        let to_length = 1.0 / length_squared.sqrt() * BODY_DISTANCE;
        let centre = p.map(|c| c * to_length);
        let spin = (random.double() * f64::from(std::f32::consts::PI) * 2.0) as f32;
        // The square's own axes: `ahead` looks from the star at the middle, `left` is
        // level, `above` completes them.
        let inverse =
            1.0 / (centre[0] * centre[0] + centre[1] * centre[1] + centre[2] * centre[2]).sqrt();
        let ahead = centre.map(|c| -c * inverse);
        let level = [ahead[2], 0.0, -ahead[0]];
        let inverse = 1.0 / (level[0] * level[0] + level[2] * level[2]).sqrt();
        let left = level.map(|c| c * inverse);
        let above = [
            ahead[1] * left[2] - ahead[2] * left[1],
            ahead[2] * left[0] - ahead[0] * left[2],
            ahead[0] * left[1] - ahead[1] * left[0],
        ];
        // Turned about `ahead` by minus the spin.
        let (s, c) = (-spin).sin_cos();
        let x_axis: [f32; 3] = std::array::from_fn(|i| left[i] * c + above[i] * s);
        let y_axis: [f32; 3] = std::array::from_fn(|i| above[i] * c - left[i] * s);
        for (x, y) in [(size, -size), (size, size), (-size, size), (-size, -size)] {
            out.push(std::array::from_fn(|i| {
                x_axis[i] * x + y_axis[i] * y + centre[i]
            }));
        }
    }
    out
}

/// The End's sky: a box of six faces around the camera (`buildEndSky`). Each is the
/// floor turned into place.
fn end_box(out: &mut Vec<u8>) {
    let quarter = std::f32::consts::FRAC_PI_2;
    let turns = [
        IDENTITY,
        turn(0, quarter),
        turn(0, -quarter),
        turn(0, std::f32::consts::PI),
        turn(2, quarter),
        turn(2, -quarter),
    ];
    let floor = [
        ([-END_HALF, -END_HALF, -END_HALF], [0.0, 0.0]),
        ([-END_HALF, -END_HALF, END_HALF], [0.0, END_REPEATS]),
        ([END_HALF, -END_HALF, END_HALF], [END_REPEATS, END_REPEATS]),
        ([END_HALF, -END_HALF, -END_HALF], [END_REPEATS, 0.0]),
    ];
    for m in turns {
        let face = floor.map(|(p, uv)| {
            let at: [f32; 3] =
                std::array::from_fn(|row| m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2]);
            (at, uv)
        });
        quad(out, face, END_GREY);
    }
}

/// Where each mesh is in the buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pieces {
    dome: Piece,
    dark: Piece,
    sunrise: Piece,
    sun: Piece,
    moon: Piece,
    stars: Piece,
    end: Piece,
}

/// Every mesh of the sky, packed, and where each is.
fn meshes() -> (Vec<u8>, Pieces) {
    let mut out = Vec::new();
    let pieces = Pieces {
        dome: piece(&mut out, |o| dome(o, true)),
        dark: piece(&mut out, |o| dome(o, false)),
        sunrise: piece(&mut out, sunrise),
        sun: piece(&mut out, |o| body(o, false)),
        moon: piece(&mut out, |o| body(o, true)),
        stars: piece(&mut out, |o| {
            for star in star_corners().as_chunks::<4>().0 {
                quad(o, star.map(|p| (p, [0.0; 2])), WHITE);
            }
        }),
        end: piece(&mut out, end_box),
    };
    (out, pieces)
}

/// One draw of the sky, before it is recorded.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Draw {
    piece: Piece,
    /// Which of the three blends: 0 covers, 1 blends over, 2 adds light.
    blend: usize,
    model: Matrix,
    colour: [f32; 4],
    kind: u32,
    texture: TextureId,
    sampler: SamplerId,
}

/// The pictures a frame can draw from: those that have arrived.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Pictures {
    sun: Option<TextureId>,
    moon: [Option<TextureId>; 8],
    end_sky: Option<TextureId>,
}

/// The draws of `sky` into `draws`, in the order they are blended.
fn plan(sky: &Sky, pieces: &Pieces, pictures: &Pictures, draws: &mut Vec<Draw>) {
    draws.clear();
    let none = TextureId::MISSING;
    match *sky {
        Sky::None => {}
        Sky::End => {
            if let Some(texture) = pictures.end_sky {
                draws.push(Draw {
                    piece: pieces.end,
                    blend: 1,
                    model: IDENTITY,
                    colour: [1.0; 4],
                    kind: TEXTURED,
                    texture,
                    sampler: SamplerId::NearestRepeat,
                });
            }
        }
        Sky::Overworld(s) => {
            let flat = |piece, blend, model, colour, kind| Draw {
                piece,
                blend,
                model,
                colour,
                kind,
                texture: none,
                sampler: SamplerId::NearestClamp,
            };
            let [r, g, b] = s.colour;
            draws.push(flat(pieces.dome, 0, IDENTITY, [r, g, b, 1.0], FOGGED));
            let glow = s.sunrise[3];
            if glow > 0.001 {
                // Over the horizon on the side the sun is on, as tall as it is strong.
                let side = if mth::sin(f64::from(s.sun_angle)) < 0.0 {
                    180.0_f32
                } else {
                    0.0
                };
                let model = chain(&[
                    turn(0, 90.0_f32.to_radians()),
                    turn(2, (side + 90.0).to_radians()),
                    scaled([1.0, 1.0, glow]),
                ]);
                draws.push(flat(pieces.sunrise, 1, model, s.sunrise, COLOURED));
            }
            // The sun rises in the east and sets in the west, and the moon and the
            // stars turn about the same axis.
            let east = turn(1, (-90.0_f32).to_radians());
            let light = [1.0, 1.0, 1.0, s.rain_brightness];
            let across = |angle: f32, size: f32| {
                chain(&[
                    east,
                    turn(0, angle),
                    moved([0.0, BODY_DISTANCE, 0.0]),
                    scaled([size, 1.0, size]),
                ])
            };
            let body = |piece, model, texture| Draw {
                piece,
                blend: 2,
                model,
                colour: light,
                kind: TEXTURED,
                texture,
                sampler: SamplerId::NearestClamp,
            };
            if let Some(texture) = pictures.sun {
                draws.push(body(pieces.sun, across(s.sun_angle, SUN_SIZE), texture));
            }
            if let Some(texture) = pictures.moon[usize::from(s.moon_phase.min(7))] {
                draws.push(body(pieces.moon, across(s.moon_angle, MOON_SIZE), texture));
            }
            if s.star_brightness > 0.0 {
                let model = chain(&[east, turn(0, s.star_angle)]);
                draws.push(flat(
                    pieces.stars,
                    2,
                    model,
                    [s.star_brightness; 4],
                    COLOURED,
                ));
            }
            if s.dark_disc {
                draws.push(flat(
                    pieces.dark,
                    0,
                    moved([0.0, DARK_RAISE, 0.0]),
                    [0.0, 0.0, 0.0, 1.0],
                    FOGGED,
                ));
            }
        }
    }
}

/// Draws the sky.
#[derive(Debug)]
pub(crate) struct SkyRenderer {
    pool: BufferPool,
    range: RangeHandle,
    /// The meshes until they are staged, and their upload after.
    bytes: Vec<u8>,
    upload: Option<UploadTicket>,
    ready: bool,
    pieces: Pieces,
    sun: Option<TextureId>,
    moon: [Option<TextureId>; 8],
    end_sky: Option<TextureId>,
    draws: Vec<Draw>,
}

/// Bytes of the sky's push constants.
pub(crate) const SKY_PUSH_BYTES: usize = 60;
/// Where they are, in bytes: two words the vertex stage reads (the word offset of the
/// draw's record, and that the draw is a real one), then what every fragment reads:
/// the kind of draw, its texture, its colour, the fog's colour and where the sky's fog
/// is complete, the sampler, and one over that end as the scale of a fog line
/// ([`fog_line`]). `sky.frag` declares the same block.
pub(crate) const PUSH_KIND: usize = 8;
pub(crate) const PUSH_TEXTURE: usize = 12;
pub(crate) const PUSH_COLOUR: usize = 16;
pub(crate) const PUSH_FOG_COLOUR: usize = 32;
pub(crate) const PUSH_FOG_END: usize = 48;
pub(crate) const PUSH_SAMPLER: usize = 52;
pub(crate) const PUSH_FOG_SCALE: usize = 56;

/// The sky's push constants for one draw whose record is at `record`.
fn sky_push_of(record: u32, draw: &Draw, fog: [f32; 4], fog_end: f32) -> [u8; SKY_PUSH_BYTES] {
    sky_push(
        record,
        draw.kind,
        draw.texture,
        draw.sampler,
        draw.colour,
        fog,
        fog_end,
    )
}

pub(crate) fn sky_push(
    record: u32,
    kind: u32,
    texture: TextureId,
    sampler: SamplerId,
    colour: [f32; 4],
    fog: [f32; 4],
    fog_end: f32,
) -> [u8; SKY_PUSH_BYTES] {
    let mut push = [0u8; SKY_PUSH_BYTES];
    let mut put = |at: usize, word: [u8; 4]| push[at..at + 4].copy_from_slice(&word);
    put(0, record.to_ne_bytes());
    put(4, 1u32.to_ne_bytes());
    put(PUSH_KIND, kind.to_ne_bytes());
    put(PUSH_TEXTURE, texture.index().to_ne_bytes());
    for i in 0..4 {
        put(PUSH_COLOUR + i * 4, colour[i].to_ne_bytes());
        put(PUSH_FOG_COLOUR + i * 4, fog[i].to_ne_bytes());
    }
    put(PUSH_FOG_END, fog_end.to_ne_bytes());
    put(PUSH_SAMPLER, sampler.index().to_ne_bytes());
    // The fog towards the dome's rim: from none at the camera to all at its end.
    put(PUSH_FOG_SCALE, fog_line([0.0, fog_end])[0].to_ne_bytes());
    push
}

impl SkyRenderer {
    /// Builds the meshes and creates the pictures' textures. Not on a frame's path.
    pub fn new(
        gpu: &Arc<Gpu>,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
        images: &SkyImages,
    ) -> Result<SkyRenderer, Error> {
        let (bytes, pieces) = meshes();
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
            .map_err(|e| Error::Unsupported(format!("no room for the sky's meshes: {e:?}")))?;
        let mut texture = |image: &Option<SkyImage>| {
            let i = image.as_ref()?;
            if i.rgba.len() != i.width as usize * i.height as usize * 4 || i.rgba.is_empty() {
                return None;
            }
            textures
                .create_with(
                    uploads,
                    TextureDesc::rgba8(i.width, i.height),
                    &[&i.rgba],
                    Arrival::NextFrame,
                )
                .ok()
        };
        let sun = texture(&images.sun);
        let moon = std::array::from_fn(|i| texture(&images.moon[i]));
        let end_sky = texture(&images.end_sky);
        Ok(SkyRenderer {
            pool,
            range,
            bytes,
            upload: None,
            ready: false,
            pieces,
            sun,
            moon,
            end_sky,
            draws: Vec::new(),
        })
    }

    /// Stages the meshes, until that has worked once: before the frame's
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

    /// Notes whether the meshes can be read: after the frame's `UploadRing::record`.
    pub fn note_ready(&mut self, uploads: &UploadRing) {
        if !self.ready {
            self.ready = self.upload.is_some_and(|t| uploads.is_ready(t));
        }
    }

    /// Draws `sky` into `pass`, which has a depth attachment: over whatever is there,
    /// or with `behind` only where nothing has written its depth. `fog` is the fog's
    /// colour and `fog_end` where the sky's fog is complete.
    ///
    /// # Safety
    /// `pass` is of the frame being recorded, whose `UploadRing::record` and
    /// [`Self::note_ready`] have run.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &mut self,
        pass: &mut Pass<'_>,
        frame_data: &mut FrameData,
        textures: &TextureTable,
        view: &View,
        sky: &Sky,
        fog: [f32; 4],
        fog_end: f32,
        behind: bool,
    ) {
        profiling::scope!("sky_draw");
        let Some(range) = self.pool.resolve(self.range).filter(|_| self.ready) else {
            return;
        };
        let usable = |id: Option<TextureId>| id.filter(|t| textures.is_ready(*t));
        let pictures = Pictures {
            sun: usable(self.sun),
            moon: self.moon.map(usable),
            end_sky: usable(self.end_sky),
        };
        plan(sky, &self.pieces, &pictures, &mut self.draws);
        if self.draws.is_empty() {
            return;
        }
        let pipelines = if behind {
            [
                PipelineId::SkyFlatBehind,
                PipelineId::SkyBlendBehind,
                PipelineId::SkyGlowBehind,
            ]
        } else {
            [
                PipelineId::SkyFlat,
                PipelineId::SkyBlend,
                PipelineId::SkyGlow,
            ]
        };
        // SAFETY: the vertex buffer's copy can be read (`ready`), and every draw reads
        // vertices inside what was copied; each record is written just before the draw
        // that reads it; the textures are of the table and have arrived (`plan`).
        unsafe {
            pass.bind_vertex_buffer(range.buffer, range.offset);
            for d in &self.draws {
                let Some((record, bytes)) = frame_data.alloc(SKY.words(), 4) else {
                    return;
                };
                let transform = after(&view.view_projection, &d.model);
                for (i, c) in transform.iter().enumerate() {
                    bytes[i * 4..i * 4 + 4].copy_from_slice(&c.to_ne_bytes());
                }
                pass.draw_vertices(
                    pipelines[d.blend].index(),
                    &sky_push_of(record, d, fog, fog_end),
                    d.piece.first,
                    d.piece.count,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Matrix, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row]
        })
    }

    fn close(a: [f32; 3], b: [f32; 3], within: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= within)
    }

    /// The vertices of a piece: position, texture coordinate, colour.
    fn vertices(bytes: &[u8], piece: Piece) -> Vec<([f32; 3], [f32; 2], [u8; 4])> {
        let float = |b: &[u8]| f32::from_le_bytes(b.try_into().unwrap());
        bytes
            .chunks(SKY_VERTEX_BYTES)
            .skip(piece.first as usize)
            .take(piece.count as usize)
            .map(|v| {
                (
                    std::array::from_fn(|i| float(&v[i * 4..i * 4 + 4])),
                    std::array::from_fn(|i| float(&v[12 + i * 4..16 + i * 4])),
                    v[20..24].try_into().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn turns_go_counter_clockwise_about_their_axes() {
        let quarter = std::f32::consts::FRAC_PI_2;
        assert!(close(
            apply(&turn(0, quarter), [0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0],
            1e-6
        ));
        assert!(close(
            apply(&turn(1, quarter), [0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0],
            1e-6
        ));
        assert!(close(
            apply(&turn(2, quarter), [1.0, 0.0, 0.0]),
            [0.0, 1.0, 0.0],
            1e-6
        ));
        // A chain's last step is done first: scaled, then moved.
        let m = chain(&[moved([0.0, 10.0, 0.0]), scaled([2.0, 3.0, 4.0])]);
        assert_eq!(apply(&m, [1.0, 1.0, 1.0]), [2.0, 13.0, 4.0]);
        let other = chain(&[scaled([2.0, 3.0, 4.0]), moved([0.0, 10.0, 0.0])]);
        assert_eq!(apply(&other, [1.0, 1.0, 1.0]), [2.0, 33.0, 4.0]);
    }

    #[test]
    fn the_stars_are_the_games() {
        // What 26.3's own random source and matrices give (research/tools/StarGolden):
        // 780 of the 1500 tries are stars; the first six stars' corners; and the sums
        // of every corner's coordinates and of their sizes.
        let stars = star_corners();
        assert_eq!(stars.len(), 780 * 4);
        let first: [[f32; 3]; 24] = [
            [-53.2247, 70.0560, 47.5324],
            [-53.0685, 70.0037, 47.7835],
            [-53.2690, 69.7954, 47.8649],
            [-53.4253, 69.8478, 47.6138],
            [-96.9452, -18.1247, 16.5303],
            [-96.8228, -18.5197, 16.8076],
            [-96.8366, -18.8086, 16.4023],
            [-96.9590, -18.4136, 16.1249],
            [-98.8290, 9.6915, 11.7895],
            [-98.7680, 9.9266, 12.1009],
            [-98.7696, 9.6116, 12.3392],
            [-98.8306, 9.3764, 12.0278],
            [49.7843, 61.2451, 61.4057],
            [50.0122, 60.9006, 61.5629],
            [49.7039, 60.8383, 61.8734],
            [49.4760, 61.1828, 61.7162],
            [-48.6572, 30.4923, -81.8704],
            [-48.3497, 30.6280, -82.0018],
            [-48.4204, 30.9435, -81.8415],
            [-48.7279, 30.8078, -81.7101],
            [3.8311, 57.6360, 81.6303],
            [4.1664, 57.8597, 81.4555],
            [4.4498, 57.5787, 81.6394],
            [4.1144, 57.3550, 81.8143],
        ];
        for (i, want) in first.iter().enumerate() {
            assert!(close(stars[i], *want, 2e-4), "corner {i}: {:?}", stars[i]);
        }
        let sum: [f64; 3] =
            std::array::from_fn(|a| stars.iter().map(|c| f64::from(c[a])).sum::<f64>());
        for (got, want) in sum.iter().zip([6383.723, -5206.500, 624.726]) {
            assert!((got - want).abs() < 0.05, "{sum:?}");
        }
        let sizes: f64 = stars.iter().flatten().map(|c| f64::from(c.abs())).sum();
        assert!((sizes - 467_386.083).abs() < 0.5, "{sizes}");
    }

    #[test]
    fn the_meshes_are_whole_triangles_where_the_pieces_say() {
        let (bytes, pieces) = meshes();
        assert_eq!(bytes.len() % SKY_VERTEX_BYTES, 0);
        let counts = [
            (pieces.dome, 8 * 3),
            (pieces.dark, 8 * 3),
            (pieces.sunrise, 16 * 3),
            (pieces.sun, 6),
            (pieces.moon, 6),
            (pieces.stars, 780 * 6),
            (pieces.end, 6 * 6),
        ];
        // One after the other, with nothing between.
        let mut next = 0;
        for (piece, count) in counts {
            assert_eq!((piece.first, piece.count), (next, count));
            next += count;
        }
        assert_eq!(next as usize * SKY_VERTEX_BYTES, bytes.len());
        // The dome: its middle 16 blocks up, its rim 512 out, eight slices that meet
        // the rim at every eighth of a turn; the dark disc the same below.
        for (piece, y) in [(pieces.dome, 16.0), (pieces.dark, -16.0)] {
            let v = vertices(&bytes, piece);
            let mut rim = std::collections::BTreeSet::new();
            for triangle in v.chunks(3) {
                assert_eq!(triangle[0].0, [0.0, y, 0.0]);
                for (p, ..) in &triangle[1..] {
                    assert_eq!(p[1], y);
                    assert!(((p[0] * p[0] + p[2] * p[2]).sqrt() - 512.0).abs() < 0.05);
                    rim.insert(((p[0] / 64.0).round() as i32, (p[2] / 64.0).round() as i32));
                }
            }
            assert_eq!(rim.len(), 8);
        }
        // The glow: an opaque middle 100 out, a transparent rim.
        let glow = vertices(&bytes, pieces.sunrise);
        for triangle in glow.chunks(3) {
            assert_eq!((triangle[0].0, triangle[0].2), ([0.0, 100.0, 0.0], WHITE));
            for (p, _, colour) in &triangle[1..] {
                assert_eq!(*colour, [255, 255, 255, 0]);
                assert!(((p[0] * p[0] + p[1] * p[1]).sqrt() - 120.0).abs() < 0.01);
                assert!((p[2] + p[1] / 3.0).abs() < 0.01);
            }
        }
        // The sun shows its picture as it is, the moon's turned half round.
        let (sun, moon) = (vertices(&bytes, pieces.sun), vertices(&bytes, pieces.moon));
        assert_eq!((sun[0].0, sun[0].1), ([-1.0, 0.0, -1.0], [0.0, 0.0]));
        assert_eq!((sun[2].0, sun[2].1), ([1.0, 0.0, 1.0], [1.0, 1.0]));
        assert_eq!((moon[0].0, moon[0].1), ([-1.0, 0.0, -1.0], [1.0, 1.0]));
        assert_eq!((moon[1].0, moon[1].1), ([1.0, 0.0, -1.0], [0.0, 1.0]));
        // The End's box: six faces, each flat 100 out along one axis, each way once,
        // its picture sixteen times across.
        let end = vertices(&bytes, pieces.end);
        let mut faces = std::collections::BTreeSet::new();
        for face in end.chunks(6) {
            let flat: Vec<(usize, i32)> = (0..3)
                .filter(|&a| {
                    face.iter()
                        .all(|(p, ..)| (p[a] - face[0].0[a]).abs() < 1e-3)
                })
                .map(|a| (a, face[0].0[a].round() as i32))
                .collect();
            assert_eq!(flat.len(), 1, "{face:?}");
            assert_eq!(flat[0].1.abs(), 100);
            faces.insert(flat[0]);
            assert!(face.iter().all(|(_, uv, colour)| {
                *colour == END_GREY && uv.iter().all(|c| *c == 0.0 || *c == 16.0)
            }));
        }
        assert_eq!(faces.len(), 6);
    }

    fn noon() -> OverworldSky {
        OverworldSky {
            colour: [0.4, 0.6, 1.0],
            sunrise: [0.0; 4],
            sun_angle: 0.0,
            moon_angle: std::f32::consts::PI,
            star_angle: 0.0,
            moon_phase: 0,
            rain_brightness: 1.0,
            star_brightness: 0.0,
            dark_disc: false,
        }
    }

    fn planned(sky: &Sky, pictures: &Pictures) -> Vec<Draw> {
        let (_, pieces) = meshes();
        let mut draws = Vec::new();
        plan(sky, &pieces, pictures, &mut draws);
        draws
    }

    fn all_pictures() -> Pictures {
        Pictures {
            sun: Some(TextureId::MISSING),
            moon: [Some(TextureId::MISSING); 8],
            end_sky: Some(TextureId::MISSING),
        }
    }

    #[test]
    fn a_clear_noon_is_the_dome_the_sun_and_the_moon() {
        let (_, pieces) = meshes();
        let draws = planned(&Sky::Overworld(noon()), &all_pictures());
        let what: Vec<(Piece, usize, u32)> =
            draws.iter().map(|d| (d.piece, d.blend, d.kind)).collect();
        assert_eq!(
            what,
            [
                (pieces.dome, 0, FOGGED),
                (pieces.sun, 2, TEXTURED),
                (pieces.moon, 2, TEXTURED)
            ]
        );
        assert_eq!(draws[0].colour, [0.4, 0.6, 1.0, 1.0]);
        assert_eq!(draws[0].model, IDENTITY);
        // The sun stands overhead, 100 blocks up and 60 across; the moon underfoot,
        // 40 across.
        assert!(close(
            apply(&draws[1].model, [0.0; 3]),
            [0.0, 100.0, 0.0],
            1e-3
        ));
        assert!(close(
            apply(&draws[1].model, [1.0, 0.0, 1.0]),
            [-30.0, 100.0, 30.0],
            1e-3
        ));
        assert!(close(
            apply(&draws[2].model, [0.0; 3]),
            [0.0, -100.0, 0.0],
            1e-3
        ));
        let across = apply(&draws[2].model, [1.0, 0.0, 1.0]);
        assert!((across[0].abs() - 20.0).abs() < 1e-3 && (across[2].abs() - 20.0).abs() < 1e-3);
        // Nothing of the sun without its picture; the dome stays.
        let bare = planned(&Sky::Overworld(noon()), &Pictures::default());
        assert_eq!(bare.len(), 1);
        assert_eq!(planned(&Sky::None, &all_pictures()), []);
    }

    #[test]
    fn the_sun_sets_in_the_west() {
        let evening = OverworldSky {
            sun_angle: 80.0_f32.to_radians(),
            ..noon()
        };
        let draws = planned(&Sky::Overworld(evening), &all_pictures());
        let sun = apply(&draws[1].model, [0.0; 3]);
        // Ten degrees over the horizon, towards -x.
        assert!(close(sun, [-98.4808, 17.3648, 0.0], 1e-3), "{sun:?}");
        let morning = OverworldSky {
            sun_angle: 280.0_f32.to_radians(),
            ..noon()
        };
        let draws = planned(&Sky::Overworld(morning), &all_pictures());
        assert!(close(
            apply(&draws[1].model, [0.0; 3]),
            [98.4808, 17.3648, 0.0],
            1e-3
        ));
    }

    #[test]
    fn a_sunset_glows_on_the_suns_side_as_tall_as_it_is_strong() {
        let (_, pieces) = meshes();
        let glow = [1.0, 0.6, 0.2, 0.5];
        let evening = OverworldSky {
            sunrise: glow,
            sun_angle: 80.0_f32.to_radians(),
            ..noon()
        };
        let draws = planned(&Sky::Overworld(evening), &all_pictures());
        assert_eq!(
            (draws[1].piece, draws[1].blend, draws[1].kind),
            (pieces.sunrise, 1, COLOURED)
        );
        assert_eq!(draws[1].colour, glow);
        // Its middle lies on the horizon in the west; its rim's top, 40 back from the
        // fan's plane, stands half of that up.
        assert!(close(
            apply(&draws[1].model, [0.0, 100.0, 0.0]),
            [-100.0, 0.0, 0.0],
            1e-3
        ));
        let top = apply(&draws[1].model, [0.0, 120.0, -40.0]);
        assert!(close(top, [-120.0, 20.0, 0.0], 1e-3), "{top:?}");
        // In the east in the morning.
        let morning = OverworldSky {
            sunrise: glow,
            sun_angle: 280.0_f32.to_radians(),
            ..noon()
        };
        let draws = planned(&Sky::Overworld(morning), &all_pictures());
        assert!(close(
            apply(&draws[1].model, [0.0, 100.0, 0.0]),
            [100.0, 0.0, 0.0],
            1e-3
        ));
        // Too faint to draw.
        let faint = OverworldSky {
            sunrise: [1.0, 0.6, 0.2, 0.001],
            ..noon()
        };
        assert_eq!(planned(&Sky::Overworld(faint), &all_pictures()).len(), 3);
    }

    #[test]
    fn a_rainy_night_below_the_horizon_draws_everything_in_order() {
        let (_, pieces) = meshes();
        let night = OverworldSky {
            sunrise: [0.5, 0.5, 0.5, 0.25],
            moon_phase: 5,
            rain_brightness: 0.25,
            star_brightness: 0.5,
            star_angle: std::f32::consts::FRAC_PI_2,
            dark_disc: true,
            ..noon()
        };
        let mut pictures = all_pictures();
        let draws = planned(&Sky::Overworld(night), &pictures);
        let order: Vec<Piece> = draws.iter().map(|d| d.piece).collect();
        assert_eq!(
            order,
            [
                pieces.dome,
                pieces.sunrise,
                pieces.sun,
                pieces.moon,
                pieces.stars,
                pieces.dark
            ]
        );
        // The rain dims the sun and the moon; the stars are as bright as they are.
        assert_eq!(draws[2].colour, [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(draws[3].colour, [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(
            (draws[4].colour, draws[4].blend, draws[4].kind),
            ([0.5; 4], 2, COLOURED)
        );
        // A star overhead at midnight has turned a quarter towards the west.
        assert!(close(
            apply(&draws[4].model, [0.0, 100.0, 0.0]),
            [-100.0, 0.0, 0.0],
            1e-3
        ));
        // The dark half: black, raised 12 blocks, fogged like the dome.
        assert_eq!(
            (draws[5].colour, draws[5].kind),
            ([0.0, 0.0, 0.0, 1.0], FOGGED)
        );
        assert_eq!(apply(&draws[5].model, [0.0, -16.0, 0.0]), [0.0, -4.0, 0.0]);
        // The moon is drawn from its phase's picture, and not without it.
        pictures.moon[5] = None;
        let draws = planned(&Sky::Overworld(night), &pictures);
        assert!(draws.iter().all(|d| d.piece != pieces.moon));
    }

    #[test]
    fn the_end_is_its_box() {
        let (_, pieces) = meshes();
        let draws = planned(&Sky::End, &all_pictures());
        assert_eq!(draws.len(), 1);
        let d = draws[0];
        assert_eq!((d.piece, d.blend, d.kind), (pieces.end, 1, TEXTURED));
        assert_eq!(
            (d.sampler, d.colour, d.model),
            (SamplerId::NearestRepeat, [1.0; 4], IDENTITY)
        );
        assert_eq!(planned(&Sky::End, &Pictures::default()), []);
    }

    #[test]
    fn the_orders_have_names() {
        for order in SkyOrder::ALL {
            assert_eq!(SkyOrder::parse(order.name()), Some(order));
        }
        assert_eq!(SkyOrder::default(), SkyOrder::Late);
        assert_eq!(SkyOrder::parse("never"), None);
        // The push constants, where sky.frag's block has them.
        let push = sky_push(
            7,
            TEXTURED,
            TextureId::MISSING,
            SamplerId::NearestRepeat,
            [0.1, 0.2, 0.3, 0.4],
            [0.5, 0.6, 0.7, 0.8],
            192.0,
        );
        assert_eq!(push.len(), 60);
        assert_eq!(push[..8], [7, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(push[8..12], TEXTURED.to_ne_bytes());
        assert_eq!(push[12..16], TextureId::MISSING.index().to_ne_bytes());
        let float = |at: usize| f32::from_ne_bytes(push[at..at + 4].try_into().unwrap());
        assert_eq!(
            [float(16), float(20), float(24), float(28)],
            [0.1, 0.2, 0.3, 0.4]
        );
        assert_eq!(
            [float(32), float(36), float(40), float(44)],
            [0.5, 0.6, 0.7, 0.8]
        );
        assert_eq!(float(48), 192.0);
        assert_eq!(push[52..56], SamplerId::NearestRepeat.index().to_ne_bytes());
        assert_eq!(float(56), 1.0 / 192.0);
    }
}
