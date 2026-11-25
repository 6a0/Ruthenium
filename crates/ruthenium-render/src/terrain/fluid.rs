//! Fluids: the faces of the water or lava in a block, as 26.3's `FluidRenderer` makes
//! them.
//!
//! A fluid is not drawn from a model. Its top slopes between four corner heights, each
//! the mean of the fluid's height in the blocks that meet at the corner; its sides hang
//! from that top; a flowing top's texture is turned to the way the fluid flows. A face
//! against the same fluid is left out, as is one the block's own shape or the
//! neighbour's covers. Faces seen from inside the fluid are drawn too, as second quads
//! wound the other way. Every face has one light (the brighter of the block's and the
//! one above it) and one colour: its tint times the shade of the way it looks.
//!
//! The arithmetic is the game's, in single precision and in its order, so that a
//! corner's height is the same number here and there.

use ruthenium_assets::Id;
use ruthenium_assets::Layer;
use ruthenium_assets::atlas::{Atlas, Sprite, missing_id};
use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::ShapeData;
use ruthenium_sim::fluid::{Fluid, FluidState, normalize};

use super::table::{BlockTable, ShapeKind, StateInfo, sticks_out};

const DOWN: usize = 0;
const UP: usize = 1;
const NORTH: usize = 2;
const SOUTH: usize = 3;
const WEST: usize = 4;
const EAST: usize = 5;

/// `Direction.Plane.HORIZONTAL` in its order: ordinal and step.
const HORIZONTAL: [(usize, [i32; 3]); 4] = [
    (NORTH, [0, 0, -1]),
    (EAST, [1, 0, 0]),
    (SOUTH, [0, 0, 1]),
    (WEST, [-1, 0, 0]),
];

/// How high a source stands in its block (`MAX_FLUID_HEIGHT`).
const MAX_FLUID_HEIGHT: f32 = 0.888_888_9;
/// How far a face is drawn inside its block's boundary, and a top below its height.
const INSET: f32 = 0.001;

/// Where a sprite is in its atlas, in the atlas's coordinates (0 to 1), as the game
/// keeps it (`TextureAtlasSprite`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpriteUv {
    pub u0: f32,
    pub u1: f32,
    pub v0: f32,
    pub v1: f32,
}

impl SpriteUv {
    pub fn of(sprite: &Sprite, atlas_width: u32, atlas_height: u32) -> SpriteUv {
        let (w, h) = (atlas_width as f32, atlas_height as f32);
        SpriteUv {
            u0: (sprite.x + sprite.padding) as f32 / w,
            u1: (sprite.x + sprite.padding + sprite.width) as f32 / w,
            v0: (sprite.y + sprite.padding) as f32 / h,
            v1: (sprite.y + sprite.padding + sprite.height) as f32 / h,
        }
    }

    /// `getU`: 0 is the sprite's left edge, 1 its right.
    fn u(&self, offset: f32) -> f32 {
        let diff = self.u1 - self.u0;
        self.u0 + diff * offset
    }

    fn v(&self, offset: f32) -> f32 {
        let diff = self.v1 - self.v0;
        self.v0 + diff * offset
    }
}

/// What one fluid is drawn with (`FluidModel`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FluidModel {
    /// The layer all its faces go to: by the texels of all its sprites together.
    pub layer: Layer,
    pub still: SpriteUv,
    pub flowing: SpriteUv,
    /// What a side against glass or leaves is drawn with instead of `flowing`.
    pub overlay: Option<SpriteUv>,
    /// Its faces take the biomes' water colour.
    pub tinted: bool,
}

/// The two fluids' models (`FluidStateModelSet`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FluidModels {
    pub(crate) water: FluidModel,
    pub(crate) lava: FluidModel,
    /// The atlas's size in texels.
    pub(crate) atlas: [f32; 2],
}

