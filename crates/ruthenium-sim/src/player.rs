//! The local player: its movement each tick, ported from vanilla's `LocalPlayer`,
//! `Player`, `LivingEntity` and `Entity` so the same inputs give the same doubles,
//! and the movement packets it sends (`LocalPlayer.sendChanges`).
//!
//! Covered so far: walking, sprinting, sneaking and jumping on land and in the air,
//! block collisions with step-up, block friction and speed and jump factors, bouncing,
//! the push out of suffocating blocks, poses, climbing. Not yet: fluids and swimming,
//! flying and elytra, effects, vehicles, entity collisions and the world border, and
//! the fence, wall and bounce-suppression tags.

// Negated float comparisons (`!(a < b)`) are kept as the game writes them: they differ
// from the plain comparison when a value is NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ruthenium_javamath::mth;
use ruthenium_protocol::packets::play::PlayerPosition;
use ruthenium_protocol::wire;

use crate::blocks::{BlockTags, Blocks};
use crate::shape::{Aabb, Axis, EPSILON, Placed, collide_all};
use crate::world::Level;

/// `Relative` bits.
const X: u32 = 1 << 0;
const Y: u32 = 1 << 1;
const Z: u32 = 1 << 2;
const Y_ROT: u32 = 1 << 3;
const X_ROT: u32 = 1 << 4;
const DELTA_X: u32 = 1 << 5;
const DELTA_Y: u32 = 1 << 6;
const DELTA_Z: u32 = 1 << 7;
const ROTATE_DELTA: u32 = 1 << 8;

/// Java's `Math.toRadians` constant, and the game's float degrees-to-radians.
const DEGREES_TO_RADIANS: f64 = 0.017453292519943295;
const DEG_TO_RAD_F: f32 = 0.017453292;

/// The keys the game reads for movement (`Input`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub shift: bool,
    pub sprint: bool,
}

impl Keys {
    /// `ServerboundPlayerInputPacket`'s flags.
    pub fn bits(self) -> u8 {
        u8::from(self.forward)
            | u8::from(self.backward) << 1
            | u8::from(self.left) << 2
            | u8::from(self.right) << 3
            | u8::from(self.jump) << 4
            | u8::from(self.shift) << 5
            | u8::from(self.sprint) << 6
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    Standing,
    Crouching,
    Swimming,
}

impl Pose {
    /// Width and height (`Avatar.POSES`).
    fn size(self) -> (f32, f32) {
        match self {
            Pose::Standing => (0.6, 1.8),
            Pose::Crouching => (0.6, 1.5),
            Pose::Swimming => (0.6, 0.6),
        }
    }

    /// `EntityDimensions.makeBoundingBox`: half the width in floats.
    fn bounding_box(self, p: [f64; 3]) -> Aabb {
        let (w, h) = self.size();
        let w = f64::from(w / 2.0);
        let h = f64::from(h);
        Aabb {
            min: [p[0] - w, p[1], p[2] - w],
            max: [p[0] + w, p[1] + h, p[2] + w],
        }
    }
}

/// An attribute modifier (`AttributeModifier`): operation 0 adds, 1 adds a multiple of
/// the base, 2 multiplies the total.
#[derive(Debug, Clone, PartialEq)]
pub struct Modifier {
    pub id: String,
    pub amount: f64,
    pub operation: u8,
}

/// One attribute's base and modifiers (`AttributeInstance`), clamped to its range.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub base: f64,
    pub modifiers: Vec<Modifier>,
    min: f64,
    max: f64,
}

impl Attribute {
    fn new(base: f64, min: f64, max: f64) -> Attribute {
        Attribute {
            base,
            modifiers: Vec::new(),
            min,
            max,
        }
    }

    /// `AttributeInstance.calculateValue`.
    pub fn value(&self) -> f64 {
        let mut base = self.base;
        for m in self.modifiers.iter().filter(|m| m.operation == 0) {
            base += m.amount;
        }
        let mut result = base;
        for m in self.modifiers.iter().filter(|m| m.operation == 1) {
            result += base * m.amount;
        }
        for m in self.modifiers.iter().filter(|m| m.operation == 2) {
            result *= 1.0 + m.amount;
        }
        result.clamp(self.min, self.max)
    }

    fn remove(&mut self, id: &str) {
        self.modifiers.retain(|m| m.id != id);
    }
}

/// The sprinting modifier's id (`LivingEntity.SPRINTING_MODIFIER_ID`).
const SPRINTING: &str = "minecraft:sprinting";

/// The player's movement attributes, with the player's defaults and vanilla's ranges.
#[derive(Debug, Clone, PartialEq)]
pub struct Attributes {
    pub movement_speed: Attribute,
    pub jump_strength: Attribute,
    pub gravity: Attribute,
    pub step_height: Attribute,
    pub sneaking_speed: Attribute,
    pub friction_modifier: Attribute,
    pub air_drag_modifier: Attribute,
    pub movement_efficiency: Attribute,
    pub bounciness: Attribute,
}

impl Default for Attributes {
    fn default() -> Attributes {
        Attributes {
            movement_speed: Attribute::new(0.10000000149011612, 0.0, 1024.0),
            jump_strength: Attribute::new(0.41999998688697815, 0.0, 32.0),
            gravity: Attribute::new(0.08, -1.0, 1.0),
            step_height: Attribute::new(0.6, 0.0, 10.0),
            sneaking_speed: Attribute::new(0.3, 0.0, 1.0),
            friction_modifier: Attribute::new(1.0, 0.0, 2048.0),
            air_drag_modifier: Attribute::new(1.0, 0.0, 2048.0),
            movement_efficiency: Attribute::new(0.0, 0.0, 1.0),
            bounciness: Attribute::new(0.0, 0.0, 1.0),
        }
    }
}

