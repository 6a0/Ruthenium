//! A development check of the sky's and the clouds' fog (`--exercise-sky`): small
//! made-up pieces of sky and cloud faces are drawn every frame with the real sky and
//! cloud pipelines, push constants and records, into an off-screen target with a depth
//! attachment, and every pixel that comes back is compared with what vanilla's
//! arithmetic says it must be.
//!
//! - **The sky's fog**: eight squares of the sky's fogged kind of draw (the dome's),
//!   each at one distance in a straight line and one sideways or in height from the
//!   camera (all four corners alike, so that every pixel has the same), from a few
//!   blocks to a few hundred. The dome goes towards the fog's colour as far as it is
//!   towards where the sky's fog ends, and is all fog beyond it, sideways or in height;
//!   the amount is no more than all, however far beyond the square is.
//! - **The sky's colour**: a square of the coloured kind (the stars'), its vertices'
//!   colour times the draw's, and the ground the clouds are blended over.
//! - **The clouds' fade**: eight cloud faces, one of each direction and two with the
//!   faces' flags, each centred on the camera's line so that its four corners are as
//!   far away, from ten blocks to a hundred and thirty. A face's alpha goes from all of it
//!   at the camera to nothing where the clouds' fog ends, and is nothing beyond (no
//!   less), blended over the ground.
//! - **Fog on and off**: every frame draws in the next of four skies' fogs (none, one
//!   ending far, one ending near, one half there) and three clouds' ends (none, far,
//!   near), and with the sky's pipelines that ignore depth or test it, in turn.
//!
//! The pieces are laid out with matrices that map them flat onto their squares of the
//! target, so that what the stages hand on is the same at every pixel of a square.

use std::sync::Arc;

use ruthenium_gpu::{
    BufferPool, DepthTarget, Encoder, Error, FrameData, Gpu, Pipelines, PoolConfig, RangeHandle,
    Readback, SamplerId, TextureId, UploadRing, UploadTicket, vk,
};
use ruthenium_shaders::interface::{SKY, WORLD_MATRIX};

use crate::clouds::{self, CELL_HEIGHT, CELL_SIZE, FACE_INSIDE, FACE_TOP_COLOUR, cloud_push};
use crate::pipelines::PipelineId;
use crate::sky::{self, sky_push};
use crate::terrain_check::fog_amount;

/// Pixels a side of each square, and the squares across: two rows.
const SQUARE: u32 = 16;
const COLUMNS: u32 = 9;
const WIDTH: u32 = SQUARE * COLUMNS;
const HEIGHT: u32 = SQUARE * 2;
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;

