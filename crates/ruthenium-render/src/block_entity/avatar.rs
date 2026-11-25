//! How a player or mannequin stands: its place and turn, and those of its limbs, at a
//! moment between two ticks.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6, PI, TAU};

use ruthenium_javamath::mth;
use ruthenium_sim::avatar::{ArmPose, AvatarRecord, Chest, Swing};
use ruthenium_sim::entity::pose;
use ruthenium_sim::scene::EntityRecord;

use super::affine::Affine;
use super::model::Moved;

const DEGREE: f32 = 0.017_453_292;
/// Ticks a crossbow takes to load, without enchantments.
const CROSSBOW_TICKS: f32 = 25.0;

/// The second layers' bits.
mod part {
    pub const JACKET: u8 = 1 << 1;
    pub const LEFT_SLEEVE: u8 = 1 << 2;
    pub const RIGHT_SLEEVE: u8 = 1 << 3;
    pub const LEFT_PANTS: u8 = 1 << 4;
    pub const RIGHT_PANTS: u8 = 1 << 5;
    pub const HAT: u8 = 1 << 6;
    pub const CAPE: u8 = 1;
}

/// What a figure's limbs are posed by (`AvatarRenderState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Anim {
    /// The head's angles from the body, in degrees.
    pub x_rot: f32,
    pub y_rot: f32,
    pub walk_position: f32,
    pub walk_speed: f32,
    pub speed_value: f32,
    pub swim: f32,
    pub age: f32,
    pub fall_flying: bool,
    pub passenger: bool,
    pub crouching: bool,
    pub left_handed: bool,
    pub left_arm: ArmPose,
    pub right_arm: ArmPose,
    /// Ticks an item has been used for, and whether by the off hand.
    pub using: Option<(f32, bool)>,
    pub swing: Option<(Swing, f32)>,
    /// Ticks it has been dying for; 0 if it is not.
    pub dying: f32,
    /// How far gliding has tilted it, 0 to 1.
    pub glide: f32,
}

/// A limb: how far it is from its place (or for some, where it is), and its turn.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Limb {
    pub add: [f32; 3],
    /// Where it is instead, sideways and forwards.
    pub at: Option<(f32, f32)>,
    pub turn: [f32; 3],
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Limbs {
    pub head: Limb,
    pub body: Limb,
    pub right_arm: Limb,
    pub left_arm: Limb,
    pub right_leg: Limb,
    pub left_leg: Limb,
}

fn cos(v: f32) -> f32 {
    mth::cos(f64::from(v))
}

fn sin(v: f32) -> f32 {
    mth::sin(f64::from(v))
}

/// `Mth.rotLerpRad`.
fn lerp_turn(by: f32, from: f32, to: f32) -> f32 {
    let mut d = to - from;
    while d < -PI {
        d += TAU;
    }
    while d >= PI {
        d -= TAU;
    }
    from + by * d
}

impl Limbs {
    /// `HumanoidModel.setupAnim`.
    pub fn new(a: &Anim) -> Limbs {
        let mut l = Limbs::default();
        l.head.turn = [a.x_rot * DEGREE, a.y_rot * DEGREE, 0.0];
        if a.fall_flying {
            l.head.turn[0] = -FRAC_PI_4;
        }
        let (at, speed) = (a.walk_position, a.walk_speed);
        let stride = at * 0.6662;
        l.right_arm.turn[0] = cos(stride + PI) * 2.0 * speed * 0.5 / a.speed_value;
        l.left_arm.turn[0] = cos(stride) * 2.0 * speed * 0.5 / a.speed_value;
        l.right_leg.turn = [cos(stride) * 1.4 * speed / a.speed_value, 0.005, 0.005];
        l.left_leg.turn = [
            cos(stride + PI) * 1.4 * speed / a.speed_value,
            -0.005,
            -0.005,
        ];
        if a.passenger {
            l.right_arm.turn[0] += -0.628_318_55;
            l.left_arm.turn[0] += -0.628_318_55;
            l.right_leg.turn = [-1.413_716_7, 0.314_159_27, 0.078_539_82];
            l.left_leg.turn = [-1.413_716_7, -0.314_159_27, -0.078_539_82];
        }
        // The arm in use is posed first; else the off hand's, unless it needs both.
        let arm_of = |off_hand: bool| off_hand != a.left_handed;
        let left_first = match a.using {
            Some((_, off_hand)) => arm_of(off_hand),
            None => {
                let off = arm_of(true);
                let pose = if off { a.left_arm } else { a.right_arm };
                if pose.two_handed() { !off } else { off }
            }
        };
        if left_first {
            l.pose_arm(a, true);
            if !a.left_arm.affects_other() {
                l.pose_arm(a, false);
            }
        } else {
            l.pose_arm(a, false);
            if !a.right_arm.affects_other() {
                l.pose_arm(a, true);
            }
        }
        l.strike(a);
        if a.crouching {
            l.body.turn[0] = 0.5;
            l.right_arm.turn[0] += 0.4;
            l.left_arm.turn[0] += 0.4;
            l.right_leg.add[2] += 4.0;
            l.left_leg.add[2] += 4.0;
            l.head.add[1] += 4.2;
            l.body.add[1] += 3.2;
            l.left_arm.add[1] += 3.2;
            l.right_arm.add[1] += 3.2;
        }
        // They are never quite still.
        for (arm, pose, way) in [
            (&mut l.right_arm, a.right_arm, 1.0),
            (&mut l.left_arm, a.left_arm, -1.0),
        ] {
            if pose != ArmPose::Spyglass {
                arm.turn[2] += way * (cos(a.age * 0.09) * 0.05 + 0.05);
                arm.turn[0] += way * (sin(a.age * 0.067) * 0.05);
            }
        }
        if a.swim > 0.0 {
            l.swim(a);
        }
        l
    }

