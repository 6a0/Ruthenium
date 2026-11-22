//! The entities the client tracks besides its own player, as vanilla's `ClientLevel`
//! keeps them: added, moved and removed as the server says, interpolated towards
//! where it says they are, ticked in vanilla's order, and pushing the local player.
//!
//! Pushing: on the client the local player never pushes itself away from others
//! (`ClientLevel.getPushableEntities` only ever returns the local player). Each other
//! living entity pushes the local player during its own tick (`LivingEntity.aiStep`'s
//! `pushEntities`), and boats do in theirs, so the order of the tick list decides
//! which push lands before which move. Minecarts don't push on the client.
//!
//! Collisions: boats, live shulkers and grown happy ghasts are solid to the local
//! player (`canBeCollidedWith`); [`Entities::colliders`] gives their boxes for its move.
//!
//! Order: entities tick in the order they started ticking (`EntityTickList`), the
//! local player among them. An entity ticks while the chunk it is in is loaded
//! (`TransientEntitySectionManager`); a chunk that loads starts its entities section by
//! section in vanilla's order.
//!
//! Not yet: passengers are ticked after their vehicle but not moved with it
//! (`positionRider`), so they don't push; sizes that depend on more than the pose,
//! youth, a slime's size and the scale attribute (a sitting camel, a peeking shulker, a
//! puffed pufferfish, a phantom's size) are the type's default; the local player riding
//! anything.

use std::collections::{BTreeMap, HashMap, VecDeque};

use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::EntityTypeData;
use ruthenium_protocol::packets::play::{self, DataValue, PositionPath, VecDelta};

use crate::player::{Attribute, Env, LocalPlayer, Modifier};
use crate::shape::Aabb;

/// `Pose` ordinals.
pub mod pose {
    pub const STANDING: i32 = 0;
    pub const FALL_FLYING: i32 = 1;
    pub const SLEEPING: i32 = 2;
    pub const SWIMMING: i32 = 3;
    pub const SPIN_ATTACK: i32 = 4;
    pub const CROUCHING: i32 = 5;
    pub const DYING: i32 = 7;
}

/// `Team.CollisionRule` ids.
mod rule {
    pub const ALWAYS: i32 = 0;
    pub const NEVER: i32 = 1;
    pub const PUSH_OTHER_TEAMS: i32 = 2;
    pub const PUSH_OWN_TEAM: i32 = 3;
}

/// `EntityDimensions`: a width and height, scaled by the entity's scale unless fixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
    pub fixed: bool,
}

impl Size {
    const fn scalable(width: f32, height: f32) -> Size {
        Size {
            width,
            height,
            fixed: false,
        }
    }

    /// `EntityDimensions.scale`.
    fn scale(self, factor: f32) -> Size {
        if self.fixed || factor == 1.0 {
            self
        } else {
            Size::scalable(self.width * factor, self.height * factor)
        }
    }

    /// `EntityDimensions.makeBoundingBox`: half the width in floats.
    pub fn bounding_box(self, p: [f64; 3]) -> Aabb {
        let w = f64::from(self.width / 2.0);
        let h = f64::from(self.height);
        Aabb {
            min: [p[0] - w, p[1], p[2] - w],
            max: [p[0] + w, p[1] + h, p[2] + w],
        }
    }
}

/// `LivingEntity.SLEEPING_DIMENSIONS`.
const SLEEPING: Size = Size {
    width: 0.2,
    height: 0.2,
    fixed: true,
};

/// `Avatar.POSES`: a player's size in each pose (standing for the others).
fn avatar_size(pose: i32) -> Size {
    match pose {
        pose::SLEEPING => SLEEPING,
        pose::FALL_FLYING | pose::SWIMMING | pose::SPIN_ATTACK => Size::scalable(0.6, 0.6),
        pose::CROUCHING => Size::scalable(0.6, 1.5),
        pose::DYING => Size {
            width: 0.2,
            height: 0.2,
            fixed: true,
        },
        _ => Size::scalable(0.6, 1.8),
    }
}

/// Whether the local player collides with an entity (`canBeCollidedWith`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Collider {
    No,
    /// Boats.
    Always,
    /// Shulkers, while alive.
    Alive,
    /// A grown happy ghast the player is above (on the client).
    HappyGhast,
}

/// How an entity follows the positions the server sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follow {
    /// Set at once (`InterpolationHandler.NO_OP`).
    Snap,
    /// `LinearInterpolationHandler` over this many ticks.
    Linear(i32),
    /// `SteppedInterpolationHandler` over the type's update interval.
    Stepped(i32),
}

/// What the client needs of an entity type.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub key: String,
    size: Size,
    living: bool,
    /// Players and mannequins (`Avatar`): sized by pose.
    avatar: bool,
    player: bool,
    boat: bool,
    minecart: bool,
    /// Living entities that never push players (`pushEntities` or `doPush` overridden:
    /// bats, armor stands, parrots; the ender dragon goes through everything).
    no_push: bool,
    /// Vexes go through everything while they tick (`noPhysics`).
    vex: bool,
    /// What the local player collides with (`canBeCollidedWith`).
    collider: Collider,
    follow: Follow,
    baby_field: Option<u8>,
    baby_size: Option<Size>,
    /// `getAgeScale` of a young one without a size of its own.
    age_scale: f32,
    /// `AbstractCubeMob.ID_SIZE`: slimes and magma cubes scale by it.
    cube_size_field: Option<u8>,
}

/// Every entity type, by network id.
#[derive(Debug, Clone, Default)]
pub struct EntityTypes {
    types: Vec<Option<TypeInfo>>,
    /// `Entity.DATA_POSE`.
    pose_field: Option<u8>,
    /// `LivingEntity.DATA_HEALTH_ID`.
    health_field: Option<u8>,
}

impl EntityTypes {
    pub fn new(data: &[EntityTypeData]) -> EntityTypes {
        let mut types = Vec::new();
        let mut pose_field = None;
        let mut health_field = None;
        for d in data {
            let living = d.is("LivingEntity");
            let follow = if living {
                if d.is("Shulker") {
                    Follow::Snap
                } else {
                    Follow::Stepped(d.update_interval)
                }
            } else if d.is("AbstractBoat")
                || d.is("AbstractMinecart")
                || d.is("ExperienceOrb")
                || d.is("FishingHook")
            {
                Follow::Linear(3)
            } else if d.is("Display") {
                Follow::Linear(0)
            } else {
                Follow::Snap
            };
            // `getAgeScale` where a class changes it; the others halve.
            let age_scale = match d.key.as_str() {
                "minecraft:dolphin" => 0.65,
                "minecraft:goat" => 0.55,
                "minecraft:camel" | "minecraft:armadillo" => 0.6,
                "minecraft:happy_ghast" => 0.2375,
                "minecraft:turtle" => 0.3,
                _ => 0.5,
            };
            pose_field = pose_field.or(d.field("Entity.DATA_POSE"));
            health_field = health_field.or(d.field("LivingEntity.DATA_HEALTH_ID"));
            let collider = if d.is("AbstractBoat") {
                Collider::Always
            } else if d.is("Shulker") {
                Collider::Alive
            } else if d.is("HappyGhast") {
                Collider::HappyGhast
            } else {
                Collider::No
            };
            let info = TypeInfo {
                key: d.key.clone(),
                size: Size {
                    width: d.width,
                    height: d.height,
                    fixed: d.fixed,
                },
                living,
                avatar: d.is("Avatar"),
                player: d.key == "minecraft:player",
                boat: d.is("AbstractBoat"),
                minecart: d.is("AbstractMinecart"),
                no_push: d.is("Bat") || d.is("ArmorStand") || d.is("Parrot") || d.is("EnderDragon"),
                vex: d.is("Vex"),
                collider,
                follow,
                baby_field: d.nearest_field("DATA_BABY_ID"),
                baby_size: d.baby.map(|(width, height, fixed)| Size {
                    width,
                    height,
                    fixed,
                }),
                age_scale,
                cube_size_field: d.field("AbstractCubeMob.ID_SIZE"),
            };
            let i = d.id as usize;
            if types.len() <= i {
                types.resize(i + 1, None);
            }
            types[i] = Some(info);
        }
        EntityTypes {
            types,
            pose_field,
            health_field,
        }
    }

