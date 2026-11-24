//! The reference mesher: one 16 x 16 x 16 section's block models as packed vertices,
//! block by block and quad by quad, with nothing merged or reordered.
//!
//! It does what 26.3's section compiler does for blocks drawn from models, with
//! lighting as with Smooth Lighting off (`ModelBlockRenderer.tesselateFlat`): a face
//! whose neighbour hides it is left out (`Block.shouldRenderFace`); a face on the
//! block's boundary is lit by the block it looks into, any other by the block's own
//! place; every face is shaded by the direction it looks in (the dimension's cardinal
//! light) and multiplied by its tint. Quads go to the solid or the cutout layer by
//! their texture; translucent ones are counted and left out, as are fluids and blocks
//! that block entities draw, until their steps.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_assets::{BakedQuad, Layer, ModelPart};
use ruthenium_sim::blocks::{offset as block_offset, seed};
use ruthenium_sim::scene::{HALO_SIDE, Neighbourhood};

use super::table::{BlockTable, ShapeKind, StateInfo, sticks_out};
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
    /// Leaves have holes (the Fancy setting); off, they are opaque and hide each
    /// other's faces.
    pub cutout_leaves: bool,
}

/// One section's mesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectionMesh {
    /// Packed vertices, four per quad: the solid layer's quads, then the cutout
    /// layer's.
    pub vertices: Vec<u8>,
    pub solid_quads: u32,
    pub cutout_quads: u32,
    /// The least and the greatest corner of everything drawn, in blocks from the
    /// section's corner; none for an empty mesh.
    pub bounds: Option<[[f32; 3]; 2]>,
    /// Quads of the translucent layer, which are not drawn yet.
    pub translucent_quads: u32,
    /// Corners and texture coordinates outside what a vertex can hold (they were moved
    /// to the nearest value it can).
    pub out_of_range: u32,
    /// Which of the section's faces can be seen from which, through its blocks that
    /// are not solid.
    pub connectivity: Connectivity,
}

impl SectionMesh {
    pub fn quads(&self) -> u32 {
        self.solid_quads + self.cutout_quads
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

/// A quad's colour as red, green, blue, alpha: its shade as a grey (`ARGB.gray`),
/// times its tint.
fn colour(shade: f32, tint: Option<u32>) -> [u8; 4] {
    let grey = (shade * 255.0).floor() as u32 & 0xFF;
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

    /// Meshes the section `n` was pinned for.
    pub fn mesh(&mut self, n: &Neighbourhood, options: &MeshOptions) -> SectionMesh {
        n.fill_blocks(&mut self.blocks);
        let table = self.table.clone();
        let section = n.section();
        let origin = [section[0] << 4, section[1] << 4, section[2] << 4];
        let mut out = SectionMesh::default();
        let mut cutout = std::mem::take(&mut self.cutout);
        cutout.clear();
        let mut parts: Vec<&ModelPart> = Vec::with_capacity(8);
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        // The blocks nothing is seen through (`SectionCompiler.java:78`), in the planes'
        // order.
        let mut solid = [0u64; 64];

        // `LightCoordsUtil.getLightCoords` of the state at a place: block and sky light
        // there, the block light not below what the state gives off itself.
        let light_at = |state: &StateInfo, p: [i32; 3]| -> (u8, u8) {
            if state.flags & StateInfo::EMISSIVE != 0 {
                return (15, 15);
            }
            (
                n.block_light(p[0], p[1], p[2]).max(state.light_emission),
                n.sky_light(p[0], p[1], p[2]),
            )
        };

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
                    let mut emit = |q: &BakedQuad, light: (u8, u8)| {
                        let layer = match q.layer {
                            Layer::Solid => &mut out.vertices,
                            Layer::Cutout | Layer::Translucent if opaque_leaves => {
                                &mut out.vertices
                            }
                            Layer::Cutout => &mut cutout,
                            Layer::Translucent => {
                                out.translucent_quads += 1;
                                return;
                            }
                        };
                        // The element's own light raises both lights
                        // (`lightCoordsWithEmission`).
                        let e = q.light_emission;
                        let (block, sky) = (light.0.max(e), light.1.max(e));
                        let tint = usize::try_from(q.tint_index)
                            .ok()
                            .filter(|i| *i < usize::from(state.tints.1))
                            .map(|i| table.tints[state.tints.0 as usize + i]);
                        let shade =
                            options.shades[usize::from(q.shade_direction.unwrap_or(q.direction))];
                        let colour = colour(shade, tint);
                        for i in 0..4 {
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
                                block_light: block << 4,
                                sky_light: sky << 4,
                                colour,
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
                            let light = light_at(&state, beside);
                            for q in quads {
                                emit(q, light);
                            }
                        }
                        for q in part.unculled() {
                            let step =
                                if face_cubic(q, state.flags & StateInfo::COLLISION_FULL != 0) {
                                    STEPS[usize::from(q.direction)]
                                } else {
                                    [0; 3]
                                };
                            emit(q, light_at(&state, [x + step[0], y + step[1], z + step[2]]));
                        }
                    }
                }
            }
        }
        out.solid_quads = (out.vertices.len() / (4 * crate::terrain_vertex::VERTEX_BYTES)) as u32;
        out.cutout_quads = (cutout.len() / (4 * crate::terrain_vertex::VERTEX_BYTES)) as u32;
        out.vertices.extend_from_slice(&cutout);
        self.cutout = cutout;
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

    const OPTIONS: MeshOptions = MeshOptions {
        shades: [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
        cutout_leaves: true,
    };

    fn mesh(world: &mut World, section: [i32; 3]) -> SectionMesh {
        let n = world.pin(section);
        Mesher::new(testing::table()).mesh(&n, &OPTIONS)
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
        let west = Mesher::new(testing::table()).mesh(&n, &OPTIONS);
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
        let north = colour(0.8, Some(GRASS_TINT));
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
        assert_eq!(colour(1.0, Some(GRASS_TINT)), [0x40, 0x80, 0x20, 255]);
        assert_eq!(colour(0.5, None), [127, 127, 127, 255]);
        assert_eq!(colour(0.6, Some(u32::MAX)), [153, 153, 153, 255]);
    }

    #[test]
    fn translucent_quads_are_counted_and_left_out() {
        let mut world = World::new(&[(0, 0)]);
        world.set(3, 3, 3, testing::ice());
        world.set(4, 3, 3, testing::ice());
        let m = mesh(&mut world, [0, 0, 0]);
        assert_eq!(
            (m.solid_quads, m.cutout_quads, m.translucent_quads),
            (0, 0, 10)
        );
        assert!(m.vertices.is_empty());
        assert_eq!(m.bounds, None);
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
        let first = mesher.mesh(&n, &OPTIONS);
        assert!(first.quads() > 300);
        assert_eq!(mesher.mesh(&n, &OPTIONS), first);
        assert_eq!(Mesher::new(testing::table()).mesh(&n, &OPTIONS), first);
        // Opaque leaves and a nether's shades change it; nothing else does.
        let nether = MeshOptions {
            shades: [0.9, 0.9, 0.8, 0.8, 0.6, 0.6],
            ..OPTIONS
        };
        assert_ne!(mesher.mesh(&n, &nether), first);
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
