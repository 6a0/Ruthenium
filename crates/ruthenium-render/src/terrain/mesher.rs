//! The reference mesher: one 16 x 16 x 16 section's block models as packed vertices,
//! block by block and quad by quad, with nothing merged or reordered.
//!
//! It does what 26.3's section compiler does for blocks drawn from models. A face whose
//! neighbour hides it is left out (`Block.shouldRenderFace`). With Smooth Lighting on,
//! the quads of a block that gives off no light and whose model has ambient occlusion
//! are lit corner by corner from the blocks around ([`super::smooth`]). Any other quad is
//! lit flat (`ModelBlockRenderer.tesselateFlat`): on the block's boundary by the block
//! it looks into, else by the block's own place. Every face is shaded by the direction
//! it looks in (the dimension's cardinal light) and multiplied by its tint. Quads go to
//! the solid, the cutout or the translucent layer by their texture. A block's fluid is
//! meshed before its model ([`super::fluid`]), and goes to the layer its sprites say.
//! Every translucent quad also leaves the point it is sorted by ([`super::sort`]), in
//! the order the quads were made. Blocks that block entities draw are left out until
//! their step.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_assets::{BakedQuad, Layer, ModelPart};
use ruthenium_sim::blocks::{offset as block_offset, seed};
use ruthenium_sim::fluid::FluidState;
use ruthenium_sim::scene::{HALO_SIDE, Neighbourhood};

use super::fluid::{self, FluidQuad};
use super::smooth::{self, Cell, Face, Lit};
use super::sort;
use super::table::{BlockTable, ShapeKind, StateInfo, Tint, sticks_out};
use super::tint::{BiomeColour, BiomeTints, TintScratch};
use super::visibility::Connectivity;
use crate::terrain_vertex::{Vertex, position_code, position_of, uv_code};

/// The six directions' steps, by `Direction` ordinal: down, up, north, south, west,
/// east.
pub(crate) const STEPS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

const HALO: usize = HALO_SIDE * HALO_SIDE * HALO_SIDE;

/// What does not come from the level but changes what a mesh looks like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshOptions {
    /// How much light a face keeps by the direction it looks in
    /// (`CardinalLighting`), by direction ordinal.
    pub shades: [f32; 6],
    /// Smooth Lighting: corners are lit and darkened by the blocks around them
    /// (ambient occlusion). Off, every quad has one light and one shade.
    pub smooth_lighting: bool,
    /// Biome Blend: how many columns each way a biome colour is the mean over (0 to 7).
    pub biome_blend: u8,
    /// Leaves have holes (the Fancy setting); off, they are opaque and hide each
    /// other's faces.
    pub cutout_leaves: bool,
}

/// One section's mesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionMesh {
    /// Packed vertices, four per quad: the solid layer's quads, then the cutout
    /// layer's, then the translucent layer's.
    pub vertices: Vec<u8>,
    pub solid_quads: u32,
    pub cutout_quads: u32,
    /// The least and the greatest corner of everything drawn, in blocks from the
    /// section's corner; none for an empty mesh.
    pub bounds: Option<[[f32; 3]; 2]>,
    pub translucent_quads: u32,
    /// The point each translucent quad is sorted by, in the quads' order, in blocks
    /// from the section's corner.
    pub centres: Vec<[f32; 3]>,
    /// Corners and texture coordinates outside what a vertex can hold (they were moved
    /// to the nearest value it can).
    pub out_of_range: u32,
    /// Which of the section's faces can be seen from which, through its blocks that
    /// are not solid.
    pub connectivity: Connectivity,
}

impl SectionMesh {
    pub fn quads(&self) -> u32 {
        self.solid_quads + self.cutout_quads + self.translucent_quads
    }
}

/// Meshes sections with one table. It keeps its scratch memory between sections, so a
/// worker has one for as long as the table lives.
pub struct Mesher {
    table: Arc<BlockTable>,
    blocks: Box<[u16; HALO]>,
    /// Whether a face shape sticks out of the shape over it, by the two shapes.
    pairs: HashMap<(u16, u16), bool>,
    cutout: Vec<u8>,
    translucent: Vec<u8>,
    /// What smooth lighting has read of each block of the scratch so far, packed
    /// ([`pack_cell`]); 0: not read yet.
    cells: Vec<u16>,
    tints: TintScratch,
}

impl std::fmt::Debug for Mesher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mesher")
            .field("pairs", &self.pairs.len())
            .finish_non_exhaustive()
    }
}

/// `ARGB.multiply` of two 0xAARRGGBB colours: white leaves the other as it is.
fn multiply(a: u32, b: u32) -> u32 {
    if a == u32::MAX {
        return b;
    }
    if b == u32::MAX {
        return a;
    }
    let channel = |shift: u32| ((a >> shift & 0xFF) * (b >> shift & 0xFF) / 255) << shift;
    channel(24) | channel(16) | channel(8) | channel(0)
}

/// A shade as a grey (`ARGB.gray`).
fn grey(shade: f32) -> u8 {
    (shade * 255.0).floor() as u32 as u8
}

/// A vertex's colour as red, green, blue, alpha: its grey times the quad's tint.
fn colour(grey: u8, tint: Option<u32>) -> [u8; 4] {
    let grey = u32::from(grey);
    let mut c = 0xFF00_0000 | grey << 16 | grey << 8 | grey;
    if let Some(t) = tint {
        c = multiply(c, t);
    }
    [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]
}

/// Whether a quad that no neighbour culls is lit by the block it looks into: it lies
/// flat on the block's boundary on its side, or anywhere across a block whose
/// collision shape is the full block (`BlockModelLighter.prepareQuadShape`'s
/// `faceCubic`).
fn face_cubic(q: &BakedQuad, collision_full: bool) -> bool {
    let axis = [1, 1, 2, 2, 0, 0][usize::from(q.direction)];
    let (mut min, mut max) = (32.0_f32, -32.0_f32);
    for p in &q.positions {
        min = min.min(p[axis]);
        max = max.max(p[axis]);
    }
    let outward = q.direction & 1 == 1;
    min == max && (collision_full || if outward { max > 0.9999 } else { min < 1.0e-4 })
}

/// `LightCoordsUtil.getLightCoords` of a state at a place: block and sky light there,
/// the block light not below what the state gives off itself.
fn light_at(n: &Neighbourhood, state: &StateInfo, p: [i32; 3]) -> (u8, u8) {
    if state.flags & StateInfo::EMISSIVE != 0 {
        return (15, 15);
    }
    (
        n.block_light(p[0], p[1], p[2]).max(state.light_emission),
        n.sky_light(p[0], p[1], p[2]),
    )
}

const CELL_READ: u16 = 1 << 15;

fn pack_cell(c: Cell) -> u16 {
    CELL_READ | u16::from(c.flags) << 8 | u16::from(c.sky) << 4 | u16::from(c.block)
}

fn unpack_cell(c: u16) -> Cell {
    Cell {
        block: (c & 15) as u8,
        sky: (c >> 4 & 15) as u8,
        flags: (c >> 8 & 7) as u8,
    }
}

