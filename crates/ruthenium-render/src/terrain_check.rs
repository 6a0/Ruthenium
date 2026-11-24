//! A development check of the terrain pipeline (`--exercise-terrain`): a small made-up
//! scene is drawn every frame with the real terrain pipelines, vertex layout, quad
//! indices, pooled buffer ranges and frame records, into an off-screen target with a
//! depth attachment, and every pixel that comes back is compared with what the scene
//! must show.
//!
//! What the scene holds, a block (8 x 8 pixels) or two for each:
//! - **Colour**: a mid-grey texel drawn unlit must come back as the byte that went in;
//!   times a shade and times a tint it is the product of the stored bytes.
//! - **Light**: the two lights are read as whole numbers from 0 to 240 and looked up at
//!   the light map's texel centres: both ends, a level in between, half way between two
//!   levels, and a value beyond 240 (which stays on the last texel).
//! - **Texture coordinates**: the last texel of a 4096-texel atlas (coordinate code
//!   32768), two texels side by side, and a square whose texture is turned a quarter.
//! - **Depth**: of two overlapping quads the nearer shows, whichever is drawn first; a
//!   cutout quad in the same plane as a solid one shows through its holes; texels less
//!   than half opaque are holes, half opaque ones are not.
//! - **Winding**: a quad seen from behind is not drawn; a quad in a corner of its block
//!   is in that corner.
//! - **Section borders**: two sections' quads that meet on the border between them
//!   leave no gap and no overlap, nor do corners that reach half a block across.
//! - **Mixed records**: the draws read the world's values, its matrix and the sections'
//!   records from three places in the frame's data, which move from frame to frame; and
//!   the sections' origins and the camera's offset change together so that the picture
//!   must stay the same.

use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, BufferPool, DRAW_COMMAND_BYTES, DepthTarget, Encoder, Error, FrameData, Gpu,
    Pipelines, PoolConfig, RangeHandle, Readback, TextureDesc, TextureId, TextureTable, UploadRing,
    UploadTicket, vk,
};
use ruthenium_shaders::interface::{SECTION, WORLD, WORLD_MATRIX};

use crate::pipelines::PipelineId;
use crate::terrain::world::{
    SECTION_BYTES, draw_command, quad_indices, terrain_push, write_section,
};
use crate::terrain_vertex::{VERTEX_BYTES, Vertex, position_code, uv_code};

/// Pixels per block, and the scene's size in blocks.
const SCALE: u32 = 8;
const BLOCKS: [u32; 2] = [12, 8];
const WIDTH: u32 = BLOCKS[0] * SCALE;
const HEIGHT: u32 = BLOCKS[1] * SCALE;
/// The fixture's atlas: as wide as coordinates must reach.
const ATLAS: [u32; 2] = [4096, 8];
/// Mismatches printed in detail; the rest are only counted.
const PRINTED: u64 = 8;

type Rgba = [u8; 4];

const CLEAR: Rgba = [0, 0, 0, 0];
/// Every texel no quad may show.
const POISON: Rgba = [255, 0, 255, 255];
const WHITE: Rgba = [255, 255, 255, 255];
const GREY: Rgba = [0x80, 0x80, 0x80, 255];
const SOLID: Rgba = [200, 40, 40, 255];
const BELOW_HALF: Rgba = [90, 60, 30, 0x7F];
const HALF: Rgba = [90, 60, 30, 0x80];
const NEAR: Rgba = [250, 250, 0, 255];
const FAR: Rgba = [0, 0, 250, 255];
const LEFT_SECTION: Rgba = [10, 20, 30, 255];
const RIGHT_SECTION: Rgba = [200, 150, 100, 255];
const HOLE: Rgba = [9, 9, 9, 0];
const CUTOUT: Rgba = [40, 200, 40, 255];
const TURNED: [Rgba; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
];
const EDGE: Rgba = [17, 34, 51, 255];
const BEFORE_EDGE: Rgba = [1, 2, 3, 255];
const AT_EDGE: Rgba = [250, 240, 230, 255];

