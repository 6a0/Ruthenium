//! The vehicles the local player rides, as the client runs them: a vehicle the player
//! controls moves on the client (`isLocalInstanceAuthoritative`), with the physics of
//! its kind, and its rider sits where it says. Ported from vanilla's `Entity` (its
//! generic `move`), `LivingEntity`, `AbstractBoat`, `AbstractHorse` and
//! `HappyGhast`.
//!
//! Boats (rafts and chest boats too): the water, under-water and land statuses, floating,
//! paddling and turning, land friction, bubble column events. The horse family (horses,
//! donkeys, mules, skeleton and zombie horses): steering by the rider, the speed
//! attribute, step height 1, charged jumps, rearing. Happy ghasts in a harness: flying
//! where the rider looks, rising with jump. Not simulated: camels, pigs, striders,
//! nautiluses and minecarts the player steers (they wait for the server).

// Negated float comparisons are kept as the game writes them (NaN).
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ruthenium_javamath::mth;
use ruthenium_protocol::packets::play;

use crate::block::Class;
use crate::blocks::Who;
use crate::entity::Entity;
use crate::fluid::FluidInteraction;
use crate::player::{
    Attributes, DEG_TO_RAD_F, Env, Inside, InsideMover, Movement, MoverType, collide, distance_sqr,
    find_supporting_block, modified_friction,
};
use crate::shape::Aabb;

/// What kind of vehicle an entity type is, for the client's simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RideKind {
    /// Not one the client moves.
    None,
    /// `AbstractBoat`: the passengers' height as a fraction of its own
    /// (`rideHeight`), a lone passenger's offset forward, and how many it carries.
    Boat {
        ride_height: f32,
        x_offset: f32,
        max_passengers: usize,
    },
    /// `AbstractHorse` (not camels or llamas): whether it rears.
    Horse {
        rearing: bool,
    },
    HappyGhast,
}

/// `AbstractBoat.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}

/// The local player as the controlling passenger, as its last tick left it (the vehicle
/// ticks before its passengers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Controller {
    pub(crate) xxa: f32,
    pub(crate) zza: f32,
    pub(crate) jumping: bool,
    pub(crate) y_rot: f32,
    pub(crate) x_rot: f32,
}

/// The physics state of an entity the client can move.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Body {
    pub(crate) kind: RideKind,
    pub(crate) horizontal_collision: bool,
    pub(crate) vertical_collision: bool,
    vertical_collision_below: bool,
    pub(crate) fall_distance: f64,
    supporting_block: Option<[i32; 3]>,
    on_ground_no_blocks: bool,
    stuck_speed_multiplier: [f64; 3],
    movement_this_tick: Vec<Movement>,
    piston_deltas: [f64; 3],
    piston_deltas_game_time: i64,
    fluids: FluidInteraction,
    /// `wasTouchingWater`.
    in_water: bool,
    first_tick: bool,
    in_powder_snow: bool,
    was_in_powder_snow: bool,
    /// A living vehicle's attributes, as the server sends them.
    pub(crate) attributes: Attributes,
    no_jump_delay: i32,
    /// `LivingEntity.speed`.
    speed: f32,
    // AbstractBoat.
    status: Option<Status>,
    old_status: Option<Status>,
    out_of_control_ticks: f32,
    pub(crate) delta_rotation: f32,
    /// The rider's keys, as its last tick set them: left, right, forward, backward.
    pub(crate) input: [bool; 4],
    water_level: f64,
    land_friction: f32,
    last_yd: f64,
    paddles: (bool, bool),
    // AbstractHorse.
    jump_pending_scale: f32,
    allow_stand_sliding: bool,
    stand_counter: i32,
    stand_anim: f32,
    pub(crate) stand_anim_o: f32,
}

impl Body {
    pub(crate) fn new(kind: RideKind) -> Body {
        Body {
            kind,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fall_distance: 0.0,
            supporting_block: None,
            on_ground_no_blocks: false,
            stuck_speed_multiplier: [0.0; 3],
            movement_this_tick: Vec::new(),
            piston_deltas: [0.0; 3],
            piston_deltas_game_time: 0,
            fluids: FluidInteraction::default(),
            in_water: false,
            first_tick: true,
            in_powder_snow: false,
            was_in_powder_snow: false,
            attributes: Attributes::default(),
            no_jump_delay: 0,
            speed: 0.0,
            status: None,
            old_status: None,
            out_of_control_ticks: 0.0,
            delta_rotation: 0.0,
            input: [false; 4],
            water_level: 0.0,
            land_friction: 0.0,
            last_yd: 0.0,
            paddles: (false, false),
            jump_pending_scale: 0.0,
            allow_stand_sliding: false,
            stand_counter: 0,
            stand_anim: 0.0,
            stand_anim_o: 0.0,
        }
    }
}

impl Body {
    /// `AbstractBoat.isUnderWater`.
    pub(crate) fn is_under_water(&self) -> bool {
        matches!(
            self.status,
            Some(Status::UnderWater | Status::UnderFlowingWater)
        )
    }
}

impl Entity {
    /// What clicking a rideable entity does on the client (`Entity.interact`): `Some`
    /// with whether it takes the click (`SUCCESS`: the next hand isn't tried), `None`
    /// for other entities. A boat takes it unless the player sneaks or it has been under
    /// water too long; a grown horse that carries no one takes it (riding it, opening its
    /// inventory, or what the held item does with it); a harnessed happy ghast unless the
    /// player sneaks. Items held aren't looked into further: a horse's food, chests and
    /// armour all take the click as well.
    pub(crate) fn rideable_interaction(&self, secondary_use: bool) -> Option<bool> {
        let body = self.body.as_deref()?;
        Some(match body.kind {
            RideKind::None => return None,
            RideKind::Boat { .. } => !secondary_use && body.out_of_control_ticks < 60.0,
            RideKind::Horse { .. } => self.alive() && self.passengers.is_empty() && !self.baby,
            RideKind::HappyGhast => self.alive() && !self.baby && self.body_armor && !secondary_use,
        })
    }
}

/// `AbstractHorse`'s flag bits.
const HORSE_EATING: u8 = 16;
const HORSE_STANDING: u8 = 32;

impl Entity {
    fn body(&self) -> &Body {
        self.body.as_deref().expect("a vehicle's body")
    }