    /// One arm by what it holds.
    fn pose_arm(&mut self, a: &Anim, left: bool) {
        let head = self.head.turn;
        let (pose, way) = if left {
            (a.left_arm, -1.0)
        } else {
            (a.right_arm, 1.0)
        };
        let (arm, other) = if left {
            (&mut self.left_arm.turn, &mut self.right_arm.turn)
        } else {
            (&mut self.right_arm.turn, &mut self.left_arm.turn)
        };
        match pose {
            ArmPose::Empty => arm[1] = 0.0,
            ArmPose::Item => {
                arm[0] = arm[0] * 0.5 - 0.314_159_27;
                arm[1] = 0.0;
            }
            ArmPose::Block => {
                arm[0] = arm[0] * 0.5 - 0.942_477_9 + head[0].clamp(-1.396_263_4, 0.436_332_32);
                arm[1] = -30.0 * way * DEGREE + head[1].clamp(-FRAC_PI_6, FRAC_PI_6);
            }
            ArmPose::Bow => {
                arm[1] = -0.1 * way + head[1];
                other[1] = 0.1 * way + head[1] + 0.4 * way;
                arm[0] = -FRAC_PI_2 + head[0];
                other[0] = -FRAC_PI_2 + head[0];
            }
            ArmPose::Trident => {
                arm[0] = arm[0] * 0.5 - PI;
                arm[1] = 0.0;
            }
            ArmPose::CrossbowCharge => {
                let ticks = a.using.map_or(0.0, |u| u.0);
                let by = ticks.clamp(0.0, CROSSBOW_TICKS) / CROSSBOW_TICKS;
                arm[1] = -0.8 * way;
                arm[0] = -0.970_796_35;
                other[1] = mth::lerp(by, 0.4, 0.85) * way;
                other[0] = mth::lerp(by, arm[0], -FRAC_PI_2);
            }
            ArmPose::CrossbowHold => {
                arm[1] = -0.3 * way + head[1];
                other[1] = 0.6 * way + head[1];
                arm[0] = -FRAC_PI_2 + head[0] + 0.1;
                other[0] = -1.5 + head[0];
            }
            ArmPose::Spyglass => {
                let lower = if a.crouching { 0.261_799_4 } else { 0.0 };
                arm[0] = (head[0] - 1.919_862_2 - lower).clamp(-2.4, 3.3);
                arm[1] = head[1] - 0.261_799_4 * way;
            }
            ArmPose::TootHorn => {
                arm[0] = head[0].clamp(-1.2, 1.2) - 1.483_529_8;
                arm[1] = head[1] - FRAC_PI_6 * way;
            }
            ArmPose::Brush => {
                arm[0] = arm[0] * 0.5 - 0.628_318_55;
                arm[1] = 0.0;
            }
            // `SpearAnimations.thirdPersonHandUse`, at rest: the sway of a spear in use
            // is not there.
            ArmPose::Spear => {
                arm[1] = -0.1 * way + head[1];
                arm[0] = -FRAC_PI_2 + head[0] + 0.8;
                if a.fall_flying || a.swim > 0.0 {
                    arm[0] -= 0.959_931_1;
                }
                arm[1] = DEGREE * (57.295_776 * arm[1]).clamp(-60.0, 60.0);
                arm[0] = DEGREE * (57.295_776 * arm[0]).clamp(-120.0, 30.0);
            }
        }
    }

    /// A swing of an arm. (A stab turns the body; the arm's thrust is not there.)
    fn strike(&mut self, a: &Anim) {
        let Some((swing, progress)) = a.swing.filter(|s| s.1 > 0.0) else {
            return;
        };
        let left = swing.off_hand != a.left_handed;
        let mut turn = sin(progress.sqrt() * TAU) * 0.2;
        if left {
            turn = -turn;
        }
        self.body.turn[1] = turn;
        self.right_arm.at = Some((-cos(turn) * 5.0, sin(turn) * 5.0));
        self.left_arm.at = Some((cos(turn) * 5.0, -sin(turn) * 5.0));
        self.right_arm.turn[1] += turn;
        self.left_arm.turn[1] += turn;
        self.left_arm.turn[0] += turn;
        if swing.kind == 1 {
            let left_over = (1.0 - progress) * (1.0 - progress);
            let raise = sin((1.0 - left_over * left_over) * PI);
            let reach = sin(progress * PI) * -(self.head.turn[0] - 0.7) * 0.75;
            let arm = if left {
                &mut self.left_arm.turn
            } else {
                &mut self.right_arm.turn
            };
            arm[0] -= raise * 1.2 + reach;
            arm[1] += turn * 2.0;
            arm[2] += sin(progress * PI) * -0.4;
        }
    }