/// Where each texel a quad shows is in the atlas.
const TEXELS: [([u32; 2], Rgba); 20] = [
    ([0, 0], WHITE),
    ([1, 0], GREY),
    ([2, 0], SOLID),
    ([5, 0], BELOW_HALF),
    ([6, 0], HALF),
    ([7, 0], NEAR),
    ([8, 0], FAR),
    ([9, 0], LEFT_SECTION),
    ([10, 0], RIGHT_SECTION),
    ([12, 0], HOLE),
    ([13, 0], CUTOUT),
    ([16, 2], TURNED[0]),
    ([17, 2], TURNED[1]),
    ([16, 3], TURNED[2]),
    ([17, 3], TURNED[3]),
    ([4095, 4], EDGE),
    ([4094, 6], BEFORE_EDGE),
    ([4095, 6], AT_EDGE),
    ([4094, 7], BEFORE_EDGE),
    ([4095, 7], AT_EDGE),
];

/// The fixture's light map: red says the block light's texel, green the sky light's,
/// and blue goes from nothing to everything between one block light level and the
/// next, so that a lookup a few hundredths of a texel off its centre shows.
fn light_texel(block: u32, sky: u32) -> Rgba {
    [
        (block * 17) as u8,
        (sky * 17) as u8,
        if block % 2 == 1 { 255 } else { 0 },
        255,
    ]
}

/// A rectangle of pixels and what every one of them must read, within `tolerance`
/// steps per channel.
#[derive(Debug, Clone, Copy)]
struct Probe {
    what: &'static str,
    /// `[x0, y0, x1, y1]` in pixels, the far corner excluded.
    rect: [u32; 4],
    want: Rgba,
    tolerance: u8,
}

/// The scene: vertices by draw, and what it must look like.
#[derive(Debug, Default)]
struct Scene {
    /// Solid quads of the three sections, then the cutout quads (of the first).
    vertices: Vec<u8>,
    /// Quads of each of the four draws.
    quads: [u32; 4],
    probes: Vec<Probe>,
}

/// One quad of the scene. Corners are given as the block's left, bottom, right and top
/// (in blocks from the section's corner) and a depth `z`; the quad faces the camera,
/// which looks along -z, in the bakery's order for a south face. `uv` is the texture
/// rectangle `[u0, v0, u1, v1]` in texels.
struct Quad {
    rect: [f32; 4],
    z: f32,
    uv: [f32; 4],
    /// Quarter turns of the texture (a face's `rotation`).
    turns: usize,
    /// Block and sky light, 0 to 255.
    light: [u8; 2],
    colour: Rgba,
    /// The corners in the opposite order: seen from behind.
    reversed: bool,
}

impl Quad {
    fn texel(rect: [f32; 4], at: [u32; 2]) -> Quad {
        let (u, v) = (at[0] as f32, at[1] as f32);
        Quad {
            rect,
            z: 0.0,
            uv: [u, v, u + 1.0, v + 1.0],
            turns: 0,
            light: [240, 240],
            colour: WHITE,
            reversed: false,
        }
    }

    fn pack(&self, out: &mut Vec<u8>) {
        let [left, bottom, right, top] = self.rect;
        // A south face: top left, bottom left, bottom right, top right.
        let mut corners = [[left, top], [left, bottom], [right, bottom], [right, top]];
        let [u0, v0, u1, v1] = self.uv;
        let mut uvs = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
        uvs.rotate_left(self.turns);
        if self.reversed {
            corners.reverse();
            uvs.reverse();
        }
        for (corner, uv) in corners.iter().zip(uvs) {
            Vertex {
                position: [corner[0], corner[1], self.z]
                    .map(|c| position_code(c).expect("inside a section's reach")),
                block_light: self.light[0],
                sky_light: self.light[1],
                colour: self.colour,
                uv: uv.map(|t| uv_code(t).expect("inside the atlas")),
            }
            .pack(out);
        }
    }
}

/// The pixels a block rectangle (left, bottom, right, top, in blocks of the whole
/// scene) covers.
fn pixels(rect: [f32; 4]) -> [u32; 4] {
    let s = SCALE as f32;
    [
        (rect[0] * s) as u32,
        ((BLOCKS[1] as f32 - rect[3]) * s) as u32,
        (rect[2] * s) as u32,
        ((BLOCKS[1] as f32 - rect[1]) * s) as u32,
    ]
}

fn times(a: Rgba, b: Rgba) -> Rgba {
    std::array::from_fn(|i| (f64::from(a[i]) * f64::from(b[i]) / 255.0).round() as u8)
}