impl FluidModels {
    /// The models as the block atlas has their sprites; a sprite it lacks is the
    /// missing one's.
    pub fn from_atlas(atlas: &Atlas) -> FluidModels {
        let sprite = |path: &str| -> Option<&Sprite> {
            Id::minecraft(path)
                .and_then(|id| atlas.sprites.get(&id))
                .or_else(|| atlas.sprites.get(&missing_id()))
        };
        let model = |still: &str, flowing: &str, overlay: Option<&str>, tinted: bool| {
            let sprites: [Option<&Sprite>; 3] =
                [Some(still), Some(flowing), overlay].map(|p| p.and_then(sprite));
            // `Transparency.or` over the sprites, then `ChunkSectionLayer.byTransparency`.
            let any = |f: fn(&Sprite) -> bool| sprites.iter().flatten().any(|s| f(s));
            let uv = |s: Option<&Sprite>| {
                s.map_or(
                    SpriteUv {
                        u0: 0.0,
                        u1: 0.0,
                        v0: 0.0,
                        v1: 0.0,
                    },
                    |s| SpriteUv::of(s, atlas.width, atlas.height),
                )
            };
            let layer = if any(|s| s.transparency.translucent) {
                Layer::Translucent
            } else if any(|s| s.transparency.transparent) {
                Layer::Cutout
            } else {
                Layer::Solid
            };
            FluidModel {
                layer,
                still: uv(sprites[0]),
                flowing: uv(sprites[1]),
                overlay: overlay.map(|_| uv(sprites[2])),
                tinted,
            }
        };
        FluidModels {
            water: model(
                "block/water_still",
                "block/water_flow",
                Some("block/water_overlay"),
                true,
            ),
            lava: model("block/lava_still", "block/lava_flow", None, false),
            atlas: [atlas.width as f32, atlas.height as f32],
        }
    }

    pub(crate) fn get(&self, fluid: Fluid) -> Option<&FluidModel> {
        match fluid {
            Fluid::Water | Fluid::FlowingWater => Some(&self.water),
            Fluid::Lava | Fluid::FlowingLava => Some(&self.lava),
            Fluid::Empty => None,
        }
    }
}

/// One corner of a fluid's face: its place in blocks from the section's corner, and its
/// texture coordinates in the atlas's (0 to 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FluidVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// One quad of a fluid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FluidQuad {
    pub vertices: [FluidVertex; 4],
    /// 0xAARRGGBB.
    pub colour: u32,
    /// Block and sky light, 0 to 15.
    pub light: (u8, u8),
}

fn fluid_of(s: &StateInfo) -> FluidState {
    FluidState::from_bits(s.fluid)
}

/// `ARGB.scaleRGB`: the colour's red, green and blue times `scale`, cut to whole numbers.
pub(crate) fn scale_rgb(colour: u32, scale: f32) -> u32 {
    let channel = |shift: u32| {
        let c = (colour >> shift & 0xFF) as f32 * scale;
        ((c as i32).clamp(0, 255) as u32) << shift
    };
    colour & 0xFF00_0000 | channel(16) | channel(8) | channel(0)
}

/// The fluid's box as a neighbour's face is held against it: the whole block wide and
/// `height` high, and through the whole block along the axis looked along (a slice
/// of a shape has no depth).
fn slice(direction: usize, height: f32) -> ShapeData {
    let top = if direction < 2 {
        1.0
    } else {
        f64::from(height)
    };
    ShapeData {
        coords: [vec![0.0, 1.0], vec![0.0, top], vec![0.0, 1.0]],
        full: vec![true],
    }
}

/// `FluidRenderer.isFaceOccludedByState`: whether the face of a fluid `height` high
/// towards `direction` is covered by the face `state` turns to it.
fn occluded(table: &BlockTable, direction: usize, height: f32, state: &StateInfo) -> bool {
    let occluder = usize::from(state.occlusion[direction ^ 1]);
    match table.kinds[occluder] {
        ShapeKind::Empty => false,
        ShapeKind::Full => direction != UP || height == 1.0,
        // `Shapes.blockOccludes`: a top only reaches what is above when the fluid fills
        // its block; then, as for any face, nothing of it may show beside the other.
        ShapeKind::Partial => {
            (direction != UP || (f64::from(height) - 1.0).abs() <= 1.0e-7)
                && !sticks_out(&slice(direction, height), &table.shapes[occluder])
        }
    }
}

