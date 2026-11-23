//! Pistons and shulker boxes as the client runs them, from the server's block events:
//! a piston's push worked out again on the client (`PistonBaseBlock.triggerEvent`,
//! `moveBlocks`, `PistonStructureResolver`), the moving blocks' ticks
//! (`PistonMovingBlockEntity.tick`: the player pushed ahead of them, launched by slime,
//! carried by honey, the block put down at the end), and a shulker box's lid
//! (`ShulkerBoxBlockEntity`), which lifts or pushes the player as it opens.
//!
//! Only the local player and the vehicle it controls are moved: other entities follow
//! the server's moves.

// `!(a <= b)` keeps Java's comparisons as written (NaN included).
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::block::state;
use crate::block::{Class, Pos};
use crate::block_entity::{Data, Lid, MovingPiston, progress_delta_aabb, shape_max};
use crate::blocks::AIR;
use crate::direction::Direction;
use crate::entity::Entity;
use crate::interact::World;
use crate::player::{Env, LocalPlayer, MoverType};
use crate::shape::{Aabb, Axis, Shape};

/// `PistonStructureResolver.MAX_PUSH_DEPTH`.
const MAX_PUSH: usize = 12;

/// `PushReaction`'s ordinals.
const PUSH_PULL: u8 = 0;
const PUSH_ONLY: u8 = 1;
const POPPED: u8 = 2;
const IMMOVEABLE: u8 = 3;

/// What a piston or a shulker box's lid moves on the client: the local player, and the
/// vehicle it controls (`canSimulateMovement`; the rest follow the server).
#[derive(Debug)]
pub enum Pushed<'p> {
    Player(&'p mut LocalPlayer),
    Vehicle(&'p mut Entity),
}

impl Pushed<'_> {
    fn bounding_box(&self) -> Aabb {
        match self {
            Pushed::Player(p) => p.bounding_box(),
            Pushed::Vehicle(e) => e.bounding_box,
        }
    }

    /// Whether `getEntities` in `area` finds it: in the level (the player not a
    /// spectator), its box in the area.
    fn found_in(&self, area: &Aabb) -> bool {
        let present = match self {
            Pushed::Player(p) => !p.removed && !p.spectator,
            Pushed::Vehicle(e) => !e.removed,
        };
        present && self.bounding_box().intersects(area)
    }

    fn delta(&mut self) -> &mut [f64; 3] {
        match self {
            Pushed::Player(p) => &mut p.delta,
            Pushed::Vehicle(e) => &mut e.delta,
        }
    }

    fn position(&self) -> [f64; 3] {
        match self {
            Pushed::Player(p) => p.position,
            Pushed::Vehicle(e) => e.position,
        }
    }

    fn on_ground(&self) -> bool {
        match self {
            Pushed::Player(p) => p.on_ground,
            Pushed::Vehicle(e) => e.on_ground,
        }
    }

    fn supporting_block(&self) -> Option<[i32; 3]> {
        match self {
            Pushed::Player(p) => p.supporting_block(),
            Pushed::Vehicle(e) => e.supporting_block(),
        }
    }

    fn push(&mut self, env: &Env<'_>, kind: MoverType, v: [f64; 3], game_time: i64) {
        match self {
            Pushed::Player(p) => p.move_by(env, kind, v, game_time),
            Pushed::Vehicle(e) => e.entity_move(env, kind, v, game_time, true),
        }
    }

    fn effects_between(&mut self, env: &Env<'_>, from: [f64; 3], to: [f64; 3]) {
        match self {
            Pushed::Player(p) => p.apply_effects_between(env, from, to),
            Pushed::Vehicle(e) => e.apply_effects_between(env, from, to),
        }
    }

    fn remove_latest_movement_recording(&mut self) {
        match self {
            Pushed::Player(p) => p.remove_latest_movement_recording(),
            Pushed::Vehicle(e) => e.remove_latest_movement_recording(),
        }
    }
}