    /// Reads the prepared game's export (`entity_dimensions.bin`).
    pub fn load(exports: &std::path::Path) -> Result<EntityTypes, String> {
        let path = exports.join(ruthenium_prepare::exporters::ENTITY_DIMENSIONS);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(EntityTypes::new(
            &ruthenium_prepare::exporters::read_entity_types(&bytes)?,
        ))
    }

    pub fn get(&self, id: i32) -> Option<&TypeInfo> {
        self.types.get(usize::try_from(id).ok()?)?.as_ref()
    }

    pub fn len(&self) -> usize {
        self.types.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A position and rotation (`PositionAndRotation`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct PosRot {
    position: [f64; 3],
    y_rot: f32,
    x_rot: f32,
}

impl PosRot {
    /// `PositionAndRotation.is`: rotations by value, the position as `Vec3.equals`
    /// (`Double.compare`: -0 differs from 0, NaN equals NaN).
    fn is(&self, position: [f64; 3], y_rot: f32, x_rot: f32) -> bool {
        self.y_rot == y_rot && self.x_rot == x_rot && same_vec(self.position, position)
    }

    fn add_delta(&mut self, d: [f64; 3]) {
        self.position = add(self.position, d);
    }

    fn add_rotation(&mut self, y: f32, x: f32) {
        self.y_rot += y;
        self.x_rot += x;
    }
}

fn same_vec(a: [f64; 3], b: [f64; 3]) -> bool {
    let same = |a: f64, b: f64| a.is_nan() && b.is_nan() || a.to_bits() == b.to_bits();
    (0..3).all(|i| same(a[i], b[i]))
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `Vec3.lerp`.
fn lerp_vec(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        mth::lerp_f64(t, a[0], b[0]),
        mth::lerp_f64(t, a[1], b[1]),
        mth::lerp_f64(t, a[2], b[2]),
    ]
}

fn length_sqr(v: [f64; 3]) -> f64 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// The interpolation handler's state (`AbstractInterpolationHandler` and its kinds).
#[derive(Debug, Clone)]
enum Interpolation {
    Snap,
    Linear {
        steps: i32,
        /// Where it is heading.
        target: PosRot,
        remaining: i32,
        last: PosRot,
    },
    Stepped(Box<Stepped>),
}

/// `SteppedInterpolationHandler`'s data.
#[derive(Debug, Clone)]
struct Stepped {
    steps: i32,
    /// The last target (the data's own position and rotation).
    target: PosRot,
    /// The steps still to take, each with its ticks.
    queue: VecDeque<(PosRot, i32)>,
    last_step: PosRot,
    current_step_ticks: f32,
    remaining_ticks: f32,
    speed: f32,
    /// `lastPositionAndRotation`: where the handler last left the entity.
    last: PosRot,
}

impl Stepped {
    fn add_step(&mut self, p: PosRot, ticks: i32) {
        self.queue.push_back((p, ticks));
        self.remaining_ticks += ticks as f32;
    }

    /// `InterpolationData.addSteps`.
    fn add_steps(&mut self, path: &PositionPath, y_rot: f32, x_rot: f32) {
        match path {
            PositionPath::Linear(p) => self.add_step(
                PosRot {
                    position: *p,
                    y_rot,
                    x_rot,
                },
                self.steps,
            ),
            PositionPath::Stepped(steps) => {
                if y_rot == self.target.y_rot && x_rot == self.target.x_rot {
                    for s in steps {
                        self.add_step(
                            PosRot {
                                position: s.position,
                                y_rot,
                                x_rot,
                            },
                            s.ticks,
                        );
                    }
                    return;
                }
                let total: i32 = steps.iter().map(|s| s.ticks).sum();
                let mut offset = 0;
                let (from_y, from_x) = (self.target.y_rot, self.target.x_rot);
                for s in steps {
                    offset += s.ticks;
                    let a = offset as f32 / total as f32;
                    self.add_step(
                        PosRot {
                            position: s.position,
                            y_rot: mth::rot_lerp(a, from_y, y_rot),
                            x_rot: mth::lerp(a, from_x, x_rot),
                        },
                        s.ticks,
                    );
                }
            }
        }
    }

    /// `InterpolationData.getNewPositionAndRotation`.
    fn next(&mut self) -> PosRot {
        while let Some(&(step, ticks)) = self.queue.front() {
            if self.current_step_ticks < ticks as f32 {
                let a = self.current_step_ticks / ticks as f32;
                return PosRot {
                    position: lerp_vec(self.last_step.position, step.position, f64::from(a)),
                    y_rot: mth::rot_lerp(a, self.last_step.y_rot, step.y_rot),
                    x_rot: mth::lerp(a, self.last_step.x_rot, step.x_rot),
                };
            }
            self.current_step_ticks -= ticks as f32;
            self.last_step = step;
            self.queue.pop_front();
        }
        self.target
    }

    /// `InterpolationData.advance`: speeds up while it lags behind.
    fn advance(&mut self, mut ticks: f32) {
        let steps = self.steps as f32;
        let target_speed = (self.remaining_ticks / steps).max(1.0);
        self.speed = mth::lerp(1.0 / steps, self.speed, target_speed);
        if ticks * self.speed < self.remaining_ticks {
            ticks *= self.speed;
        } else {
            ticks = self.remaining_ticks;
            self.speed = 1.0;
        }
        self.current_step_ticks += ticks;
        self.remaining_ticks -= ticks;
    }

    fn add_delta(&mut self, d: [f64; 3]) {
        self.target.add_delta(d);
        for (s, _) in &mut self.queue {
            s.add_delta(d);
        }
        self.last_step.add_delta(d);
    }

    fn add_rotation(&mut self, y: f32, x: f32) {
        self.target.add_rotation(y, x);
        for (s, _) in &mut self.queue {
            s.add_rotation(y, x);
        }
        self.last_step.add_rotation(y, x);
    }
}

/// An entity the server told the client about.
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: i32,
    pub uuid: u128,
    pub entity_type: i32,
    pub position: [f64; 3],
    pub old_position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub delta: [f64; 3],
    pub on_ground: bool,
    pub pose: i32,
    pub baby: bool,
    /// A living entity's health (1 until the server says otherwise).
    pub health: f32,
    /// A slime's or magma cube's size.
    pub cube_size: i32,
    pub scale: Attribute,
    pub size: Size,
    pub bounding_box: Aabb,
    pub vehicle: Option<i32>,
    pub passengers: Vec<i32>,
    pub removed: bool,
    pub tick_count: i32,
    /// `VecDeltaCodec`'s base: the last position the server sent.
    codec_base: [f64; 3],
    interpolation: Interpolation,
    /// A remote player that is a spectator goes through everything (`noPhysics`).
    no_physics: bool,
    /// The name scoreboard teams know it by: a player's name, else its UUID.
    scoreboard_name: String,
    section: i64,
    info: TypeInfo,
}

const ATTRIBUTE_SCALE: &str = "minecraft:scale";

/// `UUID.toString`.
pub fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

impl Entity {
    fn new(
        p: &play::AddEntity,
        info: &TypeInfo,
        name: Option<&str>,
        minecart_improvements: bool,
    ) -> Entity {
        let follow = if info.minecart && minecart_improvements {
            Follow::Snap
        } else {
            info.follow
        };
        let interpolation = match follow {
            Follow::Snap => Interpolation::Snap,
            Follow::Linear(steps) => Interpolation::Linear {
                steps,
                target: PosRot::default(),
                remaining: 0,
                last: PosRot::default(),
            },
            Follow::Stepped(steps) => Interpolation::Stepped(Box::new(Stepped {
                steps,
                target: PosRot::default(),
                queue: VecDeque::new(),
                last_step: PosRot::default(),
                current_step_ticks: 0.0,
                remaining_ticks: 0.0,
                speed: 1.0,
                last: PosRot::default(),
            })),
        };
        let mut e = Entity {
            id: p.entity_id,
            uuid: p.uuid,
            entity_type: p.entity_type,
            position: [0.0; 3],
            old_position: [0.0; 3],
            y_rot: 0.0,
            x_rot: 0.0,
            delta: [0.0; 3],
            on_ground: false,
            pose: pose::STANDING,
            baby: false,
            health: 1.0,
            cube_size: 1,
            // `Attributes.SCALE`'s default and range.
            scale: Attribute::new(1.0, 0.0625, 16.0),
            size: info.size,
            bounding_box: info.size.bounding_box([0.0; 3]),
            vehicle: None,
            passengers: Vec::new(),
            removed: false,
            tick_count: 0,
            codec_base: [0.0; 3],
            interpolation,
            no_physics: info.player,
            scoreboard_name: name.map_or_else(|| uuid_string(p.uuid), str::to_owned),
            section: 0,
            info: info.clone(),
        };
        e.size = e.dimensions();
        // `recreateFromPacket`.
        let (x, y, z) = (p.position[0], p.position[1], p.position[2]);
        let y_rot = mth::unpack_degrees(p.y_rot);
        let x_rot = mth::unpack_degrees(p.x_rot);
        e.codec_base = p.position;
        if info.living {
            // `absSnapTo`, `absSnapRotationTo`.
            let x = x.clamp(-3.0E7, 3.0E7);
            let z = z.clamp(-3.0E7, 3.0E7);
            e.old_position = [x, y, z];
            e.set_pos([x, y, z]);
            e.set_y_rot(y_rot % 360.0);
            e.set_x_rot(x_rot.clamp(-90.0, 90.0) % 360.0);
        } else {
            e.snap_to(p.position, y_rot, x_rot);
        }
        e.delta = p.movement;
        e.section = section_key(e.position);
        e
    }

    pub fn info(&self) -> &TypeInfo {
        &self.info
    }

    /// `getDimensions(getPose())`.
    fn dimensions(&self) -> Size {
        let t = &self.info;
        if !t.living {
            return t.size;
        }
        if self.pose == pose::SLEEPING {
            return SLEEPING;
        }
        let default = if t.avatar {
            avatar_size(self.pose)
        } else if t.cube_size_field.is_some() {
            t.size.scale(self.cube_size as f32)
        } else if self.baby {
            t.baby_size.unwrap_or_else(|| t.size.scale(t.age_scale))
        } else {
            t.size
        };
        default.scale(self.scale.value() as f32)
    }

    /// `refreshDimensions` (the client never nudges the entity to fit).
    fn refresh_dimensions(&mut self) {
        self.size = self.dimensions();
        self.set_pos(self.position);
    }

    /// `setPos`: the position and the box.
    fn set_pos(&mut self, p: [f64; 3]) {
        self.position = p;
        self.bounding_box = self.size.bounding_box(p);
    }

    fn set_y_rot(&mut self, y: f32) {
        if y.is_finite() {
            self.y_rot = y;
        }
    }

    fn set_x_rot(&mut self, x: f32) {
        if x.is_finite() {
            self.x_rot = (x % 360.0).clamp(-90.0, 90.0);
        }
    }

    /// `setRot`.
    fn set_rot(&mut self, y: f32, x: f32) {
        self.set_y_rot(y % 360.0);
        self.set_x_rot(x % 360.0);
    }

    /// `snapTo`: there at once, the old position too.
    fn snap_to(&mut self, p: [f64; 3], y: f32, x: f32) {
        self.position = p;
        self.set_y_rot(y);
        self.set_x_rot(x);
        self.old_position = p;
        self.set_pos(p);
    }

    fn current(&self) -> PosRot {
        PosRot {
            position: self.position,
            y_rot: self.y_rot,
            x_rot: self.x_rot,
        }
    }

    pub fn is_interpolating(&self) -> bool {
        match &self.interpolation {
            Interpolation::Snap => false,
            Interpolation::Linear { remaining, .. } => *remaining > 0,
            Interpolation::Stepped(s) => !s.queue.is_empty(),
        }
    }

    /// `getClientPositionAndRotation`: where it is heading, else where it is.
    fn client_pos_rot(&self) -> PosRot {
        if self.is_interpolating() {
            match &self.interpolation {
                Interpolation::Linear { target, .. } => *target,
                Interpolation::Stepped(s) => s.target,
                Interpolation::Snap => self.current(),
            }
        } else {
            self.current()
        }
    }

    fn cancel(&mut self) {
        match &mut self.interpolation {
            Interpolation::Snap => {}
            Interpolation::Linear { remaining, .. } => *remaining = 0,
            Interpolation::Stepped(s) => {
                s.queue.clear();
                s.remaining_ticks = 0.0;
                s.speed = 1.0;
            }
        }
    }

    /// `moveOrInterpolateTo`.
    fn move_or_interpolate_to(
        &mut self,
        path: Option<&PositionPath>,
        y: f32,
        x: f32,
        has_rotation: bool,
    ) {
        if matches!(self.interpolation, Interpolation::Snap) {
            if let Some(p) = path {
                self.set_pos(p.end());
            }
            if has_rotation {
                self.set_rot(y, x);
            }
            return;
        }
        // `AbstractInterpolationHandler.interpolateTo`.
        let current = self.client_pos_rot();
        let path = path
            .cloned()
            .unwrap_or(PositionPath::Linear(current.position));
        let (y, x) = if has_rotation {
            (y, x)
        } else {
            (current.y_rot, current.x_rot)
        };
        let end = path.end();
        let steps = match &self.interpolation {
            Interpolation::Linear { steps, .. } => *steps,
            Interpolation::Stepped(s) => s.steps,
            Interpolation::Snap => unreachable!(),
        };
        if steps == 0 {
            self.snap_to(end, y, x);
            self.cancel();
            return;
        }
        let active = self.is_interpolating();
        if active && self.client_pos_rot().is(end, y, x) {
            return;
        }
        let here = self.current();
        match &mut self.interpolation {
            Interpolation::Linear {
                target,
                remaining,
                steps,
                last,
            } => {
                *target = PosRot {
                    position: end,
                    y_rot: y,
                    x_rot: x,
                };
                *remaining = *steps;
                *last = here;
            }
            Interpolation::Stepped(s) => {
                if !active {
                    s.last_step = here;
                    s.current_step_ticks = 1.0;
                }
                if same_vec(end, s.target.position) {
                    let step = PosRot {
                        position: end,
                        y_rot: y,
                        x_rot: x,
                    };
                    let ticks = s.steps;
                    s.add_step(step, ticks);
                } else {
                    s.add_steps(&path, y, x);
                }
                s.target = PosRot {
                    position: end,
                    y_rot: y,
                    x_rot: x,
                };
                s.last = here;
            }
            Interpolation::Snap => unreachable!(),
        }
    }

    /// `InterpolationHandler.interpolate`, from `commonTick`. `free` says whether a box
    /// is clear of collisions (`noCollision`).
    fn interpolate(&mut self, relative_tick_speed: f32, free: &dyn Fn(&Aabb) -> bool) {
        if !self.is_interpolating() {
            self.cancel();
            return;
        }
        // `adjustInterpolationTargetFromDeltas`: something else moved it meanwhile.
        let here = self.current();
        let last = match &self.interpolation {
            Interpolation::Linear { last, .. } => *last,
            Interpolation::Stepped(s) => s.last,
            Interpolation::Snap => return,
        };
        let moved = sub(here.position, last.position);
        if length_sqr(moved) > 9.999999747378752E-6 {
            let target = self.client_pos_rot().position;
            if free(&self.size.bounding_box(add(target, moved))) {
                match &mut self.interpolation {
                    Interpolation::Linear { target, .. } => target.add_delta(moved),
                    Interpolation::Stepped(s) => s.add_delta(moved),
                    Interpolation::Snap => {}
                }
            }
        }
        let (dy, dx) = (here.y_rot - last.y_rot, here.x_rot - last.x_rot);
        match &mut self.interpolation {
            Interpolation::Linear { target, .. } => target.add_rotation(dy, dx),
            Interpolation::Stepped(s) => s.add_rotation(dy, dx),
            Interpolation::Snap => {}
        }
        // `doInterpolate`.
        match &mut self.interpolation {
            Interpolation::Linear {
                target, remaining, ..
            } => {
                let alpha = 1.0 / f64::from(*remaining);
                let to = *target;
                *remaining -= 1;
                let p = lerp_vec(here.position, to.position, alpha);
                let y = mth::rot_lerp_f64(alpha, f64::from(here.y_rot), f64::from(to.y_rot)) as f32;
                let x = mth::lerp_f64(alpha, f64::from(here.x_rot), f64::from(to.x_rot)) as f32;
                self.set_pos(p);
                self.set_rot(y, x);
            }
            Interpolation::Stepped(s) => {
                let next = s.next();
                s.advance(relative_tick_speed);
                self.set_pos(next.position);
                self.set_rot(next.y_rot, next.x_rot);
            }
            Interpolation::Snap => {}
        }
        let now = self.current();
        match &mut self.interpolation {
            Interpolation::Linear { last, .. } => *last = now,
            Interpolation::Stepped(s) => s.last = now,
            Interpolation::Snap => {}
        }
    }

    /// One field of entity data.
    fn set_data(&mut self, field: u8, value: &DataValue, types: &EntityTypes) {
        let t = &self.info;
        if t.living
            && Some(field) == types.health_field
            && let DataValue::Float(h) = value
        {
            self.health = *h;
        } else if Some(field) == types.pose_field
            && let DataValue::Pose(p) = value
        {
            self.pose = *p;
            self.refresh_dimensions();
        } else if Some(field) == t.baby_field
            && let DataValue::Boolean(b) = value
        {
            self.baby = *b;
            self.refresh_dimensions();
        } else if Some(field) == t.cube_size_field
            && let DataValue::Int(n) = value
        {
            self.cube_size = *n;
            self.refresh_dimensions();
        }
    }
}

/// `SectionPos.asLong` of a position's block.
fn section_key(p: [f64; 3]) -> i64 {
    let (x, y, z) = (
        mth::floor(p[0]) >> 4,
        mth::floor(p[1]) >> 4,
        mth::floor(p[2]) >> 4,
    );
    pack_section(x, y, z)
}

fn pack_section(x: i32, y: i32, z: i32) -> i64 {
    (i64::from(x) & 0x3F_FFFF) << 42 | i64::from(y) & 0xF_FFFF | (i64::from(z) & 0x3F_FFFF) << 20
}

fn section_chunk(key: i64) -> (i32, i32) {
    ((key >> 42) as i32, (key << 22 >> 42) as i32)
}

/// A scoreboard team, as far as pushing cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub name: String,
    pub collision_rule: i32,
}

/// The scoreboard's teams and who is on which (`Scoreboard`'s team part).
#[derive(Debug, Clone, Default)]
pub struct Teams {
    teams: HashMap<String, Team>,
    members: HashMap<String, String>,
}

impl Teams {
    /// `handleSetPlayerTeamPacket`.
    pub fn apply(
        &mut self,
        name: &str,
        method: u8,
        parameters: Option<&play::TeamParameters>,
        players: &[String],
    ) {
        if method == 0 {
            // `addPlayerTeam` (an existing name is refused by vanilla's scoreboard,
            // which keeps the old team).
            self.teams.entry(name.to_owned()).or_insert(Team {
                name: name.to_owned(),
                collision_rule: rule::ALWAYS,
            });
        } else if !self.teams.contains_key(name) {
            return;
        }
        if let Some(p) = parameters
            && let Some(t) = self.teams.get_mut(name)
        {
            // Unknown ids read as the first rule.
            t.collision_rule = if (0..=3).contains(&p.collision_rule) {
                p.collision_rule
            } else {
                rule::ALWAYS
            };
        }
        match method {
            0 | 3 => {
                for p in players {
                    self.members.insert(p.clone(), name.to_owned());
                }
            }
            4 => {
                for p in players {
                    if self.members.get(p).is_some_and(|t| t == name) {
                        self.members.remove(p);
                    }
                }
            }
            _ => {}
        }
        if method == 1 {
            self.teams.remove(name);
            self.members.retain(|_, t| t != name);
        }
    }

