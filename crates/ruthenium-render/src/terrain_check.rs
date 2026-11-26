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
//! - **Translucency**: sorted indices put one pane over another. A fragment that
//!   passes the one-tenth alpha cutoff blends and writes depth. A hole leaves both
//!   colour and depth available to a later draw.
//! - **Winding**: a quad seen from behind is not drawn; a quad in a corner of its block
//!   is in that corner.
//! - **Section borders**: two sections' quads that meet on the border between them
//!   leave no gap and no overlap, nor do corners that reach half a block across.
//! - **Mixed records**: the draws read the world's values, its matrix and the sections'
//!   records from three places in the frame's data, which move from frame to frame; and
//!   the sections' origins and the camera's offset change together so that the picture
//!   must stay the same.
//! - **Fade**: a fourth section, a quarter faded in, far behind the others: its quads
//!   are all there within 16 blocks of the camera and go towards the fog's colour
//!   beyond, with their texels' alpha kept.
//! - **Fog and filtering**: every frame draws the scene with the next of the terrain's
//!   shaders and, every fourth frame, the next of a few fogs: none, the fog of the
//!   place, the fog at the edge of the view, both, and a fog that is half there. What
//!   comes back is compared pixel by pixel with the scene as a small rasteriser here
//!   draws it from the same vertices, by the shaders' own arithmetic. The scene's
//!   texels are larger than its pixels, where every filter must give the same bytes;
//!   one quad shows four texels a pixel of an evenly coloured stretch of the atlas.

use std::sync::Arc;

use ruthenium_gpu::{
    Arrival, BufferPool, DRAW_COMMAND_BYTES, DepthTarget, Encoder, Error, FrameData, Gpu,
    Pipelines, PoolConfig, RangeHandle, Readback, TextureDesc, TextureId, TextureTable, UploadRing,
    UploadTicket, vk,
};
use ruthenium_shaders::interface::{SECTION, WORLD, WORLD_MATRIX};

use crate::atmosphere::Fog;
use crate::terrain::TerrainShader;
use crate::terrain::world::{
    SECTION_BYTES, draw_command, quad_indices, terrain_push, write_section,
};
use crate::terrain_vertex::{VERTEX_BYTES, Vertex, position_code, position_of, uv_code};

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
/// Texels just under and just over a tenth opaque: what the translucent layer leaves
/// out and what it blends.
const UNDER_A_TENTH: Rgba = [200, 100, 50, 25];
const OVER_A_TENTH: Rgba = [200, 100, 50, 26];
/// Two half opaque texels for the translucent layer's order.
const GLASS_NEAR: Rgba = [250, 250, 0, 0x80];
const GLASS_FAR: Rgba = [0, 0, 250, 0x80];
const TURNED: [Rgba; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
];
const EDGE: Rgba = [17, 34, 51, 255];
const BEFORE_EDGE: Rgba = [1, 2, 3, 255];
const AT_EDGE: Rgba = [250, 240, 230, 255];
/// An even stretch of the atlas, for the quad that shows four of its texels a pixel:
/// its colour, where it starts and how many texels it is wide; it fills rows 4 to 6.
const EVEN: Rgba = [30, 144, 222, 255];
const EVEN_AT: u32 = 96;
const EVEN_WIDTH: u32 = 48;

/// Where each section's corner is, in blocks from the scene's corner, and how far each
/// has faded in. The fourth is 12 blocks behind the others.
const ORIGINS: [[i32; 3]; 4] = [[0, 0, 0], [-12, 0, 0], [4, 0, 0], [0, 0, -12]];
const VISIBILITY: [f32; 4] = [1.0, 1.0, 1.0, 0.25];

/// The fogs the scene is drawn in, in turn. Their ranges lie inside the scene's own
/// distances (at most 21 blocks from its corner, where the camera is).
const FOGS: [Fog; 5] = [
    // None, in a colour no pixel may show but the fading section's.
    Fog {
        colour: [1.0, 0.0, 1.0, 1.0],
        environmental: [f32::MAX; 2],
        render_distance: [f32::MAX; 2],
        sky_end: f32::MAX,
    },
    // The fog of the place alone, by the distance in a straight line.
    Fog {
        colour: [0.2, 0.6, 1.0, 1.0],
        environmental: [3.0, 19.0],
        render_distance: [f32::MAX; 2],
        sky_end: f32::MAX,
    },
    // The fog at the edge of the view alone, sideways or in height.
    Fog {
        colour: [0.9, 0.5, 0.1, 1.0],
        environmental: [f32::MAX; 2],
        render_distance: [2.0, 14.0],
        sky_end: f32::MAX,
    },
    // Both: the stronger counts, here one and there the other.
    Fog {
        colour: [0.2, 0.6, 1.0, 1.0],
        environmental: [0.0, 24.0],
        render_distance: [6.0, 12.5],
        sky_end: f32::MAX,
    },
    // A fog whose colour is half there.
    Fog {
        colour: [1.0, 1.0, 1.0, 0.5],
        environmental: [3.0, 19.0],
        render_distance: [f32::MAX; 2],
        sky_end: f32::MAX,
    },
];

/// How a frame draws the scene: with which shader and in which fog.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Case {
    shader: TerrainShader,
    fog: usize,
}

impl Case {
    /// The frame's: every shader in turn, and the next fog once all have drawn in one.
    fn of(frame: u64) -> Case {
        let shaders = TerrainShader::ALL;
        Case {
            shader: shaders[(frame % shaders.len() as u64) as usize],
            fog: ((frame / shaders.len() as u64) % FOGS.len() as u64) as usize,
        }
    }

