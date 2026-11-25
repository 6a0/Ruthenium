//! What the mesher knows of every block state: its baked model, and what the exporters
//! say of it that the model files don't (which faces it hides, which it leaves out by a
//! rule of its block, its light, its tints, where its model's variants are seeded
//! from).

use std::path::Path;

use super::fluid::FluidModels;
use super::tint::BiomeColour;
use crate::block_entity::place::Kind;
use ruthenium_assets::BlockModels;
use ruthenium_data::v26_3::blocks::STATE_COUNT;
use ruthenium_data::v26_3::{Block, BlockState};
use ruthenium_prepare::exporters::{
    self, BlockBehaviours, BlockPhysics, BlockRender, ShapeData, SkipRule, StateBehaviour,
    StatePhysics, StateRender, WorldTint,
};

/// What a face occlusion shape is, as face culling asks first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Empty,
    /// The whole face (`Shapes.block()`).
    Full,
    Partial,
}

impl ShapeKind {
    pub(crate) fn of(s: &ShapeData) -> ShapeKind {
        let unit = s.coords.iter().all(|c| c[..] == [0.0, 1.0]);
        if s.full.iter().all(|f| !f) {
            ShapeKind::Empty
        } else if unit && s.full[..] == [true] {
            ShapeKind::Full
        } else {
            ShapeKind::Partial
        }
    }
}

/// A block state as the mesher reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StateInfo {
    pub flags: u16,
    pub skip: SkipRule,
    /// `getLightEmission`.
    pub light_emission: u8,
    /// For bars and panes: the sides they connect on (a bit per direction ordinal).
    pub connects: u8,
    /// The block's registry id.
    pub block: u16,
    /// Face occlusion shapes by direction, indices into the table's shapes.
    pub occlusion: [u16; 6],
    pub seed_offset: [i8; 3],
    /// An index into the table's offsets; 0: the block is not offset by its position.
    pub offset: u8,
    /// The state's tint layers: a range of the table's tints.
    pub tints: (u32, u8),
    /// The state's fluid (`getFluidState`), as the physics export packs it
    /// (`ruthenium_sim::fluid::FluidState::from_bits`); 0: none.
    pub fluid: u8,
    /// `isFaceSturdy`, a bit per direction ordinal.
    pub sturdy: u8,
}

impl StateInfo {
    /// Drawn from a model that has quads.
    pub const DRAWN: u16 = 1;
    /// `emissiveRendering`: lit fully whatever the light.
    pub const EMISSIVE: u16 = 2;
    /// A `LeavesBlock`.
    pub const LEAVES: u16 = 4;
    /// `isCollisionShapeFullBlock`.
    pub const COLLISION_FULL: u16 = 8;
    /// In the `bars` block tag.
    pub const BARS: u16 = 16;
    /// `isSolidRender`: nothing is seen through it.
    pub const SOLID_RENDER: u16 = 32;
    /// `isAir`.
    pub const AIR: u16 = 64;
    /// `getShadeBrightness` is 0.2 (else 1): it darkens the corners of faces beside it
    /// under smooth lighting.
    pub const DARK: u16 = 128;
    /// `isLightPermeable`: something is seen through it, or it takes no light away.
    pub const PERMEABLE: u16 = 256;
    /// `isSolid` (the legacy flag): a fluid's surface does not slope down towards it.
    pub const SOLID: u16 = 512;
    /// A `HalfTransparentBlock` or a `LeavesBlock`: water's side against it is drawn
    /// with the overlay sprite.
    pub const OVERLAY: u16 = 1024;
    /// An `IceBlock`: falling fluid does not count its faces as walls.
    pub const ICE: u16 = 2048;
    /// In the `blocks_fluid_flow` block tag.
    pub const BLOCKS_FLOW: u16 = 4096;
    /// Drawn as a block entity, besides its model if it has one.
    pub const BLOCK_ENTITY: u16 = 8192;
}

/// How a block is moved by its position (`BlockBehaviour.OffsetType`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Offset {
    pub vertical: bool,
    pub max_horizontal: f32,
    pub max_vertical: f32,
}

/// See the module docs.
#[derive(Debug)]
pub struct BlockTable {
    pub(crate) models: BlockModels,
    pub(crate) states: Vec<StateInfo>,
    pub(crate) shapes: Vec<ShapeData>,
    pub(crate) kinds: Vec<ShapeKind>,
    pub(crate) tints: Vec<Tint>,
    pub(crate) offsets: Vec<Offset>,
    pub(crate) mangrove_roots: u16,
    pub(crate) powder_snow: u16,
    pub(crate) fluids: FluidModels,
}