impl Scene {
    fn build() -> Scene {
        let mut scene = Scene::default();
        let mut solid: [Vec<Quad>; 3] = Default::default();
        let mut cutout: Vec<Quad> = Vec::new();
        let mut probes = Vec::new();
        let mut probe = |what, rect: [f32; 4], want: Rgba, tolerance: u8| {
            probes.push(Probe {
                what,
                rect: pixels(rect),
                want,
                tolerance,
            });
        };
        let cell = |x: u32, y: u32| [x as f32, y as f32, (x + 1) as f32, (y + 1) as f32];

        // Colour: the stored byte, and its products.
        solid[0].push(Quad::texel(cell(0, 7), [1, 0]));
        probe("unlit grey", cell(0, 7), GREY, 0);
        // (At full light the fixture's light map is white.)
        let shade = [204, 204, 204, 255];
        solid[0].push(Quad {
            colour: shade,
            ..Quad::texel(cell(1, 7), [1, 0])
        });
        probe("grey times a shade", cell(1, 7), times(GREY, shade), 1);
        let tint = [51, 102, 25, 255];
        solid[0].push(Quad {
            colour: tint,
            ..Quad::texel(cell(2, 7), [1, 0])
        });
        probe("grey times a tint", cell(2, 7), times(GREY, tint), 1);

        // Light: raw levels at texel centres.
        for (x, light, want, tolerance, what) in [
            (3, [0, 0], light_texel(0, 0), 0, "no light"),
            (4, [240, 240], light_texel(15, 15), 0, "full light"),
            (5, [112, 48], light_texel(7, 3), 0, "light levels 7 and 3"),
            // Half way between two levels: half way between their texels.
            (
                6,
                [120, 200],
                [127, 212, 127, 255],
                1,
                "light between two levels",
            ),
            (
                7,
                [248, 0],
                light_texel(15, 0),
                0,
                "light beyond the last level",
            ),
        ] {
            solid[0].push(Quad {
                light,
                ..Quad::texel(cell(x, 7), [0, 0])
            });
            probe(what, cell(x, 7), want, tolerance);
        }

        // Cutout: less than half opaque is a hole, half opaque is not.
        cutout.push(Quad::texel(cell(8, 7), [5, 0]));
        probe("a texel under half opaque", cell(8, 7), CLEAR, 0);
        cutout.push(Quad::texel(cell(9, 7), [6, 0]));
        probe("a half opaque texel", cell(9, 7), HALF, 0);

        // Winding: seen from behind, nothing; and a quad in its block's top left.
        solid[0].push(Quad {
            reversed: true,
            ..Quad::texel(cell(10, 7), [2, 0])
        });
        probe("a quad seen from behind", cell(10, 7), CLEAR, 0);
        solid[0].push(Quad::texel([11.0, 7.5, 11.5, 8.0], [2, 0]));
        probe("a quarter quad", [11.0, 7.5, 11.5, 8.0], SOLID, 0);
        probe("beside the quarter quad", [11.5, 7.0, 12.0, 8.0], CLEAR, 0);
        probe("below the quarter quad", [11.0, 7.0, 11.5, 7.5], CLEAR, 0);

        // Texture coordinates: the atlas's last texel, two texels, a turned square.
        solid[0].push(Quad::texel(cell(0, 6), [4095, 4]));
        probe("the atlas's last texel", cell(0, 6), EDGE, 0);
        solid[0].push(Quad {
            uv: [4094.0, 6.0, 4096.0, 7.0],
            ..Quad::texel([1.0, 6.0, 3.0, 7.0], [4094, 6])
        });
        probe("the texel before the last", cell(1, 6), BEFORE_EDGE, 1);
        probe("the last texel beside it", cell(2, 6), AT_EDGE, 1);
        solid[0].push(Quad {
            uv: [16.0, 2.0, 18.0, 4.0],
            turns: 1,
            ..Quad::texel([3.0, 5.0, 5.0, 7.0], [16, 2])
        });
        // A quarter turn: each corner shows what the next corner in order would.
        probe("turned: top left", cell(3, 6), TURNED[2], 1);
        probe("turned: bottom left", cell(3, 5), TURNED[3], 1);
        probe("turned: bottom right", cell(4, 5), TURNED[1], 1);
        probe("turned: top right", cell(4, 6), TURNED[0], 1);

        // Depth: the nearer quad, whichever is drawn first.
        let near = |y: f32| Quad {
            z: 4.0,
            ..Quad::texel([5.0, y, 6.5, y + 1.0], [7, 0])
        };
        let far = |y: f32| Quad {
            z: -4.0,
            ..Quad::texel([5.5, y, 7.0, y + 1.0], [8, 0])
        };
        solid[0].push(near(6.0));
        solid[0].push(far(6.0));
        solid[0].push(far(5.0));
        solid[0].push(near(5.0));
        for y in [5.0, 6.0] {
            probe("the nearer quad", [5.0, y, 6.5, y + 1.0], NEAR, 0);
            probe("the farther quad beside it", [6.5, y, 7.0, y + 1.0], FAR, 0);
        }
        // A cutout quad in a solid one's plane: its texels over it, the solid through
        // its holes.
        solid[0].push(Quad::texel(cell(7, 6), [2, 0]));
        cutout.push(Quad {
            uv: [12.0, 0.0, 14.0, 1.0],
            ..Quad::texel(cell(7, 6), [12, 0])
        });
        probe(
            "the solid quad through a hole",
            [7.0, 6.0, 7.5, 7.0],
            SOLID,
            1,
        );
        probe("the cutout quad over it", [7.5, 6.0, 8.0, 7.0], CUTOUT, 1);

        // Section borders. The second section's corner is 12 blocks to the left of
        // the scene's, so its east border is at 4 blocks; the third's corner is there.
        solid[1].push(Quad::texel([12.0, 3.0, 16.0, 4.0], [9, 0]));
        solid[2].push(Quad::texel([0.0, 3.0, 4.0, 4.0], [10, 0]));
        probe(
            "up to a section's border",
            [0.0, 3.0, 4.0, 4.0],
            LEFT_SECTION,
            0,
        );
        probe("from the border on", [4.0, 3.0, 8.0, 4.0], RIGHT_SECTION, 0);
        // Corners half a block across the border, from both sides.
        solid[1].push(Quad::texel([16.0, 2.0, 16.5, 3.0], [7, 0]));
        solid[2].push(Quad::texel([-0.5, 2.0, 0.0, 3.0], [8, 0]));
        probe(
            "half a block over the border",
            [4.0, 2.0, 4.5, 3.0],
            NEAR,
            0,
        );
        probe("half a block before it", [3.5, 2.0, 4.0, 3.0], FAR, 0);
        probe("beside them", [0.0, 2.0, 3.5, 3.0], CLEAR, 0);
        probe("beside them", [4.5, 2.0, 12.0, 3.0], CLEAR, 0);

        // Nothing else is drawn.
        probe("the empty rows", [0.0, 0.0, 12.0, 2.0], CLEAR, 0);
        probe("the empty row", [0.0, 4.0, 12.0, 5.0], CLEAR, 0);
        probe("right of the border quads", [8.0, 3.0, 12.0, 4.0], CLEAR, 0);
        probe("right of the cutout quad", [8.0, 6.0, 12.0, 7.0], CLEAR, 0);
        probe("left of the turned square", [0.0, 5.0, 3.0, 6.0], CLEAR, 0);
        probe("right of the depth quads", [7.0, 5.0, 12.0, 6.0], CLEAR, 0);

        for (i, quads) in solid.iter().chain([&cutout]).enumerate() {
            scene.quads[i] = quads.len() as u32;
            for q in quads {
                q.pack(&mut scene.vertices);
            }
        }
        scene.probes = probes;
        scene
    }
}

