//! Block states' physics from the exporters (`block_physics.bin`): collision shapes,
//! friction, speed and jump factors, fluids and sturdy faces, as the local player meets
//! them.

use std::path::Path;

use ruthenium_data::v26_3::blocks::BlockState;
use ruthenium_prepare::exporters::{BlockPhysics, StatePhysics, read_block_physics};

use crate::fluid::FluidState;
use crate::shape::{Placed, Shape};

/// Every block state's physics, by state id.
#[derive(Debug, Clone)]
pub struct Blocks {
    shapes: Vec<Shape>,
    states: Vec<StatePhysics>,
    /// States whose shape the local player never meets (see [`Blocks::collision`]).
    passable: Vec<bool>,
    /// `Shapes.block()`, for outlines that fill the block.
    block: Shape,
}

/// Blocks whose collision shape depends on who collides, and which never stop a
/// player: liquids (only entities that stand on fluids collide) and moving pistons
/// (their shape comes from a block entity, not tracked yet).
const PASSABLE_FOR_PLAYERS: [&str; 3] = [
    "minecraft:water",
    "minecraft:lava",
    "minecraft:moving_piston",
];

impl Blocks {
    pub fn new(data: &BlockPhysics) -> Blocks {
        let passable = (0..data.states.len() as u32)
            .map(|id| {
                data.states[id as usize].flags & StatePhysics::CONTEXT_SHAPE != 0
                    && BlockState::from_id(id)
                        .is_some_and(|s| PASSABLE_FOR_PLAYERS.contains(&s.block().name()))
            })
            .collect();
        Blocks {
            shapes: data.shapes.iter().map(Shape::new).collect(),
            states: data.states.clone(),
            passable,
            block: Shape::block(),
        }
    }

    /// Reads the export from the prepared game's exports folder.
    pub fn load(exports: &Path) -> Result<Blocks, String> {
        let path = exports.join(ruthenium_prepare::exporters::BLOCK_PHYSICS);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Blocks::new(&read_block_physics(&bytes)?))
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// A state's physics (an unknown id is treated as air, state 0).
    pub fn state(&self, id: u32) -> &StatePhysics {
        self.states.get(id as usize).unwrap_or(&self.states[0])
    }

    pub fn friction(&self, id: u32) -> f32 {
        self.state(id).friction
    }

    pub fn speed_factor(&self, id: u32) -> f32 {
        self.state(id).speed_factor
    }

    pub fn jump_factor(&self, id: u32) -> f32 {
        self.state(id).jump_factor
    }

    pub fn bounce(&self, id: u32) -> f32 {
        self.state(id).bounce
    }

    /// `isSuffocating`.
    pub fn suffocating(&self, id: u32) -> bool {
        self.state(id).flags & StatePhysics::SUFFOCATING != 0
    }

    /// The state's fluid (`BlockState.getFluidState`).
    pub fn fluid(&self, id: u32) -> FluidState {
        FluidState::from_bits(self.state(id).fluid)
    }

    /// `isFaceSturdy` for the face in a direction (`Direction` ordinal: down, up,
    /// north, south, west, east), as at the origin.
    pub fn sturdy(&self, id: u32, direction: usize) -> bool {
        self.state(id).sturdy >> direction & 1 != 0
    }

    /// The block is an `IceBlock` (ice, frosted ice).
    pub fn is_ice(&self, id: u32) -> bool {
        self.state(id).flags & StatePhysics::ICE != 0
    }