impl Attributes {
    /// An attribute by its registry name, if movement uses it.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Attribute> {
        Some(match name {
            "minecraft:movement_speed" => &mut self.movement_speed,
            "minecraft:jump_strength" => &mut self.jump_strength,
            "minecraft:gravity" => &mut self.gravity,
            "minecraft:step_height" => &mut self.step_height,
            "minecraft:sneaking_speed" => &mut self.sneaking_speed,
            "minecraft:friction_modifier" => &mut self.friction_modifier,
            "minecraft:air_drag_modifier" => &mut self.air_drag_modifier,
            "minecraft:movement_efficiency" => &mut self.movement_efficiency,
            "minecraft:bounciness" => &mut self.bounciness,
            _ => return None,
        })
    }

    /// `LivingEntity.setSprinting`: the sprint modifier off, then on if sprinting.
    fn set_sprinting(&mut self, on: bool) {
        self.movement_speed.remove(SPRINTING);
        if on {
            self.movement_speed.modifiers.push(Modifier {
                id: SPRINTING.into(),
                amount: 0.30000001192092896,
                operation: 2,
            });
        }
    }
}

/// `LivingEntity.computeModifiedFriction`.
fn modified_friction(friction: f32, modifier: f32) -> f32 {
    (1.0 - (1.0 - friction) * modifier).clamp(0.0, 1.0)
}

/// What the movement code reads from the level.
#[derive(Debug, Clone, Copy)]
pub struct Env<'a> {
    pub level: &'a Level,
    pub blocks: &'a Blocks,
    pub tags: &'a BlockTags,
}