type Rgba = [u8; 4];
/// A square of the target (`[x0, y0, x1, y1]`, the far corner excluded), what it must
/// read, and what it shows.
type Square = ([u32; 4], Rgba, &'static str);

/// The sky's fogged squares, each as the half sides across and down and the distance in
/// front of the camera of its corners: `(±x, ±y, z)`. Its distance in a straight line
/// is then the same at every corner, as is the one sideways or in height (the larger
/// of `(x, z)`'s length and `y`).
const SKY_SQUARES: [[f32; 3]; 8] = [
    [2.0, 2.0, 4.0],
    [6.0, 6.0, 12.0],
    [10.0, 10.0, 25.0],
    [15.0, 15.0, 40.0],
    [20.0, 20.0, 55.0],
    // Beyond the end of one fog in a straight line, short of it sideways.
    [30.0, 30.0, 55.0],
    // Further in height than sideways.
    [3.0, 100.0, 10.0],
    [100.0, 100.0, 300.0],
];
/// The colour of the sky's squares, far from every fog's in every channel.
const SKY_COLOUR: [f32; 4] = [0.8, 0.2, 0.3, 1.0];
/// The coloured square: its vertices' colour and the draw's.
const STAR_VERTEX: Rgba = [200, 100, 50, 255];
const STAR_COLOUR: [f32; 4] = [0.5, 1.0, 0.8, 1.0];
/// What the clouds are blended over: the coloured kind with white vertices.
const GROUND: [f32; 4] = [0.4, 0.3, 0.2, 1.0];
const WHITE: Rgba = [255; 4];

/// The skies' fogs, in turn: a colour (its alpha scales how much there is) and where
/// the sky's fog ends.
const SKY_FOGS: [([f32; 4], f32); 4] = [
    // None, in a colour no pixel may show.
    ([1.0, 0.0, 1.0, 1.0], f32::MAX),
    ([0.2, 0.6, 1.0, 1.0], 64.0),
    ([0.9, 0.5, 0.1, 1.0], 16.0),
    ([0.2, 0.6, 1.0, 0.5], 64.0),
];

/// The cloud faces: direction, flags, and how far the face's plane is from the camera
/// along its axis (its middle is on the axis).
const CLOUD_FACES: [(u32, u32, f32); 8] = [
    (clouds::DOWN, 0, 6.0),
    (clouds::UP, 0, 20.0),
    (clouds::NORTH, 0, -40.0),
    (clouds::SOUTH, 0, 60.0),
    (clouds::WEST, 0, -80.0),
    (clouds::EAST, 0, 130.0),
    (clouds::DOWN, FACE_TOP_COLOUR, 30.0),
    (clouds::UP, FACE_INSIDE, -50.0),
];
/// The clouds' colour, as the air gives it.
const CLOUD_COLOUR: [f32; 4] = [0.9, 0.85, 0.8, 0.8];
/// Where the clouds' fog ends, in turn: never, far, near.
const CLOUD_ENDS: [f32; 3] = [f32::MAX, 96.0, 24.0];

/// How light each direction's cloud faces are (`clouds.vert`).
const FACE_SHADES: [f32; 6] = [0.7, 1.0, 0.8, 0.8, 0.9, 0.9];
/// The corners of each direction's face, a bit of each coordinate a corner
/// (`clouds.vert`).
const PACKED: [u32; 3] = [0xF0_3CC3, 0x66_66F0, 0xC3_F066];

/// How a frame draws: which of the sky's and the clouds' fogs, and whether the sky's
/// pipeline tests depth (as it is drawn after the world) or not (before it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Case {
    sky_fog: usize,
    cloud_end: usize,
    behind: bool,
}

/// Every pairing of fogs, with each kind of sky pipeline.
const CASES: usize = SKY_FOGS.len() * CLOUD_ENDS.len() * 2;

impl Case {
    fn of(frame: u64) -> Case {
        let i = (frame % CASES as u64) as usize;
        Case {
            sky_fog: i % SKY_FOGS.len(),
            cloud_end: i % CLOUD_ENDS.len(),
            behind: i >= CASES / 2,
        }
    }

    fn index(self) -> usize {
        let pair = (0..SKY_FOGS.len() * CLOUD_ENDS.len())
            .find(|&i| i % SKY_FOGS.len() == self.sky_fog && i % CLOUD_ENDS.len() == self.cloud_end)
            .expect("every pairing");
        pair + if self.behind { CASES / 2 } else { 0 }
    }
}

/// The pixels of square `column` of row `row`: `[x0, y0, x1, y1]`, the far corner
/// excluded.
fn square(column: u32, row: u32) -> [u32; 4] {
    [
        column * SQUARE,
        row * SQUARE,
        (column + 1) * SQUARE,
        (row + 1) * SQUARE,
    ]
}

/// A matrix, columns first, that maps the rectangle `from` (`[a0, b0, a1, b1]`) of
/// the plane of coordinates `axes` (0 x, 1 y, 2 z) onto the pixels `to`, at depth one
/// half; the third coordinate is left out.
fn onto(axes: [usize; 2], from: [f32; 4], to: [u32; 4]) -> [f32; 16] {
    let mut m = [0.0; 16];
    let [a0, b0, a1, b1] = from;
    let ndc = |p: u32, size: u32| 2.0 * p as f32 / size as f32 - 1.0;
    let (x0, x1) = (ndc(to[0], WIDTH), ndc(to[2], WIDTH));
    let (y0, y1) = (ndc(to[1], HEIGHT), ndc(to[3], HEIGHT));
    let sx = (x1 - x0) / (a1 - a0);
    let sy = (y1 - y0) / (b1 - b0);
    m[axes[0] * 4] = sx;
    m[axes[1] * 4 + 1] = sy;
    m[12] = x0 - sx * a0;
    m[13] = y0 - sy * b0;
    m[14] = 0.5;
    m[15] = 1.0;
    m
}