    /// `getPlayersTeam`.
    pub fn team_of(&self, scoreboard_name: &str) -> Option<&Team> {
        self.teams.get(self.members.get(scoreboard_name)?)
    }
}

/// What the client knows of a player from the player list (`PlayerInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub name: String,
    pub game_mode: i32,
}

/// The player list (`ClientPacketListener.playerInfoMap`).
#[derive(Debug, Clone, Default)]
pub struct PlayerInfos {
    players: HashMap<u128, PlayerInfo>,
}

impl PlayerInfos {
    /// `handlePlayerInfoUpdate`: new players first, then every entry's updates.
    pub fn update(&mut self, actions: u8, entries: &[play::PlayerInfoEntry]) {
        if actions & play::PLAYER_INFO_ADD != 0 {
            for e in entries {
                if let Some(name) = &e.name {
                    self.players.entry(e.uuid).or_insert(PlayerInfo {
                        name: name.clone(),
                        game_mode: 0,
                    });
                }
            }
        }
        for e in entries {
            if let Some(p) = self.players.get_mut(&e.uuid)
                && let Some(m) = e.game_mode
            {
                p.game_mode = m;
            }
        }
    }

    pub fn remove(&mut self, ids: &[u128]) {
        for id in ids {
            self.players.remove(id);
        }
    }