    fn body_mut(&mut self) -> &mut Body {
        self.body.as_deref_mut().expect("a vehicle's body")
    }

    fn living(&self) -> bool {
        self.info.living
    }

    /// `isAlive`.
    fn alive(&self) -> bool {
        !self.removed && (!self.living() || self.health > 0.0)
    }

    pub(crate) fn block_position(&self) -> [i32; 3] {
        self.position.map(mth::floor)
    }

    /// The entity as a collision query's entity (it never descends: it sneaks not).
    fn who(&self) -> Who {
        Who {
            bottom: self.position[1],
            descending: false,
            fall_distance: self.body().fall_distance,
            walks_on_powder_snow: false,
        }
    }

    /// `getGravity` (`getDefaultGravity`): a boat's 0.04, a living one's attribute.
    fn gravity(&self) -> f64 {
        match self.body().kind {
            RideKind::Boat { .. } => 0.04,
            _ => self.body().attributes.gravity.value(),
        }
    }

    /// `maxUpStep`: a living vehicle steps at least a block under a player.
    fn max_up_step(&self, controlled: bool) -> f32 {
        if !self.living() {
            return 0.0;
        }
        let step = self.body().attributes.step_height.value() as f32;
        if controlled { step.max(1.0) } else { step }
    }

    /// The eyes' height (`getEyeY`).
    fn eye_y(&self) -> f64 {
        self.position[1]
            + f64::from(self.info.eye_height * self.size.height / self.info.size.height)
    }

    /// `Entity.baseTick`'s movement parts: powder snow's flags, the fluids (with the
    /// currents' push), lava halving a fall.
    fn base_tick(&mut self, env: &Env<'_>) {
        let b = self.bounding_box;
        let eye = (
            mth::floor(self.position[0]),
            self.eye_y(),
            mth::floor(self.position[2]),
        );
        let body = self.body.as_deref_mut().expect("body");
        body.was_in_powder_snow = body.in_powder_snow;
        body.in_powder_snow = false;
        update_fluids(body, env, &b, eye, &mut self.delta);
        if !body.first_tick && body.fluids.is_in(false) {
            body.fall_distance *= 0.5;
        }
        body.first_tick = false;
    }

    /// One tick of a vehicle the client runs, after `commonTick`: `controller` is the
    /// local player when it controls it. Packets it sends go to `out`.
    pub(crate) fn vehicle_tick(
        &mut self,
        env: &Env<'_>,
        controller: Option<Controller>,
        out: &mut Vec<Vec<u8>>,
    ) {
        match self.body().kind {
            RideKind::None => {}
            RideKind::Boat { .. } => self.boat_tick(env, controller.is_some(), out),
            RideKind::Horse { .. } => {
                match controller {
                    Some(c) => self.living_tick(env, c),
                    None => self.living_idle(),
                }
                self.horse_animation();
            }
            RideKind::HappyGhast => match controller {
                Some(c) => self.living_tick(env, c),
                None => self.living_idle(),
            },
        }
    }

    // --- AbstractBoat ---------------------------------------------------------------

    /// `AbstractBoat.tick` (hurt time, damage, sounds and the bubbles' look aside).
    fn boat_tick(&mut self, env: &Env<'_>, controlled: bool, out: &mut Vec<Vec<u8>>) {
        let status = self.boat_status(env);
        let body = self.body_mut();
        body.old_status = body.status;
        body.status = Some(status);
        if matches!(status, Status::UnderWater | Status::UnderFlowingWater) {
            body.out_of_control_ticks += 1.0;
        } else {
            body.out_of_control_ticks = 0.0;
        }
        self.base_tick(env);
        if controlled {
            self.float_boat(env);
            self.control_boat();
            let (left, right) = self.body().paddles;
            out.push(play::paddle_boat(left, right));
            let delta = self.delta;
            self.entity_move(env, MoverType::SelfMove, delta, 0, true);
        } else {
            self.delta = [0.0; 3];
            self.body_mut().delta_rotation = 0.0;
        }
        self.apply_effects_from_blocks(env, controlled);
        self.apply_effects_from_blocks(env, controlled);
    }

    /// `getStatus`.
    fn boat_status(&mut self, env: &Env<'_>) -> Status {
        if let Some(s) = self.under_water(env) {
            self.body_mut().water_level = self.bounding_box.max[1];
            return s;
        }
        if self.check_in_water(env) {
            return Status::InWater;
        }
        let friction = self.ground_friction(env);
        if friction > 0.0 {
            self.body_mut().land_friction = friction;
            Status::OnLand
        } else {
            Status::InAir
        }
    }