impl World<'_> {
    /// `ClientLevel.blockEvent`: the event goes to whatever block is at `pos` now
    /// (pistons and shulker boxes are the ones that move anything).
    pub fn block_event(&mut self, pos: Pos, b0: u8, b1: u8) {
        let s = self.get(pos);
        if self.blocks.is_class(s, Class::PistonBaseBlock) {
            self.piston_event(s, pos, b0, b1);
        } else if self.blocks.is_class(s, Class::ShulkerBoxBlock) {
            // `BaseEntityBlock.triggerEvent`: the block entity (made if missing).
            if self.level.block_entities.shulker(pos).is_none() {
                self.level.block_entities.on_block_changed(pos, AIR, s);
            }
            if let Some(b) = self.level.block_entities.shulker_mut(pos) {
                b.trigger(b0, b1);
            }
        }
    }

    /// `Level.tickBlockEntities`, after the entities: each ticker in turn, the ones
    /// whose block entity is gone taken out. `game_time` is the level's (pistons
    /// move an entity at most 0.51 an axis in one).
    pub fn tick_block_entities(&mut self, player: &mut LocalPlayer, game_time: i64) {
        self.tick_block_entities_riding(player, None, game_time);
    }

    /// [`Self::tick_block_entities`] also pushing the vehicle the player controls
    /// (`vehicle`, with the boxes it collides with), which the client moves too.
    pub fn tick_block_entities_riding(
        &mut self,
        player: &mut LocalPlayer,
        vehicle: Option<(&mut Entity, &[Aabb])>,
        game_time: i64,
    ) {
        let colliders = self.entities.colliders(player);
        let mut targets: Vec<(Pushed<'_>, &[Aabb])> = Vec::with_capacity(2);
        if let Some((v, c)) = vehicle {
            targets.push((Pushed::Vehicle(v), c));
        }
        targets.push((Pushed::Player(player), &colliders));
        self.level.block_entities.begin_ticking();
        let mut i = 0;
        while let Some(at) = self.level.block_entities.ticker(i) {
            let Some(pos) = at else {
                continue;
            };
            i += 1;
            // `BoundTickingBlockEntity.tick`: only while the block there has this kind.
            let s = self.get(pos);
            match self.level.block_entities.get(pos) {
                Some(Data::Piston(_)) if state::is(s, "minecraft:moving_piston") => {
                    self.tick_moving_piston(pos, &mut targets, game_time)
                }
                Some(Data::Shulker(_)) if self.blocks.is_class(s, Class::ShulkerBoxBlock) => {
                    self.tick_shulker_box(pos, s, &mut targets, game_time)
                }
                _ => {}
            }
        }
        self.level.block_entities.end_ticking();
    }

    // --- Pistons ---------------------------------------------------------------------

    /// `PistonBaseBlock.triggerEvent` on the client: 0 extends (if the client finds the
    /// blocks can move), 1 retracts (a sticky piston pulling its block back), 2 retracts
    /// leaving it.
    fn piston_event(&mut self, s: u32, pos: Pos, b0: u8, b1: u8) {
        let Some(direction) = state::get_dir(s, "facing") else {
            return;
        };
        let sticky = state::is(s, "minecraft:sticky_piston");
        if b0 == 0 {
            if self.move_blocks(pos, direction, true, sticky) {
                self.set_block(pos, state::set_bool(s, "extended", true), 67);
            }
        } else if b0 == 1 || b0 == 2 {
            let arm = direction.step(pos);
            if self.level.block_entities.piston(arm).is_some() {
                self.final_tick(arm);
            }
            let moving = moving_piston(direction, sticky);
            self.set_block(pos, moving, 276);
            let base = state::default_of_block(state::block_id(s));
            let facing = Direction::from_id((b1 & 7) % 6).unwrap_or(Direction::Down);
            let moved = state::set_dir(base, "facing", facing);
            self.add_moving_piston(pos, MovingPiston::new(moved, direction, false, true));
            self.update_neighbour_shapes(pos, moving, 2, crate::block::level::UPDATE_LIMIT);
            if sticky {
                let two = direction.step(arm);
                let there = self.get(two);
                let mut piece = false;
                if state::is(there, "minecraft:moving_piston")
                    && self
                        .level
                        .block_entities
                        .piston(two)
                        .is_some_and(|p| p.direction == direction && p.extending)
                {
                    self.final_tick(two);
                    piece = true;
                }
                if !piece {
                    let reaction = self.behaviour(there).push_reaction();
                    if b0 != 1
                        || self.blocks.is_air(there)
                        || !self.is_pushable(there, two, direction.opposite(), false, direction)
                        || reaction != PUSH_PULL
                            && !state::is(there, "minecraft:piston")
                            && !state::is(there, "minecraft:sticky_piston")
                    {
                        self.remove_block(arm);
                    } else {
                        self.move_blocks(pos, direction, false, sticky);
                    }
                }
            } else {
                self.remove_block(arm);
            }
        }
    }

    /// `Level.removeBlock`: what the block's fluid leaves.
    fn remove_block(&mut self, pos: Pos) {
        let left = self.blocks.left_behind(self.get(pos));
        self.set_block(pos, left, 3);
    }

    /// `PistonBaseBlock.isPushable`.
    fn is_pushable(
        &self,
        s: u32,
        pos: Pos,
        direction: Direction,
        allow_destroy: bool,
        connection: Direction,
    ) -> bool {
        if pos[1] < self.min_y()
            || pos[1] > self.max_y()
            || !self.level.border.block_within_bounds(pos)
        {
            return false;
        }
        if self.blocks.is_air(s) {
            return true;
        }
        if direction == Direction::Down && pos[1] == self.min_y()
            || direction == Direction::Up && pos[1] == self.max_y()
        {
            return false;
        }
        if state::is(s, "minecraft:piston") || state::is(s, "minecraft:sticky_piston") {
            if state::get_bool(s, "extended") {
                return false;
            }
        } else {
            if self.blocks.destroy_time(s) == -1.0 {
                return false;
            }
            match self.behaviour(s).push_reaction() {
                IMMOVEABLE => return false,
                POPPED => return allow_destroy,
                PUSH_ONLY => return direction == connection,
                _ => {}
            }
        }
        !self.blocks.has_block_entity(s)
    }

    /// `PistonBaseBlock.moveBlocks`: the blocks become moving pistons on their way
    /// (and the head, extending), the ones in the way pop, the places left empty are
    /// cleared. False if the blocks can't move.
    fn move_blocks(
        &mut self,
        piston: Pos,
        direction: Direction,
        extending: bool,
        sticky: bool,
    ) -> bool {
        let arm = direction.step(piston);
        if !extending && state::is(self.get(arm), "minecraft:piston_head") {
            self.set_block(arm, AIR, 276);
        }
        let Some((to_push, to_destroy)) = self.resolve(piston, direction, extending) else {
            return false;
        };
        let mut delete = JavaHashMap::default();
        let shapes: Vec<u32> = to_push.iter().map(|&p| self.get(p)).collect();
        for (&p, &s) in to_push.iter().zip(&shapes) {
            delete.put(p, s);
        }
        let push_direction = if extending {
            direction
        } else {
            direction.opposite()
        };
        let mut to_update = Vec::new();
        for &p in to_destroy.iter().rev() {
            let s = self.get(p);
            // Drops and particles are the server's and the renderer's.
            self.set_block(p, AIR, 18);
            to_update.push(s);
        }
        for (i, &p) in to_push.iter().enumerate().rev() {
            let to = push_direction.step(p);
            delete.remove(to);
            let moving = state::set_dir(
                state::default_of("minecraft:moving_piston"),
                "facing",
                direction,
            );
            self.set_block(to, moving, 324);
            self.add_moving_piston(
                to,
                MovingPiston::new(shapes[i], direction, extending, false),
            );
        }
        if extending {
            let kind = if sticky { "sticky" } else { "normal" };
            let head = state::set(
                state::set_dir(
                    state::default_of("minecraft:piston_head"),
                    "facing",
                    direction,
                ),
                "type",
                kind,
            );
            delete.remove(arm);
            self.set_block(arm, moving_piston(direction, sticky), 324);
            self.add_moving_piston(arm, MovingPiston::new(head, direction, true, true));
        }
        let entries = delete.entries();
        for &(p, _) in &entries {
            self.set_block(p, AIR, 82);
        }
        let limit = crate::block::level::UPDATE_LIMIT;
        for &(p, old) in &entries {
            crate::block::update_indirect_neighbour_shapes(self, old, p, 2, limit);
            self.update_neighbour_shapes(p, AIR, 2, limit);
            crate::block::update_indirect_neighbour_shapes(self, AIR, p, 2, limit);
        }
        // `updateNeighborsAt` does nothing on the client; destroyed blocks still tell
        // their indirect neighbours.
        for (&p, &s) in to_destroy.iter().rev().zip(&to_update) {
            crate::block::update_indirect_neighbour_shapes(self, s, p, 2, limit);
        }
        true
    }

    /// `Level.setBlockEntity` for a moving piston: only where a moving piston is.
    fn add_moving_piston(&mut self, pos: Pos, mut be: MovingPiston) {
        if !state::is(self.get(pos), "minecraft:moving_piston") || !self.in_world_bounds(pos) {
            return;
        }
        refresh_piston_shapes(self.blocks, &mut be);
        self.level.block_entities.add(pos, Data::Piston(be));
    }

    /// `PistonMovingBlockEntity.finalTick` on the client: the block entity goes, and
    /// the block lands where it was going (the piston's own head or base leaves air).
    fn final_tick(&mut self, pos: Pos) {
        let Some(be) = self.level.block_entities.piston(pos).cloned() else {
            return;
        };
        self.level.block_entities.remove(pos);
        if state::is(self.get(pos), "minecraft:moving_piston") {
            let new = if be.source {
                AIR
            } else {
                self.update_from_neighbour_shapes(be.moved, pos)
            };
            self.set_block(pos, new, 3);
        }
    }

    /// `PistonMovingBlockEntity.tick`: half a block a tick, moving what is in the way;
    /// once there the client waits 5 more ticks before putting the block down (the
    /// server usually has by then).
    fn tick_moving_piston(
        &mut self,
        pos: Pos,
        targets: &mut [(Pushed<'_>, &[Aabb])],
        game_time: i64,
    ) {
        let Some(be) = self.level.block_entities.piston_mut(pos) else {
            return;
        };
        be.progress_o = be.progress;
        if be.progress_o >= 1.0 {
            if be.death_ticks < 5 {
                be.death_ticks += 1;
                return;
            }
            let moved = be.moved;
            self.level.block_entities.remove(pos);
            if state::is(self.get(pos), "minecraft:moving_piston") {
                let new = self.update_from_neighbour_shapes(moved, pos);
                if self.blocks.is_air(new) {
                    self.set_block(pos, moved, 340);
                    self.update_or_destroy(moved, new, pos, 3, crate::block::level::UPDATE_LIMIT);
                } else {
                    let new =
                        if state::has(new, "waterlogged") && state::get_bool(new, "waterlogged") {
                            state::set_bool(new, "waterlogged", false)
                        } else {
                            new
                        };
                    self.set_block(pos, new, 67);
                }
            }
            return;
        }
        let new_progress = be.progress + 0.5;
        let be = be.clone();
        for (target, colliders) in targets.iter_mut() {
            self.move_collided_entities(pos, new_progress, &be, target, colliders, game_time);
        }
        for (target, colliders) in targets.iter_mut() {
            self.move_stuck_entities(pos, new_progress, &be, target, colliders, game_time);
        }
        if let Some(be) = self.level.block_entities.piston_mut(pos) {
            be.progress = new_progress.min(1.0);
            refresh_piston_shapes(self.blocks, be);
        }
    }

    /// `moveCollidedEntities`: an entity in the space the block sweeps this tick is
    /// pushed out ahead of it (by at most the tick's half block, plus 0.01); slime
    /// launches it along the push. A retracting piston's base also pushes out an entity
    /// caught in it.
    fn move_collided_entities(
        &mut self,
        pos: Pos,
        new_progress: f32,
        be: &MovingPiston,
        target: &mut Pushed<'_>,
        colliders: &[Aabb],
        game_time: i64,
    ) {
        let movement = be.movement_direction();
        let delta_progress = f64::from(new_progress - be.progress);
        let related = collision_related_state(be);
        let shape = self.blocks.shape(related);
        let Some(bounds) = shape.bounds() else {
            return;
        };
        let aabb = move_by_progress(pos, &bounds, be);
        let area = movement_area(&aabb, movement, delta_progress).minmax(&aabb);
        if !target.found_in(&area) {
            return;
        }
        let boxes = shape.aabbs();
        if state::is(be.moved, "minecraft:slime_block") {
            // `canSimulateMovement`: the client moves these.
            let n = movement.normal();
            let axis = movement.axis() as usize;
            target.delta()[axis] = f64::from(n[axis]);
        }
        let mut delta: f64 = 0.0;
        for b in &boxes {
            let moving = movement_area(&move_by_progress(pos, b, be), movement, delta_progress);
            let entity = target.bounding_box();
            if moving.intersects(&entity) {
                delta = delta.max(get_movement(&moving, movement, &entity));
                if delta >= delta_progress {
                    break;
                }
            }
        }
        if !(delta <= 0.0) {
            let delta = delta.min(delta_progress) + 0.01;
            self.move_by_piston(movement, target, delta, movement, colliders, game_time);
            if !be.extending && be.source {
                self.fix_within_piston_base(
                    pos,
                    target,
                    movement,
                    delta_progress,
                    colliders,
                    game_time,
                );
            }
        }
    }

    /// `fixEntityWithinPistonBase`: an entity left inside the base's block is pushed
    /// out the way the piston faces.
    fn fix_within_piston_base(
        &mut self,
        pos: Pos,
        target: &mut Pushed<'_>,
        direction: Direction,
        delta_progress: f64,
        colliders: &[Aabb],
        game_time: i64,
    ) {
        let entity = target.bounding_box();
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        let block = Aabb {
            min: [0.0; 3],
            max: [1.0; 3],
        }
        .moved(at);
        if !entity.intersects(&block) {
            return;
        }
        let opposite = direction.opposite();
        let delta = get_movement(&block, opposite, &entity) + 0.01;
        let intersected = get_movement(&block, opposite, &entity.intersect(&block)) + 0.01;
        if (delta - intersected).abs() < 0.01 {
            let delta = delta.min(delta_progress) + 0.01;
            self.move_by_piston(direction, target, delta, opposite, colliders, game_time);
        }
    }

    /// `moveStuckEntities`: honey moved sideways carries an entity standing on it.
    fn move_stuck_entities(
        &mut self,
        pos: Pos,
        new_progress: f32,
        be: &MovingPiston,
        target: &mut Pushed<'_>,
        colliders: &[Aabb],
        game_time: i64,
    ) {
        if !state::is(be.moved, "minecraft:honey_block") {
            return;
        }
        let movement = be.movement_direction();
        if !movement.is_horizontal() {
            return;
        }
        let top = shape_max(self.blocks.shape(be.moved), Axis::Y);
        let area = Aabb::new([0.0, top, 0.0], [1.0, 1.5000010000000001, 1.0]);
        let aabb = move_by_progress(pos, &area, be);
        let delta_progress = f64::from(new_progress - be.progress);
        // `matchesStickyCritera`: on the ground, on this block or over the area.
        let at = target.position();
        let over = at[0] >= aabb.min[0]
            && at[0] <= aabb.max[0]
            && at[2] >= aabb.min[2]
            && at[2] <= aabb.max[2];
        if target.found_in(&aabb)
            && target.on_ground()
            && (target.supporting_block() == Some(pos) || over)
        {
            self.move_by_piston(
                movement,
                target,
                delta_progress,
                movement,
                colliders,
                game_time,
            );
        }
    }

    /// `moveEntityByPiston`: the entity moved by a piston going `piston`'s way
    /// (blocks moving that way don't stop it), then the blocks it went through.
    fn move_by_piston(
        &mut self,
        piston: Direction,
        target: &mut Pushed<'_>,
        delta: f64,
        movement: Direction,
        colliders: &[Aabb],
        game_time: i64,
    ) {
        self.level.noclip = Some(piston);
        let from = target.position();
        let n = movement.normal();
        let v = [
            delta * f64::from(n[0]),
            delta * f64::from(n[1]),
            delta * f64::from(n[2]),
        ];
        let env = Env {
            level: &*self.level,
            blocks: self.blocks,
            tags: self.tags,
            colliders,
            others: None,
        };
        target.push(&env, MoverType::Piston, v, game_time);
        let to = target.position();
        target.effects_between(&env, from, to);
        target.remove_latest_movement_recording();
        self.level.noclip = None;
    }

    // --- Shulker boxes ---------------------------------------------------------------

    /// `ShulkerBoxBlockEntity.updateAnimation`: the lid a tenth of the way a tick; an
    /// opening lid pushes what is in front of it (by the tick's sweep plus 0.01).
    fn tick_shulker_box(
        &mut self,
        pos: Pos,
        s: u32,
        targets: &mut [(Pushed<'_>, &[Aabb])],
        game_time: i64,
    ) {
        let facing = state::get_dir(s, "facing").unwrap_or(Direction::Up);
        let Some(b) = self.level.block_entities.shulker_mut(pos) else {
            return;
        };
        b.progress_old = b.progress;
        let mut updates = 0;
        let mut push = None;
        match b.lid {
            Lid::Closed => b.progress = 0.0,
            Lid::Opening => {
                b.progress += 0.1;
                if b.progress_old == 0.0 {
                    updates += 1;
                }
                if b.progress >= 1.0 {
                    b.lid = Lid::Opened;
                    b.progress = 1.0;
                    updates += 1;
                }
                push = Some((b.progress_old, b.progress));
            }
            Lid::Opened => b.progress = 1.0,
            Lid::Closing => {
                b.progress -= 0.1;
                if b.progress_old == 1.0 {
                    updates += 1;
                }
                if b.progress <= 0.0 {
                    b.lid = Lid::Closed;
                    b.progress = 0.0;
                    updates += 1;
                }
            }
        }
        b.refresh(facing);
        // `doNeighborUpdates`: the shape updates (the neighbour updates are the server's).
        for _ in 0..updates {
            self.update_neighbour_shapes(pos, s, 3, crate::block::level::UPDATE_LIMIT);
        }
        let Some((old, new)) = push else {
            return;
        };
        let bottom = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let aabb = progress_delta_aabb(1.0, facing, old, new, bottom);
        for (target, colliders) in targets.iter_mut() {
            if !target.found_in(&aabb) {
                continue;
            }
            let n = facing.normal();
            let v = [
                (aabb.size(Axis::X) + 0.01) * f64::from(n[0]),
                (aabb.size(Axis::Y) + 0.01) * f64::from(n[1]),
                (aabb.size(Axis::Z) + 0.01) * f64::from(n[2]),
            ];
            let env = Env {
                level: &*self.level,
                blocks: self.blocks,
                tags: self.tags,
                colliders,
                others: None,
            };
            target.push(&env, MoverType::ShulkerBox, v, game_time);
        }
    }

    // --- The structure a piston moves ------------------------------------------------

    /// `PistonStructureResolver.resolve`: the blocks to push (at most 12, slime and
    /// honey pulling their neighbours along) and the ones to pop; `None` if they
    /// can't move.
    fn resolve(
        &self,
        piston: Pos,
        direction: Direction,
        extending: bool,
    ) -> Option<(Vec<Pos>, Vec<Pos>)> {
        let (push_direction, start) = if extending {
            (direction, direction.step(piston))
        } else {
            (direction.opposite(), direction.step(direction.step(piston)))
        };
        let mut r = Resolver {
            w: self,
            piston,
            push_direction,
            to_push: Vec::new(),
            to_destroy: Vec::new(),
        };
        let next = self.get(start);
        if !self.is_pushable(next, start, push_direction, false, direction) {
            if extending && self.behaviour(next).push_reaction() == POPPED {
                r.to_destroy.push(start);
                return Some((r.to_push, r.to_destroy));
            }
            return None;
        }
        if !r.add_block_line(start, push_direction) {
            return None;
        }
        let mut i = 0;
        while i < r.to_push.len() {
            let p = r.to_push[i];
            if is_sticky(self.get(p)) && !r.add_branching_blocks(p) {
                return None;
            }
            i += 1;
        }
        Some((r.to_push, r.to_destroy))
    }
}

/// `PistonStructureResolver`'s work.
struct Resolver<'w, 'a> {
    w: &'w World<'a>,
    piston: Pos,
    push_direction: Direction,
    to_push: Vec<Pos>,
    to_destroy: Vec<Pos>,
}

impl Resolver<'_, '_> {
    fn add_block_line(&mut self, start: Pos, direction: Direction) -> bool {
        let w = self.w;
        let mut next = w.get(start);
        if w.blocks.is_air(next)
            || !w.is_pushable(next, start, self.push_direction, false, direction)
            || start == self.piston
            || self.to_push.contains(&start)
        {
            return true;
        }
        let mut count = 1;
        if count + self.to_push.len() > MAX_PUSH {
            return false;
        }
        let back = self.push_direction.opposite();
        while is_sticky(next) {
            let p = offset(start, back, count as i32);
            let previous = next;
            next = w.get(p);
            if w.blocks.is_air(next)
                || !can_stick_to_each_other(previous, next)
                || !w.is_pushable(next, p, self.push_direction, false, back)
                || p == self.piston
            {
                break;
            }
            count += 1;
            if count + self.to_push.len() > MAX_PUSH {
                return false;
            }
        }
        let mut added = 0;
        for i in (0..count).rev() {
            self.to_push.push(offset(start, back, i as i32));
            added += 1;
        }
        let mut i = 1;
        loop {
            let p = offset(start, self.push_direction, i);
            if let Some(collision) = self.to_push.iter().position(|&q| q == p) {
                self.reorder_at_collision(added, collision);
                for j in 0..=collision + added {
                    let q = self.to_push[j];
                    if is_sticky(w.get(q)) && !self.add_branching_blocks(q) {
                        return false;
                    }
                }
                return true;
            }
            let next = w.get(p);
            if w.blocks.is_air(next) {
                return true;
            }
            if !w.is_pushable(next, p, self.push_direction, true, self.push_direction)
                || p == self.piston
            {
                return false;
            }
            if w.behaviour(next).push_reaction() == POPPED {
                self.to_destroy.push(p);
                return true;
            }
            if self.to_push.len() >= MAX_PUSH {
                return false;
            }
            self.to_push.push(p);
            added += 1;
            i += 1;
        }
    }

    /// `reorderListAtCollision`.
    fn reorder_at_collision(&mut self, added: usize, collision: usize) {
        let len = self.to_push.len();
        let mut out = self.to_push[..collision].to_vec();
        out.extend_from_slice(&self.to_push[len - added..]);
        out.extend_from_slice(&self.to_push[collision..len - added]);
        self.to_push = out;
    }

    /// `addBranchingBlocks`: the sticky block's neighbours off the push's axis.
    fn add_branching_blocks(&mut self, from: Pos) -> bool {
        let from_state = self.w.get(from);
        for d in Direction::ALL {
            if d.axis() != self.push_direction.axis() {
                let n = d.step(from);
                if can_stick_to_each_other(self.w.get(n), from_state) && !self.add_block_line(n, d)
                {
                    return false;
                }
            }
        }
        true
    }
}

fn offset(p: Pos, d: Direction, n: i32) -> Pos {
    let s = d.normal();
    [p[0] + s[0] * n, p[1] + s[1] * n, p[2] + s[2] * n]
}

fn is_sticky(s: u32) -> bool {
    state::is(s, "minecraft:slime_block") || state::is(s, "minecraft:honey_block")
}

/// `canStickToEachOther`: slime and honey don't stick to each other.
fn can_stick_to_each_other(a: u32, b: u32) -> bool {
    let (slime, honey) = ("minecraft:slime_block", "minecraft:honey_block");
    if state::is(a, honey) && state::is(b, slime) || state::is(a, slime) && state::is(b, honey) {
        return false;
    }
    is_sticky(a) || is_sticky(b)
}

/// A moving piston block facing `direction`, of a sticky piston or not.
fn moving_piston(direction: Direction, sticky: bool) -> u32 {
    let s = state::set_dir(
        state::default_of("minecraft:moving_piston"),
        "facing",
        direction,
    );
    state::set(s, "type", if sticky { "sticky" } else { "normal" })
}

/// `getCollisionRelatedBlockState`: a retracting piston's base pushes with its head's
/// shape (short past a quarter of the way).
fn collision_related_state(be: &MovingPiston) -> u32 {
    if !be.extending && be.source && is_piston_base(be.moved) {
        let facing = state::get_dir(be.moved, "facing").unwrap_or(Direction::Down);
        let head = state::set_dir(state::default_of("minecraft:piston_head"), "facing", facing);
        let head = state::set_bool(head, "short", be.progress > 0.25);
        let sticky = state::is(be.moved, "minecraft:sticky_piston");
        state::set(head, "type", if sticky { "sticky" } else { "normal" })
    } else {
        be.moved
    }
}

fn is_piston_base(s: u32) -> bool {
    state::is(s, "minecraft:piston") || state::is(s, "minecraft:sticky_piston")
}

/// `getCollisionShape`'s two answers, made again after the block entity changed: the
/// moving block (or the piston's head) where it is now, with a retracting base's
/// extended shape; and that base's shape alone, for an entity a piston moves the same
/// way.
fn refresh_piston_shapes(blocks: &crate::blocks::Blocks, be: &mut MovingPiston) {
    let base: Option<&Shape> = (!be.extending && be.source && is_piston_base(be.moved))
        .then(|| blocks.shape(state::set_bool(be.moved, "extended", true)));
    be.noclip_shape = base.cloned().unwrap_or_else(Shape::empty);
    let s = if be.source {
        let head = state::set_dir(
            state::default_of("minecraft:piston_head"),
            "facing",
            be.direction,
        );
        state::set_bool(head, "short", be.extending != (1.0 - be.progress < 0.25))
    } else {
        be.moved
    };
    let ext = be.extended_progress(be.progress);
    let n = be.direction.normal();
    let d: [f64; 3] = std::array::from_fn(|i| f64::from(n[i] as f32 * ext));
    // `Shapes.or`: the union, optimized (`forAllBoxes` of the joined shape).
    let mut boxes: Vec<Aabb> = base.map(Shape::aabbs).unwrap_or_default();
    boxes.extend(blocks.shape(s).aabbs().into_iter().map(|b| b.moved(d)));
    be.shape = Shape::from_boxes(&Shape::from_boxes(&boxes).aabbs());
}

/// `moveByPositionAndProgress`: a box of the block's shape where the block is now.
fn move_by_progress(pos: Pos, b: &Aabb, be: &MovingPiston) -> Aabb {
    let current = f64::from(be.extended_progress(be.progress));
    let n = be.direction.normal();
    b.moved(std::array::from_fn(|i| {
        f64::from(pos[i]) + current * f64::from(n[i])
    }))
}

/// `PistonMath.getMovementArea`: the slab `amount` thick the box's face towards
/// `direction` sweeps.
fn movement_area(b: &Aabb, direction: Direction, amount: f64) -> Aabb {
    let step = if direction.is_positive() { 1.0 } else { -1.0 };
    let delta = amount * step;
    let lo = delta.min(0.0);
    let hi = delta.max(0.0);
    let (mut min, mut max) = (b.min, b.max);
    let a = direction.axis() as usize;
    let face = if direction.is_positive() {
        b.max[a]
    } else {
        b.min[a]
    };
    min[a] = face + lo;
    max[a] = face + hi;
    Aabb::new(min, max)
}

/// `getMovement`: how far `b` must go along `movement` to be out of `outside`.
fn get_movement(outside: &Aabb, movement: Direction, b: &Aabb) -> f64 {
    let a = movement.axis() as usize;
    if movement.is_positive() {
        outside.max[a] - b.min[a]
    } else {
        b.max[a] - outside.min[a]
    }
}

/// A `HashMap<BlockPos, BlockState>` as Java iterates it: by bucket (the position's
/// `hashCode`, spread, in a table of 16, which twelve entries never outgrow), then in
/// the order put.
#[derive(Default)]
struct JavaHashMap {
    entries: Vec<(Pos, u32)>,
}

impl JavaHashMap {
    fn put(&mut self, p: Pos, s: u32) {
        match self.entries.iter_mut().find(|(q, _)| *q == p) {
            Some(e) => e.1 = s,
            None => self.entries.push((p, s)),
        }
    }

    fn remove(&mut self, p: Pos) {
        self.entries.retain(|(q, _)| *q != p);
    }

    fn entries(&self) -> Vec<(Pos, u32)> {
        let bucket = |p: &Pos| {
            // `Vec3i.hashCode`, then `HashMap.hash`.
            let h = p[1]
                .wrapping_add(p[2].wrapping_mul(31))
                .wrapping_mul(31)
                .wrapping_add(p[0]) as u32;
            (h ^ (h >> 16)) & 15
        };
        let mut out = self.entries.clone();
        out.sort_by_key(|(p, _)| bucket(p));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_areas_sweep_the_leading_face() {
        let b = Aabb::new([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let east = movement_area(&b, Direction::East, 0.5);
        assert_eq!((east.min[0], east.max[0]), (1.0, 1.5));
        let down = movement_area(&b, Direction::Down, 0.5);
        assert_eq!((down.min[1], down.max[1]), (-0.5, 0.0));
        let player = Aabb::new([1.2, 0.0, 0.2], [1.8, 1.8, 0.8]);
        assert!((get_movement(&east, Direction::East, &player) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn java_hash_map_order() {
        let mut m = JavaHashMap::default();
        // Hashes 31*31*1 + 0 = 961 (bucket 1) and 1 (bucket 1): put order kept; 2 (bucket 2).
        m.put([0, 0, 1], 1);
        m.put([2, 0, 0], 2);
        m.put([1, 0, 0], 3);
        let order: Vec<u32> = m.entries().into_iter().map(|e| e.1).collect();
        assert_eq!(order, [1, 3, 2]);
    }
}