    pub fn get(&self, uuid: u128) -> Option<&PlayerInfo> {
        self.players.get(&uuid)
    }
}

/// What pushing needs to know of the local player besides its state.
#[derive(Debug, Clone, Copy)]
pub struct Local<'a> {
    /// Its scoreboard name: the profile's name.
    pub name: &'a str,
    pub spectator: bool,
    pub health: f32,
}

/// What one tick of the entities needs.
#[derive(Debug)]
pub struct TickContext<'a, 'e> {
    pub env: &'a Env<'e>,
    pub teams: &'a Teams,
    pub players: &'a PlayerInfos,
    pub local: Local<'a>,
    /// The server's tick rate.
    pub tick_rate: f32,
    /// `TickRateManager.runsNormally`: false while the game is frozen.
    pub runs_normally: bool,
}

/// A level's entities (`ClientLevel`'s entity storage and tick list).
#[derive(Debug, Clone, Default)]
pub struct Entities {
    entities: HashMap<i32, Entity>,
    /// `EntityTickList`: ids in the order they started ticking.
    ticking: Vec<i32>,
    /// `EntitySectionStorage`: each section's entities, in the order they arrived.
    sections: BTreeMap<i64, Vec<i32>>,
    ticking_chunks: std::collections::HashSet<(i32, i32)>,
    local_id: Option<i32>,
    /// The local player's vehicle and passengers.
    local_vehicle: Option<i32>,
    local_passengers: Vec<i32>,
    minecart_improvements: bool,
}