/// The corners of a sky square, as two triangles.
fn sky_corners([x, y, z]: [f32; 3]) -> [[f32; 3]; 6] {
    let c = [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]];
    [0, 1, 2, 2, 3, 0].map(|i| c[i])
}

/// A sky square's two distances, as the vertex stage works them out at each corner:
/// in a straight line, and sideways or in height.
fn sky_distances([x, y, z]: [f32; 3]) -> (f32, f32) {
    let spherical = (x * x + y * y + z * z).sqrt();
    let cylindrical = (x * x + z * z).sqrt().max(y.abs());
    (spherical, cylindrical)
}

/// A cloud face's corners in blocks from the camera, with the layer's offset `offset`
/// (`clouds.vert`, for the cell at the camera's).
fn cloud_corners(direction: u32, flags: u32, offset: [f32; 3]) -> [[f32; 3]; 4] {
    let size = [CELL_SIZE, CELL_HEIGHT, CELL_SIZE];
    std::array::from_fn(|v| {
        let v = if flags & FACE_INSIDE != 0 { 3 - v } else { v };
        let bit = direction as usize * 4 + v;
        std::array::from_fn(|a| ((PACKED[a] >> bit) & 1) as f32 * size[a] + offset[a])
    })
}

/// The layer's offset that puts a face's middle on the camera's line along its axis,
/// `along` blocks out; the axes across and down the face; and the face's rectangle in
/// them.
fn cloud_placement(direction: u32, along: f32) -> ([f32; 3], [usize; 2], [f32; 4]) {
    let (half, half_height) = (CELL_SIZE / 2.0, CELL_HEIGHT / 2.0);
    // Where the face's plane is in the cell: 0 or the cell's size along its axis.
    let (axis, at) = match direction {
        clouds::DOWN => (1, 0.0),
        clouds::UP => (1, CELL_HEIGHT),
        clouds::NORTH => (2, 0.0),
        clouds::SOUTH => (2, CELL_SIZE),
        clouds::WEST => (0, 0.0),
        _ => (0, CELL_SIZE),
    };
    let mut offset = [-half, -half_height, -half];
    offset[axis] = along - at;
    let (axes, from) = match axis {
        1 => ([0, 2], [-half, -half, half, half]),
        2 => ([0, 1], [-half, -half_height, half, half_height]),
        _ => ([2, 1], [-half, -half_height, half, half_height]),
    };
    (offset, axes, from)
}