    fn swim(&mut self, a: &Anim) {
        if !a.fall_flying {
            self.head.turn[0] = lerp_turn(a.swim, self.head.turn[0], -FRAC_PI_4);
        }
        let at = a.walk_position % 26.0;
        // The arm that swings does not swim.
        let swings = a.swing.map(|(s, _)| s.off_hand != a.left_handed);
        let left_by = if swings == Some(true) { 0.0 } else { a.swim };
        let right_by = if swings == Some(false) { 0.0 } else { a.swim };
        if a.using.is_none() {
            let reach = |x: f32| -65.0 * x + x * x;
            let (l, r) = if at < 14.0 {
                let out = 1.870_796_4 * reach(at) / reach(14.0);
                ([0.0, PI, PI + out], [0.0, PI, PI - out])
            } else if at < 22.0 {
                let on = (at - 14.0) / 8.0;
                (
                    [FRAC_PI_2 * on, PI, 5.012_389 - 1.870_796_4 * on],
                    [FRAC_PI_2 * on, PI, 1.270_796_3 + 1.870_796_4 * on],
                )
            } else {
                let on = (at - 22.0) / 4.0;
                let x = FRAC_PI_2 - FRAC_PI_2 * on;
                ([x, PI, PI], [x, PI, PI])
            };
            if at < 26.0 {
                for i in 0..3 {
                    self.left_arm.turn[i] = lerp_turn(left_by, self.left_arm.turn[i], l[i]);
                    self.right_arm.turn[i] = mth::lerp(right_by, self.right_arm.turn[i], r[i]);
                }
            }
        }
        let kick = a.walk_position * 0.333_333_34;
        self.left_leg.turn[0] = mth::lerp(a.swim, self.left_leg.turn[0], 0.3 * cos(kick + PI));
        self.right_leg.turn[0] = mth::lerp(a.swim, self.right_leg.turn[0], 0.3 * cos(kick));
    }

    /// Moves the part of the model called `name`.
    fn place(&self, name: &str, m: &mut Moved) {
        let limb = match name {
            "head" => &self.head,
            "body" => &self.body,
            "right_arm" => &self.right_arm,
            "left_arm" => &self.left_arm,
            "right_leg" => &self.right_leg,
            "left_leg" => &self.left_leg,
            _ => return,
        };
        if let Some((x, z)) = limb.at {
            m.position[0] = x;
            m.position[2] = z;
        }
        for i in 0..3 {
            m.position[i] += limb.add[i];
        }
        m.rotation = limb.turn;
    }
}

/// Whether a part is one of a skin's second layers.
pub(super) fn is_layer(name: &str) -> bool {
    layer_bit(name).is_some()
}

fn layer_bit(name: &str) -> Option<u8> {
    Some(match name {
        "hat" => part::HAT,
        "jacket" => part::JACKET,
        "left_sleeve" => part::LEFT_SLEEVE,
        "right_sleeve" => part::RIGHT_SLEEVE,
        "left_pants" => part::LEFT_PANTS,
        "right_pants" => part::RIGHT_PANTS,
        _ => return None,
    })
}

/// A cape's swing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct CapePose {
    pub flap: f32,
    pub lean: f32,
    pub lean_sideways: f32,
}

/// A figure ready to be drawn.
#[derive(Debug, Clone, Copy)]
pub(super) struct Figure {
    /// From the model's space to blocks from the camera.
    pub transform: Affine,
    pub limbs: Limbs,
    cape: CapePose,
    chest: Chest,
    parts: u8,
    spectator: bool,
}

/// What of a moment figures are posed by.
#[derive(Debug, Clone, Copy)]
pub(super) struct Moment {
    pub partial: f32,
    pub camera: [f64; 3],
}

/// Where an entity is between its last two ticks.
pub(super) fn position(e: &EntityRecord, partial: f32) -> [f64; 3] {
    std::array::from_fn(|i| mth::lerp_f64(f64::from(partial), e.old_position[i], e.position[i]))
}