impl Entities {
    /// A new level's entities with the local player in it (`addEntity` of the player:
    /// it always ticks).
    pub fn new(local_id: i32, minecart_improvements: bool) -> Entities {
        Entities {
            ticking: vec![local_id],
            local_id: Some(local_id),
            minecart_improvements,
            ..Entities::default()
        }
    }

    pub fn get(&self, id: i32) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// The ids in tick order, the local player's among them.
    pub fn tick_order(&self) -> &[i32] {
        &self.ticking
    }

    /// The local player came back (`handleRespawn` in the same level): a new player
    /// with the same id, so it ticks last now.
    pub fn local_respawned(&mut self) {
        if let Some(id) = self.local_id {
            self.ticking.retain(|&t| t != id);
            self.ticking.push(id);
            self.unride_local();
        }
    }

    fn start_ticking(&mut self, id: i32) {
        if !self.ticking.contains(&id) {
            self.ticking.push(id);
        }
    }

    fn stop_ticking(&mut self, id: i32) {
        self.ticking.retain(|&t| t != id);
    }

    fn chunk_ticks(&self, section: i64) -> bool {
        self.ticking_chunks.contains(&section_chunk(section))
    }

    /// A chunk arrived (`onChunkLoaded` → `startTicking`): its entities start ticking,
    /// section by section.
    pub fn chunk_loaded(&mut self, x: i32, z: i32) {
        if !self.ticking_chunks.insert((x, z)) {
            return;
        }
        for id in self.chunk_entities(x, z) {
            self.start_ticking(id);
        }
    }

    /// A chunk went (`unload` → `stopTicking`).
    pub fn chunk_unloaded(&mut self, x: i32, z: i32) {
        if !self.ticking_chunks.remove(&(x, z)) {
            return;
        }
        for id in self.chunk_entities(x, z) {
            self.stop_ticking(id);
        }
    }

    /// The entities in a chunk's sections, in `getExistingSectionsInChunk`'s order:
    /// sections by their key's low 20 bits (y 0 upwards, then the negative ones).
    fn chunk_entities(&self, x: i32, z: i32) -> Vec<i32> {
        let from = pack_section(x, 0, z);
        let to = pack_section(x, -1, z);
        self.sections
            .range(from..=to)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect()
    }