/// The scene's camera: it looks along -z at the 12 x 8 blocks from the scene's corner,
/// x to the right and y up, and maps z from -16 to 16 onto depth 0 to 1 (nearer is
/// greater, as the real projection's). Columns first.
fn matrix() -> [f32; 16] {
    let (w, h) = (BLOCKS[0] as f32, BLOCKS[1] as f32);
    [
        2.0 / w,
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / h,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0 / 32.0,
        0.0,
        -1.0,
        1.0,
        0.5,
        1.0,
    ]
}

/// What the check has seen so far.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerrainCheckReport {
    /// Frames drawn, read back and compared.
    pub compared: u64,
    /// How many of them had a wrong pixel.
    pub mismatches: u64,
    /// Rectangles compared in each.
    pub probes: usize,
}

impl TerrainCheckReport {
    pub fn passed(&self) -> bool {
        self.compared > 0 && self.mismatches == 0
    }
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct TerrainCheck {
    gpu: Arc<Gpu>,
    target: Readback,
    depth: DepthTarget,
    pool: BufferPool,
    scene: Scene,
    vertices: RangeHandle,
    indices: RangeHandle,
    /// The two buffers' uploads, once staged.
    uploads: [Option<UploadTicket>; 2],
    atlas: TextureId,
    lightmap: TextureId,
    report: TerrainCheckReport,
}

fn extent() -> vk::Extent2D {
    vk::Extent2D {
        width: WIDTH,
        height: HEIGHT,
    }
}

impl TerrainCheck {
    pub fn new(
        gpu: Arc<Gpu>,
        format: vk::Format,
        textures: &mut TextureTable,
        uploads: &mut UploadRing,
    ) -> Result<TerrainCheck, Error> {
        let scene = Scene::build();
        let target = Readback::new(gpu.clone(), format, extent())?;
        let depth = DepthTarget::new(gpu.clone(), extent())?;
        let page = 1 << 20;
        let mut pool = BufferPool::new(
            gpu.clone(),
            PoolConfig {
                page_bytes: page,
                budget_bytes: page,
                low_watermark: 0,
                high_watermark: 0,
                release_after: u32::MAX,
                granule: VERTEX_BYTES as u64,
            },
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER,
        );
        pool.grow_now()?;
        let alloc = |pool: &mut BufferPool, bytes: usize| {
            pool.alloc(bytes as u64, VERTEX_BYTES as u64)
                .map_err(|e| Error::Unsupported(format!("the terrain check's buffers: {e:?}")))
        };
        // Other ranges first, so that neither starts its page.
        alloc(&mut pool, 3 * VERTEX_BYTES)?;
        let vertices = alloc(&mut pool, scene.vertices.len())?;
        let indices = alloc(&mut pool, quad_indices().len())?;
        let mut texels = vec![0u8; (ATLAS[0] * ATLAS[1] * 4) as usize];
        for t in texels.as_chunks_mut::<4>().0 {
            *t = POISON;
        }
        for (at, colour) in TEXELS {
            let i = ((at[1] * ATLAS[0] + at[0]) * 4) as usize;
            texels[i..i + 4].copy_from_slice(&colour);
        }
        let atlas = textures.create_with(
            uploads,
            TextureDesc::rgba8(ATLAS[0], ATLAS[1]),
            &[&texels],
            Arrival::NextFrame,
        )?;
        let light: Vec<u8> = (0..16)
            .flat_map(|sky| (0..16).flat_map(move |block| light_texel(block, sky)))
            .collect();
        let lightmap = textures.create_with(
            uploads,
            TextureDesc::rgba8(16, 16),
            &[&light],
            Arrival::NextFrame,
        )?;
        let report = TerrainCheckReport {
            probes: scene.probes.len(),
            ..TerrainCheckReport::default()
        };
        Ok(TerrainCheck {
            gpu,
            target,
            depth,
            pool,
            scene,
            vertices,
            indices,
            uploads: [None; 2],
            atlas,
            lightmap,
            report,
        })
    }

