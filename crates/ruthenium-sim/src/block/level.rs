//! The level as block behaviour reads and changes it on the client: block and fluid
//! lookups, `Level.setBlock` with the shape updates it sends to the neighbours
//! (through `CollectingNeighborUpdater`, the only updates a client level runs), the
//! predictions that keep the server's states, redstone signals, supporting faces, and
//! whether entities stand where a block would go.

use ruthenium_prepare::exporters::StateBehaviour;

use super::{Pos, state};
use crate::direction::Direction;
use crate::fluid::FluidState;
use crate::interact::World;
use crate::shape::{Aabb, Axis};

/// `Block.UPDATE_*`: what a `setBlock` call asks for.
pub mod flags {
    /// Tell the neighbours (only the server does).
    pub const NEIGHBORS: u32 = 1;
    /// Send to clients and re-render.
    pub const CLIENTS: u32 = 2;
    /// Don't re-render.
    pub const INVISIBLE: u32 = 4;
    /// Re-render at once.
    pub const IMMEDIATE: u32 = 8;
    /// The neighbours already know: no shape updates.
    pub const KNOWN_SHAPE: u32 = 16;
    /// A block that can't stay doesn't drop its items.
    pub const SUPPRESS_DROPS: u32 = 32;
    pub const MOVE_BY_PISTON: u32 = 64;
    /// Redstone wire ignores the shape update.
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: u32 = 128;
    pub const SKIP_BLOCK_ENTITY_SIDEEFFECTS: u32 = 256;
    pub const SKIP_ON_PLACE: u32 = 512;
    /// `UPDATE_ALL`.
    pub const ALL: u32 = NEIGHBORS | CLIENTS;
    /// `UPDATE_ALL_IMMEDIATE`.
    pub const ALL_IMMEDIATE: u32 = ALL | IMMEDIATE;
}

/// One `setBlock` call, as tests log them: position, state, flags, update limit.
pub type SetCall = (Pos, u32, u32, i32);

/// How far shape updates may chain from one `setBlock` (`Level.setBlock`'s default
/// limit).
pub const UPDATE_LIMIT: i32 = 512;

/// `BlockBehaviour.UPDATE_SHAPE_ORDER`: the order neighbours hear of a change.
pub const UPDATE_SHAPE_ORDER: [Direction; 6] = [
    Direction::West,
    Direction::East,
    Direction::North,
    Direction::South,
    Direction::Down,
    Direction::Up,
];

/// `SupportType`: how much of a face must be there to hold a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Full = 0,
    Center = 1,
    Rigid = 2,
}

/// Whose collision shapes an obstruction check uses (`CollisionContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionMode {
    /// `CollisionContext.empty()`: the shapes as exported.
    Empty,
    /// `placementContext(player)`: scaffolding and powder snow have none.
    Placement,
}

/// A shape update waiting its turn (`CollectingNeighborUpdater.ShapeUpdate`).
#[derive(Debug, Clone, Copy)]
struct ShapeUpdate {
    direction: Direction,
    neighbour: u32,
    pos: Pos,
    neighbour_pos: Pos,
    flags: u32,
    limit: i32,
}

/// The client level's neighbour updater, of which only shape updates run: an update
/// made while others run waits for the current one to finish, the ones one update
/// adds then run (in the order they were added) before the rest of the stack.
#[derive(Debug, Clone, Default)]
pub struct ShapeUpdates {
    stack: Vec<ShapeUpdate>,
    added: Vec<ShapeUpdate>,
    count: u32,
}