    /// The block's name (`minecraft:stone`).
    pub fn name(&self, id: u32) -> &'static str {
        BlockState::from_id(id).map_or("minecraft:air", |s| s.block().name())
    }

    /// `isAir`: air, cave air and void air.
    pub fn is_air(&self, id: u32) -> bool {
        matches!(
            self.name(id),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }

    /// `hasLargeCollisionShape`.
    pub fn large_shape(&self, id: u32) -> bool {
        self.state(id).flags & StatePhysics::LARGE_SHAPE != 0
    }

    /// The local player's collision shape for the state at `pos`, placed in the world;
    /// `None` when it has none.
    ///
    /// Context: a player never stands on fluids (`LiquidBlock`: only the empty shape
    /// unless the entity can stand on the fluid), and moving pistons need their block
    /// entity, which isn't tracked yet. Powder snow and scaffolding use the shape the
    /// empty context gives until their rules (boots and falling, standing above and
    /// sneaking) are ported.
    pub fn collision(&self, id: u32, pos: [i32; 3]) -> Option<Placed<'_>> {
        let s = self.state(id);
        let shape = &self.shapes[s.shape as usize];
        if shape.is_empty() || self.passable.get(id as usize) == Some(&true) {
            return None;
        }
        let offset = s
            .offset
            .map(|o| offset(pos, o.vertical, o.max_horizontal, o.max_vertical));
        Some(Placed {
            shape,
            offset,
            pos: [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
        })
    }

    /// The outline shape (`getShape` with the picking entity's context), placed in the
    /// world; `None` when it is empty. `held_item` is the item in the entity's main
    /// hand (`minecraft:air` for none): light blocks and scaffolding fill the block
    /// for one holding their item (`isHoldingItem`), the only outlines that depend on
    /// the context. Offset blocks offset their outline like their collision shape.
    pub fn outline(&self, id: u32, pos: [i32; 3], held_item: &str) -> Option<Placed<'_>> {
        let place = |shape, offset| Placed {
            shape,
            offset,
            pos: [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
        };
        let name = self.name(id);
        if matches!(name, "minecraft:light" | "minecraft:scaffolding") && held_item == name {
            return Some(place(&self.block, None));
        }
        let s = self.state(id);
        let shape = &self.shapes[s.outline as usize];
        if shape.is_empty() {
            return None;
        }
        let offset = s
            .offset
            .map(|o| offset(pos, o.vertical, o.max_horizontal, o.max_vertical));
        Some(place(shape, offset))
    }

    /// The interaction shape (`getInteractionShape`: cauldrons, composters, hoppers,
    /// scaffolding), placed in the world; `None` when empty. No block with one is
    /// offset by position.
    pub fn interaction_shape(&self, id: u32, pos: [i32; 3]) -> Option<Placed<'_>> {
        let shape = &self.shapes[self.state(id).interaction_shape as usize];
        (!shape.is_empty()).then(|| Placed {
            shape,
            offset: None,
            pos: [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
        })
    }

    /// Whether the state's shape is the full block (`Shapes.block()`).
    pub fn is_full_block(&self, id: u32) -> bool {
        self.shapes[self.state(id).shape as usize].is_block()
    }

    /// The raw shape of a state.
    pub fn shape(&self, id: u32) -> &Shape {
        &self.shapes[self.state(id).shape as usize]
    }

    /// How hard the block is to break (`getDestroySpeed`): -1 when it can't be.
    pub fn destroy_time(&self, id: u32) -> f32 {
        self.state(id).destroy_time
    }

    /// `requiresCorrectToolForDrops`.
    pub fn requires_correct_tool(&self, id: u32) -> bool {
        self.state(id).flags & StatePhysics::REQUIRES_TOOL != 0
    }

    /// The block is a `GameMasterBlock` (command, structure, jigsaw and test blocks).
    pub fn is_game_master(&self, id: u32) -> bool {
        self.state(id).flags & StatePhysics::GAME_MASTER != 0
    }

    /// What breaking the block leaves (`getFluidState().createLegacyBlock()`): air, or
    /// the fluid it held as a liquid block (a waterlogged block leaves a water source).
    pub fn left_behind(&self, id: u32) -> u32 {
        let f = self.fluid(id);
        if f.is_empty() {
            return AIR;
        }
        let name = if f.fluid.is_water() {
            "minecraft:water"
        } else {
            "minecraft:lava"
        };
        // `FlowingFluid.getLegacyLevel`.
        let level = if f.is_source() {
            0
        } else {
            8 - f.amount.min(8) + if f.falling { 8 } else { 0 }
        };
        ruthenium_data::v26_3::Block::from_name(name)
            .and_then(|b| b.default_state().with("level", &level.to_string()))
            .map_or(AIR, |s| s.id())
    }
}

/// The air block's state.
pub const AIR: u32 = 0;

/// The block (registry id) a state belongs to.
pub fn block_of(state: u32) -> Option<u32> {
    BlockState::from_id(state).map(|s| s.block().id())
}