impl Env<'_> {
    fn state(&self, p: [i32; 3]) -> u32 {
        self.level.block(p[0], p[1], p[2])
    }

    fn collisions(&self, b: &Aabb) -> Vec<Placed<'_>> {
        self.level.block_collisions(self.blocks, b)
    }

    fn no_collision(&self, b: &Aabb) -> bool {
        self.collisions(b).is_empty()
    }

    fn has_chunk_at(&self, p: [i32; 3]) -> bool {
        self.level.has_chunk(p[0] >> 4, p[2] >> 4)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalPlayer {
    pub entity_id: i32,
    pub position: [f64; 3],
    pub delta: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub minor_horizontal_collision: bool,
    pub vertical_collision: bool,
    pub vertical_collision_below: bool,
    pub fall_distance: f64,
    pub sprinting: bool,
    pub pose: Pose,
    pub attributes: Attributes,
    pub food: i32,
    /// `LocalPlayer.crouching`.
    crouching: bool,
    /// The keys read this tick (`ClientInput.keyPresses`) and the move vector.
    keys: Keys,
    move_vector: (f32, f32),
    xxa: f32,
    zza: f32,
    jumping: bool,
    no_jump_delay: i32,
    sprint_trigger_time: i32,
    supporting_block: Option<[i32; 3]>,
    on_ground_no_blocks: bool,
    // `sendChanges` state.
    last_sent_keys: Keys,
    last_sent: [f64; 3],
    last_sent_rot: (f32, f32),
    last_on_ground: bool,
    last_horizontal_collision: bool,
    was_sprinting: bool,
    position_reminder: i32,
}

impl LocalPlayer {
    /// A new player (`handleLogin` turns it to face -180).
    pub fn new() -> LocalPlayer {
        LocalPlayer {
            entity_id: 0,
            position: [0.0; 3],
            delta: [0.0; 3],
            y_rot: -180.0,
            x_rot: 0.0,
            on_ground: false,
            horizontal_collision: false,
            minor_horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fall_distance: 0.0,
            sprinting: false,
            pose: Pose::Standing,
            attributes: Attributes::default(),
            food: 20,
            crouching: false,
            keys: Keys::default(),
            move_vector: (0.0, 0.0),
            xxa: 0.0,
            zza: 0.0,
            jumping: false,
            no_jump_delay: 0,
            sprint_trigger_time: 0,
            supporting_block: None,
            on_ground_no_blocks: false,
            last_sent_keys: Keys::default(),
            last_sent: [0.0; 3],
            last_sent_rot: (0.0, 0.0),
            last_on_ground: false,
            last_horizontal_collision: false,
            was_sprinting: false,
            position_reminder: 0,
        }
    }

    /// A player standing still on the ground where it is, as after a few idle ticks:
    /// resting vertical motion, the supporting block found, the send state settled.
    pub fn stand(&mut self, env: &Env<'_>) {
        self.on_ground = true;
        self.vertical_collision = true;
        self.vertical_collision_below = true;
        self.delta = [0.0, -0.0784000015258789, 0.0];
        self.check_supporting_block(env, Some([0.0; 3]));
        self.mark_sent();
    }

    /// Ticks since the last position was sent (the heartbeat sends one at 20).
    pub fn set_position_reminder(&mut self, ticks: i32) {
        self.position_reminder = ticks;
    }

    /// Settles the send state as if the current values had just been sent (a player
    /// standing still, as a replay starts).
    pub fn mark_sent(&mut self) {
        self.last_sent = self.position;
        self.last_sent_rot = (self.y_rot, self.x_rot);
        self.last_on_ground = self.on_ground;
        self.last_horizontal_collision = self.horizontal_collision;
        self.was_sprinting = self.sprinting;
        self.last_sent_keys = self.keys;
    }

    pub fn bounding_box(&self) -> Aabb {
        self.pose.bounding_box(self.position)
    }

    /// Applies a teleport (`PositionMoveRotation.calculateAbsolute`, then set without
    /// interpolation).
    pub fn teleport(&mut self, t: &PlayerPosition) {
        let rel = |bit: u32| t.relatives & bit != 0;
        let off = |bit: u32, v: f64| if rel(bit) { v } else { 0.0 };
        let position = [
            off(X, self.position[0]) + t.position[0],
            off(Y, self.position[1]) + t.position[1],
            off(Z, self.position[2]) + t.position[2],
        ];
        let y_rot = (if rel(Y_ROT) { self.y_rot } else { 0.0 }) + t.y_rot;
        let x_rot = ((if rel(X_ROT) { self.x_rot } else { 0.0 }) + t.x_rot).clamp(-90.0, 90.0);
        let mut moving = self.delta;
        if rel(ROTATE_DELTA) {
            let dy = self.y_rot - y_rot;
            let dx = self.x_rot - x_rot;
            moving = rotate_x(moving, (f64::from(dx) * DEGREES_TO_RADIANS) as f32);
            moving = rotate_y(moving, (f64::from(dy) * DEGREES_TO_RADIANS) as f32);
        }
        let delta = |cur: f64, change: f64, bit: u32| if rel(bit) { cur + change } else { change };
        self.delta = [
            delta(moving[0], t.delta[0], DELTA_X),
            delta(moving[1], t.delta[1], DELTA_Y),
            delta(moving[2], t.delta[2], DELTA_Z),
        ];
        self.position = position;
        self.y_rot = y_rot;
        self.x_rot = x_rot;
    }

    /// `Entity.turn`: the mouse's turn for one frame (from [`crate::input::turn_amount`]),
    /// in floats.
    pub fn turn(&mut self, xo: f64, yo: f64) {
        let x_delta = yo as f32 * 0.15;
        let y_delta = xo as f32 * 0.15;
        self.set_x_rot(self.x_rot + x_delta);
        self.set_y_rot(self.y_rot + y_delta);
        self.set_x_rot(mth_clamp(self.x_rot, -90.0, 90.0));
    }

    /// `Entity.setXRot`: a value that isn't finite is dropped.
    fn set_x_rot(&mut self, x_rot: f32) {
        if x_rot.is_finite() {
            self.x_rot = (x_rot % 360.0).clamp(-90.0, 90.0);
        }
    }

    /// `Entity.setYRot`.
    fn set_y_rot(&mut self, y_rot: f32) {
        if y_rot.is_finite() {
            self.y_rot = y_rot;
        }
    }

    /// The chunk the camera (the player's block position) is in.
    pub fn chunk(&self) -> (i32, i32) {
        (
            mth::floor(self.position[0]) >> 4,
            mth::floor(self.position[2]) >> 4,
        )
    }

    fn block_position(&self) -> [i32; 3] {
        [
            mth::floor(self.position[0]),
            mth::floor(self.position[1]),
            mth::floor(self.position[2]),
        ]
    }

    fn set_sprinting(&mut self, on: bool) {
        self.sprinting = on;
        self.attributes.set_sprinting(on);
    }

    /// `Player.hasEnoughFoodToDoExhaustiveManoeuvres` without creative flight.
    fn enough_food(&self) -> bool {
        self.food > 6
    }

    fn can_fit(&self, env: &Env<'_>, pose: Pose) -> bool {
        env.no_collision(&pose.bounding_box(self.position).inflate([-EPSILON; 3]))
    }

    /// `isMovingSlowly` (crawling isn't possible without swimming yet).
    fn moving_slowly(&self) -> bool {
        self.crouching
    }

    fn has_forward_impulse(&self) -> bool {
        self.move_vector.1 > 1.0E-5
    }

    /// One client tick of the player (`LocalPlayer.tick`, only called once the player
    /// has loaded into the level). `keys` are the movement keys held.
    pub fn tick(&mut self, env: &Env<'_>, keys: Keys) {
        // Player.tick: Entity.baseTick has nothing to do on land, then aiStep.
        self.ai_step(env, keys);
        let clamp = |v: f64| v.clamp(-2.9999999E7, 2.9999999E7);
        self.position[0] = clamp(self.position[0]);
        self.position[2] = clamp(self.position[2]);
        self.update_pose(env);
    }

    /// `Player.updatePlayerPose`.
    fn update_pose(&mut self, env: &Env<'_>) {
        if self.can_fit(env, Pose::Swimming) {
            let desired = if self.keys.shift {
                Pose::Crouching
            } else {
                Pose::Standing
            };
            self.pose = if self.can_fit(env, desired) {
                desired
            } else if self.can_fit(env, Pose::Crouching) {
                Pose::Crouching
            } else {
                Pose::Swimming
            };
        }
    }

    /// `LocalPlayer.aiStep` for a survival player on land.
    fn ai_step(&mut self, env: &Env<'_>, keys: Keys) {
        if self.sprint_trigger_time > 0 {
            self.sprint_trigger_time -= 1;
        }
        let was_shift = self.keys.shift;
        let had_forward = self.has_forward_impulse();
        self.crouching = self.can_fit(env, Pose::Crouching)
            && (self.keys.shift || !self.can_fit(env, Pose::Standing));
        // KeyboardInput.tick.
        self.keys = keys;
        let impulse = |pos: bool, neg: bool| {
            if pos == neg {
                0.0f32
            } else if pos {
                1.0
            } else {
                -1.0
            }
        };
        let (x, y) = (
            impulse(keys.left, keys.right),
            impulse(keys.forward, keys.backward),
        );
        let dist = (f64::from(x * x + y * y)).sqrt() as f32;
        self.move_vector = if dist < 1.0E-4 {
            (0.0, 0.0)
        } else {
            (x / dist, y / dist)
        };

        // Push out of blocks the player is stuck in.
        let w = f64::from(0.6f32) * 0.35;
        let (px, pz) = (self.position[0], self.position[2]);
        for (x, z) in [
            (px - w, pz + w),
            (px - w, pz - w),
            (px + w, pz - w),
            (px + w, pz + w),
        ] {
            self.move_towards_closest_space(env, x, z);
        }
        if was_shift || keys.backward {
            self.sprint_trigger_time = 0;
        }
        let can_start_sprinting = !self.sprinting
            && self.has_forward_impulse()
            && self.enough_food()
            && !self.moving_slowly();
        if can_start_sprinting {
            if !had_forward {
                if self.sprint_trigger_time > 0 {
                    self.set_sprinting(true);
                } else {
                    // The double-tap window (`Options.sprintWindow`, default 7).
                    self.sprint_trigger_time = 7;
                }
            }
            if keys.sprint {
                self.set_sprinting(true);
            }
        }
        if self.sprinting {
            let stop = !self.enough_food()
                || !self.has_forward_impulse()
                || self.horizontal_collision && !self.minor_horizontal_collision;
            if stop {
                self.set_sprinting(false);
            }
        }
        // Player.aiStep -> LivingEntity.aiStep.
        self.living_ai_step(env);
    }

    /// `LocalPlayer.moveTowardsClosestSpace`.
    fn move_towards_closest_space(&mut self, env: &Env<'_>, x: f64, z: f64) {
        let pos = [mth::floor(x), mth::floor(self.position[1]), mth::floor(z)];
        if !self.suffocates_at(env, pos) {
            return;
        }
        let xd = x - f64::from(pos[0]);
        let zd = z - f64::from(pos[2]);
        // West, east, north, south.
        let dirs: [([i32; 3], f64); 4] = [
            ([-1, 0, 0], xd),
            ([1, 0, 0], 1.0 - xd),
            ([0, 0, -1], zd),
            ([0, 0, 1], 1.0 - zd),
        ];
        let mut best: Option<[i32; 3]> = None;
        let mut closest = f64::MAX;
        for (d, dist) in dirs {
            let next = [pos[0] + d[0], pos[1], pos[2] + d[2]];
            if dist < closest && !self.suffocates_at(env, next) {
                closest = dist;
                best = Some(d);
            }
        }
        if let Some(d) = best {
            if d[0] != 0 {
                self.delta[0] = 0.1 * f64::from(d[0]);
            } else {
                self.delta[2] = 0.1 * f64::from(d[2]);
            }
        }
    }

    fn suffocates_at(&self, env: &Env<'_>, pos: [i32; 3]) -> bool {
        let b = self.bounding_box();
        let area = Aabb::new(
            [f64::from(pos[0]), b.min[1], f64::from(pos[2])],
            [f64::from(pos[0]) + 1.0, b.max[1], f64::from(pos[2]) + 1.0],
        )
        .inflate([-EPSILON; 3]);
        !env.level
            .block_collisions_at(env.blocks, &area, true)
            .is_empty()
    }

    /// `LivingEntity.aiStep`, then the travel.
    fn living_ai_step(&mut self, env: &Env<'_>) {
        if self.no_jump_delay > 0 {
            self.no_jump_delay -= 1;
        }
        let [mut dx, mut dy, mut dz] = self.delta;
        if dx * dx + dz * dz < 9.0E-6 {
            dx = 0.0;
            dz = 0.0;
        }
        if dy.abs() < 0.003 {
            dy = 0.0;
        }
        self.delta = [dx, dy, dz];
        // LocalPlayer.applyInput.
        let (mx, my) = self.modify_input(self.move_vector);
        self.xxa = mx;
        self.zza = my;
        self.jumping = self.keys.jump;
        if self.jumping {
            if self.on_ground && self.no_jump_delay == 0 {
                self.jump_from_ground(env);
                self.no_jump_delay = 10;
            }
        } else {
            self.no_jump_delay = 0;
        }
        self.travel_in_air(env);
    }

    /// `LocalPlayer.modifyInput`.
    fn modify_input(&self, (x, y): (f32, f32)) -> (f32, f32) {
        if x * x + y * y == 0.0 {
            return (x, y);
        }
        let (mut x, mut y) = (x * 0.98, y * 0.98);
        if self.moving_slowly() {
            let f = self.attributes.sneaking_speed.value() as f32;
            x *= f;
            y *= f;
        }
        // modifyInputSpeedForSquareMovement.
        let length = (f64::from(x * x + y * y)).sqrt() as f32;
        if length <= 0.0 {
            return (x, y);
        }
        let inv = 1.0 / length;
        let (dx, dy) = (x * inv, y * inv);
        let (ax, ay) = (dx.abs(), dy.abs());
        let tan = if ay > ax { ax / ay } else { ay / ax };
        let to_square = (f64::from(1.0 + tan * tan)).sqrt() as f32;
        let modified = (length * to_square).min(1.0);
        (dx * modified, dy * modified)
    }

    /// `LivingEntity.jumpFromGround`.
    fn jump_from_ground(&mut self, env: &Env<'_>) {
        let power =
            self.attributes.jump_strength.value() as f32 * 1.0 * self.block_jump_factor(env) + 0.0;
        if power > 1.0E-5 {
            self.delta[1] = f64::from(power).max(self.delta[1]);
            if self.sprinting {
                let angle = self.y_rot * DEG_TO_RAD_F;
                self.delta[0] += f64::from(-mth::sin(f64::from(angle))) * 0.2;
                self.delta[2] += f64::from(mth::cos(f64::from(angle))) * 0.2;
            }
        }
    }

    /// `LivingEntity.travelInAir`.
    fn travel_in_air(&mut self, env: &Env<'_>) {
        let below = self.block_pos_below_affecting_movement(env);
        let block_friction = if self.on_ground {
            modified_friction(
                env.blocks.friction(env.state(below)),
                self.attributes.friction_modifier.value() as f32,
            )
        } else {
            1.0
        };
        // handleRelativeFrictionAndCalculateMovement; `Player.getSpeed` reads the
        // attribute itself, so a change applies this tick.
        let own = self.attributes.movement_speed.value() as f32;
        let speed = if self.on_ground {
            if f64::from(block_friction) > 0.6 {
                own * (0.21600002 / (block_friction * block_friction * block_friction))
            } else {
                own
            }
        } else if self.sprinting {
            0.025999999
        } else {
            0.02
        };
        self.move_relative(speed);
        let climbing = self.on_climbable(env);
        if climbing {
            // handleOnClimbable.
            self.fall_distance = 0.0;
            let max = 0.15000000596046448;
            let mut d = self.delta;
            d[0] = d[0].clamp(-max, max);
            d[2] = d[2].clamp(-max, max);
            d[1] = d[1].max(-max);
            let in_scaffolding =
                env.blocks.name(env.state(self.block_position())) == "minecraft:scaffolding";
            if d[1] < 0.0 && !in_scaffolding && self.keys.shift {
                d[1] = 0.0;
            }
            self.delta = d;
        }
        self.do_move(env, self.delta);
        let mut movement = self.delta;
        // Pushing against the ladder or jumping climbs (powder snow walking isn't
        // ported).
        if (self.horizontal_collision || self.jumping) && self.on_climbable(env) {
            movement[1] = 0.2;
        }
        let movement_y = if env.has_chunk_at(below) {
            movement[1] - self.attributes.gravity.value()
        } else if self.position[1] > f64::from(env.level.min_y()) {
            -0.1
        } else {
            0.0
        };
        let air_drag = modified_friction(0.91, self.attributes.air_drag_modifier.value() as f32);
        let friction = block_friction * air_drag;
        let vertical = modified_friction(0.98, self.attributes.air_drag_modifier.value() as f32);
        self.delta = [
            movement[0] * f64::from(friction),
            movement_y * f64::from(vertical),
            movement[2] * f64::from(friction),
        ];
    }

    /// `LivingEntity.onClimbable`: the block the player is in is climbable, or an open
    /// trapdoor above a ladder facing the same way.
    fn on_climbable(&self, env: &Env<'_>) -> bool {
        let pos = self.block_position();
        let state = env.state(pos);
        if env.tags.climbable(state) {
            return true;
        }
        let Some(s) = ruthenium_data::v26_3::BlockState::from_id(state) else {
            return false;
        };
        if !s.block().name().ends_with("_trapdoor") || s.value("open") != Some("true") {
            return false;
        }
        let below =
            ruthenium_data::v26_3::BlockState::from_id(env.state([pos[0], pos[1] - 1, pos[2]]));
        below.is_some_and(|b| {
            b.block().name() == "minecraft:ladder" && b.value("facing") == s.value("facing")
        })
    }

    /// `Entity.moveRelative` with `getInputVector`.
    fn move_relative(&mut self, speed: f32) {
        let input = [f64::from(self.xxa), 0.0, f64::from(self.zza)];
        let length = input[0] * input[0] + input[1] * input[1] + input[2] * input[2];
        if length < 1.0E-7 {
            return;
        }
        let v = if length > 1.0 {
            let l = length.sqrt();
            if l < f64::from(1.0E-5f32) {
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

    /// `Entity.getOnPos(offset)`.
    fn on_pos(&self, env: &Env<'_>, offset: f32) -> [i32; 3] {
        match self.supporting_block {
            Some(p) => {
                if !(offset > 1.0E-5) {
                    return p;
                }
                // Fences, walls and fence gates keep the supporting block; their tags
                // aren't read yet.
                let _ = env;
                [p[0], mth::floor(self.position[1] - f64::from(offset)), p[2]]
            }
            None => [
                mth::floor(self.position[0]),
                mth::floor(self.position[1] - f64::from(offset)),
                mth::floor(self.position[2]),
            ],
        }
    }

    fn block_pos_below_affecting_movement(&self, env: &Env<'_>) -> [i32; 3] {
        self.on_pos(env, 0.500001)
    }

    /// `Entity.getBlockSpeedFactor` (with `LivingEntity`'s movement efficiency lerp).
    fn block_speed_factor(&self, env: &Env<'_>) -> f32 {
        let here_state = env.state(self.block_position());
        let here = env.blocks.speed_factor(here_state);
        let name = env.blocks.name(here_state);
        let base = if name != "minecraft:water" && name != "minecraft:bubble_column" {
            if f64::from(here) == 1.0 {
                env.blocks
                    .speed_factor(env.state(self.block_pos_below_affecting_movement(env)))
            } else {
                here
            }
        } else {
            here
        };
        let e = self.attributes.movement_efficiency.value() as f32;
        base + e * (1.0 - base)
    }

    fn block_jump_factor(&self, env: &Env<'_>) -> f32 {
        let here = env.blocks.jump_factor(env.state(self.block_position()));
        let below = env
            .blocks
            .jump_factor(env.state(self.block_pos_below_affecting_movement(env)));
        if f64::from(here) == 1.0 { below } else { here }
    }

    /// `Entity.move` for the player moving itself.
    fn do_move(&mut self, env: &Env<'_>, delta: [f64; 3]) {
        let delta = self.back_off_from_edge(env, delta);
        let movement = self.collide(env, delta);
        let len = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        let dlen = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
        if len > 1.0E-7 || dlen - len < 1.0E-7 {
            self.position = [
                self.position[0] + movement[0],
                self.position[1] + movement[1],
                self.position[2] + movement[2],
            ];
        }
        let equal = |a: f64, b: f64| (b - a).abs() < 9.999999747378752E-6;
        let x_collision = !equal(delta[0], movement[0]);
        let z_collision = !equal(delta[2], movement[2]);
        self.horizontal_collision = x_collision || z_collision;
        // The local player is authoritative, so this always runs.
        self.vertical_collision = delta[1] != movement[1];
        self.vertical_collision_below = self.vertical_collision && delta[1] < 0.0;
        self.on_ground = self.vertical_collision_below;
        self.check_supporting_block(env, Some(movement));
        self.minor_horizontal_collision =
            self.horizontal_collision && self.horizontal_collision_minor(movement);
        let effect_pos = self.on_pos(env, 0.2);
        let effect_state = env.state(effect_pos);
        // checkFallDamage (out of water).
        if movement[1] < 0.0 {
            self.fall_distance -= f64::from(movement[1] as f32);
        }
        if self.on_ground {
            self.fall_distance = 0.0;
        }
        let moved_vertically = delta[1].abs() > 0.0;
        if moved_vertically && self.vertical_collision || self.horizontal_collision {
            self.restitute(env, effect_state, x_collision, z_collision, movement);
        }
        let f = f64::from(self.block_speed_factor(env));
        self.delta = [self.delta[0] * f, self.delta[1], self.delta[2] * f];
    }

    /// `Entity.restituteMovementAfterCollisions`.
    fn restitute(
        &mut self,
        env: &Env<'_>,
        effect_state: u32,
        x_collision: bool,
        z_collision: bool,
        movement: [f64; 3],
    ) {
        let suppress = self.keys.shift;
        let mut restitution = if suppress {
            0.0
        } else {
            self.attributes.bounciness.value()
        };
        let current = self.delta;
        let mut after = current;
        if x_collision {
            after[0] = -current[0] * restitution;
        }
        if z_collision {
            after[2] = -current[2] * restitution;
        }
        if self.vertical_collision {
            let gravity = self.attributes.gravity.value();
            if self.vertical_collision_below {
                restitution = if !(-current[1] <= gravity) && !suppress {
                    restitution.max(f64::from(env.blocks.bounce(effect_state)))
                } else {
                    0.0
                };
            }
            let (compensation, drag) = if restitution > 0.0 {
                let portion = movement[1] / current[1];
                let air = f64::from(modified_friction(
                    0.98,
                    self.attributes.air_drag_modifier.value() as f32,
                ));
                (portion * gravity, 1.0 + portion * (air - 1.0))
            } else {
                (0.0, 1.0)
            };
            after[1] = (compensation - current[1]) * drag * restitution;
        }
        self.delta = after;
    }

    /// `LocalPlayer.isHorizontalCollisionMinor`.
    fn horizontal_collision_minor(&self, movement: [f64; 3]) -> bool {
        let r = self.y_rot * DEG_TO_RAD_F;
        let sin = f64::from(mth::sin(f64::from(r)));
        let cos = f64::from(mth::cos(f64::from(r)));
        let (xxa, zza) = (f64::from(self.xxa), f64::from(self.zza));
        let gx = xxa * cos - zza * sin;
        let gz = zza * cos + xxa * sin;
        let a = gx * gx + gz * gz;
        let m = movement[0] * movement[0] + movement[2] * movement[2];
        if a < 9.999999747378752E-6 || m < 9.999999747378752E-6 {
            return false;
        }
        let dot = gx * movement[0] + gz * movement[2];
        (dot / (a * m).sqrt()).acos() < 0.13962633907794952
    }

    /// `Entity.checkSupportingBlock`.
    fn check_supporting_block(&mut self, env: &Env<'_>, movement: Option<[f64; 3]>) {
        if !self.on_ground {
            self.on_ground_no_blocks = false;
            self.supporting_block = None;
            return;
        }
        let b = self.bounding_box();
        let area = Aabb {
            min: [b.min[0], b.min[1] - 1.0E-6, b.min[2]],
            max: [b.max[0], b.min[1], b.max[2]],
        };
        let mut found = self.find_supporting_block(env, &area);
        if found.is_some() || self.on_ground_no_blocks {
            self.supporting_block = found;
        } else if let Some(m) = movement {
            found = self.find_supporting_block(env, &area.moved([-m[0], 0.0, -m[2]]));
            self.supporting_block = found;
        }
        self.on_ground_no_blocks = found.is_none();
    }

    /// `CollisionGetter.findSupportingBlock`: the closest block, ties to the greatest
    /// by y, then z, then x.
    fn find_supporting_block(&self, env: &Env<'_>, area: &Aabb) -> Option<[i32; 3]> {
        let mut best: Option<([i32; 3], f64)> = None;
        for (p, _) in env.level.block_collisions_at(env.blocks, area, false) {
            let d = |i: usize| f64::from(p[i]) + 0.5 - self.position[i];
            let dist = d(0) * d(0) + d(1) * d(1) + d(2) * d(2);
            let better = match best {
                None => true,
                Some((bp, bd)) => {
                    dist < bd || dist == bd && (bp[1], bp[2], bp[0]) < (p[1], p[2], p[0])
                }
            };
            if better {
                best = Some((p, dist));
            }
        }
        best.map(|(p, _)| p)
    }

    /// `Player.maybeBackOffFromEdge`: sneaking stops at edges.
    fn back_off_from_edge(&self, env: &Env<'_>, delta: [f64; 3]) -> [f64; 3] {
        let max_down = self.attributes.step_height.value() as f32;
        if delta[1] > 0.0 || !self.keys.shift || !self.above_ground(env, max_down) {
            return delta;
        }
        let max_down = f64::from(max_down);
        let (mut dx, mut dz) = (delta[0], delta[2]);
        let (sx, sz) = (dx.signum() * 0.05, dz.signum() * 0.05);
        let step = |v: f64, s: f64| if v.abs() <= 0.05 { 0.0 } else { v - s };
        while dx != 0.0 && self.can_fall_at_least(env, dx, 0.0, max_down) {
            if dx.abs() <= 0.05 {
                dx = 0.0;
                break;
            }
            dx -= sx;
        }
        while dz != 0.0 && self.can_fall_at_least(env, 0.0, dz, max_down) {
            if dz.abs() <= 0.05 {
                dz = 0.0;
                break;
            }
            dz -= sz;
        }
        while dx != 0.0 && dz != 0.0 && self.can_fall_at_least(env, dx, dz, max_down) {
            dx = step(dx, sx);
            dz = step(dz, sz);
        }
        [dx, delta[1], dz]
    }

    fn above_ground(&self, env: &Env<'_>, max_down: f32) -> bool {
        self.on_ground
            || self.fall_distance < f64::from(max_down)
                && !self.can_fall_at_least(env, 0.0, 0.0, f64::from(max_down) - self.fall_distance)
    }

    fn can_fall_at_least(&self, env: &Env<'_>, dx: f64, dz: f64, min_height: f64) -> bool {
        let b = self.bounding_box();
        env.no_collision(&Aabb::new(
            [
                b.min[0] + EPSILON + dx,
                b.min[1] - min_height - EPSILON,
                b.min[2] + EPSILON + dz,
            ],
            [b.max[0] - EPSILON + dx, b.min[1], b.max[2] - EPSILON + dz],
        ))
    }

    /// `Entity.collide`: block collisions, then stepping up.
    fn collide(&self, env: &Env<'_>, movement: [f64; 3]) -> [f64; 3] {
        let b = self.bounding_box();
        let step = self.attributes.step_height.value() as f32;
        let len = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        let result = if len == 0.0 {
            movement
        } else {
            let shapes = env.collisions(&b.expand_towards(movement));
            collide_with_shapes(movement, &b, &shapes)
        };
        let x_c = movement[0] != result[0];
        let y_c = movement[1] != result[1];
        let z_c = movement[2] != result[2];
        let grounded_after = y_c && movement[1] < 0.0;
        if step > 0.0 && (grounded_after || self.on_ground) && (x_c || z_c) {
            let grounded = if grounded_after {
                b.moved([0.0, result[1], 0.0])
            } else {
                b
            };
            let mut area = grounded.expand_towards([movement[0], f64::from(step), movement[2]]);
            if !grounded_after {
                area = area.expand_towards([0.0, -9.999999747378752E-6, 0.0]);
            }
            let shapes = env.collisions(&area);
            let skip = result[1] as f32;
            for h in candidate_step_heights(&grounded, &shapes, step, skip) {
                let stepped = collide_with_shapes(
                    [movement[0], f64::from(h), movement[2]],
                    &grounded,
                    &shapes,
                );
                if stepped[0] * stepped[0] + stepped[2] * stepped[2]
                    > result[0] * result[0] + result[2] * result[2]
                {
                    let to_ground = b.min[1] - grounded.min[1];
                    return [stepped[0], stepped[1] - to_ground, stepped[2]];
                }
            }
        }
        result
    }

    /// The packets `LocalPlayer.sendChanges` sends this tick, in order: the keys if
    /// they changed, the sprint command if it changed, then the movement packet if one
    /// is due (moved more than 2e-4, turned, every 20 ticks, or ground or collision
    /// changed).
    pub fn send_changes(&mut self) -> Vec<Vec<u8>> {
        use ruthenium_data::v26_3::packets::play::Serverbound as S;
        let mut out = Vec::new();
        if self.keys != self.last_sent_keys {
            out.push(wire::packet(S::PlayerInput.id(), |o| {
                o.push(self.keys.bits())
            }));
            self.last_sent_keys = self.keys;
        }
        if self.sprinting != self.was_sprinting {
            let action = if self.sprinting { 1 } else { 2 };
            out.push(wire::packet(S::PlayerCommand.id(), |o| {
                wire::write_varint(o, self.entity_id);
                wire::write_varint(o, action);
                wire::write_varint(o, 0);
            }));
            self.was_sprinting = self.sprinting;
        }
        let d = [
            self.position[0] - self.last_sent[0],
            self.position[1] - self.last_sent[1],
            self.position[2] - self.last_sent[2],
        ];
        let dy_rot = f64::from(self.y_rot) - f64::from(self.last_sent_rot.0);
        let dx_rot = f64::from(self.x_rot) - f64::from(self.last_sent_rot.1);
        self.position_reminder += 1;
        let moved = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 2.0E-4 * 2.0E-4
            || self.position_reminder >= 20;
        let turned = dy_rot != 0.0 || dx_rot != 0.0;
        let flags = u8::from(self.on_ground) | u8::from(self.horizontal_collision) << 1;
        let pos = |o: &mut Vec<u8>| {
            for v in self.position {
                wire::write_f64(o, v);
            }
        };
        let rot = |o: &mut Vec<u8>| {
            wire::write_f32(o, self.y_rot);
            wire::write_f32(o, self.x_rot);
        };
        if moved && turned {
            out.push(wire::packet(S::MovePlayerPosRot.id(), |o| {
                pos(o);
                rot(o);
                o.push(flags);
            }));
        } else if moved {
            out.push(wire::packet(S::MovePlayerPos.id(), |o| {
                pos(o);
                o.push(flags);
            }));
        } else if turned {
            out.push(wire::packet(S::MovePlayerRot.id(), |o| {
                rot(o);
                o.push(flags);
            }));
        } else if self.last_on_ground != self.on_ground
            || self.last_horizontal_collision != self.horizontal_collision
        {
            out.push(wire::packet(S::MovePlayerStatusOnly.id(), |o| {
                o.push(flags)
            }));
        }
        if moved {
            self.last_sent = self.position;
            self.position_reminder = 0;
        }
        if turned {
            self.last_sent_rot = (self.y_rot, self.x_rot);
        }
        self.last_on_ground = self.on_ground;
        self.last_horizontal_collision = self.horizontal_collision;
        out
    }
}

impl Default for LocalPlayer {
    fn default() -> LocalPlayer {
        LocalPlayer::new()
    }
}

/// `Entity.collideWithShapes`: y first, then the larger horizontal axis last.
fn collide_with_shapes(movement: [f64; 3], b: &Aabb, shapes: &[Placed<'_>]) -> [f64; 3] {
    if shapes.is_empty() {
        return movement;
    }
    let order = if movement[0].abs() < movement[2].abs() {
        [Axis::Y, Axis::Z, Axis::X]
    } else {
        [Axis::Y, Axis::X, Axis::Z]
    };
    let mut resolved = [0.0; 3];
    for axis in order {
        let m = movement[axis as usize];
        if m != 0.0 {
            resolved[axis as usize] = collide_all(axis, &b.moved(resolved), shapes, m);
        }
    }
    resolved
}

/// `Entity.collectCandidateStepUpHeights`: the colliders' y coordinates above the
/// box's bottom, up to the step height, as floats, sorted.
fn candidate_step_heights(b: &Aabb, shapes: &[Placed<'_>], max: f32, skip: f32) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::with_capacity(4);
    for s in shapes {
        for coord in s.coords(Axis::Y) {
            let rel = (coord - b.min[1]) as f32;
            if !(rel < 0.0) && rel != skip {
                if rel > max {
                    break;
                }
                if !out.contains(&rel) {
                    out.push(rel);
                }
            }
        }
    }
    out.sort_by(f32::total_cmp);
    out
}

/// `Vec3.xRot`: float sine and cosine from vanilla's table, products in doubles.
fn rotate_x(v: [f64; 3], radians: f32) -> [f64; 3] {
    let cos = f64::from(mth::cos(f64::from(radians)));
    let sin = f64::from(mth::sin(f64::from(radians)));
    [v[0], v[1] * cos + v[2] * sin, v[2] * cos - v[1] * sin]
}

/// `Vec3.yRot`.
fn rotate_y(v: [f64; 3], radians: f32) -> [f64; 3] {
    let cos = f64::from(mth::cos(f64::from(radians)));
    let sin = f64::from(mth::sin(f64::from(radians)));
    [v[0] * cos + v[2] * sin, v[1], v[2] * cos - v[0] * sin]
}

/// `Mth.clamp` for floats.
fn mth_clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: [f64; 3], relatives: u32) -> PlayerPosition {
        PlayerPosition {
            id: 1,
            position,
            delta: [0.0; 3],
            y_rot: 90.0,
            x_rot: 100.0,
            relatives,
        }
    }

    #[test]
    fn absolute_and_relative_teleports() {
        let mut p = LocalPlayer::new();
        p.teleport(&at([1.5, -60.0, 9.5], 0));
        assert_eq!(p.position, [1.5, -60.0, 9.5]);
        assert_eq!((p.y_rot, p.x_rot), (90.0, 90.0));
        p.teleport(&at([1.0, 0.0, -1.0], X | Y | Z | Y_ROT));
        assert_eq!(p.position, [2.5, -60.0, 8.5]);
        assert_eq!(p.y_rot, 180.0);
        assert_eq!(p.chunk(), (0, 0));
        p.teleport(&at([-0.5, 0.0, -16.0], 0));
        assert_eq!(p.chunk(), (-1, -1));
    }

    #[test]
    fn sprint_speed_is_vanillas() {
        let mut a = Attributes::default();
        assert_eq!(a.movement_speed.value() as f32, 0.1);
        a.set_sprinting(true);
        a.set_sprinting(true);
        assert_eq!(a.movement_speed.modifiers.len(), 1);
        assert_eq!(
            a.movement_speed.value(),
            0.10000000149011612 * (1.0 + 0.30000001192092896)
        );
        assert_eq!(modified_friction(0.6, 1.0), 0.6);
        assert_eq!(modified_friction(0.91, 1.0), 0.91);
    }
}