    /// The entity moved: into another section (`Callback.onMove`), maybe into a chunk
    /// that doesn't tick.
    fn moved(&mut self, id: i32) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let new = section_key(e.position);
        let old = e.section;
        if new == old {
            return;
        }
        e.section = new;
        if let Some(s) = self.sections.get_mut(&old) {
            s.retain(|&i| i != id);
            if s.is_empty() {
                self.sections.remove(&old);
            }
        }
        self.sections.entry(new).or_default().push(id);
        match (self.chunk_ticks(old), self.chunk_ticks(new)) {
            (true, false) => self.stop_ticking(id),
            (false, true) => self.start_ticking(id),
            _ => {}
        }
    }

    /// `handleAddEntity`: a player needs its player-list entry first.
    pub fn add(&mut self, p: &play::AddEntity, types: &EntityTypes, players: &PlayerInfos) {
        if Some(p.entity_id) == self.local_id {
            eprintln!(
                "[entities] the server added an entity with the player's id {}; ignored",
                p.entity_id
            );
            return;
        }
        let Some(info) = types.get(p.entity_type) else {
            eprintln!(
                "[entities] skipping entity {} of unknown type {}",
                p.entity_id, p.entity_type
            );
            return;
        };
        let name = if info.player {
            match players.get(p.uuid) {
                Some(i) => Some(i.name.as_str()),
                None => {
                    eprintln!(
                        "[entities] the server added player {} before its player info",
                        uuid_string(p.uuid)
                    );
                    return;
                }
            }
        } else {
            None
        };
        // `ClientLevel.addEntity`: one with the same id goes first.
        self.remove(p.entity_id);
        let e = Entity::new(p, info, name, self.minecart_improvements);
        let (id, section) = (e.id, e.section);
        self.entities.insert(id, e);
        self.sections.entry(section).or_default().push(id);
        if self.chunk_ticks(section) {
            self.start_ticking(id);
        }
    }

    /// `handleRemoveEntities` for one id (`removeEntity`; the local player isn't one
    /// of these, so it stays).
    pub fn remove(&mut self, id: i32) {
        let Some(e) = self.entities.remove(&id) else {
            return;
        };
        if let Some(s) = self.sections.get_mut(&e.section) {
            s.retain(|&i| i != id);
            if s.is_empty() {
                self.sections.remove(&e.section);
            }
        }
        self.stop_ticking(id);
        // `onTrackingEnd` → `unRide`: off its vehicle, its passengers off it.
        self.stop_riding(id, e.vehicle);
        for p in e.passengers {
            self.set_vehicle(p, None);
        }
    }

    fn set_vehicle(&mut self, id: i32, vehicle: Option<i32>) {
        if Some(id) == self.local_id {
            self.local_vehicle = vehicle;
        } else if let Some(e) = self.entities.get_mut(&id) {
            e.vehicle = vehicle;
        }
    }

    fn vehicle_of(&self, id: i32) -> Option<i32> {
        if Some(id) == self.local_id {
            self.local_vehicle
        } else {
            self.entities.get(&id)?.vehicle
        }
    }

    fn passengers_of(&mut self, id: i32) -> Option<&mut Vec<i32>> {
        if Some(id) == self.local_id {
            Some(&mut self.local_passengers)
        } else {
            self.entities.get_mut(&id).map(|e| &mut e.passengers)
        }
    }

    /// `stopRiding`: off `vehicle`.
    fn stop_riding(&mut self, id: i32, vehicle: Option<i32>) {
        let Some(v) = vehicle else {
            return;
        };
        self.set_vehicle(id, None);
        if let Some(ps) = self.passengers_of(v) {
            ps.retain(|&p| p != id);
        }
    }

    fn unride_local(&mut self) {
        if let Some(id) = self.local_id {
            self.stop_riding(id, self.local_vehicle);
            for p in std::mem::take(&mut self.local_passengers) {
                self.set_vehicle(p, None);
            }
        }
    }

    fn exists(&self, id: i32) -> bool {
        Some(id) == self.local_id || self.entities.contains_key(&id)
    }

    /// `handleSetEntityPassengersPacket`: the vehicle's passengers replaced.
    pub fn set_passengers(&mut self, vehicle: i32, passengers: &[i32]) {
        if !self.exists(vehicle) {
            return;
        }
        // `ejectPassengers`.
        let old = self
            .passengers_of(vehicle)
            .map(std::mem::take)
            .unwrap_or_default();
        for p in old {
            self.set_vehicle(p, None);
        }
        for &p in passengers {
            if !self.exists(p) || self.vehicle_of(p) == Some(vehicle) {
                continue;
            }
            // Not onto something that rides it.
            let mut v = Some(vehicle);
            let mut cycle = false;
            while let Some(id) = v {
                if id == p {
                    cycle = true;
                    break;
                }
                v = self.vehicle_of(id);
            }
            if cycle {
                continue;
            }
            self.stop_riding(p, self.vehicle_of(p));
            self.set_vehicle(p, Some(vehicle));
            if let Some(ps) = self.passengers_of(vehicle) {
                ps.push(p);
            }
        }
    }

    /// `getRootVehicle`.
    fn root_vehicle(&self, mut id: i32) -> i32 {
        let mut guard = 0;
        while let Some(v) = self.vehicle_of(id) {
            id = v;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        id
    }

    /// `handleMoveEntity`.
    pub fn move_entity(
        &mut self,
        id: i32,
        delta: Option<&VecDelta>,
        rotation: Option<(i8, i8)>,
        on_ground: bool,
    ) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let rot = rotation.map(|(y, x)| (mth::unpack_degrees(y), mth::unpack_degrees(x)));
        if let Some(d) = delta {
            let path = decode_delta(&mut e.codec_base, d);
            e.codec_base = path.end();
            match rot {
                Some((y, x)) => e.move_or_interpolate_to(Some(&path), y, x, true),
                None => e.move_or_interpolate_to(Some(&path), 0.0, 0.0, false),
            }
        } else if let Some((y, x)) = rot {
            e.move_or_interpolate_to(None, y, x, true);
        }
        e.on_ground = on_ground;
        self.moved(id);
    }

    /// `handleEntityPositionSync`.
    pub fn position_sync(
        &mut self,
        id: i32,
        path: &PositionPath,
        y_rot: f32,
        x_rot: f32,
        on_ground: bool,
    ) {
        let ticking = self.ticking.contains(&id);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let end = path.end();
        e.codec_base = end;
        let too_far = length_sqr(sub(e.position, end)) > 4096.0;
        if ticking && !too_far {
            e.move_or_interpolate_to(Some(path), y_rot, x_rot, true);
        } else {
            e.snap_to(end, y_rot, x_rot);
        }
        e.on_ground = on_ground;
        self.moved(id);
    }

    /// `handleTeleportEntity` (`setValuesFromPositionPacket`, interpolating unless it
    /// is too far).
    pub fn teleport(&mut self, id: i32, change: &play::PositionChange, on_ground: bool) {
        let play::PositionChange {
            position,
            delta,
            y_rot,
            x_rot,
            relatives,
        } = *change;
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let current = e.client_pos_rot();
        let (pos, new_delta, y, x) = crate::player::calculate_absolute(
            current.position,
            e.delta,
            current.y_rot,
            current.x_rot,
            position,
            delta,
            y_rot,
            x_rot,
            relatives,
        );
        if length_sqr(sub(current.position, pos)) <= 4096.0 {
            e.move_or_interpolate_to(Some(&PositionPath::Linear(pos)), y, x, true);
            e.delta = new_delta;
        } else {
            e.set_pos(pos);
            e.delta = new_delta;
            e.set_y_rot(y);
            e.set_x_rot(x);
            let (old, _, _, _) = crate::player::calculate_absolute(
                e.old_position,
                [0.0; 3],
                0.0,
                0.0,
                position,
                delta,
                y_rot,
                x_rot,
                relatives,
            );
            e.old_position = old;
        }
        e.on_ground = on_ground;
        self.moved(id);
    }

    /// `handleSetEntityMotion` for another entity (`lerpMotion`; remote players ease
    /// into it, which only shows in how they're drawn).
    pub fn set_motion(&mut self, id: i32, movement: [f64; 3]) {
        if let Some(e) = self.entities.get_mut(&id) {
            e.delta = movement;
        }
    }

    /// `handleSetEntityData`.
    pub fn set_data(&mut self, id: i32, values: &[(u8, DataValue)], types: &EntityTypes) {
        if let Some(e) = self.entities.get_mut(&id) {
            for (field, v) in values {
                e.set_data(*field, v, types);
            }
        }
    }

    /// `handleUpdateAttributes` for another entity: only the scale changes its size.
    pub fn update_attributes(&mut self, id: i32, attributes: &[play::AttributeSnapshot]) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        if !e.info.living {
            return;
        }
        for a in attributes {
            let name = u32::try_from(a.attribute)
                .ok()
                .and_then(ruthenium_data::v26_3::registries::Attribute::from_id)
                .map(|a| a.name());
            if name == Some(ATTRIBUTE_SCALE) {
                e.scale.base = a.base;
                e.scale.modifiers = a
                    .modifiers
                    .iter()
                    .map(|(id, amount, operation)| Modifier {
                        id: id.clone(),
                        amount: *amount,
                        operation: *operation,
                    })
                    .collect();
                e.refresh_dimensions();
            }
        }
    }

    /// `ClientLevel.tickEntities`: every entity that ticks, in order, the local player
    /// in its place (`tick_player` runs its tick), each vehicle's passengers right after
    /// it.
    pub fn tick(
        &mut self,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        mut tick_player: impl FnMut(&mut LocalPlayer, &[Aabb]),
    ) {
        // The list as it was when the iteration began (`EntityTickList` copies it if
        // it changes meanwhile).
        let order = self.ticking.clone();
        let speed = if cx.tick_rate > 20.0 {
            cx.tick_rate / 20.0
        } else {
            1.0
        };
        for id in order {
            if Some(id) == self.local_id {
                // Riding isn't simulated yet: the player ticks in its place as if on
                // foot, where vanilla ticks it after its vehicle.
                let colliders = self.colliders(player);
                tick_player(player, &colliders);
                continue;
            }
            let Some(e) = self.entities.get(&id) else {
                continue;
            };
            // Frozen: only players (and what carries one) tick.
            let frozen = !cx.runs_normally && !e.info.player;
            if e.removed || e.vehicle.is_some() || frozen {
                continue;
            }
            self.tick_one(id, cx, player, speed, false);
        }
    }

    /// `tickNonPassenger` / `tickPassenger`: the entity's tick, then its passengers'.
    fn tick_one(
        &mut self,
        id: i32,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        speed: f32,
        riding: bool,
    ) {
        let free = |b: &Aabb| cx.env.no_collision(b);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        // `commonTick`.
        e.old_position = e.position;
        e.interpolate(speed, &free);
        e.tick_count += 1;
        if riding {
            e.delta = [0.0; 3];
        }
        self.moved(id);
        self.entity_tick(id, cx, player, riding);
        let passengers = self
            .entities
            .get(&id)
            .map(|e| e.passengers.clone())
            .unwrap_or_default();
        for p in passengers {
            let ok = self
                .entities
                .get(&p)
                .is_some_and(|e| !e.removed && e.vehicle == Some(id));
            if ok && self.ticking.contains(&p) {
                self.tick_one(p, cx, player, speed, true);
            }
        }
    }

    /// `Entity.tick`, for what it does to the local player.
    fn entity_tick(
        &mut self,
        id: i32,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        riding: bool,
    ) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        if e.info.player {
            // `Player.tick`.
            e.no_physics = cx
                .players
                .get(e.uuid)
                .is_some_and(|i| i.game_mode == play::SPECTATOR);
        }
        let e = &self.entities[&id];
        // Passengers aren't moved with their vehicle yet, so they'd push from a stale
        // place.
        if riding {
            return;
        }
        if e.info.living && !e.info.no_push {
            // `pushEntities` → `getPushableEntities` → `doPush` → `LocalPlayer.push`.
            if player.bounding_box().intersects(&e.bounding_box) && self.pushable_by(e, cx, player)
            {
                let no_physics = e.no_physics || e.info.vex;
                self.push_player(player, cx, e.position, no_physics, id, -1.0);
            }
        } else if e.info.boat {
            // `AbstractBoat.tick`'s pushes, with `AbstractBoat.push`'s height rule.
            let reach = e.bounding_box.inflate([
                0.20000000298023224,
                -0.009999999776482582,
                0.20000000298023224,
            ]);
            let b = player.bounding_box();
            if b.intersects(&reach)
                && self.pushable_by(e, cx, player)
                && !self.local_passengers.contains(&id)
                && b.min[1] <= e.bounding_box.min[1]
            {
                self.push_player(player, cx, e.position, e.no_physics, id, 1.0);
            }
        }
    }

    /// The boxes the local player collides with as it moves (`getEntityCollisions`'s
    /// candidates: `canCollideWith`, not spectators; the area test is the player's).
    pub fn colliders(&self, player: &LocalPlayer) -> Vec<Aabb> {
        let Some(local) = self.local_id else {
            return Vec::new();
        };
        let root = self.root_vehicle(local);
        self.entities
            .values()
            .filter(|e| {
                let can = match e.info.collider {
                    Collider::No => false,
                    Collider::Always => true,
                    Collider::Alive => e.health > 0.0,
                    Collider::HappyGhast => {
                        !e.baby && e.health > 0.0 && player.position[1] >= e.bounding_box.max[1]
                    }
                };
                can && !e.removed && self.root_vehicle(e.id) != root
            })
            .map(|e| e.bounding_box)
            .collect()
    }

    /// `EntitySelector.pushableBy(pusher)` tested on the local player.
    fn pushable_by(&self, pusher: &Entity, cx: &TickContext<'_, '_>, player: &LocalPlayer) -> bool {
        let own = cx.teams.team_of(&pusher.scoreboard_name);
        let own_rule = own.map_or(rule::ALWAYS, |t| t.collision_rule);
        if own_rule == rule::NEVER || cx.local.spectator || !self.player_pushable(cx, player) {
            return false;
        }
        let theirs = cx.teams.team_of(cx.local.name);
        let their_rule = theirs.map_or(rule::ALWAYS, |t| t.collision_rule);
        if their_rule == rule::NEVER {
            return false;
        }
        let same_team = own.is_some() && own.map(|t| &t.name) == theirs.map(|t| &t.name);
        if (own_rule == rule::PUSH_OWN_TEAM || their_rule == rule::PUSH_OWN_TEAM) && same_team {
            false
        } else {
            own_rule != rule::PUSH_OTHER_TEAMS && their_rule != rule::PUSH_OTHER_TEAMS || same_team
        }
    }

    /// `LivingEntity.isPushable` for the local player: alive, not a spectator, not
    /// climbing.
    fn player_pushable(&self, cx: &TickContext<'_, '_>, player: &LocalPlayer) -> bool {
        cx.local.health > 0.0 && !cx.local.spectator && !player.on_climbable(cx.env)
    }

    /// `Entity.push(Entity)` between the local player and `other`: `sign` -1 when
    /// the player is the one pushed by its own `push` (the other's position minus the
    /// player's, negated), 1 when the other pushes it (the player's minus the other's).
    fn push_player(
        &self,
        player: &mut LocalPlayer,
        cx: &TickContext<'_, '_>,
        other: [f64; 3],
        other_no_physics: bool,
        other_id: i32,
        sign: f64,
    ) {
        let Some(local) = self.local_id else {
            return;
        };
        if self.root_vehicle(local) == self.root_vehicle(other_id) {
            return;
        }
        // The player goes through everything as a spectator (`noPhysics`).
        if other_no_physics || cx.local.spectator {
            return;
        }
        let (mut xa, mut za) = if sign < 0.0 {
            (other[0] - player.position[0], other[2] - player.position[2])
        } else {
            (player.position[0] - other[0], player.position[2] - other[2])
        };
        let mut dd = mth::abs_max(xa, za);
        if dd >= 0.009999999776482582 {
            dd = dd.sqrt();
            xa /= dd;
            za /= dd;
            let pow = (1.0 / dd).min(1.0);
            xa *= pow;
            za *= pow;
            xa *= 0.05000000074505806;
            za *= 0.05000000074505806;
            if self.local_passengers.is_empty() && self.player_pushable(cx, player) {
                let (xa, za) = (sign * xa, sign * za);
                if xa.is_finite() && za.is_finite() {
                    let d = &mut player.delta;
                    *d = [d[0] + xa, d[1], d[2] + za];
                }
            }
        }
    }
}