/// The block tags, from the server's tags packet in a session: the ones movement reads
/// per state, and every tag by block for the rest (tools' rules, adventure-mode
/// predicates).
#[derive(Debug, Clone, Default)]
pub struct BlockTags {
    climbable: Vec<bool>,
    blocks_fluid_flow: Vec<bool>,
    /// Each tag's blocks, by block registry id.
    all: std::collections::HashMap<String, Vec<bool>>,
}

impl BlockTags {
    pub fn new() -> BlockTags {
        BlockTags::default()
    }

    /// Sets a tag's blocks, by block name (`minecraft:ladder`); unknown names are
    /// skipped.
    pub fn set<'a>(&mut self, tag: &str, blocks: impl IntoIterator<Item = &'a str>) {
        let blocks: Vec<_> = blocks
            .into_iter()
            .filter_map(ruthenium_data::v26_3::Block::from_name)
            .collect();
        let mut by_block = Vec::new();
        for b in &blocks {
            let i = b.id() as usize;
            if by_block.len() <= i {
                by_block.resize(i + 1, false);
            }
            by_block[i] = true;
        }
        self.all.insert(tag.to_owned(), by_block);
        let flags = match tag {
            "minecraft:climbable" => &mut self.climbable,
            "minecraft:blocks_fluid_flow" => &mut self.blocks_fluid_flow,
            _ => return,
        };
        *flags = vec![false; ruthenium_data::v26_3::blocks::STATE_COUNT];
        for b in blocks {
            for s in b.states() {
                flags[s.id() as usize] = true;
            }
        }
    }

    /// `BlockState.is(tag)`: an unknown tag holds no blocks.
    pub fn is(&self, state: u32, tag: &str) -> bool {
        block_of(state).is_some_and(|b| {
            self.all
                .get(tag)
                .and_then(|t| t.get(b as usize))
                .copied()
                .unwrap_or(false)
        })
    }

    /// `BlockTags.CLIMBABLE`.
    pub fn climbable(&self, state: u32) -> bool {
        self.climbable.get(state as usize) == Some(&true)
    }

    /// `BlockTags.BLOCKS_FLUID_FLOW`.
    pub fn blocks_fluid_flow(&self, state: u32) -> bool {
        self.blocks_fluid_flow.get(state as usize) == Some(&true)
    }
}

/// `Mth.getSeed`.
pub fn seed(x: i32, y: i32, z: i32) -> i64 {
    let mut seed =
        i64::from(x.wrapping_mul(3129871)) ^ i64::from(z).wrapping_mul(116129781) ^ i64::from(y);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42317861)
        .wrapping_add(seed.wrapping_mul(11));
    seed >> 16
}

/// The offset of a block offset by position (`BlockBehaviour.Properties.offsetType`).
pub fn offset(pos: [i32; 3], vertical: bool, max_h: f32, max_v: f32) -> [f64; 3] {
    let s = seed(pos[0], 0, pos[2]);
    let clamp = |v: f64| v.clamp(-f64::from(max_h), f64::from(max_h));
    let x = clamp((f64::from((s & 15) as f32 / 15.0) - 0.5) * 0.5);
    let z = clamp((f64::from((s >> 8 & 15) as f32 / 15.0) - 0.5) * 0.5);
    let y = if vertical {
        (f64::from((s >> 4 & 15) as f32 / 15.0) - 1.0) * f64::from(max_v)
    } else {
        0.0
    };
    [x, y, z]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_match_java() {
        // Mth.getSeed(1, 0, 1) and (-7, 0, 300), computed with Java's long arithmetic.
        assert_eq!(seed(0, 0, 0), 0);
        let s = seed(1, 0, 1);
        let x = 3129871i64 ^ 116129781i64;
        assert_eq!(
            s,
            (x.wrapping_mul(x)
                .wrapping_mul(42317861)
                .wrapping_add(x * 11))
                >> 16
        );
        let big = seed(-7, 0, 300);
        let v = i64::from((-7i32).wrapping_mul(3129871)) ^ (300i64 * 116129781);
        assert_eq!(
            big,
            (v.wrapping_mul(v)
                .wrapping_mul(42317861)
                .wrapping_add(v * 11))
                >> 16
        );
    }
}