/// What smooth lighting reads of the block at a place in or around the section (up to
/// two blocks outside it), remembered for the places the scratch holds.
fn cell_at(
    n: &Neighbourhood,
    table: &BlockTable,
    blocks: &[u16; HALO],
    cells: &mut [u16],
    p: [i32; 3],
) -> Cell {
    let held = p.iter().all(|c| (-1..17).contains(c));
    let i = if held {
        halo_index(p[0], p[1], p[2])
    } else {
        0
    };
    if held && cells[i] != 0 {
        return unpack_cell(cells[i]);
    }
    let id = if held {
        blocks[i]
    } else {
        n.block(p[0], p[1], p[2])
    };
    let state = table.state(u32::from(id));
    let (block, sky) = light_at(n, state, p);
    let mut flags = 0;
    if state.flags & StateInfo::DARK != 0 {
        flags |= Cell::DARK;
    }
    if state.flags & StateInfo::PERMEABLE != 0 {
        flags |= Cell::PERMEABLE;
    }
    if state.flags & StateInfo::SOLID_RENDER != 0 {
        flags |= Cell::SOLID;
    }
    let cell = Cell { block, sky, flags };
    if held {
        cells[i] = pack_cell(cell);
    }
    cell
}

fn halo_index(x: i32, y: i32, z: i32) -> usize {
    ((x + 1) + HALO_SIDE as i32 * (z + 1) + (HALO_SIDE * HALO_SIDE) as i32 * (y + 1)) as usize
}

impl Mesher {
    pub fn new(table: Arc<BlockTable>) -> Mesher {
        Mesher {
            table,
            blocks: vec![0; HALO]
                .into_boxed_slice()
                .try_into()
                .expect("the size it was made with"),
            pairs: HashMap::new(),
            cutout: Vec::new(),
            translucent: Vec::new(),
            cells: Vec::new(),
            tints: TintScratch::default(),
        }
    }

    pub fn table(&self) -> &Arc<BlockTable> {
        &self.table
    }

    /// `Block.shouldRenderFace`: whether the face of `state` towards `direction` is
    /// drawn against `neighbour`.
    fn shows_face(
        &mut self,
        state: &StateInfo,
        neighbour: &StateInfo,
        direction: usize,
        cutout_leaves: bool,
    ) -> bool {
        let table = &*self.table;
        let occluder = neighbour.occlusion[direction ^ 1];
        let kind = table.kinds[usize::from(occluder)];
        if kind == ShapeKind::Full {
            return false;
        }
        if table.skips(state, neighbour, direction, cutout_leaves) {
            return false;
        }
        if kind == ShapeKind::Empty {
            return true;
        }
        let shape = state.occlusion[direction];
        if table.kinds[usize::from(shape)] == ShapeKind::Empty {
            return true;
        }
        *self.pairs.entry((shape, occluder)).or_insert_with(|| {
            sticks_out(
                &table.shapes[usize::from(shape)],
                &table.shapes[usize::from(occluder)],
            )
        })
    }