    /// `getWaterLevelAbove`.
    fn water_level_above(&self, env: &Env<'_>) -> f32 {
        let b = self.bounding_box;
        let min_x = mth::floor(b.min[0]);
        let max_x = mth::ceil(b.max[0]);
        let min_y = mth::floor(b.max[1]);
        let max_y = mth::ceil(b.max[1] - self.body().last_yd);
        let min_z = mth::floor(b.min[2]);
        let max_z = mth::ceil(b.max[2]);
        'rows: for y in min_y..max_y {
            let mut height = 0.0f32;
            for x in min_x..max_x {
                for z in min_z..max_z {
                    let p = [x, y, z];
                    let f = env.fluid(p);
                    if f.fluid.is_water() {
                        height = height.max(env.fluid_height(p, f));
                    }
                    if height >= 1.0 {
                        continue 'rows;
                    }
                }
            }
            if height < 1.0 {
                // The position's last value: the row's last block.
                return y as f32 + height;
            }
        }
        (max_y + 1) as f32
    }

    /// `getGroundFriction`: the blocks just under the boat (not lily pads), their
    /// friction averaged; the corners of the area left out.
    fn ground_friction(&self, env: &Env<'_>) -> f32 {
        let bb = self.bounding_box;
        let area = Aabb {
            min: [bb.min[0], bb.min[1] - 0.001, bb.min[2]],
            max: [bb.max[0], bb.min[1], bb.max[2]],
        };
        let x0 = mth::floor(area.min[0]) - 1;
        let x1 = mth::ceil(area.max[0]) + 1;
        let y0 = mth::floor(area.min[1]) - 1;
        let y1 = mth::ceil(area.max[1]) + 1;
        let z0 = mth::floor(area.min[2]) - 1;
        let z1 = mth::ceil(area.max[2]) + 1;
        let mut friction = 0.0f32;
        let mut count = 0;
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = i32::from(x == x0 || x == x1 - 1) + i32::from(z == z0 || z == z1 - 1);
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges > 0 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let state = env.state([x, y, z]);
                    if env.blocks.is_class(state, Class::LilyPadBlock) {
                        continue;
                    }
                    if env
                        .blocks
                        .collision(state, [x, y, z])
                        .is_some_and(|p| p.overlaps(&area))
                    {
                        friction += env.blocks.friction(state);
                        count += 1;
                    }
                }
            }
        }
        friction / count as f32
    }

    /// `checkInWater`: water around the boat's bottom, its level kept.
    fn check_in_water(&mut self, env: &Env<'_>) -> bool {
        let bb = self.bounding_box;
        let min_x = mth::floor(bb.min[0]);
        let max_x = mth::ceil(bb.max[0]);
        let min_y = mth::floor(bb.min[1]);
        let max_y = mth::ceil(bb.min[1] + 0.001);
        let min_z = mth::floor(bb.min[2]);
        let max_z = mth::ceil(bb.max[2]);
        let mut in_water = false;
        let mut level = -1.7976931348623157E308;
        for x in min_x..max_x {
            for y in min_y..max_y {
                for z in min_z..max_z {
                    let p = [x, y, z];
                    let f = env.fluid(p);
                    if f.fluid.is_water() {
                        let height = y as f32 + env.fluid_height(p, f);
                        level = f64::from(height).max(level);
                        in_water |= bb.min[1] < f64::from(height);
                    }
                }
            }
        }
        self.body_mut().water_level = level;
        in_water
    }

    /// `isUnderwater`: water over the boat's top; flowing water first.
    fn under_water(&self, env: &Env<'_>) -> Option<Status> {
        let bb = self.bounding_box;
        let max_y = bb.max[1] + 0.001;
        let x0 = mth::floor(bb.min[0]);
        let x1 = mth::ceil(bb.max[0]);
        let y0 = mth::floor(bb.max[1]);
        let y1 = mth::ceil(max_y);
        let z0 = mth::floor(bb.min[2]);
        let z1 = mth::ceil(bb.max[2]);
        let mut under = false;
        for x in x0..x1 {
            for y in y0..y1 {
                for z in z0..z1 {
                    let p = [x, y, z];
                    let f = env.fluid(p);
                    if f.fluid.is_water() && max_y < f64::from(y as f32 + env.fluid_height(p, f)) {
                        if !f.is_source() {
                            return Some(Status::UnderFlowingWater);
                        }
                        under = true;
                    }
                }
            }
        }
        under.then_some(Status::UnderWater)
    }

    /// `floatBoat`.
    fn float_boat(&mut self, env: &Env<'_>) {
        let mut vspeed = -self.gravity();
        let mut buoyancy = 0.0;
        let mut inv_friction = 0.05f32;
        let (old, status) = (self.body().old_status, self.body().status);
        if old == Some(Status::InAir) && !matches!(status, Some(Status::InAir | Status::OnLand)) {
            // Landing in water: onto its surface.
            self.body_mut().water_level = self.position[1] + f64::from(self.size.height);
            // getWaterLevelAbove() - getBbHeight() in floats.
            let target_y = f64::from(self.water_level_above(env) - self.size.height) + 0.101;
            let moved = self
                .bounding_box
                .moved([0.0, target_y - self.position[1], 0.0]);
            if env.no_collision_for(&moved, Some(self.who())) {
                self.set_pos([self.position[0], target_y, self.position[2]]);
                self.delta[1] *= 0.0;
                self.body_mut().last_yd = 0.0;
            }
            self.body_mut().status = Some(Status::InWater);
            return;
        }
        let land_friction = self.body().land_friction;
        match status {
            Some(Status::InWater) => {
                buoyancy =
                    (self.body().water_level - self.position[1]) / f64::from(self.size.height);
                inv_friction = 0.9;
            }
            Some(Status::UnderFlowingWater) => {
                vspeed = -7.0E-4;
                inv_friction = 0.9;
            }
            Some(Status::UnderWater) => {
                buoyancy = 0.009999999776482582;
                inv_friction = 0.45;
            }
            Some(Status::InAir) => inv_friction = 0.9,
            Some(Status::OnLand) => {
                inv_friction = land_friction;
                // A player rides it (the controller is the local player).
                self.body_mut().land_friction /= 2.0;
            }
            None => {}
        }
        let f = f64::from(inv_friction);
        self.delta = [self.delta[0] * f, self.delta[1] + vspeed, self.delta[2] * f];
        self.body_mut().delta_rotation *= inv_friction;
        if buoyancy > 0.0 {
            self.delta[1] = (self.delta[1] + buoyancy * (0.04 / 0.65)) * 0.75;
        }
    }

    /// `controlBoat`: the rider's keys turn it and push it along its heading.
    fn control_boat(&mut self) {
        let [left, right, up, down] = self.body().input;
        let mut acceleration = 0.0f32;
        let body = self.body_mut();
        if left {
            body.delta_rotation -= 1.0;
        }
        if right {
            body.delta_rotation += 1.0;
        }
        if right != left && !up && !down {
            acceleration += 0.005;
        }
        let y_rot = self.y_rot + self.body().delta_rotation;
        self.set_y_rot(y_rot);
        if up {
            acceleration += 0.04;
        }
        if down {
            acceleration -= 0.005;
        }
        let sin = mth::sin(f64::from(-self.y_rot * DEG_TO_RAD_F));
        let cos = mth::cos(f64::from(self.y_rot * DEG_TO_RAD_F));
        // Vec3.add: the y gains 0.0 too (which makes a -0.0 a 0.0).
        self.delta = [
            self.delta[0] + f64::from(sin * acceleration),
            self.delta[1] + 0.0,
            self.delta[2] + f64::from(cos * acceleration),
        ];
        self.body_mut().paddles = (right && !left || up, left && !right || up);
    }

    /// `AbstractBoat.checkFallDamage`.
    fn boat_fall(&mut self, env: &Env<'_>, ya: f64) {
        let on_ground = self.on_ground;
        let below = self.block_position();
        let water_below = env
            .fluid([below[0], below[1] - 1, below[2]])
            .fluid
            .is_water();
        let last_yd = self.delta[1];
        let body = self.body_mut();
        body.last_yd = last_yd;
        if on_ground {
            body.fall_distance = 0.0;
        } else if !water_below && ya < 0.0 {
            body.fall_distance -= f64::from(ya as f32);
        }
    }

    /// The boat's entity events: the bubble column it floats over throwing it up (72)
    /// or dragging it down (71), when the client moves it.
    pub(crate) fn boat_event(&mut self, event: u8, controlled: bool) {
        if !controlled {
            return;
        }
        match event {
            71 => {
                self.delta = [
                    self.delta[0] + 0.0,
                    self.delta[1] - 0.7,
                    self.delta[2] + 0.0,
                ]
            }
            72 => self.delta[1] = 0.6,
            _ => {}
        }
    }

    // --- Living vehicles ------------------------------------------------------------

    /// `LivingEntity.tick` (`baseTick`, `aiStep`) for a vehicle the local player
    /// controls.
    fn living_tick(&mut self, env: &Env<'_>, c: Controller) {
        self.base_tick(env);
        let body = self.body_mut();
        if body.no_jump_delay > 0 {
            body.no_jump_delay -= 1;
        }
        // Small motion stops (on each axis, for all but players).
        let [mut dx, mut dy, mut dz] = self.delta;
        if dx.abs() < 0.003 {
            dx = 0.0;
        }
        if dy.abs() < 0.003 {
            dy = 0.0;
        }
        if dz.abs() < 0.003 {
            dz = 0.0;
        }
        self.delta = [dx, dy, dz];
        // The mob itself never jumps on the client.
        self.body_mut().no_jump_delay = 0;
        // travelRidden.
        let input = self.ridden_input(c);
        self.tick_ridden(env, c, input);
        self.body_mut().speed = self.body().attributes.movement_speed.value() as f32;
        self.ridden_travel(env, input);
        self.apply_effects_from_blocks(env, true);
    }

    /// `LivingEntity.aiStep`'s motion for a mob the client doesn't move: it slows
    /// while it isn't interpolating, and small motion stops.
    fn living_idle(&mut self) {
        let body = self.body_mut();
        if body.no_jump_delay > 0 {
            body.no_jump_delay -= 1;
        }
        if !self.is_interpolating() {
            self.delta = self.delta.map(|v| v * 0.98);
        }
        let [mut dx, mut dy, mut dz] = self.delta;
        if dx.abs() < 0.003 {
            dx = 0.0;
        }
        if dy.abs() < 0.003 {
            dy = 0.0;
        }
        if dz.abs() < 0.003 {
            dz = 0.0;
        }
        self.delta = [dx, dy, dz];
    }

    /// `getRiddenInput`.
    fn ridden_input(&self, c: Controller) -> [f64; 3] {
        match self.body().kind {
            RideKind::Horse { .. } => {
                let b = self.body();
                if self.on_ground
                    && b.jump_pending_scale == 0.0
                    && self.horse_flags & HORSE_STANDING != 0
                    && !b.allow_stand_sliding
                {
                    return [0.0; 3];
                }
                let sideways = c.xxa * 0.5;
                let mut forward = c.zza;
                if forward <= 0.0 {
                    forward *= 0.25;
                }
                [f64::from(sideways), 0.0, f64::from(forward)]
            }
            RideKind::HappyGhast => {
                let strafe = c.xxa;
                let mut forward = 0.0f32;
                let mut up = 0.0f32;
                if c.zza != 0.0 {
                    let mut forward_look = mth::cos(f64::from(c.x_rot * DEG_TO_RAD_F));
                    let mut up_look = -mth::sin(f64::from(c.x_rot * DEG_TO_RAD_F));
                    if c.zza < 0.0 {
                        forward_look *= -0.5;
                        up_look *= -0.5;
                    }
                    up = up_look;
                    forward = forward_look;
                }
                if c.jumping {
                    up += 0.5;
                }
                let s = 3.9000000953674316 * self.body().attributes.flying_speed.value();
                [
                    f64::from(strafe) * s,
                    f64::from(up) * s,
                    f64::from(forward) * s,
                ]
            }
            _ => [0.0; 3],
        }
    }

    /// `tickRidden`: the vehicle turns as its rider looks; a horse on the ground jumps
    /// when its rider let go of a charged jump.
    fn tick_ridden(&mut self, env: &Env<'_>, c: Controller, input: [f64; 3]) {
        match self.body().kind {
            RideKind::Horse { .. } => {
                self.set_rot(c.y_rot, c.x_rot * 0.5);
                if self.on_ground {
                    let pending = self.body().jump_pending_scale;
                    if pending > 0.0 {
                        self.execute_riders_jump(env, pending, input);
                    }
                    self.body_mut().jump_pending_scale = 0.0;
                }
            }
            RideKind::HappyGhast => {
                let diff = mth::wrap_degrees(c.y_rot - self.y_rot);
                let y_rot = self.y_rot + diff * 0.08;
                self.set_rot(y_rot, c.x_rot * 0.5);
            }
            _ => {}
        }
    }

    /// `AbstractHorse.executeRidersJump`.
    fn execute_riders_jump(&mut self, env: &Env<'_>, amount: f32, input: [f64; 3]) {
        let impulse = self.jump_power(env, amount);
        self.delta[1] = f64::from(impulse);
        if input[2] > 0.0 {
            let sin = mth::sin(f64::from(self.y_rot * DEG_TO_RAD_F));
            let cos = mth::cos(f64::from(self.y_rot * DEG_TO_RAD_F));
            self.delta = [
                self.delta[0] + f64::from(-0.4 * sin * amount),
                self.delta[1] + 0.0,
                self.delta[2] + f64::from(0.4 * cos * amount),
            ];
        }
    }

    /// `getJumpPower(multiplier)` (no jump boost: mobs' effects aren't tracked).
    fn jump_power(&self, env: &Env<'_>, multiplier: f32) -> f32 {
        let strength = self.body().attributes.jump_strength.value() as f32;
        strength * multiplier * self.block_jump_factor(env)
    }

    /// `PlayerRideableJumping.onPlayerJump` of a saddled horse: the rider let go of
    /// a jump charged to `amount` (0 to 100).
    pub(crate) fn on_player_jump(&mut self, amount: i32) {
        if !matches!(self.body().kind, RideKind::Horse { .. }) || !self.saddled {
            return;
        }
        let amount = if amount < 0 {
            0
        } else {
            self.body_mut().allow_stand_sliding = true;
            self.stand_if_possible();
            amount
        };
        self.body_mut().jump_pending_scale = if amount >= 90 {
            1.0
        } else {
            0.4 + 0.4 * amount as f32 / 90.0
        };
    }

    /// `standIfPossible` on the client that moves the horse (`isEffectiveAi`: its AI
    /// isn't off): it rears for 20 ticks.
    fn stand_if_possible(&mut self) {
        if let RideKind::Horse { rearing: true } = self.body().kind
            && !self.no_ai
        {
            self.horse_flags = (self.horse_flags & !HORSE_EATING) | HORSE_STANDING;
            self.body_mut().stand_counter = 20;
        }
    }

    /// `AbstractHorse.tick`'s rearing, after the move: the counter, and the rear's
    /// animation (which lifts the rider).
    fn horse_animation(&mut self) {
        let body = self.body.as_deref_mut().expect("body");
        if body.stand_counter > 0 {
            body.stand_counter -= 1;
            if body.stand_counter <= 0 {
                self.horse_flags &= !HORSE_STANDING;
                body.stand_counter = 0;
            }
        }
        let standing = self.horse_flags & HORSE_STANDING != 0;
        body.stand_anim_o = body.stand_anim;
        if standing {
            body.stand_anim += (1.0 - body.stand_anim) * 0.4 + 0.05;
            if body.stand_anim > 1.0 {
                body.stand_anim = 1.0;
            }
        } else {
            body.allow_stand_sliding = false;
            let s = body.stand_anim;
            body.stand_anim = s + ((0.8 * s * s * s - s) * 0.6 - 0.05);
            if body.stand_anim < 0.0 {
                body.stand_anim = 0.0;
            }
        }
    }

    /// The vehicle's `travel` with the rider's input.
    fn ridden_travel(&mut self, env: &Env<'_>, input: [f64; 3]) {
        match self.body().kind {
            RideKind::HappyGhast => {
                let speed = self.body().attributes.flying_speed.value() as f32 * 5.0 / 3.0;
                let scale = if self.body().in_water {
                    0.800000011920929
                } else if self.in_lava() {
                    0.5
                } else {
                    0.9100000262260437
                };
                self.move_relative(speed, input);
                let delta = self.delta;
                self.entity_move(env, MoverType::SelfMove, delta, 0, true);
                self.delta = self.delta.map(|v| v * scale);
            }
            _ => self.living_travel(env, input),
        }
    }

    fn in_lava(&self) -> bool {
        let b = self.body();
        !b.first_tick && b.fluids.is_in(false)
    }

    /// `LivingEntity.travel` for a ridden mob.
    fn living_travel(&mut self, env: &Env<'_>, input: [f64; 3]) {
        if self.body().in_water || self.in_lava() {
            self.travel_in_fluid(env, input);
        } else {
            self.travel_in_air(env, input);
        }
    }

    /// `travelInAir`.
    fn travel_in_air(&mut self, env: &Env<'_>, input: [f64; 3]) {
        let below = self.on_pos(env, 0.500001);
        let attrs = self.body().attributes.clone();
        let block_friction = if self.on_ground {
            modified_friction(
                env.blocks.friction(env.state(below)),
                attrs.friction_modifier.value() as f32,
            )
        } else {
            1.0
        };
        // handleRelativeFrictionAndCalculateMovement.
        let speed = if self.on_ground {
            let own = self.body().speed;
            if f64::from(block_friction) > 0.6 {
                own * (0.21600002 / (block_friction * block_friction * block_friction))
            } else {
                own
            }
        } else {
            // getFlyingSpeed: a player rides it.
            self.body().speed * 0.1
        };
        self.move_relative(speed, input);
        let delta = self.delta;
        self.entity_move(env, MoverType::SelfMove, delta, 0, true);
        let movement = self.delta;
        let movement_y = if env.has_chunk_at(below) {
            movement[1] - self.gravity()
        } else if self.position[1] > f64::from(env.level.min_y()) {
            -0.1
        } else {
            0.0
        };
        let air_drag = modified_friction(0.91, attrs.air_drag_modifier.value() as f32);
        let friction = f64::from(block_friction * air_drag);
        let vertical = f64::from(modified_friction(
            0.98,
            attrs.air_drag_modifier.value() as f32,
        ));
        self.delta = [
            movement[0] * friction,
            movement_y * vertical,
            movement[2] * friction,
        ];
    }

    /// `travelInFluid`.
    fn travel_in_fluid(&mut self, env: &Env<'_>, input: [f64; 3]) {
        let is_falling = self.delta[1] <= 0.0;
        let old_y = self.position[1];
        let gravity = self.gravity();
        if self.body().in_water {
            // travelInWater (no sprinting, dolphins or climbing for these).
            let mut slow_down = 0.8f32;
            let mut speed = 0.02f32;
            let mut water_walker = self.body().attributes.water_movement_efficiency.value() as f32;
            if !self.on_ground {
                water_walker *= 0.5;
            }
            if water_walker > 0.0 {
                slow_down += (0.54600006 - slow_down) * water_walker;
                speed += (self.body().speed - speed) * water_walker;
            }
            self.move_relative(speed, input);
            let delta = self.delta;
            self.entity_move(env, MoverType::SelfMove, delta, 0, true);
            let m = self.delta;
            let m = [
                m[0] * f64::from(slow_down),
                m[1] * 0.800000011920929,
                m[2] * f64::from(slow_down),
            ];
            self.delta = fluid_falling_adjusted(gravity, is_falling, m);
        } else {
            self.move_relative(0.02, input);
            let delta = self.delta;
            self.entity_move(env, MoverType::SelfMove, delta, 0, true);
            if self.body().fluids.height(false) <= self.fluid_jump_threshold() {
                let m = self.delta;
                let m = [m[0] * 0.5, m[1] * 0.800000011920929, m[2] * 0.5];
                self.delta = fluid_falling_adjusted(gravity, is_falling, m);
            } else {
                self.delta = self.delta.map(|v| v * 0.5);
            }
            if gravity != 0.0 {
                self.delta[1] += -gravity / 4.0;
            }
        }
        // jumpOutOfFluid.
        let m = self.delta;
        if self.body().horizontal_collision
            && self.is_free(
                env,
                [
                    m[0],
                    m[1] + 0.6000000238418579 - self.position[1] + old_y,
                    m[2],
                ],
            )
        {
            self.delta[1] = 0.30000001192092896;
        }
        // floatInLiquidWhileRidden.
        if self.info.float_while_ridden
            && self
                .body()
                .fluids
                .height(true)
                .max(self.body().fluids.height(false))
                > self.fluid_jump_threshold()
        {
            self.delta[1] += 0.03999999910593033;
        }
    }

    /// `getFluidJumpThreshold`.
    fn fluid_jump_threshold(&self) -> f64 {
        let eye = self.info.eye_height * self.size.height / self.info.size.height;
        if f64::from(eye) < 0.4 { 0.0 } else { 0.4 }
    }

    /// `isFree`: the box moved by `d` collides with nothing and holds no liquid.
    fn is_free(&self, env: &Env<'_>, d: [f64; 3]) -> bool {
        let b = self.bounding_box.moved(d);
        env.no_collision_for(&b, Some(self.who())) && !env.contains_any_liquid(&b)
    }

    /// `Entity.moveRelative` with `getInputVector`.
    fn move_relative(&mut self, speed: f32, input: [f64; 3]) {
        let length = input[0] * input[0] + input[1] * input[1] + input[2] * input[2];
        if length < 1.0E-7 {
            return;
        }
        let v = if length > 1.0 {
            let l = length.sqrt();
            if l < 1.0E-5 {
                [0.0; 3]
            } else {
                [input[0] / l, input[1] / l, input[2] / l]
            }
        } else {
            input
        };
        let s = f64::from(speed);
        let m = [v[0] * s, v[1] * s, v[2] * s];
        let sin = f64::from(mth::sin(f64::from(self.y_rot * DEG_TO_RAD_F)));
        let cos = f64::from(mth::cos(f64::from(self.y_rot * DEG_TO_RAD_F)));
        self.delta[0] += m[0] * cos - m[2] * sin;
        self.delta[1] += m[1];
        self.delta[2] += m[2] * cos + m[0] * sin;
    }

    // --- Entity.move ----------------------------------------------------------------

    /// `Entity.getOnPos(offset)`.
    fn on_pos(&self, env: &Env<'_>, offset: f32) -> [i32; 3] {
        match self.body().supporting_block {
            Some(p) => {
                if !(offset > 1.0E-5) {
                    return p;
                }
                let below = env.state(p);
                let tall = offset <= 0.5 && env.tags.is(below, "minecraft:fences")
                    || env.tags.is(below, "minecraft:walls")
                    || env.blocks.is_class(below, Class::FenceGateBlock);
                if tall {
                    p
                } else {
                    [p[0], mth::floor(self.position[1] - f64::from(offset)), p[2]]
                }
            }
            None => [
                mth::floor(self.position[0]),
                mth::floor(self.position[1] - f64::from(offset)),
                mth::floor(self.position[2]),
            ],
        }
    }

    /// `getBlockSpeedFactor` (a living vehicle's with its movement efficiency).
    fn block_speed_factor(&self, env: &Env<'_>) -> f32 {
        let here_state = env.state(self.block_position());
        let here = env.blocks.speed_factor(here_state);
        let name = env.blocks.name(here_state);
        let base = if name != "minecraft:water" && name != "minecraft:bubble_column" {
            if f64::from(here) == 1.0 {
                env.blocks
                    .speed_factor(env.state(self.on_pos(env, 0.500001)))
            } else {
                here
            }
        } else {
            here
        };
        if self.living() {
            let e = self.body().attributes.movement_efficiency.value() as f32;
            base + e * (1.0 - base)
        } else {
            base
        }
    }

    /// `getBlockJumpFactor`.
    fn block_jump_factor(&self, env: &Env<'_>) -> f32 {
        let here = env.blocks.jump_factor(env.state(self.block_position()));
        let below = env
            .blocks
            .jump_factor(env.state(self.on_pos(env, 0.500001)));
        if f64::from(here) == 1.0 { below } else { here }
    }

    /// `Entity.move` of a vehicle the client moves (`controlled`: its rider is the
    /// local player). A piston's moves are cut to 0.51 an axis in a game tick.
    pub(crate) fn entity_move(
        &mut self,
        env: &Env<'_>,
        kind: MoverType,
        delta: [f64; 3],
        game_time: i64,
        controlled: bool,
    ) {
        let mut delta = delta;
        if kind == MoverType::Piston {
            delta = self.limit_piston_movement(delta, game_time);
            if delta.iter().all(|v| v.to_bits() == 0) {
                return;
            }
        }
        let stuck = self.body().stuck_speed_multiplier;
        if stuck[0] * stuck[0] + stuck[1] * stuck[1] + stuck[2] * stuck[2] > 1.0E-7 {
            if kind != MoverType::Piston {
                delta = [
                    delta[0] * stuck[0],
                    delta[1] * stuck[1],
                    delta[2] * stuck[2],
                ];
            }
            self.body_mut().stuck_speed_multiplier = [0.0; 3];
            self.delta = [0.0; 3];
        }
        let movement = collide(
            env,
            &self.bounding_box,
            delta,
            self.max_up_step(controlled),
            self.on_ground,
            self.who(),
        );
        let len = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        let dlen = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
        if len > 1.0E-7 || dlen - len < 1.0E-7 {
            let from = self.position;
            self.set_pos([
                self.position[0] + movement[0],
                self.position[1] + movement[1],
                self.position[2] + movement[2],
            ]);
            let to = self.position;
            let moves = &mut self.body_mut().movement_this_tick;
            if moves.len() >= 100 {
                let second = moves.remove(1);
                let first = &mut moves[0];
                *first = Movement {
                    from: first.from,
                    to: second.to,
                    asked: None,
                };
            }
            moves.push(Movement {
                from,
                to,
                asked: Some(delta),
            });
        }
        let equal = |a: f64, b: f64| (b - a).abs() < 9.999999747378752E-6;
        let x_collision = !equal(delta[0], movement[0]);
        let z_collision = !equal(delta[2], movement[2]);
        let body = self.body_mut();
        body.horizontal_collision = x_collision || z_collision;
        let moved_vertically = delta[1].abs() > 0.0;
        // The client moves it, so this always runs.
        body.vertical_collision = delta[1] != movement[1];
        body.vertical_collision_below = body.vertical_collision && delta[1] < 0.0;
        self.on_ground = self.body().vertical_collision_below;
        self.check_supporting_block(env, movement);
        let effect_pos = self.on_pos(env, 0.2);
        let effect_state = env.state(effect_pos);
        // checkFallDamage.
        match self.body().kind {
            RideKind::Boat { .. } => self.boat_fall(env, movement[1]),
            RideKind::HappyGhast => {}
            _ => {
                if !self.body().in_water {
                    let b = self.bounding_box;
                    let eye = (
                        mth::floor(self.position[0]),
                        self.eye_y(),
                        mth::floor(self.position[2]),
                    );
                    let body = self.body.as_deref_mut().expect("body");
                    update_fluids(body, env, &b, eye, &mut self.delta);
                }
                let on_ground = self.on_ground;
                let body = self.body_mut();
                if !body.in_water && movement[1] < 0.0 {
                    body.fall_distance -= f64::from(movement[1] as f32);
                }
                if on_ground {
                    body.fall_distance = 0.0;
                }
            }
        }
        if moved_vertically && self.body().vertical_collision || self.body().horizontal_collision {
            self.restitute(env, effect_state, x_collision, z_collision, movement);
        }
        let f = f64::from(self.block_speed_factor(env));
        self.delta = [self.delta[0] * f, self.delta[1], self.delta[2] * f];
    }

    /// `limitPistonMovement`.
    fn limit_piston_movement(&mut self, v: [f64; 3], game_time: i64) -> [f64; 3] {
        if v[0] * v[0] + v[1] * v[1] + v[2] * v[2] <= 1.0E-7 {
            return v;
        }
        let body = self.body_mut();
        if game_time != body.piston_deltas_game_time {
            body.piston_deltas = [0.0; 3];
            body.piston_deltas_game_time = game_time;
        }
        let Some(axis) = (0..3).find(|&i| v[i] != 0.0) else {
            return [0.0; 3];
        };
        let limited = (v[axis] + body.piston_deltas[axis]).clamp(-0.51, 0.51);
        let amount = limited - body.piston_deltas[axis];
        body.piston_deltas[axis] = limited;
        if amount.abs() <= 9.999999747378752E-6 {
            return [0.0; 3];
        }
        let mut out = [0.0; 3];
        out[axis] = amount;
        out
    }

    /// `restituteMovementAfterCollisions`: blocks a non-living vehicle bounces on
    /// give it four fifths of their bounce.
    fn restitute(
        &mut self,
        env: &Env<'_>,
        effect_state: u32,
        x_collision: bool,
        z_collision: bool,
        movement: [f64; 3],
    ) {
        let living = self.living();
        let mut restitution = if living {
            self.body().attributes.bounciness.value()
        } else {
            0.0
        };
        let current = self.delta;
        let mut after = current;
        if x_collision {
            after[0] = -current[0] * restitution;
        }
        if z_collision {
            after[2] = -current[2] * restitution;
        }
        if self.body().vertical_collision {
            let gravity = self.gravity();
            if self.body().vertical_collision_below {
                restitution = if !(-current[1] <= gravity)
                    && !env.tags.is(effect_state, "minecraft:suppresses_bounce")
                {
                    let mut bounce = env.blocks.bounce(effect_state);
                    if !living {
                        bounce *= 0.8;
                    }
                    restitution.max(f64::from(bounce))
                } else {
                    0.0
                };
            }
            let (compensation, drag) = if restitution > 0.0 {
                let portion = movement[1] / current[1];
                let air = if living {
                    f64::from(modified_friction(
                        0.98,
                        self.body().attributes.air_drag_modifier.value() as f32,
                    ))
                } else {
                    1.0
                };
                (portion * gravity, 1.0 + portion * (air - 1.0))
            } else {
                (0.0, 1.0)
            };
            after[1] = (compensation - current[1]) * drag * restitution;
        }
        self.delta = after;
    }

    /// `checkSupportingBlock`.
    fn check_supporting_block(&mut self, env: &Env<'_>, movement: [f64; 3]) {
        if !self.on_ground {
            let body = self.body_mut();
            body.on_ground_no_blocks = false;
            body.supporting_block = None;
            return;
        }
        let b = self.bounding_box;
        let area = Aabb {
            min: [b.min[0], b.min[1] - 1.0E-6, b.min[2]],
            max: [b.max[0], b.min[1], b.max[2]],
        };
        let who = self.who();
        let mut found = find_supporting_block(env, &area, self.position, who);
        let body = self.body_mut();
        if found.is_some() || body.on_ground_no_blocks {
            body.supporting_block = found;
        } else {
            found = find_supporting_block(
                env,
                &area.moved([-movement[0], 0.0, -movement[2]]),
                self.position,
                who,
            );
            self.body_mut().supporting_block = found;
        }
        self.body_mut().on_ground_no_blocks = found.is_none();
    }

    /// `mainSupportingBlockPos`.
    pub(crate) fn supporting_block(&self) -> Option<[i32; 3]> {
        self.body.as_deref().and_then(|b| b.supporting_block)
    }

    /// `removeLatestMovementRecording`.
    pub(crate) fn remove_latest_movement_recording(&mut self) {
        self.body_mut().movement_this_tick.pop();
    }

    /// `applyEffectsFromBlocks(from, to)`: the blocks one move (a piston's) passed
    /// through.
    pub(crate) fn apply_effects_between(&mut self, env: &Env<'_>, from: [f64; 3], to: [f64; 3]) {
        self.apply_effects_of(
            env,
            &[Movement {
                from,
                to,
                asked: None,
            }],
            true,
        );
    }

    /// `Entity.applyEffectsFromBlocks`: slime it stands on, then the blocks its moves
    /// went through. `controlled`: the client moves it (`canSimulateMovement`).
    fn apply_effects_from_blocks(&mut self, env: &Env<'_>, controlled: bool) {
        let mut movements = std::mem::take(&mut self.body_mut().movement_this_tick);
        match movements.last() {
            None => movements.push(Movement {
                from: self.old_position,
                to: self.position,
                asked: None,
            }),
            Some(last) if distance_sqr(last.to, self.position) > 9.999999439624929E-11 => movements
                .push(Movement {
                    from: last.to,
                    to: self.position,
                    asked: None,
                }),
            Some(_) => {}
        }
        self.apply_effects_of(env, &movements, controlled);
    }

    fn apply_effects_of(&mut self, env: &Env<'_>, movements: &[Movement], controlled: bool) {
        if self.removed {
            return;
        }
        if self.on_ground {
            let state = env.state(self.on_pos(env, 0.2));
            // SlimeBlock.stepOn.
            if env.blocks.is_class(state, Class::SlimeBlock) {
                let dy = self.delta[1].abs();
                if dy < 0.1 {
                    let scale = 0.4 + dy * 0.2;
                    self.delta = [self.delta[0] * scale, self.delta[1], self.delta[2] * scale];
                }
            }
        }
        let mut walker = InsideWalk {
            e: self,
            controlled,
        };
        let froze = crate::player::check_inside_blocks(&mut walker, env, movements);
        if froze && self.alive() {
            self.body_mut().in_powder_snow = true;
        }
    }

    /// `makeStuckInBlock`.
    fn make_stuck_in_block(&mut self, multiplier: [f64; 3]) {
        let body = self.body_mut();
        body.fall_distance = 0.0;
        body.stuck_speed_multiplier = multiplier;
    }
}