impl World<'_> {
    /// The state at a position (air outside the level or where no chunk is).
    pub fn get(&self, pos: Pos) -> u32 {
        self.level.block(pos[0], pos[1], pos[2])
    }

    /// The fluid at a position (`getFluidState`).
    pub fn fluid(&self, pos: Pos) -> FluidState {
        self.blocks.fluid(self.get(pos))
    }

    /// `isEmptyBlock`: air there.
    pub fn is_empty_block(&self, pos: Pos) -> bool {
        self.blocks.is_air(self.get(pos))
    }

    /// `isWaterAt`: the fluid there is water (`FluidTags.WATER`).
    pub fn is_water_at(&self, pos: Pos) -> bool {
        self.fluid(pos).fluid.is_water()
    }

    /// The lowest and highest block y of the level.
    pub fn min_y(&self) -> i32 {
        self.level.min_y()
    }

    pub fn max_y(&self) -> i32 {
        self.level.min_y() + self.level.height() - 1
    }

    /// `isInsideBuildHeight`.
    pub fn inside_build_height(&self, y: i32) -> bool {
        y >= self.min_y() && y <= self.max_y()
    }

    /// `isInWorldBounds` (`isInValidBounds` on the client): inside the build height
    /// and the 30 000 000 block horizontal limit.
    pub fn in_world_bounds(&self, pos: Pos) -> bool {
        self.inside_build_height(pos[1])
            && (-30_000_000..30_000_000).contains(&pos[0])
            && (-30_000_000..30_000_000).contains(&pos[2])
    }

    /// `Level.setBlock` with the default update limit.
    pub fn set_block(&mut self, pos: Pos, state: u32, flags: u32) -> bool {
        self.set_block_limited(pos, state, flags, UPDATE_LIMIT)
    }

    /// `setBlockAndUpdate`.
    pub fn set_block_and_update(&mut self, pos: Pos, state: u32) -> bool {
        self.set_block(pos, state, flags::ALL)
    }

    /// `Level.setBlock` as a client level runs it: the block changes (no change
    /// when it is the same state, no chunk is there, or air goes into an empty
    /// section), the neighbours hear of it unless `KNOWN_SHAPE` (their own shape
    /// changes go through `setBlock` too, a step further down the limit; a block that
    /// can't stay any more is left for the server), and while an action is predicted
    /// the block keeps the state the server last gave it. True if it changed.
    pub fn set_block_limited(&mut self, pos: Pos, state: u32, flags: u32, limit: i32) -> bool {
        if self.predictions.predicting() {
            if let Some(log) = &mut self.set_log {
                log.push((pos, state, flags, limit));
            }
        } else if let Some(log) = &mut self.level.set_log {
            log.push((pos, state, flags, limit));
        }
        if !self.in_world_bounds(pos) {
            return false;
        }
        if self.blocks.is_air(state) && self.section_has_only_air(pos) {
            return false;
        }
        let Some(old) = self.level.replace_block(pos[0], pos[1], pos[2], state) else {
            return false;
        };
        if flags & flags::KNOWN_SHAPE == 0 && limit > 0 {
            let f = flags & !(flags::NEIGHBORS | flags::SUPPRESS_DROPS);
            super::update_indirect_neighbour_shapes(self, old, pos, f, limit - 1);
            self.update_neighbour_shapes(pos, state, f, limit - 1);
            super::update_indirect_neighbour_shapes(self, state, pos, f, limit - 1);
        }
        let at = self.player_position;
        self.predictions.retain(pos, old, at);
        true
    }

    /// Whether the section holding `pos` has nothing but air (`hasOnlyAir`).
    fn section_has_only_air(&self, pos: Pos) -> bool {
        let base = [
            pos[0] & !15,
            ((pos[1] - self.min_y()) & !15) + self.min_y(),
            pos[2] & !15,
        ];
        if !self.level.has_chunk(pos[0] >> 4, pos[2] >> 4) {
            return false;
        }
        (0..16).all(|y| {
            (0..16).all(|z| {
                (0..16).all(|x| {
                    self.blocks
                        .is_air(self.get([base[0] + x, base[1] + y, base[2] + z]))
                })
            })
        })
    }

    /// `BlockState.updateNeighbourShapes`: each neighbour, in
    /// [`UPDATE_SHAPE_ORDER`], hears that the block at `pos` is now `state`.
    pub fn update_neighbour_shapes(&mut self, pos: Pos, state: u32, flags: u32, limit: i32) {
        for d in UPDATE_SHAPE_ORDER {
            self.neighbour_shape_changed(d.opposite(), d.step(pos), pos, state, flags, limit);
        }
    }

    /// `neighborShapeChanged`: the block at `pos` hears that its neighbour towards
    /// `direction` is now `neighbour`.
    pub fn neighbour_shape_changed(
        &mut self,
        direction: Direction,
        pos: Pos,
        neighbour_pos: Pos,
        neighbour: u32,
        flags: u32,
        limit: i32,
    ) {
        let update = ShapeUpdate {
            direction,
            neighbour,
            pos,
            neighbour_pos,
            flags,
            limit,
        };
        let running = self.updates.count > 0;
        self.updates.count += 1;
        if running {
            self.updates.added.push(update);
            return;
        }
        self.updates.stack.push(update);
        while !self.updates.stack.is_empty() || !self.updates.added.is_empty() {
            while let Some(u) = self.updates.added.pop() {
                self.updates.stack.push(u);
            }
            let Some(&next) = self.updates.stack.last() else {
                break;
            };
            self.run_shape_update(next);
            self.updates.stack.pop();
        }
        self.updates.count = 0;
    }

    /// `NeighborUpdater.executeShapeUpdate` and `Block.updateOrDestroy`.
    fn run_shape_update(&mut self, u: ShapeUpdate) {
        let current = self.get(u.pos);
        if u.flags & flags::SKIP_SHAPE_UPDATE_ON_WIRE != 0
            && state::is(current, "minecraft:redstone_wire")
        {
            return;
        }
        let new = super::update_shape(
            self,
            current,
            u.pos,
            u.direction,
            u.neighbour_pos,
            u.neighbour,
        );
        self.update_or_destroy(current, new, u.pos, u.flags, u.limit);
    }

    /// `Block.updateOrDestroy`: a changed state is set; one that became air is the
    /// server's to break.
    pub fn update_or_destroy(&mut self, old: u32, new: u32, pos: Pos, flags: u32, limit: i32) {
        if new != old && !self.blocks.is_air(new) {
            self.set_block_limited(pos, new, flags & !flags::SUPPRESS_DROPS, limit);
        }
    }

    /// `Block.updateFromNeighbourShapes`: `state` at `pos` after hearing of each
    /// neighbour in turn.
    pub fn update_from_neighbour_shapes(&self, state: u32, pos: Pos) -> u32 {
        UPDATE_SHAPE_ORDER.iter().fold(state, |s, &d| {
            let n = d.step(pos);
            super::update_shape(self, s, pos, d, n, self.get(n))
        })
    }

    /// A state's flags and faces.
    pub fn behaviour(&self, state: u32) -> StateBehaviour {
        self.blocks.kinds().state(state)
    }

    /// `isFaceSturdy` of the block at `pos` on its face towards `direction`.
    pub fn face_sturdy(&self, pos: Pos, direction: Direction, support: Support) -> bool {
        self.blocks.face_sturdy(self.get(pos), direction, support)
    }

    /// `Block.canSupportRigidBlock`: the block below `pos`'s... the top of the block
    /// at `below` holds a rigid block.
    pub fn can_support_rigid_block(&self, below: Pos) -> bool {
        self.face_sturdy(below, Direction::Up, Support::Rigid)
    }

    /// `Block.canSupportCenter`: the face of the block at `pos` towards
    /// `direction` holds something by its centre (a block tagged unstable at the
    /// bottom doesn't, downwards).
    pub fn can_support_center(&self, pos: Pos, direction: Direction) -> bool {
        let s = self.get(pos);
        if direction == Direction::Down && self.tags.is(s, "minecraft:unstable_bottom_center") {
            return false;
        }
        self.blocks.face_sturdy(s, direction, Support::Center)
    }

    /// `getDirectSignal`: the strong power the block at `pos` gives towards the
    /// block on its `direction` side... as the game indexes it: the direction from
    /// the powered block to `pos`.
    pub fn direct_signal(&self, pos: Pos, direction: Direction) -> i32 {
        i32::from(self.blocks.direct_signal(self.get(pos), direction))
    }

    /// `getDirectSignalTo`: the strongest power reaching `pos` directly.
    pub fn direct_signal_to(&self, pos: Pos) -> i32 {
        let mut best = 0;
        for d in Direction::ALL {
            best = best.max(self.direct_signal(d.step(pos), d));
            if best >= 15 {
                break;
            }
        }
        best
    }

    /// `getSignal`: the power the block at `pos` gives towards `direction`'s
    /// opposite; a conductor also passes on the strong power it receives.
    pub fn signal(&self, pos: Pos, direction: Direction) -> i32 {
        let s = self.get(pos);
        let own = i32::from(self.blocks.signal(s, direction));
        if self.blocks.redstone_conductor(s) {
            own.max(self.direct_signal_to(pos))
        } else {
            own
        }
    }

    /// `hasSignal`.
    pub fn has_signal(&self, pos: Pos, direction: Direction) -> bool {
        self.signal(pos, direction) > 0
    }

    /// `hasNeighborSignal`: any neighbour powers `pos`.
    pub fn has_neighbor_signal(&self, pos: Pos) -> bool {
        Direction::ALL
            .into_iter()
            .any(|d| self.signal(d.step(pos), d) > 0)
    }

    /// `getBestNeighborSignal`.
    pub fn best_neighbor_signal(&self, pos: Pos) -> i32 {
        let mut best = 0;
        for d in Direction::ALL {
            let s = self.signal(d.step(pos), d);
            if s >= 15 {
                return 15;
            }
            best = best.max(s);
        }
        best
    }

    /// `isUnobstructed(state, pos, context)`: no entity that blocks building
    /// (living entities, the local player, boats, minecarts, armor stands that aren't
    /// markers, end crystals, primed TNT, falling blocks; not spectators) overlaps the
    /// state's collision shape there.
    pub fn is_unobstructed(&self, state: u32, pos: Pos, mode: CollisionMode) -> bool {
        let name = state::name(state);
        if mode == CollisionMode::Placement
            && matches!(name, "minecraft:scaffolding" | "minecraft:powder_snow")
        {
            return true;
        }
        let Some(shape) = self.blocks.collision(state, pos) else {
            return true;
        };
        let last = |a: Axis| shape.get(a, shape.shape.size(a));
        let bounds = Aabb {
            min: [
                shape.get(Axis::X, 0),
                shape.get(Axis::Y, 0),
                shape.get(Axis::Z, 0),
            ],
            max: [last(Axis::X), last(Axis::Y), last(Axis::Z)],
        };
        if let Some(b) = self.player_box
            && shape.overlaps(&b)
        {
            return false;
        }
        !self
            .entities
            .in_box(&bounds)
            .into_iter()
            .any(|e| !e.removed && blocks_building(e, self) && shape.overlaps(&e.bounding_box))
    }
}

/// `Entity.blocksBuilding`, of the entities `getEntities` finds (spectating players
/// aren't among them).
fn blocks_building(e: &crate::entity::Entity, w: &World<'_>) -> bool {
    use crate::entity::BlocksBuilding;
    if w.players
        .is_some_and(|p| p.get(e.uuid).is_some_and(|i| i.game_mode == 3))
    {
        return false;
    }
    match e.info().blocks_building {
        BlocksBuilding::No => false,
        BlocksBuilding::Yes => true,
        BlocksBuilding::UnlessMarker => !e.marker,
    }
}

impl crate::predict::Predictions {
    /// `retainKnownServerState`: while an action is predicted, a block it changed
    /// keeps the state the server last gave it (the first time), and the player's
    /// position then; a block changed again moves on to the newer action.
    pub fn retain(&mut self, pos: Pos, server_state: u32, player_position: [f64; 3]) {
        self.retain_state(pos, server_state, player_position);
    }
}
