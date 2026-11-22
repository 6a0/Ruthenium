//! Block states' physics from the exporters (`block_physics.bin`): collision shapes,
//! friction, speed and jump factors, as the local player meets them.

use std::path::Path;

use ruthenium_data::v26_3::blocks::BlockState;
use ruthenium_prepare::exporters::{BlockPhysics, StatePhysics, read_block_physics};

use crate::shape::{Placed, Shape};

/// Every block state's physics, by state id.
#[derive(Debug, Clone)]
pub struct Blocks {
    shapes: Vec<Shape>,
    states: Vec<StatePhysics>,
    /// States whose shape the local player never meets (see [`Blocks::collision`]).
    passable: Vec<bool>,
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

    /// The block's name (`minecraft:stone`).
    pub fn name(&self, id: u32) -> &'static str {
        BlockState::from_id(id).map_or("minecraft:air", |s| s.block().name())
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

    /// Whether the state's shape is the full block (`Shapes.block()`).
    pub fn is_full_block(&self, id: u32) -> bool {
        self.shapes[self.state(id).shape as usize].is_block()
    }

    /// The raw shape of a state.
    pub fn shape(&self, id: u32) -> &Shape {
        &self.shapes[self.state(id).shape as usize]
    }
}

/// Block tags movement reads, per state: from the server's tags packet in a session.
#[derive(Debug, Clone, Default)]
pub struct BlockTags {
    climbable: Vec<bool>,
}

impl BlockTags {
    pub fn new() -> BlockTags {
        BlockTags::default()
    }

    /// Sets a tag's blocks, by block name (`minecraft:ladder`). Tags movement doesn't
    /// read are ignored.
    pub fn set<'a>(&mut self, tag: &str, blocks: impl IntoIterator<Item = &'a str>) {
        let flags = match tag {
            "minecraft:climbable" => &mut self.climbable,
            _ => return,
        };
        *flags = vec![false; ruthenium_data::v26_3::blocks::STATE_COUNT];
        for name in blocks {
            if let Some(b) = ruthenium_data::v26_3::Block::from_name(name) {
                for s in b.states() {
                    flags[s.id() as usize] = true;
                }
            }
        }
    }

    /// `BlockTags.CLIMBABLE`.
    pub fn climbable(&self, state: u32) -> bool {
        self.climbable.get(state as usize) == Some(&true)
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