/// An entity walking through the blocks its moves passed.
struct InsideWalk<'a> {
    e: &'a mut Entity,
    controlled: bool,
}

impl InsideMover for InsideWalk<'_> {
    fn inside_alive(&self) -> bool {
        self.e.alive()
    }

    fn inside_box(&self, at: [f64; 3]) -> Aabb {
        self.e.size.bounding_box(at)
    }

    fn inside_who(&self) -> Who {
        self.e.who()
    }

    fn entity_inside(
        &mut self,
        env: &Env<'_>,
        pos: [i32; 3],
        state: u32,
        kind: Inside,
        precise: bool,
        froze: &mut bool,
    ) {
        let e = &mut *self.e;
        match kind {
            Inside::Cobweb => e.make_stuck_in_block([0.25, 0.05000000074505806, 0.25]),
            Inside::BerryBush => {
                if e.living() {
                    e.make_stuck_in_block([0.800000011920929, 0.75, 0.800000011920929]);
                }
            }
            Inside::PowderSnow => {
                if !e.living()
                    || env
                        .blocks
                        .is_class(env.state(e.block_position()), Class::PowderSnowBlock)
                {
                    e.make_stuck_in_block([0.8999999761581421, 1.5, 0.8999999761581421]);
                }
                *froze = true;
            }
            Inside::BubbleColumn => {
                if !precise {
                    return;
                }
                let drag = ruthenium_data::v26_3::blocks::BlockState::from_id(state)
                    .and_then(|s| s.value("drag"))
                    == Some("true");
                let above_pos = [pos[0], pos[1] + 1, pos[2]];
                let above = env.state(above_pos);
                let nothing_above =
                    env.blocks.collision(above, pos).is_none() && env.fluid(above_pos).is_empty();
                let boat = matches!(e.body().kind, RideKind::Boat { .. });
                if nothing_above {
                    // A boat's own onAboveBubbleColumn only acts on the server.
                    if !boat && self.controlled {
                        let dy = e.delta[1];
                        e.delta[1] = if drag {
                            (dy - 0.03).max(-0.9)
                        } else {
                            (dy + 0.1).min(1.8)
                        };
                    }
                } else if self.controlled {
                    let dy = e.delta[1];
                    e.delta[1] = if drag {
                        (dy - 0.03).max(-0.3)
                    } else {
                        (dy + 0.06).min(0.7)
                    };
                    e.body_mut().fall_distance = 0.0;
                }
            }
            Inside::Honey => {
                if !e.honey_sliding(pos) {
                    return;
                }
                let old_dy = e.delta[1] / 0.9800000190734863 + 0.08;
                let new_dy = (-0.05 - 0.08) * 0.9800000190734863;
                if old_dy < -0.13 {
                    let reduce = -0.05 / old_dy;
                    e.delta = [e.delta[0] * reduce, new_dy, e.delta[2] * reduce];
                } else {
                    e.delta[1] = new_dy;
                }
                e.body_mut().fall_distance = 0.0;
            }
        }
    }
}