/// `VecDelta.decode`: each step from the last (the codec's base), back to the base
/// after.
fn decode_delta(base: &mut [f64; 3], d: &VecDelta) -> PositionPath {
    match d {
        VecDelta::Linear(v) => PositionPath::Linear(decode_from(*base, *v)),
        VecDelta::Stepped(steps) if steps.is_empty() => PositionPath::Linear(*base),
        VecDelta::Stepped(steps) => {
            let mut at = *base;
            let mut out = Vec::with_capacity(steps.len());
            for &(v, ticks) in steps {
                at = decode_from(at, v);
                out.push(play::PositionStep {
                    position: at,
                    ticks,
                });
            }
            PositionPath::Stepped(out)
        }
    }
}

/// `VecDeltaCodec.decode`: an axis without a delta keeps the base exactly.
fn decode_from(base: [f64; 3], v: [i16; 3]) -> [f64; 3] {
    if v == [0; 3] {
        return base;
    }
    let axis = |b: f64, d: i16| {
        if d == 0 {
            b
        } else {
            (mth::round(b * 4096.0) + i64::from(d)) as f64 / 4096.0
        }
    };
    [
        axis(base[0], v[0]),
        axis(base[1], v[1]),
        axis(base[2], v[2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{BlockTags, Blocks};
    use crate::world::Level;
    use ruthenium_prepare::exporters::{BlockPhysics, ShapeData, StatePhysics};

    const ZOMBIE: i32 = 0;
    const BOAT: i32 = 1;
    const BAT: i32 = 2;
    const PLAYER_ID: i32 = 1;

    fn data(
        id: u32,
        key: &str,
        classes: &[&str],
        size: (f32, f32),
        fields: &[(&str, u8)],
    ) -> EntityTypeData {
        EntityTypeData {
            id,
            key: key.into(),
            width: size.0,
            height: size.1,
            eye_height: size.1 * 0.85,
            fixed: false,
            update_interval: 3,
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            fields: fields
                .iter()
                .map(|(n, i)| ((*n).to_owned(), *i, 0))
                .collect(),
            baby: (key == "minecraft:zombie").then_some((0.49, 0.98, false)),
        }
    }

    fn types() -> EntityTypes {
        let living = ["Mob", "LivingEntity", "Entity"];
        let zombie: Vec<&str> = ["Zombie"].into_iter().chain(living).collect();
        let bat: Vec<&str> = ["Bat"].into_iter().chain(living).collect();
        EntityTypes::new(&[
            data(
                0,
                "minecraft:zombie",
                &zombie,
                (0.6, 1.95),
                &[
                    ("Entity.DATA_POSE", 6),
                    ("LivingEntity.DATA_HEALTH_ID", 9),
                    ("Zombie.DATA_BABY_ID", 16),
                ],
            ),
            data(
                1,
                "minecraft:oak_boat",
                &["Boat", "AbstractBoat", "Entity"],
                (1.375, 0.5625),
                &[],
            ),
            data(2, "minecraft:bat", &bat, (0.5, 0.9), &[]),
        ])
    }

    /// Air only, and one chunk at the origin.
    fn world() -> (Level, Blocks, BlockTags) {
        let blocks = Blocks::new(&BlockPhysics {
            shapes: vec![ShapeData {
                coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
                full: vec![false],
            }],
            states: vec![StatePhysics {
                shape: 0,
                flags: 0,
                friction: 0.6,
                speed_factor: 1.0,
                jump_factor: 1.0,
                bounce: 0.0,
                fluid: 0,
                sturdy: 0,
                offset: None,
            }],
        });
        let mut level = Level::new(-64, 384);
        level.add_empty_chunk(0, 0);
        (level, blocks, BlockTags::new())
    }

    fn add(e: &mut Entities, id: i32, entity_type: i32, position: [f64; 3]) {
        let p = play::AddEntity {
            entity_id: id,
            uuid: 1000 + id as u128,
            entity_type,
            position,
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: 0,
        };
        e.add(&p, &types(), &PlayerInfos::default());
    }

    fn player_at(p: [f64; 3]) -> LocalPlayer {
        let mut player = LocalPlayer::new();
        player.entity_id = PLAYER_ID;
        player.position = p;
        player
    }

    /// One tick of the entities; the player itself doesn't move.
    fn tick(e: &mut Entities, teams: &Teams, player: &mut LocalPlayer) {
        let (level, blocks, tags) = world();
        let env = Env {
            level: &level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
        };
        let players = PlayerInfos::default();
        let cx = TickContext {
            env: &env,
            teams,
            players: &players,
            local: Local {
                name: "Developer",
                spectator: false,
                health: 20.0,
            },
            tick_rate: 20.0,
            runs_normally: true,
        };
        e.tick(&cx, player, |_, _| {});
    }

    /// `Entity.push` as vanilla computes it for an `(dx, dz)` from the pushed one to the
    /// pusher: the push the pushed one gets.
    fn push(dx: f64, dz: f64) -> (f64, f64) {
        let dd = mth::abs_max(dx, dz).sqrt();
        let pow = (1.0 / dd).min(1.0);
        (
            -(dx / dd * pow * 0.05000000074505806),
            -(dz / dd * pow * 0.05000000074505806),
        )
    }

    #[test]
    fn a_zombie_pushes_the_player_standing_in_it() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, ZOMBIE, [0.8, -60.0, 0.6]);
        let mut p = player_at([0.5, -60.0, 0.5]);
        tick(&mut e, &Teams::default(), &mut p);
        let (x, z) = push(0.8 - 0.5, 0.6 - 0.5);
        assert_eq!(p.delta, [x, 0.0, z]);
        // Out of reach: no push.
        let mut p = player_at([2.0, -60.0, 0.5]);
        tick(&mut e, &Teams::default(), &mut p);
        assert_eq!(p.delta, [0.0; 3]);
    }

    #[test]
    fn bats_never_push_boats_do_from_their_level() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, BAT, [0.8, -60.0, 0.6]);
        let mut p = player_at([0.5, -60.0, 0.5]);
        tick(&mut e, &Teams::default(), &mut p);
        assert_eq!(p.delta, [0.0; 3]);
        e.remove(5);
        add(&mut e, 6, BOAT, [1.5, -60.0, 0.5]);
        tick(&mut e, &Teams::default(), &mut p);
        // Away from the boat, along x only.
        let (x, z) = push(1.5 - 0.5, 0.0);
        assert_eq!(p.delta, [x, 0.0, z]);
        // A player above the boat's bottom isn't pushed.
        let mut p = player_at([0.5, -59.9, 0.5]);
        tick(&mut e, &Teams::default(), &mut p);
        assert_eq!(p.delta, [0.0; 3]);
        // But stands on it.
        assert_eq!(e.colliders(&p).len(), 1);
    }

    #[test]
    fn team_rules_decide_who_pushes() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, ZOMBIE, [0.8, -60.0, 0.6]);
        let zombie = uuid_string(1005);
        let mut teams = Teams::default();
        let rules = |teams: &mut Teams, rule: i32| {
            let p = play::TeamParameters {
                name_tag_visibility: 0,
                collision_rule: rule,
                color: None,
                options: 0,
            };
            teams.apply("t", 2, Some(&p), &[]);
        };
        let pushed = |teams: &Teams, e: &mut Entities| {
            let mut p = player_at([0.5, -60.0, 0.5]);
            tick(e, teams, &mut p);
            p.delta != [0.0; 3]
        };
        teams.apply("t", 0, None, std::slice::from_ref(&zombie));
        assert!(pushed(&teams, &mut e), "a team that always collides");
        rules(&mut teams, rule::NEVER);
        assert!(!pushed(&teams, &mut e));
        rules(&mut teams, rule::PUSH_OTHER_TEAMS);
        assert!(!pushed(&teams, &mut e), "the player isn't on another team");
        teams.apply("t", 3, None, &["Developer".to_owned()]);
        assert!(pushed(&teams, &mut e), "both on the team");
        rules(&mut teams, rule::PUSH_OWN_TEAM);
        assert!(!pushed(&teams, &mut e));
        teams.apply("t", 4, None, &["Developer".to_owned()]);
        assert!(pushed(&teams, &mut e));
        teams.apply("t", 1, None, &[]);
        assert!(teams.team_of(&zombie).is_none());
    }

    #[test]
    fn entities_tick_where_their_chunk_is_loaded_in_vanillas_order() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, ZOMBIE, [0.5, -60.0, 0.5]);
        add(&mut e, 6, ZOMBIE, [40.5, -60.0, 0.5]);
        assert_eq!(e.tick_order(), [PLAYER_ID, 5]);
        e.chunk_loaded(2, 0);
        assert_eq!(e.tick_order(), [PLAYER_ID, 5, 6]);
        // Into a chunk that isn't there: it stops, and starts again at the end.
        let away = play::PositionChange {
            position: [90.5, -60.0, 0.5],
            delta: [0.0; 3],
            y_rot: 0.0,
            x_rot: 0.0,
            relatives: 0,
        };
        e.teleport(5, &away, true);
        assert_eq!(e.tick_order(), [PLAYER_ID, 6]);
        e.chunk_loaded(5, 0);
        assert_eq!(e.tick_order(), [PLAYER_ID, 6, 5]);
        e.chunk_unloaded(2, 0);
        assert_eq!(e.tick_order(), [PLAYER_ID, 5]);
        // A respawned player is new: it ticks last.
        e.local_respawned();
        assert_eq!(e.tick_order(), [5, PLAYER_ID]);
    }

    #[test]
    fn a_baby_zombie_is_smaller() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, ZOMBIE, [0.5, -60.0, 0.5]);
        assert_eq!(e.get(5).unwrap().size, Size::scalable(0.6, 1.95));
        e.set_data(5, &[(16, DataValue::Boolean(true))], &types());
        assert_eq!(e.get(5).unwrap().size, Size::scalable(0.49, 0.98));
        e.set_data(5, &[(6, DataValue::Pose(pose::SLEEPING))], &types());
        assert_eq!(e.get(5).unwrap().size, SLEEPING);
    }

    #[test]
    fn stepped_interpolation_takes_the_update_interval() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, ZOMBIE, [0.5, -60.0, 0.5]);
        // 0.3 blocks along x: 1228.8 sixteenths of a 4096th, rounded.
        e.move_entity(5, Some(&VecDelta::Linear([1229, 0, 0])), None, true);
        let target = (mth::round(0.5 * 4096.0) + 1229) as f64 / 4096.0;
        let mut p = player_at([10.0, -60.0, 0.5]);
        let mut xs = Vec::new();
        for _ in 0..4 {
            tick(&mut e, &Teams::default(), &mut p);
            xs.push(e.get(5).unwrap().position[0]);
        }
        let third = |a: f32| mth::lerp_f64(f64::from(a), 0.5, target);
        assert_eq!(xs, [third(1.0 / 3.0), third(2.0 / 3.0), target, target]);
    }

    #[test]
    fn deltas_keep_untouched_axes_and_round_half_up() {
        let base = [-0.5 / 4096.0, 7.123456789, 0.25];
        // Java's Math.round(-0.5) is 0, so x lands on one step.
        assert_eq!(
            decode_from(base, [1, 0, 0]),
            [1.0 / 4096.0, 7.123456789, 0.25]
        );
        assert_eq!(decode_from(base, [0; 3]), base);
        let mut b = [0.0; 3];
        let path = decode_delta(
            &mut b,
            &VecDelta::Stepped(vec![([4096, 0, 0], 1), ([4096, 0, 0], 2)]),
        );
        assert_eq!(path.end(), [2.0, 0.0, 0.0]);
        assert_eq!(b, [0.0; 3], "the base is the caller's to move");
    }
}