    pub fn report(&self) -> TerrainCheckReport {
        self.report
    }

    /// Stages the buffers' contents until both are on their way. Call it before the
    /// frame's `UploadRing::record`.
    pub fn stage(&mut self, uploads: &mut UploadRing) {
        self.pool.maintain(&uploads.completed());
        let indices = quad_indices();
        for (i, (handle, bytes)) in [
            (self.vertices, &self.scene.vertices),
            (self.indices, &indices),
        ]
        .into_iter()
        .enumerate()
        {
            if self.uploads[i].is_none()
                && let Some(range) = self.pool.resolve(handle)
            {
                self.uploads[i] = uploads
                    .upload_buffer(range.buffer, range.offset, bytes)
                    .ok();
            }
        }
    }

    /// Compares what has come back and draws the scene again. `completed` is a frame
    /// the GPU has finished; call it after `UploadRing::record` and
    /// `textures.begin_frame` for the frame `encoder` records, whose number is `frame`.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        encoder: &mut Encoder<'_>,
        completed: u64,
        frame: u64,
        textures: &TextureTable,
        uploads: &UploadRing,
        frame_data: &mut FrameData,
        pipelines: &Pipelines,
        indirect: bool,
    ) -> Result<(), Error> {
        if let Some((drawn, pixels)) = self.target.take(completed) {
            let ok = compare(&self.scene.probes, drawn, pixels, self.report.mismatches);
            self.report.compared += 1;
            self.report.mismatches += u64::from(!ok);
        }
        if self.target.format() != pipelines.format() {
            self.target = Readback::new(self.gpu.clone(), pipelines.format(), extent())?;
        }
        let ready = textures.is_ready(self.atlas)
            && textures.is_ready(self.lightmap)
            && self
                .uploads
                .iter()
                .all(|t| t.is_some_and(|t| uploads.is_ready(t)));
        let (Some(vertices), Some(indices)) = (
            self.pool.resolve(self.vertices),
            self.pool.resolve(self.indices),
        ) else {
            return Ok(());
        };
        if !ready || self.target.pending() {
            return Ok(());
        }
        // The three kinds of record, each after a gap that changes from frame to frame,
        // so that none starts where it did the frame before, or at the region's start.
        let gap = |frame_data: &mut FrameData, words: u64| {
            frame_data.alloc(1 + (frame % words) as u32, 4).is_some()
        };
        // The sections move by whole blocks and the camera's offset takes it back.
        let shift = (frame % 5) as i32;
        if !gap(frame_data, 3) {
            return Ok(());
        }
        let Some((world, bytes)) = frame_data.alloc(WORLD.words(), 4) else {
            return Ok(());
        };
        let word = |bytes: &mut [u8], at: u32, v: [u8; 4]| {
            bytes[at as usize * 4..at as usize * 4 + 4].copy_from_slice(&v);
        };
        let camera = WORLD.offset("camera_offset");
        word(bytes, camera, (-shift as f32).to_ne_bytes());
        word(bytes, camera + 1, 0f32.to_ne_bytes());
        word(bytes, camera + 2, 0f32.to_ne_bytes());
        word(
            bytes,
            WORLD.offset("atlas"),
            self.atlas.index().to_ne_bytes(),
        );
        let size = WORLD.offset("atlas_size");
        word(bytes, size, (ATLAS[0] as f32).to_ne_bytes());
        word(bytes, size + 1, (ATLAS[1] as f32).to_ne_bytes());
        word(
            bytes,
            WORLD.offset("lightmap"),
            self.lightmap.index().to_ne_bytes(),
        );
        if !gap(frame_data, 4) {
            return Ok(());
        }
        let Some((matrix_at, bytes)) = frame_data.alloc(WORLD_MATRIX.words(), 4) else {
            return Ok(());
        };
        for (i, c) in matrix().iter().enumerate() {
            word(bytes, i as u32, c.to_ne_bytes());
        }
        if !gap(frame_data, 7) {
            return Ok(());
        }
        let Some((sections, bytes)) = frame_data.alloc(3 * SECTION.words(), 4) else {
            return Ok(());
        };
        for (record, origin) in bytes
            .as_chunks_mut::<SECTION_BYTES>()
            .0
            .iter_mut()
            .zip([0, -12, 4])
        {
            write_section(record, [origin + shift, 0, 0], 1.0);
        }
        let push = terrain_push(world, matrix_at, sections);
        let first = (vertices.offset / VERTEX_BYTES as u64) as u32;
        // The scene's draws: pipeline, instance, quads and first quad.
        let mut draws = Vec::with_capacity(self.scene.quads.len());
        let mut quad = 0;
        for (i, &quads) in self.scene.quads.iter().enumerate() {
            let (pipeline, instance) = match i {
                3 => (PipelineId::TerrainCutout, 0),
                section => (PipelineId::TerrainSolid, section as u32),
            };
            if quads > 0 {
                draws.push((pipeline, instance, quads, quad));
            }
            quad += quads;
        }
        // Drawn indirectly, each pipeline's draws are commands in the frame's data,
        // after a gap that changes like the records'.
        let mut calls = Vec::new();
        if indirect {
            for pipeline in [PipelineId::TerrainSolid, PipelineId::TerrainCutout] {
                let commands: Vec<u8> = draws
                    .iter()
                    .filter(|d| d.0 == pipeline)
                    .flat_map(|&(_, instance, quads, quad)| {
                        draw_command(quads * 6, (first + quad * 4) as i32, instance)
                    })
                    .collect();
                if commands.is_empty() {
                    continue;
                }
                let count = commands.len() as u32 / DRAW_COMMAND_BYTES;
                if !gap(frame_data, 5) {
                    return Ok(());
                }
                let Some((offset, bytes)) = frame_data.alloc_draw_commands(count) else {
                    return Ok(());
                };
                bytes.copy_from_slice(&commands);
                calls.push((pipeline, offset, count));
            }
        }
        let commands = frame_data.buffer();
        let mut pass = encoder.readback_pass_with_depth(pipelines, &mut self.target, &self.depth);
        // SAFETY: the push constants are terrain.vert's and the records they point at
        // were written just above; the textures are the check's own and have arrived,
        // as have both buffers' contents; each draw stays inside the scene's vertices
        // and the quads the indices cover.
        unsafe {
            pass.bind_index_buffer(indices.buffer, indices.offset);
            pass.bind_vertex_buffer(vertices.buffer, 0);
            // The push constants go with the first draw and stay.
            let mut first_draw = true;
            let mut constants = || -> &[u8] {
                if std::mem::take(&mut first_draw) {
                    &push
                } else {
                    &[]
                }
            };
            if indirect {
                for &(pipeline, offset, count) in &calls {
                    pass.draw_indexed_indirect(
                        pipeline.index(),
                        constants(),
                        commands,
                        offset,
                        count,
                    );
                }
            } else {
                for &(pipeline, instance, quads, quad) in &draws {
                    pass.draw_indexed(
                        pipeline.index(),
                        constants(),
                        quads * 6,
                        0,
                        (first + quad * 4) as i32,
                        instance,
                    );
                }
            }
        }
        pass.finish();
        Ok(())
    }
}