    /// Meshes the section `n` was pinned for, its tinted blocks in the `biomes`' colours.
    pub fn mesh(
        &mut self,
        n: &Neighbourhood,
        options: &MeshOptions,
        biomes: &BiomeTints,
    ) -> SectionMesh {
        n.fill_blocks(&mut self.blocks);
        let table = self.table.clone();
        let section = n.section();
        let origin = [section[0] << 4, section[1] << 4, section[2] << 4];
        let mut out = SectionMesh::default();
        let mut cutout = std::mem::take(&mut self.cutout);
        cutout.clear();
        let mut translucent = std::mem::take(&mut self.translucent);
        translucent.clear();
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        if options.smooth_lighting {
            cells.resize(HALO, 0);
        }
        let mut tints = std::mem::take(&mut self.tints);
        tints.clear();
        let mut parts: Vec<&ModelPart> = Vec::with_capacity(8);
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        // The blocks nothing is seen through (`SectionCompiler.java:78`), in the planes'
        // order.
        let mut solid = [0u64; 64];

        // The order of `BlockPos.betweenClosed`: x first, then y, then z.
        for z in 0..16_i32 {
            for y in 0..16_i32 {
                for x in 0..16_i32 {
                    let id = u32::from(self.blocks[halo_index(x, y, z)]);
                    if id == 0 {
                        continue;
                    }
                    let state = *table.state(id);
                    if state.flags & StateInfo::SOLID_RENDER != 0 {
                        let i = (x | z << 4 | y << 8) as usize;
                        solid[i >> 6] |= 1 << (i & 63);
                    }
                    // `SectionCompiler.compile`: the block's fluid, then its model.
                    let in_block = FluidState::from_bits(state.fluid);
                    if !in_block.is_empty() {
                        let blocks = &*self.blocks;
                        let state_at = |p: [i32; 3]| {
                            *table.state(u32::from(blocks[halo_index(p[0], p[1], p[2])]))
                        };
                        fluid::tesselate(
                            &table,
                            &options.shades,
                            [x, y, z],
                            &state,
                            in_block,
                            &state_at,
                            &|p| light_at(n, &state_at(p), p),
                            || {
                                tints
                                    .colour(
                                        n,
                                        biomes,
                                        options.biome_blend,
                                        BiomeColour::Water,
                                        [x, y, z],
                                    )
                                    .unwrap_or(u32::MAX)
                            },
                            &mut |layer, quad: &FluidQuad| {
                                let vertices = match layer {
                                    Layer::Solid => &mut out.vertices,
                                    Layer::Cutout => &mut cutout,
                                    Layer::Translucent => {
                                        out.centres.push(sort::centre(
                                            quad.vertices[0].position,
                                            quad.vertices[2].position,
                                        ));
                                        &mut translucent
                                    }
                                };
                                let c = quad.colour;
                                let colour =
                                    [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8];
                                for v in &quad.vertices {
                                    let mut position = [0u16; 3];
                                    for a in 0..3 {
                                        let p = v.position[a];
                                        position[a] = position_code(p).unwrap_or_else(|| {
                                            out.out_of_range += 1;
                                            if p < 0.0 { 0 } else { u16::MAX }
                                        });
                                        let p = position_of(position[a]);
                                        min[a] = min[a].min(p);
                                        max[a] = max[a].max(p);
                                    }
                                    let uv = [0, 1].map(|a| {
                                        let t = v.uv[a] * table.fluids.atlas[a];
                                        uv_code(t).unwrap_or_else(|| {
                                            out.out_of_range += 1;
                                            if t < 0.0 { 0 } else { u16::MAX }
                                        })
                                    });
                                    Vertex {
                                        position,
                                        block_light: quad.light.0 << 4,
                                        sky_light: quad.light.1 << 4,
                                        colour,
                                        uv,
                                    }
                                    .pack(vertices);
                                }
                            },
                        );
                    }
                    if state.flags & StateInfo::DRAWN == 0 {
                        continue;
                    }
                    let world = [origin[0] + x, origin[1] + y, origin[2] + z];
                    let s = state.seed_offset;
                    parts.clear();
                    table.models.parts(
                        id,
                        seed(
                            world[0] + i32::from(s[0]),
                            world[1] + i32::from(s[1]),
                            world[2] + i32::from(s[2]),
                        ),
                        &mut parts,
                    );
                    let mut base = [x as f32, y as f32, z as f32];
                    if state.offset != 0 {
                        let o = table.offsets[usize::from(state.offset)];
                        let d = block_offset(world, o.vertical, o.max_horizontal, o.max_vertical);
                        for a in 0..3 {
                            base[a] += d[a] as f32;
                        }
                    }
                    let opaque_leaves =
                        !options.cutout_leaves && state.flags & StateInfo::LEAVES != 0;
                    let collision_full = state.flags & StateInfo::COLLISION_FULL != 0;
                    // `ModelBlockRenderer.tesselateBlock`: by the first part's model.
                    let smooth = options.smooth_lighting
                        && state.light_emission == 0
                        && parts.first().is_some_and(|p| p.ambient_occlusion);
                    let shade_of = |q: &BakedQuad| {
                        options.shades[usize::from(q.shade_direction.unwrap_or(q.direction))]
                    };
                    let flat = |q: &BakedQuad, light: (u8, u8)| Lit {
                        grey: [grey(shade_of(q)); 4],
                        light: [[light.0 << 4, light.1 << 4]; 4],
                    };
                    let mut emit = |q: &BakedQuad, lit: Lit| {
                        let layer = match q.layer {
                            Layer::Solid => &mut out.vertices,
                            Layer::Cutout | Layer::Translucent if opaque_leaves => {
                                &mut out.vertices
                            }
                            Layer::Cutout => &mut cutout,
                            Layer::Translucent => {
                                let corner =
                                    |i: usize| std::array::from_fn(|a| base[a] + q.positions[i][a]);
                                out.centres.push(sort::centre(corner(0), corner(2)));
                                &mut translucent
                            }
                        };
                        let tint = usize::try_from(q.tint_index)
                            .ok()
                            .filter(|i| *i < usize::from(state.tints.1))
                            .map(|i| match table.tints[state.tints.0 as usize + i] {
                                Tint::Constant(c) => c,
                                Tint::Biome { kind, dy, hand } => tints
                                    .colour(
                                        n,
                                        biomes,
                                        options.biome_blend,
                                        kind,
                                        [x, y + i32::from(dy), z],
                                    )
                                    .unwrap_or(hand),
                            });
                        for i in 0..4 {
                            // The element's own light raises both lights.
                            let [block_light, sky_light] =
                                smooth::with_emission(lit.light[i], q.light_emission);
                            let mut position = [0u16; 3];
                            for a in 0..3 {
                                let p = base[a] + q.positions[i][a];
                                position[a] = position_code(p).unwrap_or_else(|| {
                                    out.out_of_range += 1;
                                    if p < 0.0 { 0 } else { u16::MAX }
                                });
                                let p = position_of(position[a]);
                                min[a] = min[a].min(p);
                                max[a] = max[a].max(p);
                            }
                            let uv = q.uvs[i].map(|t| {
                                uv_code(t).unwrap_or_else(|| {
                                    out.out_of_range += 1;
                                    if t < 0.0 { 0 } else { u16::MAX }
                                })
                            });
                            Vertex {
                                position,
                                block_light,
                                sky_light,
                                colour: colour(lit.grey[i], tint),
                                uv,
                            }
                            .pack(layer);
                        }
                    };
                    // Whether each face is drawn is worked out once for all the parts.
                    let (mut decided, mut shown) = (0u8, 0u8);
                    for part in &parts {
                        for (d, step) in STEPS.iter().enumerate() {
                            let quads = part.culled(d);
                            if quads.is_empty() {
                                continue;
                            }
                            let bit = 1 << d;
                            let beside = [x + step[0], y + step[1], z + step[2]];
                            if decided & bit == 0 {
                                decided |= bit;
                                let neighbour = *table.state(u32::from(
                                    self.blocks[halo_index(beside[0], beside[1], beside[2])],
                                ));
                                if self.shows_face(&state, &neighbour, d, options.cutout_leaves) {
                                    shown |= bit;
                                }
                            }
                            if shown & bit == 0 {
                                continue;
                            }
                            if smooth {
                                for q in quads {
                                    let face = Face::of(&q.positions, d, collision_full);
                                    let lit = smooth::smooth(&face, d, shade_of(q), |o| {
                                        let p = [x + o[0], y + o[1], z + o[2]];
                                        cell_at(n, &table, &self.blocks, &mut cells, p)
                                    });
                                    emit(q, lit);
                                }
                            } else {
                                let light = light_at(n, &state, beside);
                                for q in quads {
                                    emit(q, flat(q, light));
                                }
                            }
                        }
                        for q in part.unculled() {
                            let d = usize::from(q.direction);
                            if smooth {
                                let face = Face::of(&q.positions, d, collision_full);
                                let lit = smooth::smooth(&face, d, shade_of(q), |o| {
                                    let p = [x + o[0], y + o[1], z + o[2]];
                                    cell_at(n, &table, &self.blocks, &mut cells, p)
                                });
                                emit(q, lit);
                                continue;
                            }
                            let step = if face_cubic(q, collision_full) {
                                STEPS[d]
                            } else {
                                [0; 3]
                            };
                            let light =
                                light_at(n, &state, [x + step[0], y + step[1], z + step[2]]);
                            emit(q, flat(q, light));
                        }
                    }
                }
            }
        }
        out.solid_quads = (out.vertices.len() / (4 * crate::terrain_vertex::VERTEX_BYTES)) as u32;
        out.cutout_quads = (cutout.len() / (4 * crate::terrain_vertex::VERTEX_BYTES)) as u32;
        out.translucent_quads = out.centres.len() as u32;
        out.vertices.extend_from_slice(&cutout);
        out.vertices.extend_from_slice(&translucent);
        self.cutout = cutout;
        self.translucent = translucent;
        self.cells = cells;
        self.tints = tints;
        if out.quads() > 0 {
            out.bounds = Some([min, max]);
        }
        out.connectivity = Connectivity::of(&solid);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, GRASS_TINT, World};
    use super::*;
    use crate::terrain_vertex::VERTEX_BYTES;

    /// Flat lighting, which most of these tests are about.
    const OPTIONS: MeshOptions = MeshOptions {
        shades: [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
        smooth_lighting: false,
        biome_blend: 2,
        cutout_leaves: true,
    };

    fn mesh(world: &mut World, section: [i32; 3]) -> SectionMesh {
        let n = world.pin(section);
        Mesher::new(testing::table()).mesh(&n, &OPTIONS, &BiomeTints::default())
    }

    /// The mesh's quads: four vertices each.
    fn quads(m: &SectionMesh) -> Vec<[Vertex; 4]> {
        m.vertices
            .as_chunks::<VERTEX_BYTES>()
            .0
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| q.map(|v| Vertex::unpack(&v)))
            .collect()
    }

    /// A quad's corners in blocks from the section's corner, and the direction it
    /// faces if it lies in a plane across an axis.
    fn corners(q: &[Vertex; 4]) -> [[f32; 3]; 4] {
        q.map(|v| v.position.map(position_of))
    }

    fn facing(q: &[Vertex; 4]) -> Option<usize> {
        let c = corners(q);
        let (a, b) = (
            std::array::from_fn::<f32, 3, _>(|i| c[1][i] - c[0][i]),
            std::array::from_fn::<f32, 3, _>(|i| c[2][i] - c[0][i]),
        );
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        STEPS
            .iter()
            .position(|s| (0..3).all(|i| (n[i] > 0.0) == (s[i] > 0) && (n[i] < 0.0) == (s[i] < 0)))
    }