impl Figure {
    pub fn new(e: &EntityRecord, r: &AvatarRecord, at: Moment) -> Figure {
        let p = at.partial;
        let body_rot = mth::rot_lerp(p, r.body_rot.0, r.body_rot.1);
        let head_rot = mth::rot_lerp(p, r.head_rot.0, r.head_rot.1);
        let body = match r.vehicle_body {
            // It turns with what it rides, as far as its head allows.
            Some((old, now)) => {
                let d = mth::wrap_degrees(head_rot - mth::rot_lerp(p, old, now));
                let d = d.clamp(-85.0, 85.0);
                head_rot - d + if d.abs() > 50.0 { d * 0.2 } else { 0.0 }
            }
            None => body_rot,
        };
        let mut y_rot = mth::wrap_degrees(head_rot - body);
        let mut x_rot = mth::lerp(p, r.old_rot.1, e.x_rot);
        if r.upside_down {
            x_rot = -x_rot;
            y_rot = -y_rot;
        }
        let passenger = e.vehicle.is_some();
        let walks = !passenger && r.alive;
        let (walk_position, walk_speed) = if walks {
            let (old, now) = r.walk_speed;
            (
                r.walk_position - now * (1.0 - p),
                mth::lerp(p, old, now).min(1.0),
            )
        } else {
            (0.0, 0.0)
        };
        let speed_value = if r.fall_flying {
            let m = r.movement.1;
            let v = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]) as f32 / 0.2;
            (v * (v * v)).max(1.0)
        } else {
            1.0
        };
        let swing = r.swing.map(|s| {
            let (old, now) = r.swing_progress;
            let to = if now >= old { now } else { now + 1.0 };
            (s, mth::lerp(p, old, to))
        });
        let age = r.age as f32 + p;
        let anim = Anim {
            x_rot,
            y_rot,
            walk_position,
            walk_speed,
            speed_value,
            swim: mth::lerp(p, r.swim.0, r.swim.1),
            age,
            fall_flying: r.fall_flying,
            passenger,
            crouching: e.pose == pose::CROUCHING,
            left_handed: r.left_handed,
            left_arm: r.left_arm,
            right_arm: r.right_arm,
            using: r.using.map(|(ticks, off)| (ticks as f32 + p, off)),
            swing,
            dying: if r.death_time > 0 {
                r.death_time as f32 + p
            } else {
                0.0
            },
            glide: glide(r, p),
        };
        let here = position(e, p);
        let from: [f32; 3] = std::array::from_fn(|i| (here[i] - at.camera[i]) as f32);
        Figure {
            transform: stand(e, r, &anim, body, from, p),
            limbs: Limbs::new(&anim),
            cape: cape_pose(r, here, body_rot, p),
            chest: r.chest,
            parts: r.parts,
            spectator: r.spectator,
        }
    }

    /// Moves a part of the player's model; whether it is shown.
    pub fn place(&self, name: &str, m: &mut Moved) -> bool {
        self.limbs.place(name, m);
        match name {
            "body" | "right_arm" | "left_arm" | "right_leg" | "left_leg" => !self.spectator,
            _ => layer_bit(name).is_none_or(|bit| self.parts & bit != 0),
        }
    }

    /// Moves a part of what is worn over the player's model: as the part it is worn on.
    pub fn place_worn(&self, name: &str, m: &mut Moved) -> bool {
        self.limbs.place(name, m);
        true
    }

    /// Whether a cape is shown, if there is one.
    pub fn wears_cape(&self) -> bool {
        self.parts & part::CAPE != 0 && !self.spectator && self.chest != Chest::Wings
    }

    /// Where the cape's model is.
    pub fn cape_transform(&self) -> Affine {
        if self.chest == Chest::Armour {
            self.transform.translate([0.0, -0.053125, 0.06875])
        } else {
            self.transform
        }
    }

    /// Moves a part of the cape's model.
    pub fn place_cape(&self, name: &str, m: &mut Moved) -> bool {
        self.limbs.place(name, m);
        if name == "cape" {
            let c = self.cape;
            m.then = Affine::IDENTITY
                .rotate_y(-PI)
                .rotate_x((6.0 + c.lean / 2.0 + c.flap) * DEGREE)
                .rotate_z(c.lean_sideways / 2.0 * DEGREE)
                .rotate_y((180.0 - c.lean_sideways / 2.0) * DEGREE);
        }
        true
    }
}

/// How far gliding has tilted a figure, 0 to 1.
fn glide(r: &AvatarRecord, partial: f32) -> f32 {
    let t = r.fall_fly_ticks as f32 + partial;
    (t * t / 100.0).clamp(0.0, 1.0)
}

/// From a figure's model to blocks from the camera (`setupRotations` and what is around
/// it).
fn stand(
    e: &EntityRecord,
    r: &AvatarRecord,
    a: &Anim,
    body: f32,
    mut from: [f32; 3],
    partial: f32,
) -> Affine {
    let degrees = |d: f32| d * (PI / 180.0);
    if a.crouching {
        from[1] += r.scale * -2.0 / 16.0;
    }
    let sleeping = e.pose == pose::SLEEPING;
    let mut t = Affine::IDENTITY.translate(from);
    if sleeping && let Some(bed) = r.bed {
        let n = bed.normal();
        let by = 1.62 * r.scale - 0.1;
        t = t.translate([-n[0] as f32 * by, 0.0, -n[2] as f32 * by]);
    }
    t = t.scale([r.scale; 3]);
    let mut body = body;
    if r.shaking {
        let shake = (f64::from(a.age.floor()) * 3.25).cos() * std::f64::consts::PI;
        body += (shake * f64::from(0.4f32)) as f32;
    }
    if !sleeping {
        t = t.rotate_y(degrees(180.0 - body));
    }
    if a.dying > 0.0 {
        let fall = (a.dying - 1.0) / 20.0 * 1.6;
        t = t.rotate_z(degrees(fall.max(0.0).sqrt().min(1.0) * 90.0));
    } else if r.spin_attack {
        t = t
            .rotate_x(degrees(-90.0 - a.x_rot))
            .rotate_y(degrees(a.age * -75.0));
    } else if sleeping {
        use ruthenium_sim::direction::Direction;
        let angle = match r.bed {
            Some(Direction::South) => 90.0,
            Some(Direction::North) => 270.0,
            Some(Direction::East) => 180.0,
            Some(_) => 0.0,
            None => body,
        };
        t = t
            .rotate_y(degrees(angle))
            .rotate_z(degrees(90.0))
            .rotate_y(degrees(270.0));
    } else if r.upside_down {
        t = t
            .translate([0.0, (r.size[1] + 0.1) / r.scale, 0.0])
            .rotate_z(PI);
    }
    if r.fall_flying {
        if !r.spin_attack {
            t = t.rotate_x(degrees(a.glide * (-90.0 - a.x_rot)));
        }
        if let Some(turn) = glide_turn(r, a, partial) {
            t = t.rotate_y(turn);
        }
    } else if a.swim > 0.0 {
        let to = if r.in_water { -90.0 - a.x_rot } else { -90.0 };
        t = t.rotate_x(degrees(mth::lerp(a.swim, 0.0, to)));
        if visually_swimming(e, r) {
            t = t.translate([0.0, -1.0, 0.3]);
        }
    }
    t.scale([-1.0, -1.0, 1.0])
        .scale([0.9375; 3])
        .translate([0.0, -1.501, 0.0])
}