/// `FluidRenderer.getHeight` of the fluid `fluid` at `p`, where `s` is: the whole block
/// under more of the same, the state's own height, nothing where something else is
/// that is not solid, and less than nothing where something solid is.
fn height(
    fluid: Fluid,
    p: [i32; 3],
    s: &StateInfo,
    state_at: &impl Fn([i32; 3]) -> StateInfo,
) -> f32 {
    let here = fluid_of(s);
    if fluid.is_same(here.fluid) {
        let above = state_at([p[0], p[1] + 1, p[2]]);
        if fluid.is_same(fluid_of(&above).fluid) {
            1.0
        } else {
            here.own_height()
        }
    } else if s.flags & StateInfo::SOLID == 0 {
        0.0
    } else {
        -1.0
    }
}

/// `addWeightedHeight`: a height of four fifths or more counts ten times.
fn weigh(sum: &mut [f32; 2], height: f32) {
    if height >= 0.8 {
        sum[0] += height * 10.0;
        sum[1] += 10.0;
    } else if height >= 0.0 {
        sum[0] += height;
        sum[1] += 1.0;
    }
}

/// `calculateAverageHeight`: the height at a corner, from the fluid's own block, the
/// two beside it there (`along_z` to the north or south, `along_x` to the east or west)
/// and the block diagonally across, which is only read where one of the two is open.
fn corner_height(
    fluid: Fluid,
    own: f32,
    along_z: f32,
    along_x: f32,
    corner: [i32; 3],
    state_at: &impl Fn([i32; 3]) -> StateInfo,
) -> f32 {
    if along_x >= 1.0 || along_z >= 1.0 {
        return 1.0;
    }
    let mut sum = [0.0_f32; 2];
    if along_x > 0.0 || along_z > 0.0 {
        let across = height(fluid, corner, &state_at(corner), state_at);
        if across >= 1.0 {
            return 1.0;
        }
        weigh(&mut sum, across);
    }
    weigh(&mut sum, own);
    weigh(&mut sum, along_x);
    weigh(&mut sum, along_z);
    sum[0] / sum[1]
}

/// `FlowingFluid.getFlow`: which way the fluid at `p` flows, normalised.
pub(crate) fn flow(
    p: [i32; 3],
    f: FluidState,
    state_at: &impl Fn([i32; 3]) -> StateInfo,
) -> [f64; 3] {
    let affects_flow = |n: FluidState| n.is_empty() || n.fluid.is_same(f.fluid);
    let (mut flow_x, mut flow_z) = (0.0_f64, 0.0_f64);
    for (_, step) in HORIZONTAL {
        let n = [p[0] + step[0], p[1], p[2] + step[2]];
        let state = state_at(n);
        let neighbour = fluid_of(&state);
        if !affects_flow(neighbour) {
            continue;
        }
        let mut neighbour_height = neighbour.own_height();
        let mut distance = 0.0_f32;
        if neighbour_height == 0.0 {
            if state.flags & StateInfo::BLOCKS_FLOW == 0 {
                let below = fluid_of(&state_at([n[0], n[1] - 1, n[2]]));
                if affects_flow(below) {
                    neighbour_height = below.own_height();
                    if neighbour_height > 0.0 {
                        distance = f.own_height() - (neighbour_height - MAX_FLUID_HEIGHT);
                    }
                }
            }
        } else if neighbour_height > 0.0 {
            distance = f.own_height() - neighbour_height;
        }
        if distance != 0.0 {
            flow_x += f64::from(step[0] as f32 * distance);
            flow_z += f64::from(step[2] as f32 * distance);
        }
    }
    let mut flow = [flow_x, 0.0, flow_z];
    if f.falling {
        // `isSolidFace`: a face that holds things up, of a block that is not ice and
        // has none of the same fluid.
        let solid_face = |q: [i32; 3], direction: usize| {
            let s = state_at(q);
            !fluid_of(&s).fluid.is_same(f.fluid)
                && s.flags & StateInfo::ICE == 0
                && s.sturdy >> direction & 1 != 0
        };
        for (direction, step) in HORIZONTAL {
            let n = [p[0] + step[0], p[1], p[2] + step[2]];
            if solid_face(n, direction) || solid_face([n[0], n[1] + 1, n[2]], direction) {
                let v = normalize(flow);
                flow = [v[0], v[1] - 6.0, v[2]];
                break;
            }
        }
    }
    normalize(flow)
}