    fn faces(m: &SectionMesh) -> Vec<usize> {
        let mut f: Vec<usize> = quads(m).iter().filter_map(facing).collect();
        f.sort_unstable();
        f
    }

    #[test]
    fn a_lone_block_is_six_shaded_faces_lit_from_outside() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 6, 7, testing::stone());
        world.light(0, 0, 0, 13, 4);
        let m = mesh(&mut world, [0, 0, 0]);
        assert_eq!((m.solid_quads, m.cutout_quads), (6, 0));
        assert_eq!(m.vertices.len(), 6 * 4 * VERTEX_BYTES);
        assert_eq!(m.bounds, Some([[5.0, 6.0, 7.0], [6.0, 7.0, 8.0]]));
        assert_eq!((m.translucent_quads, m.out_of_range), (0, 0));
        let all = quads(&m);
        assert_eq!(faces(&m), vec![0, 1, 2, 3, 4, 5]);
        for q in &all {
            let d = facing(q).unwrap();
            // floor(shade * 255), as the game makes a grey of a shade.
            let grey = [127, 255, 204, 204, 153, 153][d];
            for v in q {
                assert_eq!(v.colour, [grey, grey, grey, 255]);
                assert_eq!((v.block_light, v.sky_light), (4 * 16, 13 * 16));
                // The opaque sprite at 16 texels, in eighths.
                assert!((128..=256).contains(&v.uv[0]) && v.uv[1] <= 128);
            }
            // Every corner is a corner of the block.
            for c in corners(q) {
                for (a, at) in [5.0, 6.0, 7.0].into_iter().enumerate() {
                    assert!(c[a] == at || c[a] == at + 1.0);
                }
            }
        }
        // The other sections of the chunk have nothing.
        assert_eq!(mesh(&mut world, [0, 1, 0]), SectionMesh::default());
    }

    #[test]
    fn faces_between_two_blocks_are_left_out_also_across_sections_and_chunks() {
        // In one section.
        let mut world = World::new(&[(0, 0), (1, 0)]);
        world.set(3, 3, 3, testing::stone());
        world.set(4, 3, 3, testing::stone());
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 10);

        // Across a section's top: the block at y 15 and the one at y 16 hide each
        // other's faces, in both sections' meshes.
        let mut world = World::new(&[(0, 0), (1, 0)]);
        world.set(8, 15, 8, testing::stone());
        world.set(8, 16, 8, testing::stone());
        assert_eq!(faces(&mesh(&mut world, [0, 0, 0])), vec![0, 2, 3, 4, 5]);
        assert_eq!(faces(&mesh(&mut world, [0, 1, 0])), vec![1, 2, 3, 4, 5]);

        // Across a chunk's east side.
        let mut world = World::new(&[(0, 0), (1, 0)]);
        world.set(15, 3, 3, testing::stone());
        world.set(16, 3, 3, testing::stone());
        assert_eq!(faces(&mesh(&mut world, [0, 0, 0])), vec![0, 1, 2, 3, 4]);
        assert_eq!(faces(&mesh(&mut world, [1, 0, 0])), vec![0, 1, 2, 3, 5]);
        // A corner of the shared edge has the same place in both meshes: 16 blocks
        // from one section's corner, none from the other's.
        let n = world.pin([0, 0, 0]);
        let west = Mesher::new(testing::table()).mesh(&n, &OPTIONS, &BiomeTints::default());
        assert_eq!(west.bounds.unwrap()[1][0], 16.0);
        let east = mesh(&mut world, [1, 0, 0]);
        assert_eq!(east.bounds.unwrap()[0][0], 0.0);

        // Without the neighbouring chunk the face shows (there is only air to see).
        let mut world = World::new(&[(0, 0)]);
        world.set(15, 3, 3, testing::stone());
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 6);
        // And is left out again once the chunk is there with its block: the pinned
        // section before is stale by its stamp.
        let before = world.pin([0, 0, 0]);
        world.load(1, 0);
        world.set(16, 3, 3, testing::stone());
        world.publish();
        assert!(!world.scene.is_current(before.stamp()));
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 5);
    }

    #[test]
    fn a_face_is_lit_by_the_block_it_looks_into() {
        let mut world = World::new(&[(0, 0)]);
        world.set(8, 15, 8, testing::stone());
        // The block's own section is dark; the one above is bright.
        world.light(0, 0, 0, 2, 1);
        world.light(0, 1, 0, 15, 9);
        let m = mesh(&mut world, [0, 0, 0]);
        for q in quads(&m) {
            let lights = (q[0].block_light / 16, q[0].sky_light / 16);
            match facing(&q).unwrap() {
                1 => assert_eq!(lights, (9, 15)),
                _ => assert_eq!(lights, (1, 2)),
            }
        }
    }

    const SMOOTH: MeshOptions = MeshOptions {
        smooth_lighting: true,
        ..OPTIONS
    };

    fn smooth_mesh(world: &mut World, section: [i32; 3]) -> SectionMesh {
        let n = world.pin(section);
        Mesher::new(testing::table()).mesh(&n, &SMOOTH, &BiomeTints::default())
    }

    /// The quad facing `direction` with a corner at each of the block's corners there.
    fn face_of(m: &SectionMesh, direction: usize) -> [Vertex; 4] {
        let found: Vec<_> = quads(m)
            .into_iter()
            .filter(|q| facing(q) == Some(direction))
            .collect();
        assert_eq!(found.len(), 1, "one face looks that way");
        found[0]
    }

    #[test]
    fn in_the_open_smooth_light_is_the_flat_light() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 6, 7, testing::stone());
        world.set(9, 6, 7, testing::slab());
        world.light(0, 0, 0, 13, 4);
        let flat = mesh(&mut world, [0, 0, 0]);
        let smooth = smooth_mesh(&mut world, [0, 0, 0]);
        // All but the faces that look into the blocks' own places, which are lit from
        // there: the slab's top lies inside its block.
        assert_eq!(flat.quads(), smooth.quads());
        for (f, s) in quads(&flat).iter().zip(quads(&smooth)) {
            for (f, s) in f.iter().zip(s) {
                assert_eq!((f.position, f.uv, f.colour), (s.position, s.uv, s.colour));
                assert_eq!((f.block_light, f.sky_light), (s.block_light, s.sky_light));
            }
        }
    }

    #[test]
    fn a_block_beside_a_face_darkens_the_corners_it_touches() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 6, 7, testing::stone());
        // Above and to the east of it.
        world.set(6, 7, 7, testing::stone());
        let m = smooth_mesh(&mut world, [0, 0, 0]);
        let top = quads(&m)
            .into_iter()
            .find(|q| facing(q) == Some(1) && corners(q).iter().all(|c| c[1] == 7.0))
            .unwrap();
        for (v, c) in top.iter().zip(corners(&top)) {
            // Three places at full brightness and the block at 0.2: 0.8 of the face's
            // shade. The western corners touch nothing.
            let grey = if c[0] == 6.0 { 204 } else { 255 };
            assert_eq!(v.colour, [grey, grey, grey, 255], "at {c:?}");
            // The block is dark inside, and counts as the lit place in front of the
            // face: no light is lost.
            assert_eq!((v.block_light, v.sky_light), (0, 240));
        }
        // Glass in the same place is full to walk on and darkens nothing.
        world.set(6, 7, 7, testing::glass());
        let m = smooth_mesh(&mut world, [0, 0, 0]);
        let top = quads(&m)
            .into_iter()
            .find(|q| facing(q) == Some(1) && corners(q).iter().all(|c| c[1] == 7.0))
            .unwrap();
        assert!(top.iter().all(|v| v.colour == [255; 4]));
    }

    #[test]
    fn a_diagonal_counts_only_where_light_gets_past_the_sides() {
        // The grey of the north-eastern corner of a block's top, with a block
        // diagonally beside the face there and `cover` above the two places next to it.
        let corner = |cover: u32| {
            let mut world = World::new(&[(0, 0)]);
            world.set(5, 5, 5, testing::stone());
            world.set(6, 6, 4, testing::stone());
            world.set(6, 7, 5, cover);
            world.set(5, 7, 4, cover);
            let m = smooth_mesh(&mut world, [0, 0, 0]);
            let top = quads(&m)
                .into_iter()
                .find(|q| {
                    let c = corners(q);
                    facing(q) == Some(1)
                        && c.iter().all(|c| c[1] == 6.0 && c[2] >= 5.0 && c[0] <= 6.0)
                })
                .unwrap();
            let at = corners(&top)
                .iter()
                .position(|c| *c == [6.0, 6.0, 5.0])
                .unwrap();
            top[at].colour[0]
        };
        // Under solid blocks the diagonal is not looked at: the corner is as bright
        // as its sides.
        assert_eq!(corner(testing::stone()), 255);
        // Glass lets light past, and the block in the diagonal darkens the corner.
        assert_eq!(corner(testing::glass()), 204);
        // So does a block that is solid to the eye and takes no light away.
        assert_eq!(corner(testing::glowstone()), 204);
    }

    #[test]
    fn what_is_two_blocks_beyond_a_sections_border_decides_as_inside_it() {
        // As above for the east face of a block on its chunk's border: what covers the
        // sides lies two blocks into the next chunk.
        let corner = |cover: u32| {
            let mut world = World::new(&[(0, 0), (1, 0)]);
            world.set(15, 5, 5, testing::stone());
            world.set(16, 6, 4, testing::stone());
            world.set(17, 6, 5, cover);
            world.set(17, 5, 4, cover);
            let m = smooth_mesh(&mut world, [0, 0, 0]);
            let east = face_of(&m, 5);
            let at = corners(&east)
                .iter()
                .position(|c| *c == [16.0, 6.0, 5.0])
                .unwrap();
            east[at].colour[0]
        };
        // 0.6 of 255, and of 204.
        assert_eq!(corner(testing::stone()), 153);
        assert_eq!(corner(testing::glass()), 122);
    }

    #[test]
    fn a_face_inside_its_block_is_lit_from_there_under_a_solid_block() {
        // A slab under the top of its section, which is lit 2 by blocks; the section
        // above is lit 9.
        let top = |above: u32| {
            let mut world = World::new(&[(0, 0)]);
            world.light(0, 0, 0, 15, 2);
            world.light(0, 1, 0, 15, 9);
            world.set(5, 15, 5, testing::slab());
            world.set(5, 16, 5, above);
            let m = smooth_mesh(&mut world, [0, 0, 0]);
            face_of(&m, 1).map(|v| v.block_light)
        };
        // Solid above: the face's own place counts, with the three beside it.
        assert_eq!(top(testing::stone()), [2 * 16; 4]);
        // Else the place above does.
        assert_eq!(top(testing::glass()), [(2 + 2 + 2 + 9) * 4; 4]);
    }

    #[test]
    fn a_block_that_gives_off_light_lights_the_corners_beside_it() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 5, 5, testing::stone());
        world.set(6, 6, 5, testing::glowstone());
        world.set(5, 9, 5, testing::stone());
        world.set(6, 10, 5, testing::magma());
        let m = smooth_mesh(&mut world, [0, 0, 0]);
        for (y, lights) in [(6.0, [15 * 4, 240]), (10.0, [15 * 4, 240])] {
            let top = quads(&m)
                .into_iter()
                .find(|q| {
                    let c = corners(q);
                    facing(q) == Some(1) && c.iter().all(|c| c[1] == y && c[0] <= 6.0)
                })
                .unwrap();
            for (v, c) in top.iter().zip(corners(&top)) {
                // One of a corner's four places gives off 15; none of the level's
                // light is from blocks.
                let want = if c[0] == 6.0 { lights } else { [0, 240] };
                assert_eq!([v.block_light, v.sky_light], want, "at {c:?}");
            }
        }
    }

    #[test]
    fn light_differs_from_corner_to_corner() {
        let mut world = World::new(&[(0, 0), (1, 0)]);
        // A block on the chunks' border: its top's western corners are in a chunk lit
        // 4 by blocks, its eastern ones between that and one lit 12.
        world.set(15, 6, 7, testing::stone());
        world.light(0, 0, 0, 15, 4);
        world.light(1, 0, 0, 15, 12);
        let m = smooth_mesh(&mut world, [0, 0, 0]);
        let top = face_of(&m, 1);
        for (v, c) in top.iter().zip(corners(&top)) {
            // Sixteenths of a level: the mean of four places.
            let block = if c[0] == 16.0 {
                (4 + 4 + 12 + 12) * 4
            } else {
                4 * 16
            };
            assert_eq!((v.block_light, v.sky_light), (block, 240), "at {c:?}");
        }
    }

    #[test]
    fn blocks_that_give_off_light_and_models_without_occlusion_stay_flat() {
        let mut world = World::new(&[(0, 0)]);
        world.light(0, 0, 0, 9, 2);
        world.set(1, 1, 1, testing::glowstone());
        world.set(2, 2, 1, testing::stone());
        world.set(8, 1, 8, testing::grass());
        world.set(9, 1, 8, testing::stone());
        let flat = mesh(&mut world, [0, 0, 0]);
        let smooth = smooth_mesh(&mut world, [0, 0, 0]);
        let stone = |q: &[Vertex; 4]| {
            let c = corners(q);
            c.iter().all(|c| c[0] >= 2.0 && c[1] >= 2.0) || c.iter().all(|c| c[0] >= 9.0)
        };
        let (flat, smooth) = (quads(&flat), quads(&smooth));
        assert_eq!(flat.len(), smooth.len());
        let mut same = 0;
        for (f, s) in flat.iter().zip(&smooth) {
            if !stone(f) {
                assert_eq!(f, s);
                same += 1;
            }
        }
        // The glowing cube's six faces and the plant's four.
        assert_eq!(same, 10);
        // The stones are lit smoothly, and the cube beside one darkens its corners.
        assert_ne!(flat, smooth);
    }

    #[test]
    fn smooth_light_reads_the_same_across_a_sections_border() {
        // A step of three blocks, once in the middle of a section and once across the
        // border of two chunks and of two sections: the block's lighting reads two
        // blocks beyond it, past what the mesher's scratch holds.
        let step = |world: &mut World, at: [i32; 3]| {
            world.set(at[0], at[1], at[2], testing::stone());
            world.set(at[0] + 1, at[1], at[2], testing::stone());
            world.set(at[0] + 2, at[1], at[2], testing::glass());
            world.set(at[0] + 2, at[1] + 1, at[2], testing::stone());
            world.set(at[0], at[1] + 2, at[2], testing::stone());
            world.set(at[0], at[1] + 1, at[2] + 1, testing::slab());
        };
        let lit = |m: &SectionMesh, from: [f32; 3]| {
            let mut v: Vec<_> = quads(m)
                .iter()
                .filter(|q| {
                    // The first block's faces only.
                    let c = corners(q);
                    (0..3).all(|a| c.iter().all(|c| c[a] >= from[a] && c[a] <= from[a] + 1.0))
                })
                .map(|q| (facing(q), q.map(|v| (v.colour, v.block_light, v.sky_light))))
                .collect();
            v.sort();
            v
        };
        let mut world = World::new(&[(0, 0), (1, 0)]);
        step(&mut world, [5, 5, 5]);
        let inside = lit(&smooth_mesh(&mut world, [0, 0, 0]), [5.0, 5.0, 5.0]);
        assert_eq!(inside.len(), 5);
        for at in [[15, 5, 5], [14, 5, 5], [5, 15, 5], [5, 14, 5], [5, 5, 15]] {
            let mut world = World::new(&[(0, 0), (1, 0), (0, 1), (1, 1)]);
            step(&mut world, at);
            let across = lit(&smooth_mesh(&mut world, [0, 0, 0]), at.map(|c| c as f32));
            assert_eq!(across, inside, "at {at:?}");
        }
    }

    #[test]
    fn a_mesher_keeps_nothing_of_the_last_sections_light() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 6, 7, testing::stone());
        world.set(6, 7, 7, testing::stone());
        world.set(5, 22, 7, testing::stone());
        world.light(0, 1, 0, 3, 9);
        let (below, above) = (world.pin([0, 0, 0]), world.pin([0, 1, 0]));
        let mut mesher = Mesher::new(testing::table());
        let first = mesher.mesh(&below, &SMOOTH, &BiomeTints::default());
        let other = mesher.mesh(&above, &SMOOTH, &BiomeTints::default());
        assert_eq!(mesher.mesh(&below, &SMOOTH, &BiomeTints::default()), first);
        assert_eq!(
            Mesher::new(testing::table()).mesh(&above, &SMOOTH, &BiomeTints::default()),
            other
        );
        assert_ne!(mesher.mesh(&below, &OPTIONS, &BiomeTints::default()), first);
    }

    #[test]
    fn light_a_block_gives_off_lights_its_own_faces() {
        let mut world = World::new(&[(0, 0)]);
        world.light(0, 0, 0, 3, 2);
        world.set(1, 1, 1, testing::glowstone());
        world.set(5, 1, 1, testing::magma());
        let m = mesh(&mut world, [0, 0, 0]);
        for q in quads(&m) {
            let lights = (q[0].block_light / 16, q[0].sky_light / 16);
            if corners(&q)[0][0] < 3.0 {
                // Its own light where the level has less; the sky's as it is.
                assert_eq!(lights, (15, 3));
            } else {
                // Drawn at full light.
                assert_eq!(lights, (15, 15));
            }
        }
    }

    #[test]
    fn blocks_that_hide_nothing_still_leave_out_faces_against_themselves() {
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::glass());
        world.set(4, 3, 3, testing::glass());
        world.set(5, 3, 3, testing::stone());
        let m = mesh(&mut world, [0, 0, 0]);
        // Glass has holes: the cutout layer. Two blocks less the two faces between
        // them and the one the stone hides.
        assert_eq!(m.cutout_quads, 9);
        // The stone shows its face towards the glass, which hides nothing.
        assert_eq!(m.solid_quads, 6);
        // Solid quads come first.
        let all = quads(&m);
        assert!(all[..6].iter().all(|q| corners(q)[0][0] >= 5.0));
        assert!(all[6..].iter().all(|q| corners(q)[0][0] <= 5.0));
    }

    #[test]
    fn a_partial_face_is_hidden_only_by_a_shape_that_covers_it() {
        // Two slabs side by side hide each other's sides.
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::slab());
        world.set(4, 3, 3, testing::slab());
        let m = mesh(&mut world, [0, 0, 0]);
        assert_eq!(m.solid_quads, 10);
        // A slab's top is half way up and never culled.
        assert!(
            quads(&m)
                .iter()
                .any(|q| corners(q).iter().all(|c| c[1] == 3.5))
        );

        // A slab beside a full block: the block hides the slab's side, the slab does
        // not hide the block's.
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::slab());
        world.set(4, 3, 3, testing::stone());
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 11);

        // A slab on a block: the block's top is hidden by the slab's full bottom, and
        // the slab's bottom by the block; a block on a slab shows both faces between
        // them (the slab's top hides nothing and is not culled).
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::stone());
        world.set(3, 4, 3, testing::slab());
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 10);
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::slab());
        world.set(3, 4, 3, testing::stone());
        assert_eq!(mesh(&mut world, [0, 0, 0]).solid_quads, 12);
    }

    #[test]
    fn a_plant_is_tinted_moved_by_its_position_and_lit_where_it_stands() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 0, 9, testing::grass());
        world.set(5, -1, 9, testing::stone());
        world.light(0, 0, 0, 11, 6);
        let m = mesh(&mut world, [0, 0, 0]);
        assert_eq!((m.solid_quads, m.cutout_quads), (0, 4));
        let d = block_offset([5, 0, 9], false, 0.25, 0.2);
        assert!(d[0] != 0.0 && d[2] != 0.0 && d[1] == 0.0);
        for q in quads(&m) {
            for v in q {
                // No shade is 1: the planes face north-east and the like, whose first
                // direction in order is north, south, west or east.
                let shade = u32::from(v.colour[0]) * 255 / (GRASS_TINT >> 16 & 0xFF);
                assert!(shade == 204 || shade == 153 || shade == 203 || shade == 152);
                assert_eq!(v.colour[3], 255);
                assert_eq!((v.block_light / 16, v.sky_light / 16), (6, 11));
            }
            // The planes cross at the block's middle, moved by the offset.
            let c = corners(&q);
            let mid = [(c[0][0] + c[2][0]) / 2.0, (c[0][2] + c[2][2]) / 2.0];
            assert!((mid[0] - (5.5 + d[0] as f32)).abs() < 1.0 / 1024.0);
            assert!((mid[1] - (9.5 + d[2] as f32)).abs() < 1.0 / 1024.0);
        }
        // Tint times shade, a channel at a time, as whole numbers.
        let north = colour(grey(0.8), Some(GRASS_TINT));
        assert_eq!(
            north,
            [
                0x40 * 204 / 255,
                0x80 * 204 / 255,
                0x20 * 204 / 255,
                255_u32
            ]
            .map(|c| c as u8)
        );
        assert_eq!(colour(grey(1.0), Some(GRASS_TINT)), [0x40, 0x80, 0x20, 255]);
        assert_eq!(colour(grey(0.5), None), [127, 127, 127, 255]);
        assert_eq!(colour(grey(0.6), Some(u32::MAX)), [153, 153, 153, 255]);
    }

    #[test]
    fn tinted_blocks_take_the_colour_of_the_biome_they_are_read_in() {
        use super::super::tint::tests::{tints, world};
        use ruthenium_sim::biome::zoomed_cell;

        let seed = 99;
        let mut world = world(seed, &[(0, 0)]);
        // Badlands up to y 7, a dark forest above.
        world.level.fill_biome([0, -16, 0], [15, 7, 15], 1);
        world.level.fill_biome([0, 8, 0], [15, 47, 15], 2);
        // A column where a block at y 6 is in the dark forest's cell, as the game picks
        // it. One at y 5 never is.
        let (x, z) = (2..14)
            .flat_map(|x| (2..14).map(move |z| (x, z)))
            .find(|&(x, z)| zoomed_cell(seed, [x, 6, z])[1] == 2)
            .expect("the jitter lifts some column's block into the upper cell");
        world.set(x, 5, z, testing::tall_grass(false));
        world.set(x, 6, z, testing::tall_grass(true));
        let n = world.pin([0, 0, 0]);
        let options = MeshOptions {
            biome_blend: 0,
            ..OPTIONS
        };
        let m = Mesher::new(testing::table()).mesh(&n, &options, &tints());
        assert_eq!(m.cutout_quads, 8);
        // Both halves in the badlands' grass colour, read at the lower half; times the
        // shade of a face that looks north or south, or west or east.
        let (north, west) = (
            colour(grey(0.8), Some(0xFF90_814D)),
            colour(grey(0.6), Some(0xFF90_814D)),
        );
        for q in quads(&m) {
            for v in q {
                assert!(v.colour == north || v.colour == west, "{:?}", v.colour);
            }
        }
        // With no biome's colours known, the colour the game gives outside a world.
        let m = Mesher::new(testing::table()).mesh(&n, &options, &BiomeTints::default());
        let plain = colour(grey(0.8), Some(GRASS_TINT));
        assert!(quads(&m).iter().flatten().any(|v| v.colour == plain));
    }

    #[test]
    fn translucent_quads_come_last_with_the_points_they_are_sorted_by() {
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::ice());
        world.set(4, 3, 3, testing::ice());
        world.set(8, 3, 3, testing::stone());
        world.set(9, 3, 3, testing::glass());
        let m = mesh(&mut world, [0, 0, 0]);
        // The faces between the two blocks of ice are left out; the stone's face
        // towards the glass is not.
        assert_eq!(
            (m.solid_quads, m.cutout_quads, m.translucent_quads),
            (6, 5, 10)
        );
        assert_eq!(m.quads(), 21);
        assert_eq!(m.vertices.len(), 21 * 4 * VERTEX_BYTES);
        assert_eq!(m.bounds, Some([[3.0, 3.0, 3.0], [10.0, 4.0, 4.0]]));
        let all = quads(&m);
        let ice = &all[11..];
        assert!(ice.iter().all(|q| (3.0..=5.0).contains(&corners(q)[0][0])));
        // A point for each, in their order: half-way between a quad's first and third
        // corner, which for a block's face is its middle.
        assert_eq!(m.centres.len(), 10);
        for (q, centre) in ice.iter().zip(&m.centres) {
            let c = corners(q);
            assert_eq!(*centre, sort::centre(c[0], c[2]));
            let d = facing(q).unwrap();
            let block = if c.iter().all(|p| p[0] >= 4.0) {
                4.0
            } else {
                3.0
            };
            let middle: [f32; 3] =
                std::array::from_fn(|a| [block, 3.0, 3.0][a] + 0.5 + 0.5 * STEPS[d][a] as f32);
            assert_eq!(*centre, middle);
        }
        // The first block's faces before the second's, in the directions' order.
        assert_eq!(
            ice.iter().map(|q| facing(q).unwrap()).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 0, 1, 2, 3, 5]
        );
    }

    /// The translucent quads of a mesh.
    fn translucent(m: &SectionMesh) -> Vec<[Vertex; 4]> {
        quads(m).split_off((m.solid_quads + m.cutout_quads) as usize)
    }

    #[test]
    fn a_lone_source_of_water_slopes_down_all_round_and_is_seen_from_inside() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 5, 7, testing::stone());
        world.set(5, 6, 7, testing::water(0));
        world.light(0, 0, 0, 13, 4);
        let m = mesh(&mut world, [0, 0, 0]);
        // The stone, then the water: its top and four sides, each with its other
        // side; the stone covers its underside.
        assert_eq!(
            (m.solid_quads, m.cutout_quads, m.translucent_quads),
            (6, 0, 10)
        );
        let water = translucent(&m);
        // Every corner is the mean of the source (counted ten times) and the two open
        // blocks beside it there, less a thousandth: 80/9 over 12, in 2048ths. (The
        // block across the corner only counts where one of the two has fluid.)
        let top = corners(&water[0]);
        let height = (8.0_f32 / 9.0 * 10.0) / 12.0 - 0.001;
        for c in top {
            assert_eq!(c[1], position_of(position_code(6.0 + height).unwrap()));
        }
        assert_eq!(facing(&water[0]), Some(1));
        // The same corners the other way round.
        let below = corners(&water[1]);
        assert_eq!(
            [below[0], below[1], below[2], below[3]],
            [top[0], top[3], top[2], top[1]]
        );
        assert_eq!(facing(&water[1]), Some(0));
        // North, east, south, west, each a thousandth inside the block.
        assert_eq!(
            water[2..]
                .iter()
                .map(|q| facing(q).unwrap())
                .collect::<Vec<_>>(),
            [2, 3, 5, 4, 3, 2, 4, 5]
        );
        let inset = position_of(position_code(7.001).unwrap());
        assert!(
            corners(&water[2])
                .iter()
                .all(|c| c[2] == inset && c[2] > 7.0)
        );
        assert_eq!(m.centres.len(), 10);
        assert_eq!(m.centres[0], m.centres[1]);
        for q in &water {
            for v in q {
                // No biome's colour is known: white, shaded by the way the face looks.
                assert_eq!((v.block_light, v.sky_light), (4 << 4, 13 << 4));
                assert_eq!(v.colour[3], 255);
            }
        }
        assert_eq!(water[0][0].colour, [255, 255, 255, 255]);
        assert_eq!(water[2][0].colour, [204, 204, 204, 255]);
        assert_eq!(water[4][0].colour, [153, 153, 153, 255]);
        // The still sprite on top, the upper left quarter of the flowing one on a
        // side, from as far down it as the side is short of a block.
        let texels = |q: &[Vertex; 4]| q.map(|v| v.uv.map(|c| f32::from(c) / 8.0));
        assert_eq!(
            texels(&water[0]),
            [[64.0, 0.0], [64.0, 16.0], [80.0, 16.0], [80.0, 0.0]]
        );
        let side = texels(&water[2]);
        assert_eq!(
            (side[0][0], side[1][0], side[2], side[3]),
            (80.0, 88.0, [88.0, 8.0], [80.0, 8.0])
        );
        assert!((side[0][1] - (1.0 - height) * 8.0).abs() <= 1.0 / 16.0 + 1e-4);
    }

    #[test]
    fn water_in_a_pool_is_only_its_top() {
        let mut world = World::new(&[(0, 0)]);
        for x in 4..7 {
            for z in 4..7 {
                world.set(x, 2, z, testing::stone());
                world.set(x, 3, z, testing::water(0));
            }
        }
        let m = mesh(&mut world, [0, 0, 0]);
        let water = translucent(&m);
        // The middle block: its top and the top's underside, level at eight ninths.
        let middle: Vec<_> = water
            .iter()
            .filter(|q| {
                corners(q)
                    .iter()
                    .all(|c| (5.0..=6.0).contains(&c[0]) && (5.0..=6.0).contains(&c[2]))
            })
            .collect();
        assert_eq!(middle.len(), 2);
        let level = position_of(position_code(3.0 + 8.0_f32 / 9.0 - 0.001).unwrap());
        assert!(corners(middle[0]).iter().all(|c| c[1] == level));
        // Under more water a block fills its space and has no top at all.
        world.set(5, 4, 5, testing::water(0));
        let m = mesh(&mut world, [0, 0, 0]);
        let water = translucent(&m);
        let lower_tops = water
            .iter()
            .filter(|q| {
                let c = corners(q);
                facing(q) == Some(1)
                    && c.iter().all(|p| {
                        p[1] < 4.0 && (5.0..=6.0).contains(&p[0]) && (5.0..=6.0).contains(&p[2])
                    })
            })
            .count();
        assert_eq!(lower_tops, 0);
        // Its neighbours' corners there rise to the full block.
        assert!(water.iter().any(|q| corners(q).iter().any(|c| c[1] == 4.0)));
    }

    #[test]
    fn lava_is_solid_and_as_bright_as_it_glows() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 5, 7, testing::stone());
        world.set(5, 6, 7, testing::lava(0));
        world.light(0, 0, 0, 0, 2);
        let m = mesh(&mut world, [0, 0, 0]);
        assert_eq!(
            (m.solid_quads, m.cutout_quads, m.translucent_quads),
            (16, 0, 0)
        );
        assert!(m.centres.is_empty());
        let all = quads(&m);
        // The stone's top is hidden by nothing (lava covers no face) and lit by the
        // lava's own light; the lava itself is lit by it too.
        for q in &all[6..] {
            assert!(q.iter().all(|v| (v.block_light, v.sky_light) == (240, 0)));
            assert_eq!(q[0].colour[3], 255);
        }
        // Lava's sprites: still at 112, flowing at 128.
        assert_eq!(all[6][0].uv, [112 * 8, 0]);
    }

    #[test]
    fn a_block_with_water_in_it_is_its_water_and_then_its_model() {
        let mut world = World::new(&[(0, 0)]);
        world.set(5, 6, 7, testing::wet_slab());
        let m = mesh(&mut world, [0, 0, 0]);
        // The slab's own six faces; the water's top and sides with their other sides,
        // and no underside: the slab's own bottom face covers it.
        assert_eq!(
            (m.solid_quads, m.cutout_quads, m.translucent_quads),
            (6, 0, 10)
        );
        assert!(translucent(&m).iter().all(|q| {
            let c = corners(q);
            // Nothing of the water reaches below the block's floor.
            facing(q) != Some(0) || c.iter().any(|p| p[1] > 6.5)
        }));
        // Beside stone the side towards it is still drawn (only the same fluid and
        // the block's own shape hide a side), but a full block of water above hides
        // the top.
        world.set(5, 7, 7, testing::water(0));
        let m = mesh(&mut world, [0, 0, 0]);
        let wet: Vec<_> = translucent(&m)
            .into_iter()
            .filter(|q| corners(q).iter().all(|c| c[1] <= 7.0))
            .collect();
        assert_eq!(wet.len(), 8);
        assert!(wet.iter().all(|q| facing(q).is_some_and(|d| d >= 2)));
        // Under the same fluid the block is full to its top.
        assert!(wet.iter().any(|q| corners(q).iter().any(|c| c[1] == 7.0)));
    }

    #[test]
    fn flowing_water_turns_its_texture_the_way_it_flows() {
        let mut world = World::new(&[(0, 0)]);
        for x in 3..9 {
            world.set(x, 2, 5, testing::stone());
            world.set(x, 3, 4, testing::stone());
            world.set(x, 3, 6, testing::stone());
        }
        // Lower and lower towards the east.
        for (x, level) in [(4, 1), (5, 2), (6, 3)] {
            world.set(x, 3, 5, testing::water(level));
        }
        let m = mesh(&mut world, [0, 0, 0]);
        let water = translucent(&m);
        let top = water
            .iter()
            .find(|q| facing(q).is_none() && corners(q).iter().all(|c| (5.0..=6.0).contains(&c[0])))
            .expect("the middle block's sloping top");
        // The top slopes down to the east.
        let c = corners(top);
        assert!(c[0][1] > c[3][1] && c[1][1] > c[2][1]);
        // The flowing sprite (80 to 96), its middle half, turned a quarter so that its
        // rows run along the flow: the north-west corner has the sprite's upper right
        // quarter point, the south-west one its upper left.
        let uv = top.map(|v| v.uv.map(|c| f32::from(c) / 8.0));
        assert_eq!(uv, [[92.0, 4.0], [84.0, 4.0], [84.0, 12.0], [92.0, 12.0]]);
    }

    #[test]
    fn a_mesher_gives_the_same_mesh_again() {
        let mut world = World::new(&[(0, 0)]);
        for (i, s) in [
            testing::stone(),
            testing::slab(),
            testing::glass(),
            testing::grass(),
            testing::glowstone(),
        ]
        .into_iter()
        .enumerate()
        {
            for k in 0..40 {
                let p = (i as i32 * 37 + k * 11) % 4096;
                world.set(p & 15, p >> 4 & 15, p >> 8, s);
            }
        }
        let n = world.pin([0, 0, 0]);
        let mut mesher = Mesher::new(testing::table());
        let first = mesher.mesh(&n, &OPTIONS, &BiomeTints::default());
        assert!(first.quads() > 300);
        assert_eq!(mesher.mesh(&n, &OPTIONS, &BiomeTints::default()), first);
        assert_eq!(
            Mesher::new(testing::table()).mesh(&n, &OPTIONS, &BiomeTints::default()),
            first
        );
        // Opaque leaves and a nether's shades change it; nothing else does.
        let nether = MeshOptions {
            shades: [0.9, 0.9, 0.8, 0.8, 0.6, 0.6],
            ..OPTIONS
        };
        assert_ne!(mesher.mesh(&n, &nether, &BiomeTints::default()), first);
    }

    #[test]
    fn a_mesh_says_which_faces_its_solid_blocks_leave_connected() {
        // A stone floor across the section: above it and below it are apart.
        let mut world = World::new(&[(0, 0)]);
        for z in 0..16 {
            for x in 0..16 {
                world.set(x, 8, z, testing::stone());
            }
        }
        let m = mesh(&mut world, [0, 0, 0]);
        let c = m.connectivity;
        assert!(!c.sees(0, 1) && !c.sees(1, 0));
        assert!(c.sees(0, 2) && c.sees(1, 2) && c.sees(4, 5));
        // One block of it missing, and one that is no solid block for this: glass.
        world.set(3, 8, 3, testing::glass());
        assert_eq!(mesh(&mut world, [0, 0, 0]).connectivity, Connectivity::ALL);
        world.set(3, 8, 3, testing::stone());
        assert_eq!(mesh(&mut world, [0, 0, 0]).connectivity, c);
        // A section of nothing but stone, and one of air.
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    world.set(x, y, z, testing::stone());
                }
            }
        }
        assert_eq!(mesh(&mut world, [0, 0, 0]).connectivity, Connectivity::NONE);
        assert_eq!(mesh(&mut world, [0, 1, 0]).connectivity, Connectivity::ALL);
    }
}