fn visually_swimming(e: &EntityRecord, r: &AvatarRecord) -> bool {
    e.pose == pose::SWIMMING || e.pose == pose::FALL_FLYING && !r.fall_flying
}

/// The turn between the way a gliding figure looks and the way it goes.
fn glide_turn(r: &AvatarRecord, a: &Anim, partial: f32) -> Option<f32> {
    let (old, now) = r.movement;
    let m = [0, 2].map(|i| mth::lerp_f64(f64::from(partial), old[i], now[i]));
    // The way it looks, along the ground. A living entity looks where its head does.
    let yaw = mth::rot_lerp(partial, r.head_rot.0, r.head_rot.1);
    let x = -yaw * DEGREE;
    let flat = cos(a.x_rot * DEGREE);
    let look = [f64::from(sin(x) * flat), f64::from(cos(x) * flat)];
    let length = |v: [f64; 2]| v[0] * v[0] + v[1] * v[1];
    if length(m) <= 9.999_999_747_378_752e-6 || length(look) <= 9.999_999_747_378_752e-6 {
        return None;
    }
    let unit = |v: [f64; 2]| v.map(|c| c / length(v).sqrt());
    let (um, ul) = (unit(m), unit(look));
    let dot = um[0] * ul[0] + um[1] * ul[1];
    let side = m[0] * look[1] - m[1] * look[0];
    let sign = if side == 0.0 { 0.0 } else { side.signum() };
    Some((sign * dot.abs().min(1.0).acos()) as f32)
}