/// What a tint layer multiplies its faces by, 0xAARRGGBB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tint {
    Constant(u32),
    /// One of the biomes' colours, read `dy` blocks above the block; `hand` is what the
    /// game gives outside a world, for as long as no biome's colours are known.
    Biome {
        kind: BiomeColour,
        dy: i8,
        hand: u32,
    },
}

fn tint(layer: &exporters::TintLayer, grass_default: u32) -> Tint {
    let kind = match layer.world {
        WorldTint::Constant(c) => return Tint::Constant(c),
        WorldTint::Grass => BiomeColour::Grass,
        WorldTint::Foliage => BiomeColour::Foliage,
        WorldTint::DryFoliage => BiomeColour::DryFoliage,
        WorldTint::Water => BiomeColour::Water,
    };
    Tint::Biome {
        kind,
        dy: layer.dy,
        hand: layer.hand.unwrap_or(grass_default),
    }
}

impl BlockTable {
    /// Puts the exports, the baked models and the fluids' sprites together.
    /// `grass_default` is the grass colour map's default colour (0xAARRGGBB), and
    /// `in_tag` says whether a block tag (`bars`, `blocks_fluid_flow`) has a block (by
    /// name).
    pub fn new(
        physics: &BlockPhysics,
        behaviour: &BlockBehaviours,
        render: &BlockRender,
        models: BlockModels,
        fluids: FluidModels,
        grass_default: u32,
        in_tag: &dyn Fn(&str, &str) -> bool,
    ) -> Result<BlockTable, String> {
        for (what, n) in [
            ("block_physics", physics.states.len()),
            ("block_behaviour", behaviour.states.len()),
            ("block_render", render.states.len()),
        ] {
            if n != STATE_COUNT {
                return Err(format!(
                    "{what} has {n} block states, not {STATE_COUNT}: run `ruthenium prepare`"
                ));
            }
        }
        if render.shapes.len() > usize::from(u16::MAX) {
            return Err(format!("{} face shapes", render.shapes.len()));
        }
        let kinds = render.shapes.iter().map(ShapeKind::of).collect();
        let mut tints = Vec::new();
        let mut offsets = vec![Offset {
            vertical: false,
            max_horizontal: 0.0,
            max_vertical: 0.0,
        }];
        let mut states = Vec::with_capacity(STATE_COUNT);
        // What is asked of a block is asked once, not for each of its states.
        let mut last_block = None;
        let (mut overlay, mut blocks_flow) = (false, false);
        for (id, r) in render.states.iter().enumerate() {
            let id = id as u32;
            let state = BlockState::from_id(id).ok_or("a block state without an id")?;
            let block = state.block();
            let (p, b) = (&physics.states[id as usize], &behaviour.states[id as usize]);
            let mut flags = 0;
            if r.flags & StateRender::MODEL != 0 && !models.is_empty(id) {
                flags |= StateInfo::DRAWN;
            }
            if Kind::of(id).is_some() {
                flags |= StateInfo::BLOCK_ENTITY;
            }
            if r.flags & StateRender::EMISSIVE != 0 {
                flags |= StateInfo::EMISSIVE;
            }
            if r.flags & StateRender::LEAVES != 0 {
                flags |= StateInfo::LEAVES;
            }
            if b.flags & StateBehaviour::COLLISION_FULL_BLOCK != 0 {
                flags |= StateInfo::COLLISION_FULL;
            }
            if b.flags & StateBehaviour::SOLID_RENDER != 0 {
                flags |= StateInfo::SOLID_RENDER;
            }
            if b.flags & StateBehaviour::AIR != 0 {
                flags |= StateInfo::AIR;
            }
            if r.flags & StateRender::DARK != 0 {
                flags |= StateInfo::DARK;
            }
            if b.flags & StateBehaviour::SOLID_RENDER == 0 || p.light_dampening == 0 {
                flags |= StateInfo::PERMEABLE;
            }
            if b.flags & StateBehaviour::SOLID != 0 {
                flags |= StateInfo::SOLID;
            }
            if p.flags & StatePhysics::ICE != 0 {
                flags |= StateInfo::ICE;
            }
            if last_block != Some(block.id()) {
                last_block = Some(block.id());
                overlay = behaviour
                    .blocks
                    .get(block.id() as usize)
                    .is_some_and(|c| c.is("HalfTransparentBlock") || c.is("LeavesBlock"));
                blocks_flow = in_tag("blocks_fluid_flow", block.name());
            }
            if overlay {
                flags |= StateInfo::OVERLAY;
            }
            if blocks_flow {
                flags |= StateInfo::BLOCKS_FLOW;
            }
            let mut connects = 0;
            if r.skip == SkipRule::Bars {
                if in_tag("bars", block.name()) {
                    flags |= StateInfo::BARS;
                }
                for (direction, name) in [(2, "north"), (3, "south"), (4, "west"), (5, "east")] {
                    if state.value(name) == Some("true") {
                        connects |= 1 << direction;
                    }
                }
            }
            let offset = match p.offset {
                None => 0,
                Some(o) => {
                    let o = Offset {
                        vertical: o.vertical,
                        max_horizontal: o.max_horizontal,
                        max_vertical: o.max_vertical,
                    };
                    match offsets.iter().position(|known| *known == o) {
                        Some(i) => i as u8,
                        None => {
                            offsets.push(o);
                            u8::try_from(offsets.len() - 1)
                                .map_err(|_| "too many kinds of block offset")?
                        }
                    }
                }
            };
            let first_tint = tints.len() as u32;
            tints.extend(r.tints.iter().map(|t| tint(t, grass_default)));
            states.push(StateInfo {
                flags,
                skip: r.skip,
                light_emission: p.light_emission,
                connects,
                block: block.id() as u16,
                occlusion: r.occlusion.map(|s| s as u16),
                seed_offset: r.seed_offset,
                offset,
                tints: (first_tint, r.tints.len() as u8),
                fluid: p.fluid,
                sturdy: p.sturdy,
            });
        }
        Ok(BlockTable {
            models,
            states,
            shapes: render.shapes.clone(),
            kinds,
            tints,
            offsets,
            mangrove_roots: Block::from_name("minecraft:mangrove_roots")
                .map_or(u16::MAX, |b| b.id() as u16),
            powder_snow: Block::from_name("minecraft:powder_snow")
                .map_or(u16::MAX, |b| b.id() as u16),
            fluids,
        })
    }