    /// Whether the shader fogs and fades at all.
    fn fogged(self) -> bool {
        self.shader != TerrainShader::Plain
    }

    /// Whether the shader averages far texels. In this atlas, 512 times as wide as it
    /// is high, every texel is far by vanilla's rule: all of the scene is averaged.
    fn rgss(self) -> bool {
        matches!(
            self.shader,
            TerrainShader::Rgss | TerrainShader::RgssReference
        )
    }

    /// Where the case's picture is kept among the pictures there can be: one for each
    /// fog with hard texels and one with averaged ones, and one without fog.
    fn picture(self) -> usize {
        match (self.fogged(), self.rgss()) {
            (false, _) => 2 * FOGS.len(),
            (true, rgss) => self.fog + if rgss { FOGS.len() } else { 0 },
        }
    }
}

/// Where each texel a quad shows is in the atlas. The two texels around half opaque
/// are not side by side: averaged, their border would be a hole or not by the last bit
/// of a GPU's arithmetic.
const TEXELS: [([u32; 2], Rgba); 24] = [
    ([20, 0], UNDER_A_TENTH),
    ([21, 0], OVER_A_TENTH),
    ([22, 0], GLASS_NEAR),
    ([23, 0], GLASS_FAR),
    ([0, 0], WHITE),
    ([1, 0], GREY),
    ([2, 0], SOLID),
    ([3, 0], HALF),
    ([5, 0], BELOW_HALF),
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

/// A rectangle of pixels and what every one of them must read without fog, within
/// `tolerance` steps per channel.
#[derive(Debug, Clone, Copy)]
struct Probe {
    what: &'static str,
    /// `[x0, y0, x1, y1]` in pixels, the far corner excluded.
    rect: [u32; 4],
    /// `None`: not one colour (a quad that fades): judged by the rasteriser's picture
    /// alone. (The check itself judges every probe by that picture; the tests hold the
    /// picture to these colours.)
    #[cfg_attr(not(test), allow(dead_code))]
    want: Option<Rgba>,
    tolerance: u8,
}

/// The scene: vertices by draw, and what it must look like.
#[derive(Debug, Default)]
struct Scene {
    /// Solid quads of the four sections, then the cutout quads and the translucent
    /// quads (both of the first).
    vertices: Vec<u8>,
    /// Quads of each of the six draws.
    quads: [u32; 6],
    /// The order the translucent quads are drawn in, as their indices say: not the
    /// order of their vertices.
    order: Vec<u32>,
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

/// `over` blended over `under` as the translucent pipeline blends: by its own alpha,
/// and the alphas added up likewise.
fn blended(over: Rgba, under: Rgba) -> Rgba {
    let a = f64::from(over[3]) / 255.0;
    std::array::from_fn(|i| {
        let src = if i == 3 { 1.0 } else { a };
        (f64::from(over[i]) * src + f64::from(under[i]) * (1.0 - a)).round() as u8
    })
}

fn times(a: Rgba, b: Rgba) -> Rgba {
    std::array::from_fn(|i| (f64::from(a[i]) * f64::from(b[i]) / 255.0).round() as u8)
}

impl Scene {
    fn build() -> Scene {
        let mut scene = Scene::default();
        let mut solid: [Vec<Quad>; 4] = Default::default();
        let mut cutout: Vec<Quad> = Vec::new();
        let mut translucent: Vec<Quad> = Vec::new();
        let mut probes = Vec::new();
        let mut probe = |what, rect: [f32; 4], want: Rgba, tolerance: u8| {
            probes.push(Probe {
                what,
                rect: pixels(rect),
                want: Some(want),
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
        cutout.push(Quad::texel(cell(9, 7), [3, 0]));
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

        // The translucent layer, drawn last through indices of its own. A half opaque
        // texel is blended over what is there, a solid quad or nothing.
        solid[0].push(Quad {
            z: -2.0,
            ..Quad::texel(cell(0, 4), [2, 0])
        });
        translucent.push(Quad::texel(cell(0, 4), [3, 0]));
        probe(
            "half opaque over a solid quad",
            cell(0, 4),
            blended(HALF, SOLID),
            1,
        );
        translucent.push(Quad::texel(cell(1, 4), [3, 0]));
        probe(
            "half opaque over nothing",
            cell(1, 4),
            blended(HALF, CLEAR),
            1,
        );
        // Under a tenth opaque it is left out; from a tenth on it is not.
        translucent.push(Quad::texel(cell(2, 4), [20, 0]));
        probe("a texel under a tenth opaque", cell(2, 4), CLEAR, 0);
        solid[0].push(Quad {
            z: -2.0,
            ..Quad::texel([3.5, 4.0, 4.0, 5.0], [2, 0])
        });
        translucent.push(Quad::texel(cell(3, 4), [21, 0]));
        probe(
            "a texel a tenth opaque",
            [3.0, 4.0, 3.5, 5.0],
            blended(OVER_A_TENTH, CLEAR),
            1,
        );
        probe(
            "a texel a tenth opaque over solid terrain",
            [3.5, 4.0, 4.0, 5.0],
            blended(OVER_A_TENTH, SOLID),
            1,
        );
        // Draw the far pane second in these cells to make depth writes observable.
        // The front pane writes depth in cell 7, is discarded in cell 8, and the
        // solid terrain already owns the depth in cell 9.
        translucent.push(Quad::texel(cell(7, 4), [22, 0]));
        translucent.push(Quad::texel(cell(8, 4), [20, 0]));
        solid[0].push(Quad::texel(cell(9, 4), [2, 0]));
        translucent.push(Quad {
            z: -4.0,
            ..Quad::texel([7.0, 4.0, 10.0, 5.0], [23, 0])
        });
        probe(
            "depth written by a pane",
            cell(7, 4),
            blended(GLASS_NEAR, CLEAR),
            1,
        );
        probe(
            "depth through a discarded pane",
            cell(8, 4),
            blended(GLASS_FAR, CLEAR),
            1,
        );
        probe("a pane behind solid terrain", cell(9, 4), SOLID, 1);

        // Order: the nearer quad's vertices come first, the indices draw the farther
        // one first. Drawn in the vertices' order the nearer quad's depth would keep
        // the farther one out from behind it.
        let first_ordered = translucent.len() as u32;
        translucent.push(Quad {
            z: 4.0,
            ..Quad::texel([5.0, 4.0, 6.5, 5.0], [22, 0])
        });
        translucent.push(Quad {
            z: -4.0,
            ..Quad::texel([5.5, 4.0, 7.0, 5.0], [23, 0])
        });
        scene.order = (0..first_ordered)
            .chain([first_ordered + 1, first_ordered])
            .collect();
        probe(
            "the nearer glass alone",
            [5.0, 4.0, 5.5, 5.0],
            blended(GLASS_NEAR, CLEAR),
            1,
        );
        probe(
            "the nearer glass over the farther",
            [5.5, 4.0, 6.5, 5.0],
            blended(GLASS_NEAR, blended(GLASS_FAR, CLEAR)),
            1,
        );
        probe(
            "the farther glass alone",
            [6.5, 4.0, 7.0, 5.0],
            blended(GLASS_FAR, CLEAR),
            1,
        );

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

        // Four texels a pixel along the atlas, of an even stretch of it: whatever is
        // sampled there, and however often, is its colour.
        solid[0].push(Quad {
            uv: [(EVEN_AT + 8) as f32, 5.0, (EVEN_AT + 40) as f32, 6.0],
            ..Quad::texel(cell(0, 1), [EVEN_AT, 5])
        });
        probe("four texels a pixel", cell(0, 1), EVEN, 1);

        // Fade: the fourth section's quads, 15.5 blocks behind the scene's corner. Up
        // to 16 blocks from the camera they are as they are; beyond, they go towards
        // the fog's colour. A half opaque texel keeps its alpha.
        for (x, at) in [(0, [2, 0]), (5, [2, 0]), (8, [3, 0]), (11, [2, 0])] {
            solid[3].push(Quad {
                z: -3.5,
                ..Quad::texel(cell(x, 0), at)
            });
        }
        probe(
            "a fading section's quad near the camera",
            cell(0, 0),
            SOLID,
            0,
        );
        for x in [5, 8, 11] {
            probes.push(Probe {
                what: "a fading section's quad",
                rect: pixels(cell(x, 0)),
                want: None,
                tolerance: 1,
            });
        }
        let mut probe = |what, rect: [f32; 4], want: Rgba, tolerance: u8| {
            probes.push(Probe {
                what,
                rect: pixels(rect),
                want: Some(want),
                tolerance,
            });
        };

        // Nothing else is drawn.
        probe("between the fading quads", [1.0, 0.0, 5.0, 1.0], CLEAR, 0);
        probe("between the fading quads", [6.0, 0.0, 8.0, 1.0], CLEAR, 0);
        probe("between the fading quads", [9.0, 0.0, 11.0, 1.0], CLEAR, 0);
        probe("the empty row", [1.0, 1.0, 12.0, 2.0], CLEAR, 0);
        probe("between the translucent quads", cell(4, 4), CLEAR, 0);
        probe(
            "right of the translucent quads",
            [10.0, 4.0, 12.0, 5.0],
            CLEAR,
            0,
        );
        probe("right of the border quads", [8.0, 3.0, 12.0, 4.0], CLEAR, 0);
        probe("right of the cutout quad", [8.0, 6.0, 12.0, 7.0], CLEAR, 0);
        probe("left of the turned square", [0.0, 5.0, 3.0, 6.0], CLEAR, 0);
        probe("right of the depth quads", [7.0, 5.0, 12.0, 6.0], CLEAR, 0);

        for (i, quads) in solid.iter().chain([&cutout, &translucent]).enumerate() {
            scene.quads[i] = quads.len() as u32;
            for q in quads {
                q.pack(&mut scene.vertices);
            }
        }
        scene.probes = probes;
        scene
    }
}

impl Scene {
    /// The translucent draw's 16-bit indices: its quads in [`Self::order`], two
    /// triangles each.
    fn translucent_indices(&self) -> Vec<u8> {
        self.order
            .iter()
            .flat_map(|quad| [0, 1, 2, 2, 3, 0].map(|corner| (quad * 4 + corner) as u16))
            .flat_map(u16::to_le_bytes)
            .collect()
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

/// The fixture's atlas: every texel a quad shows, the even stretch, and poison.
fn atlas_texels() -> Vec<Rgba> {
    let mut texels = vec![POISON; (ATLAS[0] * ATLAS[1]) as usize];
    for y in 4..7 {
        for x in EVEN_AT..EVEN_AT + EVEN_WIDTH {
            texels[(y * ATLAS[0] + x) as usize] = EVEN;
        }
    }
    for (at, colour) in TEXELS {
        texels[(at[1] * ATLAS[0] + at[0]) as usize] = colour;
    }
    texels
}

/// The light map's colour for a vertex's two lights, as the vertex stage looks it up:
/// sixteenths of a level, blended between the texels' centres.
fn lit(light: [u8; 2]) -> [f32; 4] {
    let at =
        |l: u8| (f32::from(l) / 256.0 + 0.5 / 16.0).clamp(0.5 / 16.0, 15.5 / 16.0) * 16.0 - 0.5;
    let (block, sky) = (at(light[0]), at(light[1]));
    let (b, s) = (block.floor(), sky.floor());
    let (fb, fs) = (block - b, sky - s);
    let texel = |db: u32, ds: u32| {
        light_texel((b as u32 + db).min(15), (s as u32 + ds).min(15)).map(|c| f32::from(c) / 255.0)
    };
    let (t00, t10, t01, t11) = (texel(0, 0), texel(1, 0), texel(0, 1), texel(1, 1));
    std::array::from_fn(|i| {
        (t00[i] * (1.0 - fb) + t10[i] * fb) * (1.0 - fs) + (t01[i] * (1.0 - fb) + t11[i] * fb) * fs
    })
}

/// The atlas read between its texels' centres at `at` (in texels), as the linear
/// sampler reads its one level: nothing beyond the edges but the edge.
fn bilinear(texels: &[Rgba], at: [f32; 2]) -> [f32; 4] {
    let (x, y) = (at[0] - 0.5, at[1] - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let texel = |dx: f32, dy: f32| {
        let tx = (x0 + dx).clamp(0.0, (ATLAS[0] - 1) as f32) as u32;
        let ty = (y0 + dy).clamp(0.0, (ATLAS[1] - 1) as f32) as u32;
        texels[(ty * ATLAS[0] + tx) as usize].map(|c| f32::from(c) / 255.0)
    };
    let (t00, t10, t01, t11) = (
        texel(0.0, 0.0),
        texel(1.0, 0.0),
        texel(0.0, 1.0),
        texel(1.0, 1.0),
    );
    std::array::from_fn(|i| {
        (t00[i] * (1.0 - fx) + t10[i] * fx) * (1.0 - fy) + (t01[i] * (1.0 - fx) + t11[i] * fx) * fy
    })
}

/// The atlas's colour at `at` (in texels) as vanilla's sampleNearest and sampleRGSS
/// work it out, square roots and divisions and all (`terrain_frag.glsl` keeps that
/// arithmetic for rgss-reference; its other shaders multiply instead). `du` and
/// `dv` are how far the place moves, in texels, from one pixel to the next across and
/// down the screen. `rgss`: with far texels averaged.
fn sampled(texels: &[Rgba], at: [f32; 2], du: [f32; 2], dv: [f32; 2], rgss: bool) -> [f32; 4] {
    let size = [ATLAS[0] as f32, ATLAS[1] as f32];
    // The shader works in coordinates from 0 to 1 across the whole atlas: a texel is
    // not as wide there as it is high, in this atlas.
    let texel = [1.0 / size[0], 1.0 / size[1]];
    let (du, dv): ([f32; 2], [f32; 2]) = (
        std::array::from_fn(|i| du[i] * texel[i]),
        std::array::from_fn(|i| dv[i] * texel[i]),
    );
    let pixel: [f32; 2] = std::array::from_fn(|i| (du[i] * du[i] + dv[i] * dv[i]).sqrt());
    // Hard edges: a texel's centre, but for a band one pixel wide between two texels.
    let snapped: [f32; 2] = std::array::from_fn(|i| {
        let border = at[i].round() - 0.5;
        let along = ((at[i] - border - 0.5) * texel[i] / pixel[i] + 0.5).clamp(0.0, 1.0);
        border + along
    });
    let nearest = bilinear(texels, snapped);
    if !rgss {
        return nearest;
    }
    let texel_size = texel[0].min(texel[1]);
    let edge = ((pixel[0].max(pixel[1]) - texel_size) / texel_size).clamp(0.0, 1.0);
    let blend = edge * edge * (3.0 - 2.0 * edge);
    // Four samples a pixel. (The fixture's atlas has one level, so the two levels the
    // shader reads them at are the same.)
    let offsets = [
        [0.125, 0.375],
        [-0.125, -0.375],
        [0.375, -0.125],
        [-0.375, 0.125],
    ];
    let mut far = [0.0_f32; 4];
    for o in offsets {
        let tap = bilinear(texels, [at[0] + o[0], at[1] + o[1]]);
        for (sum, c) in far.iter_mut().zip(tap) {
            *sum += c;
        }
    }
    std::array::from_fn(|i| nearest[i] * (1.0 - blend) + far[i] * 0.25 * blend)
}

/// How much fog there is at a distance, as vanilla's linear fog works it out: the
/// shaders take it as a line (`fog_line`), which this checks them against.
fn fog_amount(distance: f32, range: [f32; 2]) -> f32 {
    if distance <= range[0] {
        0.0
    } else if distance >= range[1] {
        1.0
    } else {
        (distance - range[0]) / (range[1] - range[0])
    }
}

/// The scene as a small rasteriser draws it from the packed vertices, by the terrain
/// shaders' arithmetic: the lit colour, the two fog distances and the fade worked out
/// at each corner and carried across each of a quad's two triangles; the atlas's
/// colour as the case's shader filters it, times the lit colour; the fade towards the
/// fog's colour; the cutout layer's holes; the fog. What a frame of `case` must read,
/// pixel for pixel.
fn picture(scene: &Scene, case: Case) -> Vec<u8> {
    let fog = &FOGS[case.fog];
    let fogged = case.fogged();
    let rgss = case.rgss();
    let texels = atlas_texels();
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
    let mut next = 0;
    for (draw, &count) in scene.quads.iter().enumerate() {
        // The last two draws are the first section's cutout and translucent layers.
        let section = if draw < ORIGINS.len() { draw } else { 0 };
        let translucent = draw == ORIGINS.len() + 1;
        // Below how much alpha a fragment is left out.
        let threshold = match draw.checked_sub(ORIGINS.len()) {
            Some(0) => 0.5,
            Some(_) => 0.1,
            None => 0.0,
        };
        let origin = ORIGINS[section].map(|c| c as f32);
        let of_draw = &quads[next..next + count as usize];
        // The translucent quads in their indices' order, the others as they come.
        let in_order: Vec<&[Vertex; 4]> = if translucent {
            scene.order.iter().map(|&i| &of_draw[i as usize]).collect()
        } else {
            of_draw.iter().collect()
        };
        for q in in_order {
            let p: [[f32; 3]; 4] = q.map(|v| {
                let local = v.position.map(position_of);
                std::array::from_fn(|i| local[i] + origin[i])
            });
            // Counter-clockwise with y up: the area is positive.
            let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
            if area <= 0.0 {
                continue;
            }
            let (left, right) = (p[0][0].min(p[2][0]), p[0][0].max(p[2][0]));
            let (bottom, top) = (p[0][1].min(p[2][1]), p[0][1].max(p[2][1]));
            let z = 0.5 + p[0][2] / 32.0;
            // What the vertex stage hands on from each corner.
            let spherical = p.map(|c| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt());
            let cylindrical = p.map(|c| (c[0] * c[0] + c[2] * c[2]).sqrt().max(c[1].abs()));
            let fade = spherical.map(|d| {
                let beyond = ((d - 16.0) / 16.0).clamp(0.0, 1.0);
                (1.0 - beyond) + VISIBILITY[section] * beyond
            });
            let light = lit([q[0].block_light, q[0].sky_light]);
            let lit_colour: [f32; 4] =
                std::array::from_fn(|i| f32::from(q[0].colour[i]) / 255.0 * light[i]);
            // The texture coordinates in texels at corner 0, and how they change along
            // the quad's two sides and from one pixel to the next.
            let corner_uv = |i: usize| q[i].uv.map(|c| f32::from(c) / 8.0);
            let (uv0, uv1, uv3) = (corner_uv(0), corner_uv(1), corner_uv(3));
            let s_step = 1.0 / SCALE as f32 / (p[2][0] - p[0][0]);
            let t_step = -1.0 / SCALE as f32 / (p[2][1] - p[0][1]);
            let du: [f32; 2] = std::array::from_fn(|a| (uv3[a] - uv0[a]) * s_step);
            let dv: [f32; 2] = std::array::from_fn(|a| (uv1[a] - uv0[a]) * t_step);
            for py in 0..HEIGHT {
                for px in 0..WIDTH {
                    let (wx, wy) = (
                        (px as f32 + 0.5) / SCALE as f32,
                        BLOCKS[1] as f32 - (py as f32 + 0.5) / SCALE as f32,
                    );
                    if wx < left || wx >= right || wy < bottom || wy >= top {
                        continue;
                    }
                    // Where in the quad: corner 0 is (0, 0), corners 0 and 1 share
                    // s = 0, corners 0 and 3 share t = 0.
                    let (s, t) = (
                        (wx - p[0][0]) / (p[2][0] - p[0][0]),
                        (wy - p[0][1]) / (p[2][1] - p[0][1]),
                    );
                    // A corner's value at the pixel, in the triangle the pixel is in:
                    // corners 0, 1, 2 or corners 2, 3, 0.
                    let across = |c: [f32; 4]| {
                        if s <= t {
                            c[0] + (c[1] - c[0]) * t + (c[2] - c[1]) * s
                        } else {
                            c[0] + (c[3] - c[0]) * s + (c[2] - c[3]) * t
                        }
                    };
                    let uv: [f32; 2] = std::array::from_fn(|a| {
                        uv0[a] + (uv3[a] - uv0[a]) * s + (uv1[a] - uv0[a]) * t
                    });
                    let texel = sampled(&texels, uv, du, dv, rgss);
                    let mut c: [f32; 4] = std::array::from_fn(|i| texel[i] * lit_colour[i]);
                    if fogged {
                        // Towards the fog's colour, with the texel's own alpha.
                        let f = across(fade);
                        let alpha = c[3];
                        for (i, v) in c.iter_mut().enumerate() {
                            let from = fog.colour[i] * if i == 3 { alpha } else { 1.0 };
                            *v = from * (1.0 - f) + *v * f;
                        }
                    }
                    if c[3] < threshold {
                        continue;
                    }
                    if fogged {
                        let amount = fog_amount(across(spherical), fog.environmental)
                            .max(fog_amount(across(cylindrical), fog.render_distance))
                            * fog.colour[3];
                        for (v, f) in c.iter_mut().zip(fog.colour).take(3) {
                            *v = *v * (1.0 - amount) + f * amount;
                        }
                    }
                    let i = (py * WIDTH + px) as usize;
                    if z >= depth[i] {
                        depth[i] = z;
                        if translucent {
                            // Over what is there, by its own alpha.
                            let (under, a) = (colour[i].map(|v| f32::from(v) / 255.0), c[3]);
                            for (k, v) in c.iter_mut().enumerate() {
                                let src = if k == 3 { 1.0 } else { a };
                                *v = *v * src + under[k] * (1.0 - a);
                            }
                        }
                        colour[i] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                    }
                }
            }
        }
        next += count as usize;
    }
    colour.into_iter().flatten().collect()
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
    /// The translucent draw's own indices.
    sorted: RangeHandle,
    /// The three buffers' uploads, once staged.
    uploads: [Option<UploadTicket>; 3],
    atlas: TextureId,
    lightmap: TextureId,
    /// What each kind of frame must read ([`Case::picture`]), once one was drawn.
    pictures: Vec<Option<Vec<u8>>>,
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
        let sorted = alloc(&mut pool, scene.translucent_indices().len())?;
        let texels: Vec<u8> = atlas_texels().into_iter().flatten().collect();
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
            sorted,
            uploads: [None; 3],
            atlas,
            lightmap,
            pictures: vec![None; 2 * FOGS.len() + 1],
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
        let sorted = self.scene.translucent_indices();
        for (i, (handle, bytes)) in [
            (self.vertices, &self.scene.vertices),
            (self.indices, &indices),
            (self.sorted, &sorted),
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
            let case = Case::of(drawn);
            let scene = &self.scene;
            let expected =
                self.pictures[case.picture()].get_or_insert_with(|| picture(scene, case));
            let ok = compare(
                &self.scene.probes,
                expected,
                case,
                drawn,
                pixels,
                self.report.mismatches,
            );
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
        let (Some(vertices), Some(indices), Some(sorted)) = (
            self.pool.resolve(self.vertices),
            self.pool.resolve(self.indices),
            self.pool.resolve(self.sorted),
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
        let size = WORLD.offset("atlas_size");
        word(bytes, size, (ATLAS[0] as f32).to_ne_bytes());
        word(bytes, size + 1, (ATLAS[1] as f32).to_ne_bytes());
        word(
            bytes,
            WORLD.offset("lightmap"),
            self.lightmap.index().to_ne_bytes(),
        );
        // The frame's fog and shader: the fog goes with the push constants.
        let case = Case::of(frame);
        let fog = &FOGS[case.fog];
        let [solid, with_holes, seen_through] = case.shader.pipelines();
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
        let Some((sections, bytes)) = frame_data.alloc(ORIGINS.len() as u32 * SECTION.words(), 4)
        else {
            return Ok(());
        };
        for (record, (origin, visibility)) in bytes
            .as_chunks_mut::<SECTION_BYTES>()
            .0
            .iter_mut()
            .zip(ORIGINS.into_iter().zip(VISIBILITY))
        {
            write_section(
                record,
                [origin[0] + shift, origin[1], origin[2]],
                visibility,
            );
        }
        let push = terrain_push(
            world,
            matrix_at,
            sections,
            fog,
            self.atlas,
            [ATLAS[0] as f32, ATLAS[1] as f32],
        );
        let first = (vertices.offset / VERTEX_BYTES as u64) as u32;
        // The scene's draws: pipeline, instance, quads and first quad.
        let mut draws = Vec::with_capacity(self.scene.quads.len());
        let mut quad = 0;
        // The translucent layer's own draw: its quads and its first.
        let mut last = (0, 0);
        for (i, &quads) in self.scene.quads.iter().enumerate() {
            let (pipeline, instance) = match i {
                4 => (with_holes, 0),
                5 => {
                    last = (quads, quad);
                    quad += quads;
                    continue;
                }
                section => (solid, section as u32),
            };
            if quads > 0 {
                draws.push((pipeline, instance, quads, quad));
            }
            quad += quads;
        }
        // Its indices start where their range does, counted in indices.
        let first_sorted = (sorted.offset / 2) as u32;
        let last_vertex = (first + last.1 * 4) as i32;
        // Drawn indirectly, each pipeline's draws are commands in the frame's data,
        // after a gap that changes like the records'.
        let mut calls = Vec::new();
        if indirect {
            for pipeline in [solid, with_holes] {
                let commands: Vec<u8> = draws
                    .iter()
                    .filter(|d| d.0 == pipeline)
                    .flat_map(|&(_, instance, quads, quad)| {
                        draw_command(quads * 6, 0, (first + quad * 4) as i32, instance)
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
        let mut last_call = None;
        if indirect {
            if !gap(frame_data, 5) {
                return Ok(());
            }
            let Some((offset, bytes)) = frame_data.alloc_draw_commands(1) else {
                return Ok(());
            };
            bytes.copy_from_slice(&draw_command(last.0 * 6, first_sorted, last_vertex, 0));
            last_call = Some(offset);
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
            // Last, the translucent layer through its own indices.
            pass.bind_index_buffer_of(sorted.buffer, 0, false);
            match last_call {
                Some(offset) => pass.draw_indexed_indirect(
                    seen_through.index(),
                    constants(),
                    commands,
                    offset,
                    1,
                ),
                None => pass.draw_indexed(
                    seen_through.index(),
                    constants(),
                    last.0 * 6,
                    first_sorted,
                    last_vertex,
                    0,
                ),
            }
        }
        pass.finish();
        Ok(())
    }
}

/// Judges every probe's pixels in an RGBA image of the scene drawn as `case` against
/// `expected`, the rasteriser's picture of that case, and says so if one is wrong (in
/// detail for the first few mismatching frames). In fog a pixel may be a step further
/// off than its probe allows, and with averaged texels two: its colour went through
/// more arithmetic.
fn compare(
    probes: &[Probe],
    expected: &[u8],
    case: Case,
    frame: u64,
    pixels: &[u8],
    mismatches: u64,
) -> bool {
    let at = |image: &[u8], x: u32, y: u32| -> Rgba {
        let i = ((y * WIDTH + x) * 4) as usize;
        image[i..i + 4].try_into().expect("4 bytes")
    };
    // Averaged texels are read between their centres, where a GPU's weights are
    // coarser than the arithmetic here.
    let slack = u8::from(case.fogged() && case.fog != 0) + if case.rgss() { 2 } else { 0 };
    let mut ok = true;
    for p in probes {
        let [x0, y0, x1, y1] = p.rect;
        let tolerance = p.tolerance + slack;
        let wrong = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .find(|&(x, y)| {
                at(pixels, x, y)
                    .iter()
                    .zip(at(expected, x, y))
                    .any(|(&got, want)| got.abs_diff(want) > tolerance)
            });
        if let Some((x, y)) = wrong {
            if mismatches < PRINTED {
                eprintln!(
                    "[terrain-check] frame {frame} ({}, fog {}), {}: pixel ({x}, {y}) reads {:?}, must be {:?} within {tolerance}",
                    case.shader.name(),
                    case.fog,
                    p.what,
                    at(pixels, x, y),
                    at(expected, x, y),
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

    const HARD: Case = Case {
        shader: TerrainShader::Nearest,
        fog: 0,
    };

    fn pixel(image: &[u8], x: u32, y: u32) -> Rgba {
        let i = ((y * WIDTH + x) * 4) as usize;
        image[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_scene_is_what_its_probes_say() {
        let scene = Scene::build();
        assert_eq!(
            scene.quads.iter().sum::<u32>() as usize * 4 * VERTEX_BYTES,
            scene.vertices.len()
        );
        // Without fog the rasteriser's picture is the colours the probes name.
        let image = picture(&scene, HARD);
        for p in &scene.probes {
            let Some(want) = p.want else { continue };
            for y in p.rect[1]..p.rect[3] {
                for x in p.rect[0]..p.rect[2] {
                    let got = pixel(&image, x, y);
                    assert!(
                        got.iter()
                            .zip(want)
                            .all(|(g, w)| g.abs_diff(w) <= p.tolerance),
                        "{}: ({x}, {y}) is {got:?}, not {want:?}",
                        p.what
                    );
                }
            }
        }
        assert!(compare(&scene.probes, &image, HARD, 0, &image, 0));
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
        let good = picture(&scene, HARD);
        let fails = |change: &dyn Fn(&mut Vec<u8>)| {
            let mut image = good.clone();
            change(&mut image);
            !compare(&scene.probes, &good, HARD, 0, &image, PRINTED)
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
        // A frame drawn in one fog is not the picture of another, nor of none; and
        // the shader without fog draws the same whatever the fog.
        let pictures: Vec<Vec<u8>> = (0..FOGS.len())
            .map(|fog| picture(&scene, Case { fog, ..HARD }))
            .collect();
        for (a, drawn) in pictures.iter().enumerate() {
            for (b, expected) in pictures.iter().enumerate() {
                let case = Case { fog: b, ..HARD };
                assert_eq!(
                    compare(&scene.probes, expected, case, 0, drawn, PRINTED),
                    a == b,
                    "fog {a} judged as fog {b}"
                );
            }
        }
        let plain = |fog| {
            picture(
                &scene,
                Case {
                    shader: TerrainShader::Plain,
                    fog,
                },
            )
        };
        assert_eq!(plain(0), plain(3));
        assert_ne!(plain(0), pictures[0]);
    }

    #[test]
    fn the_rasteriser_fogs_and_fades_as_the_shaders_do() {
        let scene = Scene::build();
        let clear = picture(&scene, HARD);
        // The unlit grey block's top left pixel, in the fog of the place: its corners
        // are (0, 7), (0, 8), (1, 7), (1, 8) blocks from the camera, in its plane.
        let (x, y) = (0, 0);
        let (wx, wy) = (0.5 / SCALE as f32, 8.0 - 0.5 / SCALE as f32);
        // The pixel is in the triangle of the top left, bottom left and bottom right
        // corners: the top left's distance, then down and across.
        let (top_left, bottom_left, bottom_right) = (8.0_f32, 7.0, 50.0_f32.sqrt());
        let down = (8.0 - wy) / 1.0;
        let distance =
            top_left + (bottom_left - top_left) * down + (bottom_right - bottom_left) * wx;
        let amount = (distance - 3.0) / 16.0;
        let fog = FOGS[1].colour;
        let want: Vec<u8> = (0..3)
            .map(|i| {
                let grey = f32::from(GREY[i]) / 255.0;
                ((grey * (1.0 - amount) + fog[i] * amount) * 255.0).round() as u8
            })
            .collect();
        let fogged = picture(&scene, Case { fog: 1, ..HARD });
        assert_eq!(pixel(&fogged, x, y)[..3], want[..]);
        assert_eq!(pixel(&clear, x, y), GREY);
        // Half a fog is half as much of it.
        let half = picture(&scene, Case { fog: 4, ..HARD });
        let grey = f32::from(GREY[0]) / 255.0;
        let half_want = ((grey * (1.0 - amount * 0.5) + amount * 0.5) * 255.0).round() as u8;
        assert_eq!(pixel(&half, x, y)[0], half_want);
        // At the edge of the view the height counts where it is more than the
        // distance sideways: 8 blocks up at the top left corner, whatever x is.
        let edge = picture(&scene, Case { fog: 2, ..HARD });
        let amount = (top_left + (bottom_left - top_left) * down - 2.0) / 12.0;
        let fog = FOGS[2].colour;
        assert_eq!(
            pixel(&edge, x, y)[0],
            ((grey * (1.0 - amount) + fog[0] * amount) * 255.0).round() as u8
        );
        // The fading section: as it is up to 16 blocks out, on its way to the fog's
        // colour beyond, more so the further; its half opaque texel keeps its alpha.
        let fade = |cell_x: u32| pixel(&clear, cell_x * SCALE + 4, HEIGHT - 4);
        assert_eq!(fade(0), SOLID);
        let (middle, far) = (fade(5), fade(11));
        assert!(
            middle[2] > SOLID[2] && far[2] > middle[2],
            "{middle:?} {far:?}"
        );
        assert!(middle[1] < SOLID[1] && far[1] < middle[1]);
        assert_eq!(fade(8)[3], HALF[3]);
        // 19.2 blocks out, a fifth of the way to a quarter there: 0.85 of itself.
        let d = (11.5_f32 * 11.5 + 0.5 * 0.5 + 15.5 * 15.5).sqrt();
        let f = 1.0 - (d - 16.0) / 16.0 * 0.75;
        let want = (f32::from(SOLID[0]) / 255.0 * f + (1.0 - f)) * 255.0;
        assert!((f32::from(far[0]) - want).abs() <= 1.0, "{far:?} {want}");
        // Without fog and fade in the shader, it is as it is everywhere.
        let plain = picture(
            &scene,
            Case {
                shader: TerrainShader::Plain,
                fog: 1,
            },
        );
        assert_eq!(pixel(&plain, 11 * SCALE + 4, HEIGHT - 4), SOLID);
        assert_eq!(pixel(&plain, 0, 0), GREY);
    }

    #[test]
    fn averaged_texels_are_the_mean_of_four_samples() {
        let texels = atlas_texels();
        // A texel a pixel across and an eighth of one down, as most of the scene: by
        // vanilla's rule that is far in this atlas, where a texel is 512 times as high
        // as it is wide in the shader's coordinates.
        let (du, dv) = ([0.125, 0.0], [0.0, 0.125]);
        // In the middle of the even stretch every sample reads its colour.
        let even = sampled(&texels, [(EVEN_AT + 20) as f32 + 0.3, 5.4], du, dv, true);
        for (got, want) in even.iter().zip(EVEN) {
            assert!((got * 255.0 - f32::from(want)).abs() < 0.01);
        }
        // At the middle of the grey texel, with white to its left, red to its right
        // and poison below: the four samples lie an eighth and three eighths of a
        // texel from it, so each reads some of a neighbour.
        let grey = sampled(&texels, [1.5, 0.5], du, dv, true);
        let part = |x: f32, y: f32| -> [f32; 4] {
            // Towards the left neighbour for x below the middle, the right one above;
            // towards the row below for y above the middle (there is none above).
            let side = if x < 0.0 { WHITE } else { SOLID };
            let (fx, fy) = (x.abs(), y.max(0.0));
            std::array::from_fn(|i| {
                let row = f32::from(GREY[i]) * (1.0 - fx) + f32::from(side[i]) * fx;
                let below = f32::from(POISON[i]);
                (row * (1.0 - fy) + below * fy) / 255.0
            })
        };
        let taps = [
            part(0.125, 0.375),
            part(-0.125, -0.375),
            part(0.375, -0.125),
            part(-0.375, 0.125),
        ];
        for i in 0..4 {
            let mean = taps.iter().map(|t| t[i]).sum::<f32>() / 4.0;
            assert!((grey[i] - mean).abs() < 1e-5, "{grey:?} {mean}");
        }
        // With hard edges it is the grey itself, there and a pixel from its border.
        for x in [1.5, 1.0625, 1.9375] {
            let hard = sampled(&texels, [x, 0.5], du, dv, false);
            assert_eq!(hard.map(|c| (c * 255.0).round() as u8), GREY);
        }
        // A square atlas would count the same texel as near, and give the grey with
        // either filter; this one's shape is why the fixture cannot show that.
        let pixel_v = 0.125 / ATLAS[1] as f32;
        assert!(pixel_v > 2.0 / ATLAS[0] as f32);
    }

    #[test]
    fn every_shader_draws_in_every_fog() {
        let mut seen = std::collections::HashSet::new();
        for frame in 0..20 {
            let case = Case::of(frame);
            seen.insert((case.shader.name(), case.fog));
            assert!(case.picture() <= 2 * FOGS.len());
        }
        assert_eq!(seen.len(), TerrainShader::ALL.len() * FOGS.len());
        // The fogs' ranges lie inside the scene, so that each does something.
        for fog in &FOGS[1..] {
            for range in [fog.environmental, fog.render_distance] {
                assert!(range[0] == f32::MAX || (range[0] < range[1] && range[0] < 21.0));
            }
        }
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