/// `FluidState.shouldRenderBackwardUpFace`: whether a top is also drawn from below:
/// where any of the nine blocks over and around it is neither the same fluid nor
/// something nothing is seen through.
fn top_seen_from_below(
    fluid: Fluid,
    above: [i32; 3],
    state_at: &impl Fn([i32; 3]) -> StateInfo,
) -> bool {
    for ox in -1..=1 {
        for oz in -1..=1 {
            let s = state_at([above[0] + ox, above[1], above[2] + oz]);
            if !fluid_of(&s).fluid.is_same(fluid) && s.flags & StateInfo::SOLID_RENDER == 0 {
                return true;
            }
        }
    }
    false
}

/// `FluidRenderer.tesselate`: the quads of the fluid `fluid` in the block `block` at
/// `p`, in the game's order, each given to `out` with the layer it goes to.
///
/// Places are in blocks from the section's corner. `state_at` gives the block state at
/// a place, `light_at` the block and sky light there as `LightCoordsUtil.getLightCoords`
/// has them; both are asked within one block of `p`. `shades` is the dimension's
/// cardinal light and `tint` the colour a tinted fluid takes at `p` (0xAARRGGBB).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tesselate(
    table: &BlockTable,
    shades: &[f32; 6],
    p: [i32; 3],
    block: &StateInfo,
    fluid: FluidState,
    state_at: &impl Fn([i32; 3]) -> StateInfo,
    light_at: &impl Fn([i32; 3]) -> (u8, u8),
    tint: impl FnOnce() -> u32,
    out: &mut impl FnMut(Layer, &FluidQuad),
) {
    let kind = fluid.fluid;
    let at = |d: [i32; 3]| [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
    let down = state_at(at([0, -1, 0]));
    let up = state_at(at([0, 1, 0]));
    let north = state_at(at([0, 0, -1]));
    let south = state_at(at([0, 0, 1]));
    let west = state_at(at([-1, 0, 0]));
    let east = state_at(at([1, 0, 0]));
    let same = |s: &StateInfo| fluid_of(s).fluid.is_same(kind);
    // `shouldRenderFace`: not against the same fluid, and not where the block's own
    // shape covers the face.
    let shows = |direction: usize, neighbour: &StateInfo| {
        !same(neighbour) && !occluded(table, direction ^ 1, 1.0, block)
    };
    let render_up = !same(&up);
    let render_down = shows(DOWN, &down) && !occluded(table, DOWN, MAX_FLUID_HEIGHT, &down);
    let render_north = shows(NORTH, &north);
    let render_south = shows(SOUTH, &south);
    let render_west = shows(WEST, &west);
    let render_east = shows(EAST, &east);
    if !(render_up || render_down || render_east || render_west || render_north || render_south) {
        return;
    }
    let Some(model) = table.fluids.get(kind) else {
        return;
    };
    let layer = model.layer;
    let tint = if model.tinted { tint() } else { u32::MAX };

    let own = height(kind, p, block, state_at);
    let (mut north_east, mut north_west, mut south_east, mut south_west);
    if own >= 1.0 {
        north_east = 1.0;
        north_west = 1.0;
        south_east = 1.0;
        south_west = 1.0;
    } else {
        let h_north = height(kind, at([0, 0, -1]), &north, state_at);
        let h_south = height(kind, at([0, 0, 1]), &south, state_at);
        let h_east = height(kind, at([1, 0, 0]), &east, state_at);
        let h_west = height(kind, at([-1, 0, 0]), &west, state_at);
        north_east = corner_height(kind, own, h_north, h_east, at([1, 0, -1]), state_at);
        north_west = corner_height(kind, own, h_north, h_west, at([-1, 0, -1]), state_at);
        south_east = corner_height(kind, own, h_south, h_east, at([1, 0, 1]), state_at);
        south_west = corner_height(kind, own, h_south, h_west, at([-1, 0, 1]), state_at);
    }

    let (x, y, z) = ((p[0] & 15) as f32, (p[1] & 15) as f32, (p[2] & 15) as f32);
    let bottom = if render_down { INSET } else { 0.0 };
    let brighter = |a: (u8, u8), b: (u8, u8)| (a.0.max(b.0), a.1.max(b.1));
    let vertex = |x: f32, y: f32, z: f32, u: f32, v: f32| FluidVertex {
        position: [x, y, z],
        uv: [u, v],
    };
    let mut face = |v: [FluidVertex; 4], colour: u32, light: (u8, u8), back: bool| {
        out(
            layer,
            &FluidQuad {
                vertices: v,
                colour,
                light,
            },
        );
        if back {
            out(
                layer,
                &FluidQuad {
                    vertices: [v[0], v[3], v[2], v[1]],
                    colour,
                    light,
                },
            );
        }
    };

    let lowest = north_west.min(south_west).min(south_east.min(north_east));
    if render_up && !occluded(table, UP, lowest, &up) {
        north_west -= INSET;
        south_west -= INSET;
        south_east -= INSET;
        north_east -= INSET;
        let flow = flow(p, fluid, state_at);
        let (u00, u01, u10, u11, v00, v01, v10, v11);
        if flow[0] == 0.0 && flow[2] == 0.0 {
            let s = model.still;
            u00 = s.u0;
            v00 = s.v0;
            u01 = u00;
            v01 = s.v1;
            u10 = s.u1;
            v10 = v01;
            u11 = u10;
            v11 = v00;
        } else {
            let angle = mth::atan2(flow[2], flow[0]) as f32 - 1.570_796_4_f32;
            let s = mth::sin(f64::from(angle)) * 0.25;
            let c = mth::cos(f64::from(angle)) * 0.25;
            let f = model.flowing;
            u00 = f.u(0.5 + (-c - s));
            v00 = f.v(0.5 + (-c + s));
            u01 = f.u(0.5 + (-c + s));
            v01 = f.v(0.5 + (c + s));
            u10 = f.u(0.5 + (c + s));
            v10 = f.v(0.5 + (c - s));
            u11 = f.u(0.5 + (c - s));
            v11 = f.v(0.5 + (-c - s));
        }
        let light = brighter(light_at(p), light_at(at([0, 1, 0])));
        face(
            [
                vertex(x + 0.0, y + north_west, z + 0.0, u00, v00),
                vertex(x + 0.0, y + south_west, z + 1.0, u01, v01),
                vertex(x + 1.0, y + south_east, z + 1.0, u10, v10),
                vertex(x + 1.0, y + north_east, z + 0.0, u11, v11),
            ],
            scale_rgb(tint, shades[UP]),
            light,
            top_seen_from_below(kind, at([0, 1, 0]), state_at),
        );
    }

    if render_down {
        let s = model.still;
        let light = brighter(light_at(at([0, -1, 0])), light_at(p));
        face(
            [
                vertex(x, y + bottom, z, s.u0, s.v0),
                vertex(x + 1.0, y + bottom, z, s.u1, s.v0),
                vertex(x + 1.0, y + bottom, z + 1.0, s.u1, s.v1),
                vertex(x, y + bottom, z + 1.0, s.u0, s.v1),
            ],
            scale_rgb(tint, shades[DOWN]),
            light,
            false,
        );
    }

    let side_light = brighter(light_at(p), light_at(at([0, 1, 0])));
    for (direction, _) in HORIZONTAL {
        let (hh0, hh1, x0, z0, x1, z1, shown, beside) = match direction {
            NORTH => (
                north_west,
                north_east,
                x,
                z + INSET,
                x + 1.0,
                z + INSET,
                render_north,
                &north,
            ),
            SOUTH => (
                south_east,
                south_west,
                x + 1.0,
                z + 1.0 - INSET,
                x,
                z + 1.0 - INSET,
                render_south,
                &south,
            ),
            WEST => (
                south_west,
                north_west,
                x + INSET,
                z + 1.0,
                x + INSET,
                z,
                render_west,
                &west,
            ),
            _ => (
                north_east,
                south_east,
                x + 1.0 - INSET,
                z,
                x + 1.0 - INSET,
                z + 1.0,
                render_east,
                &east,
            ),
        };
        if !shown || occluded(table, direction, hh0.max(hh1), beside) {
            continue;
        }
        let (sprite, overlay) = match model.overlay {
            Some(o) if beside.flags & StateInfo::OVERLAY != 0 => (o, true),
            _ => (model.flowing, false),
        };
        let u0 = sprite.u(0.0);
        let u1 = sprite.u(0.5);
        let v01 = sprite.v((1.0 - hh0) * 0.5);
        let v02 = sprite.v((1.0 - hh1) * 0.5);
        let v1 = sprite.v(0.5);
        let shade = if direction == NORTH || direction == SOUTH {
            shades[NORTH]
        } else {
            shades[WEST]
        };
        face(
            [
                vertex(x0, y + hh0, z0, u0, v01),
                vertex(x1, y + hh1, z1, u1, v02),
                vertex(x1, y + bottom, z1, u1, v1),
                vertex(x0, y + bottom, z0, u0, v1),
            ],
            scale_rgb(tint, shades[UP] * shade),
            side_light,
            !overlay,
        );
    }
}

#[cfg(test)]
mod golden {
    use super::super::sort::Sorter;
    use super::super::testing;
    use super::*;
    use ruthenium_prepare::exporters::SkipRule;

    include!("fluid_golden.rs");

    /// The generator the golden file's cases were drawn from.
    struct Rng(u64);

    impl Rng {
        fn new() -> Rng {
            Rng(0x5275_7468_656e_6975)
        }

        fn next(&mut self) -> i32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as i32
        }

        fn below(&mut self, n: i32) -> i32 {
            self.next() % n
        }
    }

    fn fnv(hash: &mut u64, byte: u64) {
        *hash = (*hash ^ byte).wrapping_mul(0x0100_0000_01b3);
    }

    const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

    fn sprite(i: usize) -> SpriteUv {
        let [u0, u1, v0, v1] = SPRITES[i].map(f32::from_bits);
        SpriteUv { u0, u1, v0, v1 }
    }

    /// The golden file's kinds of block as a table: a state's id is its kind.
    pub(super) fn table() -> BlockTable {
        let states = PALETTE
            .iter()
            .map(|&(flags, fluid, sturdy, light_emission, faces)| {
                let mut f = 0;
                for (bit, flag) in [
                    (1, StateInfo::SOLID),
                    (2, StateInfo::SOLID_RENDER),
                    (4, StateInfo::OVERLAY),
                    (8, StateInfo::ICE),
                    (16, StateInfo::EMISSIVE),
                    (32, StateInfo::BLOCKS_FLOW),
                ] {
                    if flags & bit != 0 {
                        f |= flag;
                    }
                }
                StateInfo {
                    flags: f,
                    skip: SkipRule::None,
                    light_emission,
                    connects: 0,
                    block: 0,
                    occlusion: faces.map(u16::from),
                    seed_offset: [0; 3],
                    offset: 0,
                    tints: (0, 0),
                    fluid,
                    sturdy,
                }
            })
            .collect();
        let shapes = SHAPES
            .iter()
            .map(|(x, y, z, full)| ShapeData {
                coords: [x, y, z].map(|c| c.iter().map(|b| f64::from_bits(*b)).collect()),
                full: full.iter().map(|f| *f != 0).collect(),
            })
            .collect();
        let fluids = FluidModels {
            water: FluidModel {
                layer: Layer::Translucent,
                still: sprite(0),
                flowing: sprite(1),
                overlay: Some(sprite(2)),
                tinted: true,
            },
            lava: FluidModel {
                layer: Layer::Solid,
                still: sprite(3),
                flowing: sprite(4),
                overlay: None,
                tinted: false,
            },
            atlas: [1024.0, 512.0],
        };
        testing::bare_table(states, shapes, fluids)
    }

    /// A made-up arrangement: the kinds of the 27 blocks around and including the
    /// fluid's (index `(x + 1) + 3 * (y + 1) + 9 * (z + 1)`), their light, and the rest.
    pub(super) struct Case {
        pub kinds: [usize; 27],
        pub block: [u8; 27],
        pub sky: [u8; 27],
        pub centre: [i32; 3],
        pub nether: bool,
        pub tint: u32,
    }

    impl Case {
        fn draw(rng: &mut Rng, with_fluid: &[usize], waters: &[usize], lavas: &[usize]) -> Case {
            let palette = PALETTE.len() as i32;
            let centre = with_fluid[rng.below(with_fluid.len() as i32) as usize];
            let family = if PALETTE[centre].1 & 7 < 3 {
                waters
            } else {
                lavas
            };
            let mut kinds = [0; 27];
            for k in &mut kinds {
                let r = rng.below(100);
                *k = if r < 25 {
                    0
                } else if r < 35 {
                    1
                } else if r < 75 {
                    family[rng.below(family.len() as i32) as usize]
                } else {
                    rng.below(palette) as usize
                };
            }
            kinds[13] = centre;
            let (mut block, mut sky) = ([0; 27], [0; 27]);
            for i in 0..27 {
                let r = rng.below(4);
                block[i] = if r == 0 { 0 } else { rng.below(16) as u8 };
                sky[i] = if r < 2 { 0 } else { rng.below(16) as u8 };
            }
            let centre = [rng.below(48), rng.below(48), rng.below(48)];
            let nether = rng.below(4) == 0;
            let mut tint = 0xFF00_0000 | (rng.next() as u32 & 0xFF_FFFF);
            if rng.below(8) == 0 {
                tint = tint & 0xFF_FFFF | (rng.below(256) as u32) << 24;
            }
            Case {
                kinds,
                block,
                sky,
                centre,
                nether,
                tint,
            }
        }

        fn index(&self, p: [i32; 3]) -> usize {
            let d: [i32; 3] = std::array::from_fn(|i| p[i] - self.centre[i]);
            assert!(d.iter().all(|c| c.abs() <= 1), "read at {d:?}");
            ((d[0] + 1) + 3 * (d[1] + 1) + 9 * (d[2] + 1)) as usize
        }

        /// What the fluid in the middle is drawn as: per quad its layer, then for each
        /// corner five floats as bits, its colour and its packed light.
        pub fn tesselate(&self, table: &BlockTable) -> Vec<u32> {
            let state_at = |p: [i32; 3]| *table.state(self.kinds[self.index(p)] as u32);
            // `LightCoordsUtil.getLightCoords`.
            let light_at = |p: [i32; 3]| {
                let (i, s) = (self.index(p), state_at(p));
                if s.flags & StateInfo::EMISSIVE != 0 {
                    (15, 15)
                } else {
                    (self.block[i].max(s.light_emission), self.sky[i])
                }
            };
            let shades = SHADES[usize::from(self.nether)].map(f32::from_bits);
            let block = state_at(self.centre);
            let mut made = Vec::new();
            tesselate(
                table,
                &shades,
                self.centre,
                &block,
                FluidState::from_bits(block.fluid),
                &state_at,
                &light_at,
                || self.tint,
                &mut |layer, quad| {
                    made.push(match layer {
                        Layer::Solid => 0,
                        Layer::Cutout => 1,
                        Layer::Translucent => 2,
                    });
                    for v in &quad.vertices {
                        made.extend(v.position.map(f32::to_bits));
                        made.extend(v.uv.map(f32::to_bits));
                        made.push(quad.colour);
                        made.push(u32::from(quad.light.0) << 4 | u32::from(quad.light.1) << 20);
                    }
                },
            );
            made
        }
    }

    /// Every case in the generator's order.
    pub(super) fn cases() -> impl Iterator<Item = Case> {
        let kinds = |f: fn(u8) -> bool| -> Vec<usize> {
            (0..PALETTE.len())
                .filter(|k| f(PALETTE[*k].1 & 7))
                .collect()
        };
        let (with_fluid, waters, lavas) = (
            kinds(|f| f != 0),
            kinds(|f| f == 1 || f == 2),
            kinds(|f| f == 3 || f == 4),
        );
        let mut rng = Rng::new();
        (0..CASES).map(move |_| Case::draw(&mut rng, &with_fluid, &waters, &lavas))
    }

    /// The hashes of the batches of cases as this renderer makes them, and the first
    /// cases in full.
    pub(super) fn run(table: &BlockTable) -> (Vec<Vec<u32>>, Vec<u64>) {
        let (mut first, mut hashes) = (Vec::new(), Vec::new());
        let mut hash = FNV_START;
        for (c, case) in cases().enumerate() {
            let made = case.tesselate(table);
            fnv(&mut hash, made.len() as u64 & 0xFF);
            for v in &made {
                for b in v.to_le_bytes() {
                    fnv(&mut hash, u64::from(b));
                }
            }
            if c < FIRST.len() {
                first.push(made);
            }
            if c % BATCH == BATCH - 1 {
                hashes.push(hash);
                hash = FNV_START;
            }
        }
        (first, hashes)
    }

    #[test]
    fn the_games_own_fluid_renderer_makes_the_same_quads() {
        let (first, hashes) = run(&table());
        for (c, made) in first.iter().enumerate() {
            assert_eq!(made[..], *FIRST[c], "case {c}");
        }
        assert_eq!(hashes.len(), HASHES.len());
        for (b, hash) in hashes.iter().enumerate() {
            assert_eq!(
                *hash,
                HASHES[b],
                "cases {} to {}",
                b * BATCH,
                (b + 1) * BATCH - 1
            );
        }
    }

    #[test]
    fn the_cases_have_every_kind_of_face() {
        // What the comparison above is worth: the cases make tops with and without a
        // flow, undersides, overlays, faces seen from inside, and both layers.
        let table = table();
        let (mut quads, mut solid, mut translucent, mut sloped, mut none) = (0, 0, 0, 0, 0);
        for case in cases().take(4000) {
            let made = case.tesselate(&table);
            none += usize::from(made.is_empty());
            for q in made.as_chunks::<29>().0 {
                quads += 1;
                solid += usize::from(q[0] == 0);
                translucent += usize::from(q[0] == 2);
                let y = |v: usize| f32::from_bits(q[2 + 7 * v]);
                sloped += usize::from(y(0) != y(1) && y(1) != y(2) && y(0) != y(2));
            }
        }
        assert!(
            quads > 12_000 && solid > 3_000 && translucent > 8_000,
            "{quads} {solid}"
        );
        assert!(
            sloped > 1_000 && none > 100,
            "{sloped} sloped, {none} with nothing"
        );
    }

    #[test]
    fn the_games_own_sort_gives_the_same_order() {
        let mut rng = Rng::new();
        let mut sorter = Sorter::default();
        let mut hash = FNV_START;
        for _ in 0..SORTS {
            let n = 1 + rng.below(40);
            let grid = 1 + rng.below(8);
            let centres: Vec<[f32; 3]> = (0..n)
                .map(|_| {
                    let mut c = [0.0; 3];
                    for v in &mut c {
                        *v = rng.below(16 * grid + 1) as f32 / grid as f32;
                    }
                    c
                })
                .collect();
            let mut eye = [0.0; 3];
            for v in &mut eye {
                *v = (rng.below(4001) - 2000) as f32 / 50.0;
            }
            for &o in sorter.order(&centres, eye) {
                fnv(&mut hash, u64::from(o));
            }
        }
        assert_eq!(hash, SORT_HASH);
    }

    #[test]
    fn the_games_own_atan2_gives_the_same_angles() {
        let mut rng = Rng::new();
        let mut hash = FNV_START;
        for _ in 0..ANGLES {
            let mut y = f64::from(rng.below(2_000_001) - 1_000_000) / 1000.0;
            let mut x = f64::from(rng.below(2_000_001) - 1_000_000) / 1000.0;
            if rng.below(10) == 0 {
                x = 0.0;
            }
            if rng.below(10) == 0 {
                y = 0.0;
            }
            if x == 0.0 && y == 0.0 {
                x = 1.0;
            }
            for b in mth::atan2(y, x).to_bits().to_le_bytes() {
                fnv(&mut hash, u64::from(b));
            }
        }
        assert_eq!(hash, ANGLE_HASH);
    }
}