    /// Reads the three exports from the prepared game's exports folder.
    pub fn load(
        exports: &Path,
        models: BlockModels,
        fluids: FluidModels,
        grass_default: u32,
        in_tag: &dyn Fn(&str, &str) -> bool,
    ) -> Result<BlockTable, String> {
        let read = |name: &str| {
            let path = exports.join(name);
            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        BlockTable::new(
            &exporters::read_block_physics(&read(exporters::BLOCK_PHYSICS)?)?,
            &exporters::read_block_behaviour(&read(exporters::BLOCK_BEHAVIOUR)?)?,
            &exporters::read_block_render(&read(exporters::BLOCK_RENDER)?)?,
            models,
            fluids,
            grass_default,
            in_tag,
        )
    }

    /// A state's entry; air's for an id the table doesn't have.
    pub(crate) fn state(&self, id: u32) -> &StateInfo {
        self.states.get(id as usize).unwrap_or(&self.states[0])
    }

    /// `isAir` of a state: a section of nothing else has nothing to draw and hides
    /// nothing.
    pub(crate) fn is_air(&self, id: u16) -> bool {
        self.state(u32::from(id)).flags & StateInfo::AIR != 0
    }

    /// `isSolidRender` of a state.
    pub(crate) fn is_solid(&self, id: u16) -> bool {
        self.state(u32::from(id)).flags & StateInfo::SOLID_RENDER != 0
    }

    /// How many distinct model parts the states are drawn from.
    pub fn part_count(&self) -> usize {
        self.models.part_count()
    }

    /// How many states are drawn from a model.
    pub fn drawn_states(&self) -> usize {
        self.states
            .iter()
            .filter(|s| s.flags & StateInfo::DRAWN != 0)
            .count()
    }

    /// `BlockState.skipRendering`: whether `state` leaves out its face towards
    /// `direction` against `neighbour`, by its block's own rule. Leaves hide each other's
    /// faces only where leaves are drawn opaque (`cutout_leaves` off).
    pub(crate) fn skips(
        &self,
        state: &StateInfo,
        neighbour: &StateInfo,
        direction: usize,
        cutout_leaves: bool,
    ) -> bool {
        match state.skip {
            SkipRule::None | SkipRule::SameFluid => false,
            SkipRule::SameBlock => neighbour.block == state.block,
            SkipRule::Leaves => !cutout_leaves && neighbour.flags & StateInfo::LEAVES != 0,
            SkipRule::MangroveRoots => neighbour.block == self.mangrove_roots && direction < 2,
            SkipRule::Bars => {
                // The same block, or bars of another kind: then above and below always,
                // and sideways where both connect to each other. (Every block in the tag
                // has the four sides as properties.)
                let alike = neighbour.block == state.block
                    || (neighbour.flags & state.flags & StateInfo::BARS != 0);
                alike
                    && (direction < 2
                        || (state.connects >> direction & 1 != 0
                            && neighbour.connects >> (direction ^ 1) & 1 != 0))
            }
        }
    }
}

/// The cell of a shape's grid that holds `v` along an axis with these coordinates.
fn cell(coords: &[f64], v: f64) -> Option<usize> {
    if coords.len() < 2 || v < coords[0] || v > coords[coords.len() - 1] {
        return None;
    }
    Some(
        coords
            .partition_point(|c| *c <= v)
            .clamp(1, coords.len() - 1)
            - 1,
    )
}

fn full_at(s: &ShapeData, p: [f64; 3]) -> bool {
    let (Some(x), Some(y), Some(z)) = (
        cell(&s.coords[0], p[0]),
        cell(&s.coords[1], p[1]),
        cell(&s.coords[2], p[2]),
    ) else {
        return false;
    };
    let (ny, nz) = (s.coords[1].len() - 1, s.coords[2].len() - 1);
    s.full[(x * ny + y) * nz + z]
}

/// Whether some part of `a` is outside `b` (`Shapes.joinIsNotEmpty` with `ONLY_FIRST`):
/// what face culling asks of a face's shape and the shape of the neighbour's face over
/// it. Coordinates closer than the game's 1e-7 are one.
pub(crate) fn sticks_out(a: &ShapeData, b: &ShapeData) -> bool {
    const EPSILON: f64 = 1.0e-7;
    let merged: [Vec<f64>; 3] = std::array::from_fn(|axis| {
        let mut all: Vec<f64> = a.coords[axis]
            .iter()
            .chain(&b.coords[axis])
            .copied()
            .collect();
        all.sort_by(f64::total_cmp);
        all.dedup_by(|next, kept| *next - *kept < EPSILON);
        all
    });
    let mids = |axis: usize| {
        merged[axis]
            .windows(2)
            .map(|w| (w[0] + w[1]) / 2.0)
            .collect::<Vec<f64>>()
    };
    let (xs, ys, zs) = (mids(0), mids(1), mids(2));
    xs.iter().any(|&x| {
        ys.iter().any(|&y| {
            zs.iter()
                .any(|&z| full_at(a, [x, y, z]) && !full_at(b, [x, y, z]))
        })
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn boxed(from: [f64; 3], to: [f64; 3]) -> ShapeData {
        ShapeData {
            coords: std::array::from_fn(|a| vec![from[a], to[a]]),
            full: vec![true],
        }
    }

    #[test]
    fn a_shape_sticks_out_where_the_other_has_nothing() {
        let block = boxed([0.0; 3], [1.0; 3]);
        let lower = boxed([0.0; 3], [1.0, 0.5, 1.0]);
        let upper = boxed([0.0, 0.5, 0.0], [1.0; 3]);
        assert!(!sticks_out(&lower, &block));
        assert!(sticks_out(&block, &lower));
        assert!(sticks_out(&lower, &upper));
        assert!(!sticks_out(&lower, &lower));
        // A post inside a slab's face, and one that pokes out of it.
        let post = boxed([0.375, 0.0, 0.375], [0.625, 0.5, 0.625]);
        assert!(!sticks_out(&post, &lower));
        assert!(sticks_out(&post, &upper));
        // Two cells along x, the second empty: only the first counts.
        let half = ShapeData {
            coords: [vec![0.0, 0.5, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
            full: vec![true, false],
        };
        assert!(!sticks_out(&half, &boxed([0.0; 3], [0.5, 1.0, 1.0])));
        assert!(sticks_out(&half, &boxed([0.5, 0.0, 0.0], [1.0; 3])));
        // Coordinates that differ by less than the game tells apart are the same.
        let nearly = boxed([0.0; 3], [1.0, 0.5 + 5.0e-8, 1.0]);
        assert!(!sticks_out(&nearly, &lower));
        // The empty shape sticks out of nothing, and everything sticks out of it.
        let empty = ShapeData {
            coords: [vec![0.0], vec![0.0], vec![0.0]],
            full: vec![],
        };
        assert!(!sticks_out(&empty, &block));
        assert!(sticks_out(&lower, &empty));
    }
}