/// How far a cloud face's corners are from the camera, and that they all are.
fn cloud_distance(direction: u32, flags: u32, along: f32) -> f32 {
    let (offset, _, _) = cloud_placement(direction, along);
    let corners = cloud_corners(direction, flags, offset);
    let length = |p: [f32; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    let d = length(corners[0]);
    debug_assert!(corners.iter().all(|&c| (length(c) - d).abs() < 1e-4));
    d
}

fn to_bytes(c: [f32; 4]) -> Rgba {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The fog lines the model takes: how much fog there is at a distance in a fog from
/// none at the camera to all at an end. The check's own is vanilla's; tests swap in
/// broken ones.
type Fogging = fn(f32, f32) -> f32;

/// Vanilla's linear fog from the camera to `end`.
fn vanilla_fog(distance: f32, end: f32) -> f32 {
    fog_amount(distance, [0.0, end])
}

/// What a frame of `case` must read, square by square: the probes' rectangles and
/// colours.
fn expected(case: Case, fogging: Fogging) -> Vec<Square> {
    let mut out = Vec::new();
    let (fog, end) = SKY_FOGS[case.sky_fog];
    for (i, &corner) in SKY_SQUARES.iter().enumerate() {
        let (spherical, cylindrical) = sky_distances(corner);
        // Towards the rim, and all of it beyond the end sideways or in height.
        let towards = fogging(spherical, end);
        let beyond = if cylindrical <= end { 0.0 } else { 1.0 };
        let amount = f32::max(towards, beyond) * fog[3];
        let mut c = SKY_COLOUR;
        for k in 0..3 {
            c[k] = SKY_COLOUR[k] + (fog[k] - SKY_COLOUR[k]) * amount;
        }
        out.push((square(i as u32, 0), to_bytes(c), "the sky's fog"));
    }
    let star: [f32; 4] =
        std::array::from_fn(|k| f32::from(STAR_VERTEX[k]) / 255.0 * STAR_COLOUR[k]);
    out.push((square(8, 0), to_bytes(star), "the sky's coloured kind"));
    let end = CLOUD_ENDS[case.cloud_end];
    for (i, &(direction, flags, along)) in CLOUD_FACES.iter().enumerate() {
        let shade = FACE_SHADES[if flags & FACE_TOP_COLOUR != 0 {
            1
        } else {
            direction as usize
        }];
        let d = cloud_distance(direction, flags, along);
        let alpha = CLOUD_COLOUR[3] * (1.0 - fogging(d, end));
        // Blended over the ground by its alpha; the ground is opaque.
        let c: [f32; 4] = std::array::from_fn(|k| {
            if k == 3 {
                1.0
            } else {
                shade * CLOUD_COLOUR[k] * alpha + GROUND[k] * (1.0 - alpha)
            }
        });
        out.push((square(i as u32, 1), to_bytes(c), "the clouds' fade"));
    }
    out.push((square(8, 1), to_bytes(GROUND), "the ground"));
    out
}

/// Whether `pixels` (RGBA, the target's size) reads `expected` within one step, or two
/// where a fog or a blend went into it. Says what is wrong for the first few frames.
fn compare(expected: &[Square], case: Case, frame: u64, pixels: &[u8], mismatches: u64) -> bool {
    let mut ok = true;
    for &(rect, want, what) in expected {
        let tolerance = 2;
        let wrong = (rect[1]..rect[3])
            .flat_map(|y| (rect[0]..rect[2]).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let i = ((y * WIDTH + x) * 4) as usize;
                let got: Rgba = pixels[i..i + 4].try_into().expect("4 bytes");
                got.iter()
                    .zip(want)
                    .any(|(&g, w)| g.abs_diff(w) > tolerance)
                    .then_some((x, y, got))
            });
        if let Some((x, y, got)) = wrong {
            if mismatches < PRINTED {
                eprintln!(
                    "[sky-check] frame {frame} (sky fog {}, clouds' end {}, {}), {what}: pixel ({x}, {y}) reads {got:?}, must be {want:?} within {tolerance}",
                    case.sky_fog,
                    case.cloud_end,
                    if case.behind { "behind" } else { "over" },
                );
            }
            ok = false;
        }
    }
    ok
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkyCheckReport {
    /// Frames drawn, read back and compared.
    pub compared: u64,
    /// How many of them had a wrong pixel.
    pub mismatches: u64,
    /// Squares compared in each.
    pub probes: usize,
}

impl SkyCheckReport {
    pub fn passed(&self) -> bool {
        self.compared > 0 && self.mismatches == 0
    }
}

/// The sky's vertices: the fogged squares, the coloured one and the ground, six each.
fn sky_vertices() -> Vec<u8> {
    let mut out = Vec::new();
    for corner in SKY_SQUARES {
        for p in sky_corners(corner) {
            sky::vertex(&mut out, p, [0.0; 2], WHITE);
        }
    }
    for colour in [STAR_VERTEX, WHITE] {
        for p in sky_corners([1.0, 1.0, 1.0]) {
            sky::vertex(&mut out, p, [0.0; 2], colour);
        }
    }
    out
}

/// The cloud faces' words, one an instance: each of the cell at the camera's.
fn cloud_words() -> Vec<u8> {
    CLOUD_FACES
        .iter()
        .flat_map(|&(direction, flags, _)| clouds::face(0, 0, direction, flags).to_ne_bytes())
        .collect()
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct SkyCheck {
    gpu: Arc<Gpu>,
    target: Readback,
    depth: DepthTarget,
    pool: BufferPool,
    sky: RangeHandle,
    clouds: RangeHandle,
    /// The two buffers' uploads, once staged.
    uploads: [Option<UploadTicket>; 2],
    /// What each case must read, once one was drawn.
    pictures: Vec<Option<Vec<Square>>>,
    report: SkyCheckReport,
}

fn extent() -> vk::Extent2D {
    vk::Extent2D {
        width: WIDTH,
        height: HEIGHT,
    }
}

impl SkyCheck {
    pub fn new(gpu: Arc<Gpu>, format: vk::Format) -> Result<SkyCheck, Error> {
        let target = Readback::new(gpu.clone(), format, extent())?;
        let depth = DepthTarget::new(gpu.clone(), extent())?;
        let page = 1 << 16;
        let mut pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: page,
                budget_bytes: page,
                low_watermark: 0,
                high_watermark: 0,
                release_after: u32::MAX,
                granule: 8,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER,
        );
        pool.grow_now()?;
        let mut alloc = |bytes: usize| {
            pool.alloc(bytes as u64, 8)
                .map_err(|e| Error::Unsupported(format!("the sky check's buffers: {e:?}")))
        };
        let sky = alloc(sky_vertices().len())?;
        let clouds = alloc(cloud_words().len())?;
        Ok(SkyCheck {
            gpu,
            target,
            depth,
            pool,
            sky,
            clouds,
            uploads: [None; 2],
            pictures: vec![None; CASES],
            report: SkyCheckReport {
                probes: expected(Case::of(0), vanilla_fog).len(),
                ..SkyCheckReport::default()
            },
        })
    }

    pub fn report(&self) -> SkyCheckReport {
        self.report
    }

    /// Stages the buffers' contents until both are on their way. Call it before the
    /// frame's `UploadRing::record`.
    pub fn stage(&mut self, uploads: &mut UploadRing) {
        self.pool.maintain(&uploads.completed());
        for (i, (handle, bytes)) in [(self.sky, sky_vertices()), (self.clouds, cloud_words())]
            .into_iter()
            .enumerate()
        {
            if self.uploads[i].is_none()
                && let Some(range) = self.pool.resolve(handle)
            {
                self.uploads[i] = uploads
                    .upload_buffer(range.buffer, range.offset, &bytes)
                    .ok();
            }
        }
    }

    /// Compares what has come back and draws the pieces again. `completed` is a frame
    /// the GPU has finished; call it after `UploadRing::record` for the frame
    /// `encoder` records, whose number is `frame`.
    pub fn step(
        &mut self,
        encoder: &mut Encoder<'_>,
        completed: u64,
        frame: u64,
        uploads: &UploadRing,
        frame_data: &mut FrameData,
        pipelines: &Pipelines,
    ) -> Result<(), Error> {
        if let Some((drawn, pixels)) = self.target.take(completed) {
            let case = Case::of(drawn);
            let expected =
                self.pictures[case.index()].get_or_insert_with(|| expected(case, vanilla_fog));
            let ok = compare(expected, case, drawn, pixels, self.report.mismatches);
            self.report.compared += 1;
            self.report.mismatches += u64::from(!ok);
        }
        if self.target.format() != pipelines.format() {
            self.target = Readback::new(self.gpu.clone(), pipelines.format(), extent())?;
        }
        let ready = self
            .uploads
            .iter()
            .all(|t| t.is_some_and(|t| uploads.is_ready(t)));
        let (Some(sky), Some(cloud_faces)) =
            (self.pool.resolve(self.sky), self.pool.resolve(self.clouds))
        else {
            return Ok(());
        };
        if !ready || self.target.pending() {
            return Ok(());
        }
        let case = Case::of(frame);
        let (fog, fog_end) = SKY_FOGS[case.sky_fog];
        let flat = if case.behind {
            PipelineId::SkyFlatBehind
        } else {
            PipelineId::SkyFlat
        };
        // Every draw's record, and its push constants: the sky's fogged squares, the
        // coloured one, the ground, then the cloud faces.
        let mut sky_draws = Vec::with_capacity(SKY_SQUARES.len() + 2);
        let record = |frame_data: &mut FrameData, words: u32, m: [f32; 16]| {
            let (at, bytes) = frame_data.alloc(words, 4)?;
            for (i, c) in m.iter().enumerate() {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&c.to_ne_bytes());
            }
            Some(at)
        };
        for (i, &[x, y, _]) in SKY_SQUARES.iter().enumerate() {
            let m = onto([0, 1], [-x, -y, x, y], square(i as u32, 0));
            let Some(at) = record(frame_data, SKY.words(), m) else {
                return Ok(());
            };
            sky_draws.push(sky_push(
                at,
                sky::FOGGED,
                TextureId::MISSING,
                SamplerId::LinearClamp,
                SKY_COLOUR,
                fog,
                fog_end,
            ));
        }
        for (column, row, colour) in [(8, 0, STAR_COLOUR), (0, 1, GROUND)] {
            // The ground reaches across the whole row.
            let mut to = square(column, row);
            if row == 1 {
                to[2] = WIDTH;
            }
            let m = onto([0, 1], [-1.0, -1.0, 1.0, 1.0], to);
            let Some(at) = record(frame_data, SKY.words(), m) else {
                return Ok(());
            };
            sky_draws.push(sky_push(
                at,
                sky::COLOURED,
                TextureId::MISSING,
                SamplerId::LinearClamp,
                colour,
                fog,
                fog_end,
            ));
        }
        let mut cloud_draws = Vec::with_capacity(CLOUD_FACES.len());
        for (i, &(direction, _, along)) in CLOUD_FACES.iter().enumerate() {
            let (offset, axes, from) = cloud_placement(direction, along);
            let m = onto(axes, from, square(i as u32, 1));
            let Some(at) = record(frame_data, WORLD_MATRIX.words(), m) else {
                return Ok(());
            };
            cloud_draws.push(cloud_push(
                at,
                CLOUD_COLOUR,
                offset,
                CLOUD_ENDS[case.cloud_end],
            ));
        }
        let mut pass = encoder.readback_pass_with_depth(pipelines, &mut self.target, &self.depth);
        // SAFETY: the push constants are sky.vert's and clouds.vert's, and the records
        // they point at were written just above; both buffers' contents have arrived, and
        // every draw stays inside them.
        unsafe {
            pass.bind_vertex_buffer(sky.buffer, sky.offset);
            for (i, push) in sky_draws.iter().enumerate() {
                pass.draw_vertices(flat.index(), push, i as u32 * 6, 6);
            }
            pass.bind_vertex_buffer(cloud_faces.buffer, cloud_faces.offset);
            for (i, push) in cloud_draws.iter().enumerate() {
                pass.draw_instances(PipelineId::CloudsFast.index(), push, 6, i as u32, 1);
            }
        }
        pass.finish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_is_drawn() {
        let mut seen = std::collections::HashSet::new();
        for frame in 0..CASES as u64 {
            let case = Case::of(frame);
            assert!(seen.insert(case.index()), "{case:?}");
        }
        assert_eq!(seen.len(), CASES);
        assert!((0..CASES).all(|i| seen.contains(&i)));
    }

    #[test]
    fn the_squares_cover_the_target_once() {
        let mut covered = vec![0u32; (WIDTH * HEIGHT) as usize];
        for (rect, _, _) in expected(Case::of(0), vanilla_fog) {
            for y in rect[1]..rect[3] {
                for x in rect[0]..rect[2] {
                    covered[(y * WIDTH + x) as usize] += 1;
                }
            }
        }
        assert!(covered.iter().all(|&c| c == 1));
    }

    #[test]
    fn the_matrices_put_the_pieces_on_their_squares() {
        let project = |m: &[f32; 16], p: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
        };
        let m = onto([0, 1], [-2.0, -3.0, 2.0, 3.0], square(1, 1));
        let px = |ndc: f32, size: u32| (ndc + 1.0) / 2.0 * size as f32;
        let a = project(&m, [-2.0, -3.0, 7.0]);
        let b = project(&m, [2.0, 3.0, -7.0]);
        let near = |p: [f32; 2], want: f32| p.iter().all(|&v| (v - want).abs() < 1e-3);
        assert!(near([px(a[0], WIDTH), px(a[1], HEIGHT)], 16.0), "{a:?}");
        assert!(near([px(b[0], WIDTH), px(b[1], HEIGHT)], 32.0), "{b:?}");
        assert_eq!(a[2], 0.5);
        // Each cloud face's corners land on its square's corners, whichever way it faces.
        for (i, &(direction, flags, along)) in CLOUD_FACES.iter().enumerate() {
            let (offset, axes, from) = cloud_placement(direction, along);
            let m = onto(axes, from, square(i as u32, 1));
            let mut xs: Vec<f32> = Vec::new();
            let mut ys: Vec<f32> = Vec::new();
            for c in cloud_corners(direction, flags, offset) {
                let p = project(&m, c);
                xs.push(px(p[0], WIDTH));
                ys.push(px(p[1], HEIGHT));
            }
            let [x0, y0, x1, y1] = square(i as u32, 1).map(|v| v as f32);
            for (v, lo, hi) in [(&xs, x0, x1), (&ys, y0, y1)] {
                assert!(
                    v.iter()
                        .all(|&p| (p - lo).abs() < 1e-3 || (p - hi).abs() < 1e-3),
                    "{i}: {v:?}"
                );
                assert!(
                    v.iter().any(|&p| (p - lo).abs() < 1e-3)
                        && v.iter().any(|&p| (p - hi).abs() < 1e-3)
                );
            }
        }
    }

    #[test]
    fn every_cloud_face_is_as_far_at_each_corner() {
        for &(direction, flags, along) in &CLOUD_FACES {
            let (offset, _, _) = cloud_placement(direction, along);
            let corners = cloud_corners(direction, flags, offset);
            let length = |p: [f32; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            let d = length(corners[0]);
            for c in corners {
                assert!((length(c) - d).abs() < 1e-4, "{direction} {c:?}");
            }
            // In the face's plane, along its axis.
            assert!(d >= along.abs());
        }
    }

    #[test]
    fn the_fogs_reach_every_part_of_their_lines() {
        // The sky: in some fog, a square with some fog short of all of it, one at all
        // of it short of the end sideways and in height, and one well beyond.
        let (_, end) = SKY_FOGS[1];
        let distances: Vec<(f32, f32)> = SKY_SQUARES.iter().map(|&c| sky_distances(c)).collect();
        assert!(distances.iter().any(|&(s, _)| s > 0.5 * end && s < end));
        assert!(distances.iter().any(|&(s, c)| s > end && c <= end));
        assert!(distances.iter().any(|&(s, _)| s > 2.0 * end));
        // Further in height than sideways, beyond the end.
        let (_, c) = sky_distances(SKY_SQUARES[6]);
        assert_eq!(c, SKY_SQUARES[6][1]);
        // The clouds: faces between none and all of the fade, and beyond it.
        let d: Vec<f32> = CLOUD_FACES
            .iter()
            .map(|&(direction, flags, along)| cloud_distance(direction, flags, along))
            .collect();
        for end in &CLOUD_ENDS[1..] {
            assert!(d.iter().any(|&d| d < 0.75 * end));
            assert!(d.iter().any(|&d| d > end * 1.25));
        }
    }

    /// Whether some frame of some case reads wrong by the check's own judgement when
    /// the fog is worked out with `fogging` instead of vanilla's line.
    fn caught(fogging: Fogging) -> bool {
        (0..CASES as u64).any(|frame| {
            let case = Case::of(frame);
            let good = expected(case, vanilla_fog);
            let broken = expected(case, fogging);
            let mut image = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
            for (rect, colour, _) in &broken {
                for y in rect[1]..rect[3] {
                    for x in rect[0]..rect[2] {
                        let i = ((y * WIDTH + x) * 4) as usize;
                        image[i..i + 4].copy_from_slice(colour);
                    }
                }
            }
            !compare(&good, case, frame, &image, PRINTED)
        })
    }

    #[test]
    fn the_right_picture_passes_and_a_broken_fog_fails() {
        assert!(!caught(vanilla_fog));
        // Not clamped to all of it beyond the end.
        assert!(caught(|d, end| (d / end).max(0.0)));
        // Twice as far, or half.
        assert!(caught(|d, end| (2.0 * d / end).clamp(0.0, 1.0)));
        assert!(caught(|d, end| (0.5 * d / end).clamp(0.0, 1.0)));
        // A step at the end, or none.
        assert!(caught(|d, end| if d < end { 0.0 } else { 1.0 }));
        assert!(caught(|_, _| 0.0));
        // From a start a tenth of the way out.
        assert!(caught(|d, end| fog_amount(d, [0.1 * end, end])));
    }

    #[test]
    fn the_coloured_kinds_are_their_colours_times_the_draws() {
        let e = expected(Case::of(0), vanilla_fog);
        let star = e
            .iter()
            .find(|p| p.2 == "the sky's coloured kind")
            .unwrap()
            .1;
        assert_eq!(star, [100, 100, 40, 255]);
        let ground = e.iter().find(|p| p.2 == "the ground").unwrap().1;
        assert_eq!(ground, [102, 77, 51, 255]);
        // Without fog the sky is its colour, and the clouds' alpha their own.
        for p in e.iter().filter(|p| p.2 == "the sky's fog") {
            assert_eq!(p.1, to_bytes(SKY_COLOUR));
        }
    }
}