/// Judges every probe's pixels in an RGBA image of the scene, and says so if one is
/// wrong (in detail for the first few mismatching frames).
fn compare(probes: &[Probe], frame: u64, pixels: &[u8], mismatches: u64) -> bool {
    let at = |x: u32, y: u32| -> Rgba {
        let i = ((y * WIDTH + x) * 4) as usize;
        pixels[i..i + 4].try_into().expect("4 bytes")
    };
    let mut ok = true;
    for p in probes {
        let [x0, y0, x1, y1] = p.rect;
        let wrong = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .find(|&(x, y)| {
                at(x, y)
                    .iter()
                    .zip(p.want)
                    .any(|(&got, want)| got.abs_diff(want) > p.tolerance)
            });
        if let Some((x, y)) = wrong {
            if mismatches < PRINTED {
                eprintln!(
                    "[terrain-check] frame {frame}, {}: pixel ({x}, {y}) reads {:?}, must be {:?} within {}",
                    p.what,
                    at(x, y),
                    p.want,
                    p.tolerance
                );
            }
            ok = false;
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scene as a simple rasteriser draws it from the packed vertices: what the
    /// probes must agree with if the scene's own description is consistent.
    fn render(scene: &Scene) -> Vec<u8> {
        let mut texels = vec![POISON; (ATLAS[0] * ATLAS[1]) as usize];
        for (at, colour) in TEXELS {
            texels[(at[1] * ATLAS[0] + at[0]) as usize] = colour;
        }
        let mut colour = vec![CLEAR; (WIDTH * HEIGHT) as usize];
        let mut depth = vec![0.0_f32; (WIDTH * HEIGHT) as usize];
        let quads: Vec<[Vertex; 4]> = scene
            .vertices
            .as_chunks::<VERTEX_BYTES>()
            .0
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q.map(|v| Vertex::unpack(&v)))
            .collect();
        let origins = [0.0_f32, -12.0, 4.0, 0.0];
        let mut next = 0;
        for (draw, &count) in scene.quads.iter().enumerate() {
            for q in &quads[next..next + count as usize] {
                let p = q.map(|v| v.position.map(crate::terrain_vertex::position_of));
                let x = |i: usize| p[i][0] + origins[draw];
                // Counter-clockwise with y up: the area is positive.
                let area =
                    (x(1) - x(0)) * (p[2][1] - p[0][1]) - (x(2) - x(0)) * (p[1][1] - p[0][1]);
                if area <= 0.0 {
                    continue;
                }
                let (left, right) = (x(0).min(x(2)), x(0).max(x(2)));
                let (bottom, top) = (p[0][1].min(p[2][1]), p[0][1].max(p[2][1]));
                let z = 0.5 + p[0][2] / 32.0;
                for py in 0..HEIGHT {
                    for px in 0..WIDTH {
                        let (wx, wy) = (
                            (px as f32 + 0.5) / SCALE as f32,
                            BLOCKS[1] as f32 - (py as f32 + 0.5) / SCALE as f32,
                        );
                        if wx < left || wx >= right || wy < bottom || wy >= top {
                            continue;
                        }
                        // Where in the quad, as weights of its corners 0 to 3.
                        let (s, t) = (
                            (wx - x(0)) / (x(2) - x(0)),
                            (wy - p[0][1]) / (p[2][1] - p[0][1]),
                        );
                        let uv: [f32; 2] = std::array::from_fn(|a| {
                            let c = |i: usize| f32::from(q[i].uv[a]) / 8.0;
                            // Corners 0 and 1 share s = 0, corners 0 and 3 share t = 0.
                            c(0) + (c(3) - c(0)) * s + (c(1) - c(0)) * t
                        });
                        let texel = texels[(uv[1] as u32 * ATLAS[0] + uv[0] as u32) as usize];
                        let light = light_texel(
                            (u32::from(q[0].block_light) / 16).min(15),
                            (u32::from(q[0].sky_light) / 16).min(15),
                        );
                        let out = times(times(texel, q[0].colour), light);
                        if draw == 3 && out[3] < 128 {
                            continue;
                        }
                        let i = (py * WIDTH + px) as usize;
                        if z >= depth[i] {
                            depth[i] = z;
                            colour[i] = out;
                        }
                    }
                }
            }
            next += count as usize;
        }
        colour.into_iter().flatten().collect()
    }

    #[test]
    fn the_scene_is_what_its_probes_say() {
        let scene = Scene::build();
        assert_eq!(
            scene.quads.iter().sum::<u32>() as usize * 4 * VERTEX_BYTES,
            scene.vertices.len()
        );
        let mut image = render(&scene);
        // The light between two levels is the one thing the simple rasteriser does not
        // blend: it reads the lower texels.
        let between = scene
            .probes
            .iter()
            .find(|p| p.what == "light between two levels")
            .unwrap();
        for y in between.rect[1]..between.rect[3] {
            for x in between.rect[0]..between.rect[2] {
                let i = ((y * WIDTH + x) * 4) as usize;
                assert_eq!(image[i..i + 4], light_texel(7, 12));
                image[i..i + 4].copy_from_slice(&between.want);
            }
        }
        assert!(compare(&scene.probes, 0, &image, 0));
        // Every pixel is judged by some probe.
        let mut covered = vec![false; (WIDTH * HEIGHT) as usize];
        for p in &scene.probes {
            for y in p.rect[1]..p.rect[3] {
                for x in p.rect[0]..p.rect[2] {
                    covered[(y * WIDTH + x) as usize] = true;
                }
            }
        }
        assert!(covered.iter().all(|c| *c));
    }

    #[test]
    fn a_wrong_picture_fails() {
        let scene = Scene::build();
        let good = render(&scene);
        let fails = |change: &dyn Fn(&mut Vec<u8>)| {
            let mut image = good.clone();
            change(&mut image);
            // (The light between two levels is wrong in every simple render: leave it
            // out of what is compared.)
            let probes: Vec<Probe> = scene
                .probes
                .iter()
                .copied()
                .filter(|p| p.what != "light between two levels")
                .collect();
            !compare(&probes, 0, &image, PRINTED)
        };
        assert!(!fails(&|_| {}));
        // A grey one step off (an sRGB view would be tens off).
        assert!(fails(&|i| i[0] += 1));
        // A gap of one pixel at the section border.
        let seam = ((4 * SCALE + 4) * WIDTH + 4 * SCALE) as usize * 4;
        assert!(fails(&|i| i[seam..seam + 4].copy_from_slice(&CLEAR)));
        // The farther quad over the nearer.
        let overlap = ((SCALE + 4) * WIDTH + 6 * SCALE) as usize * 4;
        assert!(fails(&|i| i[overlap..overlap + 4].copy_from_slice(&FAR)));
        // The picture upside down or mirrored.
        assert!(fails(&|i| {
            let rows: Vec<Vec<u8>> = i.chunks((WIDTH * 4) as usize).map(<[u8]>::to_vec).collect();
            *i = rows.into_iter().rev().flatten().collect();
        }));
    }

    #[test]
    fn the_matrix_puts_the_scene_on_the_target() {
        let m = matrix();
        let project = |p: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
        };
        // The scene's corner is the target's bottom left; y is up; nearer is greater.
        assert_eq!(project([0.0, 0.0, 0.0]), [-1.0, 1.0, 0.5]);
        assert_eq!(project([12.0, 8.0, 0.0]), [1.0, -1.0, 0.5]);
        assert!(project([0.0, 0.0, 4.0])[2] > project([0.0, 0.0, -4.0])[2]);
        assert_eq!(m[15], 1.0);
    }
}