impl Entity {
    /// `HoneyBlock.isSlidingDown`.
    fn honey_sliding(&self, pos: [i32; 3]) -> bool {
        if self.on_ground
            || self.position[1] > f64::from(pos[1]) + 0.9375 - 1.0E-7
            || self.delta[1] / 0.9800000190734863 + 0.08 >= -0.08
        {
            return false;
        }
        let dx = (f64::from(pos[0]) + 0.5 - self.position[0]).abs();
        let dz = (f64::from(pos[2]) + 0.5 - self.position[2]).abs();
        let overlap = 0.4375 + f64::from(self.size.width / 2.0);
        dx + 1.0E-7 > overlap || dz + 1.0E-7 > overlap
    }
}

/// `Entity.updateFluidInteraction` for a vehicle (pushed by currents).
fn update_fluids(
    body: &mut Body,
    env: &Env<'_>,
    b: &Aabb,
    eye: (i32, f64, i32),
    delta: &mut [f64; 3],
) {
    body.fluids.update(env, b, eye, false);
    let in_water = body.fluids.is_in(true);
    let in_lava = body.fluids.is_in(false);
    if in_water {
        body.fall_distance = 0.0;
    }
    body.in_water = in_water;
    if in_water {
        body.fluids.apply_current(true, delta, 0.014);
    }
    if in_lava {
        let scale = if env.level.fast_lava() {
            0.007
        } else {
            0.0023333333333333335
        };
        body.fluids.apply_current(false, delta, scale);
    }
}

/// `getFluidFallingAdjustedMovement` (not sprinting).
fn fluid_falling_adjusted(gravity: f64, is_falling: bool, m: [f64; 3]) -> [f64; 3] {
    if gravity == 0.0 {
        return m;
    }
    let yd = if is_falling && (m[1] - 0.005).abs() >= 0.003 && (m[1] - gravity / 16.0).abs() < 0.003
    {
        -0.003
    } else {
        m[1] - gravity / 16.0
    };
    [m[0], yd, m[2]]
}