/// `AvatarRenderer.extractCapeState`.
fn cape_pose(r: &AvatarRecord, here: [f64; 3], body_rot: f32, partial: f32) -> CapePose {
    let (old, now) = r.cloak;
    let d: [f64; 3] =
        std::array::from_fn(|i| mth::lerp_f64(f64::from(partial), old[i], now[i]) - here[i]);
    let forward = [
        f64::from(sin(body_rot * DEGREE)),
        f64::from(-cos(body_rot * DEGREE)),
    ];
    let lean = (d[0] * forward[0] + d[2] * forward[1]) as f32 * 100.0;
    let lean = lean * (1.0 - glide(r, partial));
    let sideways = (d[0] * forward[1] - d[2] * forward[0]) as f32 * 100.0;
    CapePose {
        flap: (d[1] as f32 * 10.0).clamp(-6.0, 32.0),
        lean: lean.clamp(0.0, 150.0),
        lean_sideways: sideways.clamp(-20.0, 20.0),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Model, ModelSet};
    use super::super::player_golden::{BATCH, CASES, LIMBS, SKINS, STANDS};
    use super::*;
    use ruthenium_prepare::exporters::{ModelFace, ModelPart};
    use ruthenium_sim::avatar::{SkinSource, default_skin};
    use ruthenium_sim::direction::Direction;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    const POSES: [ArmPose; 10] = [
        ArmPose::Empty,
        ArmPose::Item,
        ArmPose::Block,
        ArmPose::Bow,
        ArmPose::Trident,
        ArmPose::CrossbowCharge,
        ArmPose::CrossbowHold,
        ArmPose::Spyglass,
        ArmPose::TootHorn,
        ArmPose::Brush,
    ];

    struct Numbers(u64);

    impl Numbers {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % n
        }
        fn f(&mut self, n: u64) -> f32 {
            self.next(n) as f32
        }
        fn yes(&mut self) -> bool {
            self.next(2) == 1
        }
    }

    fn still() -> Anim {
        Anim {
            x_rot: 0.0,
            y_rot: 0.0,
            walk_position: 0.0,
            walk_speed: 0.0,
            speed_value: 1.0,
            swim: 0.0,
            age: 0.0,
            fall_flying: false,
            passenger: false,
            crouching: false,
            left_handed: false,
            left_arm: ArmPose::Empty,
            right_arm: ArmPose::Empty,
            using: None,
            swing: None,
            dying: 0.0,
            glide: 0.0,
        }
    }

    /// As `PlayerGolden` makes them.
    fn made_up(n: &mut Numbers) -> Anim {
        let mut a = still();
        a.x_rot = (n.f(1801) - 900.0) / 10.0;
        a.y_rot = (n.f(1701) - 850.0) / 10.0;
        a.walk_position = n.f(100_000) / 1000.0;
        a.walk_speed = n.f(1001) / 1000.0;
        a.fall_flying = n.next(4) == 0;
        if a.fall_flying {
            a.speed_value = 1.0 + n.f(300) / 100.0;
        }
        if n.next(3) == 0 {
            a.swim = n.f(1001) / 1000.0;
        }
        a.age = n.f(200_000) / 100.0;
        a.passenger = n.next(5) == 0;
        a.crouching = n.next(3) == 0;
        a.left_handed = n.yes();
        a.left_arm = POSES[n.next(10) as usize];
        a.right_arm = POSES[n.next(10) as usize];
        let uses = n.yes();
        let ticks = n.f(400) / 10.0;
        let off_hand = n.yes();
        a.using = uses.then_some((ticks, off_hand));
        let swings = n.yes();
        let swing = Swing {
            off_hand: n.yes(),
            kind: n.next(2) as i32,
            duration: 6,
        };
        let progress = n.f(1001) / 1000.0;
        a.swing = swings.then_some((swing, progress));
        a
    }

    /// A player's parts, without their boxes.
    const REST: [(&str, [f32; 3]); 6] = [
        ("head", [0.0, 0.0, 0.0]),
        ("body", [0.0, 0.0, 0.0]),
        ("right_arm", [-5.0, 2.0, 0.0]),
        ("left_arm", [5.0, 2.0, 0.0]),
        ("right_leg", [-1.9, 12.0, 0.0]),
        ("left_leg", [1.9, 12.0, 0.0]),
    ];

    #[test]
    fn limbs_are_the_games() {
        let mut n = Numbers(7);
        let mut wrong = Vec::new();
        for (batch, want) in LIMBS.iter().enumerate() {
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for _ in 0..BATCH {
                let a = made_up(&mut n);
                let limbs = Limbs::new(&a);
                for (name, position) in REST {
                    let mut m = Moved {
                        position,
                        rotation: [0.0; 3],
                        then: Affine::IDENTITY,
                    };
                    limbs.place(name, &mut m);
                    for v in m.position.into_iter().chain(m.rotation) {
                        for b in (v + 0.0).to_bits().to_le_bytes() {
                            hash = (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
                        }
                    }
                }
            }
            if hash != *want {
                wrong.push(batch);
            }
        }
        assert_eq!(LIMBS.len() * BATCH, CASES);
        assert!(wrong.is_empty(), "{} batches: {wrong:?}", wrong.len());
    }

    fn record() -> AvatarRecord {
        AvatarRecord {
            skin: Arc::new(SkinSource::Player {
                id: 0,
                textures: None,
            }),
            parts: 0x7F,
            left_handed: false,
            on_fire: false,
            invisible: false,
            fall_flying: false,
            spin_attack: false,
            shaking: false,
            spectator: false,
            in_water: false,
            bed: None,
            upside_down: false,
            hidden: false,
            alive: true,
            old_rot: (0.0, 0.0),
            body_rot: (0.0, 0.0),
            head_rot: (0.0, 0.0),
            walk_speed: (0.0, 0.0),
            walk_position: 0.0,
            swing: None,
            swing_progress: (0.0, 0.0),
            swim: (0.0, 0.0),
            hurt: false,
            death_time: 0,
            fall_fly_ticks: 0,
            movement: ([0.0; 3], [0.0; 3]),
            cloak: ([0.0; 3], [0.0; 3]),
            chest: Chest::Bare,
            left_arm: ArmPose::Empty,
            right_arm: ArmPose::Empty,
            using: None,
            age: 0,
            scale: 1.0,
            size: [0.6, 1.8],
            eye_height: 1.62,
            vehicle_body: None,
            use_remaining: 0,
            held: Default::default(),
            worn: Default::default(),
        }
    }

    fn entity() -> EntityRecord {
        EntityRecord {
            id: 1,
            uuid: 0,
            entity_type: 0,
            position: [0.0; 3],
            old_position: [0.0; 3],
            y_rot: 0.0,
            x_rot: 0.0,
            on_ground: true,
            pose: pose::STANDING,
            baby: false,
            vehicle: None,
            avatar: None,
            name: None,
        }
    }

    fn close(got: Affine, want: [f32; 12], within: f32, what: &str) {
        for (g, w) in got.0.as_flattened().iter().zip(want) {
            assert!((g - w).abs() < within, "{what}: {got:?}, not {want:?}");
        }
    }

    #[test]
    fn they_stand_as_the_games_do() {
        for (i, (n, stands, cape)) in STANDS.iter().copied().enumerate() {
            let mut e = entity();
            let mut r = record();
            let mut a = still();
            e.pose = n[0] as i32;
            let beds = [
                None,
                Some(Direction::North),
                Some(Direction::South),
                Some(Direction::West),
                Some(Direction::East),
            ];
            r.bed = beds[n[1] as usize];
            let kind = n[2] as i32;
            r.spin_attack = kind == 1;
            r.fall_flying = kind == 2;
            r.upside_down = kind == 3;
            r.shaking = kind == 4;
            (r.scale, a.x_rot, a.age, a.dying) = (n[4], n[5], n[6], n[7]);
            (a.swim, r.in_water) = (n[8], n[9] == 1.0);
            a.glide = (n[10] * n[10] / 100.0).clamp(0.0, 1.0);
            r.size[1] = 1.8 * r.scale;
            a.crouching = e.pose == pose::CROUCHING;
            a.fall_flying = r.fall_flying;
            let got = stand(&e, &r, &a, n[3], [0.0; 3], 0.0);
            close(got, stands, 2e-5, &format!("stand {i}"));
            // The cape hangs from a body that may stoop.
            let f = Figure {
                transform: Affine::IDENTITY,
                limbs: Limbs::new(&a),
                cape: CapePose {
                    flap: n[11],
                    lean: n[12],
                    lean_sideways: n[13],
                },
                chest: Chest::Bare,
                parts: 0x7F,
                spectator: false,
            };
            let mut parts = Vec::new();
            cape_model().draw_by(Model::Cape, &mut parts, |name, m| f.place_cape(name, m));
            // (The game goes by way of angles, which costs it some digits.)
            close(parts[0].transform, cape, 6e-4, &format!("cape {i}"));
        }
    }

    /// Every model as one part without boxes, but the cape's on its body.
    fn cape_model() -> ModelSet {
        let part = |position: [f32; 3], rotation: [f32; 3], faces, children| ModelPart {
            position,
            rotation,
            scale: [1.0; 3],
            faces,
            children,
        };
        let face = ModelFace {
            corners: [[0.0; 5]; 4],
            normal: [0.0, 1.0, 0.0],
        };
        let cape = part([0.0, 0.0, 2.0], [0.0, PI, 0.0], vec![face], Vec::new());
        let body = part([0.0; 3], [0.0; 3], Vec::new(), vec![("cape".into(), cape)]);
        let root = part([0.0; 3], [0.0; 3], Vec::new(), vec![("body".into(), body)]);
        let mut layers = BTreeMap::new();
        for m in Model::ALL {
            layers.insert(m.layer().to_owned(), root.clone());
        }
        ModelSet::new(&layers.into_iter().collect()).unwrap()
    }

    #[test]
    fn default_skins_are_the_games() {
        for (id, path, slim) in SKINS.iter().copied() {
            let got = super::super::skins::default_path(default_skin(id));
            assert_eq!(format!("entity/{got}"), path, "{id:x}");
            assert_eq!(default_skin(id) < 9, slim, "{id:x}");
        }
    }

    #[test]
    fn second_layers_go_by_their_bits() {
        let mut r = record();
        let shown = |r: &AvatarRecord, name: &str| {
            let f = Figure::new(
                &entity(),
                r,
                Moment {
                    partial: 1.0,
                    camera: [0.0; 3],
                },
            );
            let mut m = Moved {
                position: [0.0; 3],
                rotation: [0.0; 3],
                then: Affine::IDENTITY,
            };
            f.place(name, &mut m)
        };
        for (i, name) in [
            "jacket",
            "left_sleeve",
            "right_sleeve",
            "left_pants",
            "right_pants",
            "hat",
        ]
        .into_iter()
        .enumerate()
        {
            r.parts = 1 << (i + 1);
            assert!(shown(&r, name), "{name}");
            r.parts = !r.parts;
            assert!(!shown(&r, name), "{name}");
            assert!(is_layer(name));
        }
        r.parts = 0;
        assert!(shown(&r, "head") && shown(&r, "left_leg") && !is_layer("head"));
        // Of a spectator only the head is seen.
        r.spectator = true;
        r.parts = 0x7F;
        assert!(shown(&r, "head") && shown(&r, "hat") && !shown(&r, "body"));
    }

    #[test]
    fn figures_are_between_their_ticks() {
        let mut e = entity();
        e.old_position = [10.0, 60.0, 5.0];
        e.position = [11.0, 60.0, 5.0];
        let mut r = record();
        r.body_rot = (170.0, 190.0);
        r.head_rot = (170.0, 200.0);
        let at = |r: &AvatarRecord, e: &EntityRecord| {
            Figure::new(
                e,
                r,
                Moment {
                    partial: 0.5,
                    camera: [10.0, 60.0, 5.0],
                },
            )
        };
        let f = at(&r, &e);
        // Half way: at 10.5, its body turned by 180, its head by 5 more.
        assert!((f.transform.0[0][3] - 0.5).abs() < 1e-6);
        assert!((f.limbs.head.turn[1] - 5.0 * DEGREE).abs() < 1e-6);
        // A swing that began again runs on to its end.
        r.swing = Some(Swing {
            off_hand: false,
            kind: 1,
            duration: 6,
        });
        r.swing_progress = (0.8, 0.2);
        let swung = at(&r, &e).limbs.body.turn[1];
        assert!(
            (swung - sin(1.0f32.sqrt() * TAU) * 0.2).abs() < 1e-4,
            "{swung}"
        );
        // Upside down, its head's angles are the other way round.
        let mut down = record();
        down.upside_down = true;
        down.old_rot = (0.0, 30.0);
        down.head_rot = (20.0, 20.0);
        e.x_rot = 30.0;
        let head = at(&down, &e).limbs.head.turn;
        assert!((head[0] + 30.0 * DEGREE).abs() < 1e-6 && (head[1] + 20.0 * DEGREE).abs() < 1e-6);
        e.x_rot = 0.0;
        // Riding, it does not walk and it turns with what it rides.
        let mut r = record();
        r.walk_speed = (1.0, 1.0);
        r.walk_position = 3.0;
        let walks = at(&r, &e).limbs.right_leg.turn[0];
        assert!((walks - cos(2.5 * 0.6662) * 1.4).abs() < 1e-6);
        r.alive = false;
        assert!((at(&r, &e).limbs.right_leg.turn[0]).abs() < 1e-6);
        r.alive = true;
        e.vehicle = Some(4);
        assert_eq!(at(&r, &e).limbs.right_leg.turn[0], -1.413_716_7);
        r.vehicle_body = Some((0.0, 0.0));
        r.head_rot = (60.0, 60.0);
        // 60 from the vehicle: the body comes 12 along.
        let head = at(&r, &e).limbs.head.turn[1];
        assert!((head - 48.0 * DEGREE).abs() < 1e-5, "{head}");
    }

    #[test]
    fn gliding_turns_towards_the_way_it_goes() {
        let mut r = record();
        let still = still();
        // Looking south, going east: a quarter turn to its left.
        r.movement = ([1.0, -0.5, 0.0], [1.0, -0.5, 0.0]);
        let turn = glide_turn(&r, &still, 1.0).unwrap();
        assert!((turn - FRAC_PI_2).abs() < 1e-3, "{turn}");
        r.movement = ([-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]);
        let turn = glide_turn(&r, &still, 1.0).unwrap();
        assert!((turn + FRAC_PI_2).abs() < 1e-3, "{turn}");
        // Half way between two movements.
        r.movement = ([2.0, 0.0, 0.0], [0.0, 0.0, 2.0]);
        let turn = glide_turn(&r, &still, 0.5).unwrap();
        assert!((turn - FRAC_PI_4).abs() < 1e-3, "{turn}");
        // Straight down, or looking straight down, there is no way to go.
        r.movement = ([0.0, -1.0, 0.0], [0.0, -1.0, 0.0]);
        assert!(glide_turn(&r, &still, 1.0).is_none());
        r.movement = ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        let down = Anim {
            x_rot: 90.0,
            ..still
        };
        assert!(glide_turn(&r, &down, 1.0).is_none());
    }

    #[test]
    fn what_is_hidden_hides_what_it_carries() {
        let set = cape_model();
        let mut parts = Vec::new();
        set.draw_by(Model::Cape, &mut parts, |name, _| name != "body");
        assert!(parts.is_empty());
        set.draw_by(Model::Cape, &mut parts, |_, _| true);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn capes_trail_behind() {
        let mut r = record();
        // Facing south, its cloak half a block north of it and a little above.
        r.cloak = ([0.0, 0.2, -0.5], [0.0, 0.2, -0.5]);
        let c = cape_pose(&r, [0.0; 3], 0.0, 1.0);
        assert!((c.lean - 49.5).abs() < 1e-3 && (c.flap - 2.0).abs() < 1e-5);
        assert!(c.lean_sideways.abs() < 1e-3);
        // In front of it there is nothing to lean on.
        r.cloak = ([0.0, 9.0, 0.5], [0.0, 9.0, 0.5]);
        let c = cape_pose(&r, [0.0; 3], 0.0, 1.0);
        assert_eq!((c.lean, c.flap), (0.0, 32.0));
        // To its left.
        r.cloak = ([0.1, 0.0, 0.0], [0.1, 0.0, 0.0]);
        let c = cape_pose(&r, [0.0; 3], 0.0, 1.0);
        assert!((c.lean_sideways + 10.0).abs() < 1e-3, "{c:?}");
        // Gliding lifts it.
        r.cloak = ([0.0, 0.0, -0.5], [0.0, 0.0, -0.5]);
        r.fall_fly_ticks = 9;
        assert!(cape_pose(&r, [0.0; 3], 0.0, 1.0).lean.abs() < 1e-3);
        // Wings take its place, and chest armour moves it.
        let mut f = Figure::new(
            &entity(),
            &r,
            Moment {
                partial: 1.0,
                camera: [0.0; 3],
            },
        );
        assert!(f.wears_cape() && f.cape_transform() == f.transform);
        f.chest = Chest::Armour;
        assert!(f.wears_cape() && f.cape_transform() != f.transform);
        f.chest = Chest::Wings;
        assert!(!f.wears_cape());
        f.chest = Chest::Bare;
        f.parts = 0x7E;
        assert!(!f.wears_cape());
    }
}
