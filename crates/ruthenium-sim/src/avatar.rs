//! How players and mannequins hold themselves: what of an avatar moves between the ticks
//! only to be drawn. The local player is not one of these (nothing draws it yet).
//!
//! Another player's tick is `RemotePlayer.tick`, a mannequin's `LivingEntity.tick`.

use std::sync::Arc;

use ruthenium_javamath::mth;
use ruthenium_protocol::packets::Property;
use ruthenium_protocol::profile::ResolvableProfile;

use crate::direction::Direction;
use crate::entity::pose;
use crate::item::{Class, Stack, animation};

/// Shared flags (`Entity.DATA_SHARED_FLAGS_ID`).
const FLAG_ON_FIRE: u8 = 1;
const FLAG_SHIFT_KEY_DOWN: u8 = 1 << 1;
const FLAG_INVISIBLE: u8 = 1 << 5;
const FLAG_FALL_FLYING: u8 = 1 << 7;
/// Living flags: an item is used, by the off hand, a riptide spin.
const LIVING_USING: u8 = 1;
const LIVING_OFF_HAND: u8 = 2;
const LIVING_SPIN: u8 = 4;
/// Ticks in powder snow after which one shakes.
const FULLY_FROZEN: i32 = 140;

/// Whose skin an avatar wears.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkinSource {
    /// A player of the list: its id and its profile's textures, which count only if
    /// they are signed.
    Player {
        id: u128,
        textures: Option<Property>,
    },
    /// What a profile says, signed or not: a head's owner, a mannequin's.
    Profile(ResolvableProfile),
}

/// How an arm holds what is in its hand (`HumanoidModel.ArmPose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArmPose {
    #[default]
    Empty,
    Item,
    Block,
    Bow,
    Trident,
    CrossbowCharge,
    CrossbowHold,
    Spyglass,
    TootHorn,
    Brush,
    /// A spear held ready.
    Spear,
}

impl SkinSource {
    /// The id its default skin goes by, before anything is looked up.
    pub fn id(&self) -> u128 {
        match self {
            SkinSource::Player { id, .. } => *id,
            SkinSource::Profile(p) => p.id.unwrap_or_else(|| {
                p.name
                    .as_deref()
                    .map_or(0, ruthenium_net::crypt::offline_uuid)
            }),
        }
    }
}

/// How many default skins there are ([`default_skin`]).
pub const DEFAULT_SKINS: usize = 18;

/// Which default skin an id has: nine of them with thin arms, then the same with wide
/// ones.
pub fn default_skin(id: u128) -> usize {
    // `UUID.hashCode`.
    let folded = (id >> 64) as u64 ^ id as u64;
    let hash = (folded >> 32) as u32 ^ folded as u32;
    (hash as i32).rem_euclid(DEFAULT_SKINS as i32) as usize
}

impl ArmPose {
    pub fn two_handed(self) -> bool {
        matches!(
            self,
            ArmPose::Bow | ArmPose::CrossbowCharge | ArmPose::CrossbowHold
        )
    }

    /// Whether the other arm is posed by this one.
    pub fn affects_other(self) -> bool {
        self.two_handed() || self == ArmPose::Trident
    }
}

/// What is worn over the chest, as far as a cape goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Chest {
    #[default]
    Bare,
    Armour,
    Wings,
}

/// A swing of a hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swing {
    pub off_hand: bool,
    /// None (0), a whack (1) or a stab (2).
    pub kind: i32,
    pub duration: i32,
}

/// `LivingEntity.SwingState`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct SwingState {
    current: Option<Swing>,
    ticks: i32,
    old: f32,
    now: f32,
}

impl SwingState {
    /// Starts a swing unless one is in its first half.
    fn start(&mut self, swing: Swing) {
        if let Some(c) = self.current
            && self.ticks <= c.duration / 2
            && self.ticks > 0
        {
            return;
        }
        if self.current != Some(swing) {
            self.old = 0.0;
            self.now = 0.0;
        }
        self.current = Some(swing);
        self.ticks = 0;
    }

    fn tick(&mut self) {
        self.old = self.now;
        let Some(c) = self.current else {
            self.now = 0.0;
            return;
        };
        if c.duration > 0 {
            self.now = (self.ticks as f32 / c.duration as f32).min(1.0);
        }
        let done = self.ticks > c.duration;
        self.ticks = self.ticks.wrapping_add(1);
        if done {
            self.current = None;
            self.now = 0.0;
        }
    }
}

/// `WalkAnimationState`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Walk {
    speed_old: f32,
    speed: f32,
    position: f32,
}

/// An avatar's state between ticks.
#[derive(Debug, Clone)]
pub struct Avatar {
    /// A player; else a mannequin.
    player: bool,
    pub(crate) skin: Arc<SkinSource>,
    /// The bits of the skin's second layers that are shown.
    parts: u8,
    left_handed: bool,
    flags: u8,
    living_flags: u8,
    frozen_ticks: i32,
    pub(crate) sleeping_pos: Option<[i32; 3]>,
    old_rot: (f32, f32),
    body_rot: f32,
    body_rot_old: f32,
    head_rot: f32,
    head_rot_old: f32,
    /// Where the head turns to, and the steps left.
    head_target: f32,
    head_steps: i32,
    walk: Walk,
    swing: SwingState,
    swim: f32,
    swim_old: f32,
    hurt_time: i32,
    death_time: i32,
    fall_fly_ticks: i32,
    /// The movement it eases into, and the steps left.
    movement: [f64; 3],
    movement_old: [f64; 3],
    movement_target: [f64; 3],
    movement_steps: i32,
    cloak: [f64; 3],
    cloak_old: [f64; 3],
    main_hand: Stack,
    off_hand: Stack,
    /// What it wears on its head, chest, legs and feet.
    worn: [Stack; 4],
    chest: Chest,
    use_item: Stack,
    use_remaining: i32,
    in_water: bool,
    bed: Option<Direction>,
    spectator: bool,
}

/// What an avatar's tick takes of the entity it is of.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TickInput {
    pub position: [f64; 3],
    pub old_position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub pose: i32,
    pub health: f32,
    pub passenger: bool,
    pub interpolating: bool,
    /// Near enough to the player for its death to go on.
    pub simulated: bool,
    pub in_water: bool,
    pub bed: Option<Direction>,
    pub spectator: bool,
}

/// Synched fields of avatars, by the ids the export gives them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AvatarFields {
    pub shared_flags: Option<u8>,
    pub ticks_frozen: Option<u8>,
    pub living_flags: Option<u8>,
    pub sleeping_pos: Option<u8>,
    pub main_hand: Option<u8>,
    pub customisation: Option<u8>,
    pub profile: Option<u8>,
}

/// An avatar as drawing needs it: what its last two ticks left.
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarRecord {
    pub skin: Arc<SkinSource>,
    /// `PlayerModelPart` bits: cape, jacket, left and right sleeve, left and right
    /// trouser leg, hat.
    pub parts: u8,
    pub left_handed: bool,
    pub on_fire: bool,
    pub invisible: bool,
    pub fall_flying: bool,
    pub spin_attack: bool,
    pub shaking: bool,
    pub spectator: bool,
    pub in_water: bool,
    pub bed: Option<Direction>,
    /// Turned upside down by its name.
    pub upside_down: bool,
    /// Not seen even by those who see the invisible of their own team.
    pub hidden: bool,
    pub alive: bool,
    /// Pitch and yaw's old values.
    pub old_rot: (f32, f32),
    pub body_rot: (f32, f32),
    pub head_rot: (f32, f32),
    /// The old and new speed of walking, and how far it has gone.
    pub walk_speed: (f32, f32),
    pub walk_position: f32,
    pub swing: Option<Swing>,
    pub swing_progress: (f32, f32),
    pub swim: (f32, f32),
    pub hurt: bool,
    pub death_time: i32,
    pub fall_fly_ticks: i32,
    pub movement: ([f64; 3], [f64; 3]),
    pub cloak: ([f64; 3], [f64; 3]),
    pub chest: Chest,
    pub left_arm: ArmPose,
    pub right_arm: ArmPose,
    /// The ticks an item has been used for and whether the off hand does.
    pub using: Option<(i32, bool)>,
    /// The ticks its use has left.
    pub use_remaining: i32,
    pub age: i32,
    pub scale: f32,
    pub size: [f32; 2],
    pub eye_height: f32,
    /// The old and new body angle of the living vehicle it rides.
    pub vehicle_body: Option<(f32, f32)>,
    /// What its main hand and its other hand hold.
    pub held: [Stack; 2],
    /// What it wears on its head, chest, legs and feet ([`WORN_HEAD`] to [`WORN_FEET`]).
    pub worn: [Stack; 4],
}

/// The places of [`AvatarRecord::worn`].
pub const WORN_HEAD: usize = 0;
pub const WORN_CHEST: usize = 1;
pub const WORN_LEGS: usize = 2;
pub const WORN_FEET: usize = 3;

/// Turns `old` by whole turns until `now` is within half of one of it.
fn unwrap(now: f32, old: &mut f32) {
    while now - *old < -180.0 {
        *old -= 360.0;
    }
    while now - *old >= 180.0 {
        *old += 360.0;
    }
}

impl Avatar {
    /// A new one, facing as its packet says.
    pub(crate) fn new(
        player: bool,
        skin: SkinSource,
        (y_rot, x_rot): (f32, f32),
        head_rot: f32,
        movement: [f64; 3],
    ) -> Avatar {
        Avatar {
            player,
            skin: Arc::new(skin),
            // Mannequins show all of it unless told otherwise.
            parts: if player { 0 } else { 0x7F },
            left_handed: false,
            flags: 0,
            living_flags: 0,
            frozen_ticks: 0,
            sleeping_pos: None,
            old_rot: (y_rot, x_rot),
            body_rot: head_rot,
            body_rot_old: head_rot,
            head_rot,
            head_rot_old: head_rot,
            head_target: 0.0,
            head_steps: 0,
            walk: Walk::default(),
            swing: SwingState::default(),
            swim: 0.0,
            swim_old: 0.0,
            hurt_time: 0,
            death_time: 0,
            fall_fly_ticks: 0,
            movement,
            movement_old: [0.0; 3],
            movement_target: [0.0; 3],
            movement_steps: 0,
            // It catches up in its first tick, or from the world's origin near it.
            cloak: [0.0; 3],
            cloak_old: [0.0; 3],
            main_hand: Stack::empty(),
            off_hand: Stack::empty(),
            worn: std::array::from_fn(|_| Stack::empty()),
            chest: Chest::Bare,
            use_item: Stack::empty(),
            use_remaining: 0,
            in_water: false,
            bed: None,
            spectator: false,
        }
    }

    /// Its old angles, as a tick begins or it is put somewhere.
    pub(crate) fn set_old_rot(&mut self, y_rot: f32, x_rot: f32) {
        self.old_rot = (y_rot, x_rot);
    }

    pub(crate) fn turn_head(&mut self, y_head_rot: f32) {
        self.head_target = y_head_rot;
        self.head_steps = 3;
    }

    pub(crate) fn damaged(&mut self) {
        self.walk.speed = 1.5;
        self.hurt_time = 10;
    }

    pub(crate) fn hurt(&mut self) {
        self.hurt_time = 10;
    }

    pub(crate) fn swing(&mut self, swing: Swing) {
        self.swing.start(swing);
    }

    /// `lerpMotion`: a player eases into its movement over `interval` ticks and one.
    pub(crate) fn lerp_motion(&mut self, movement: [f64; 3], interval: i32) {
        if self.player {
            self.movement_target = movement;
            self.movement_steps = interval + 1;
        } else {
            self.movement = movement;
        }
    }

    pub(crate) fn set_movement(&mut self, movement: [f64; 3]) {
        self.movement = movement;
    }

    /// A boat turned its body along (`clampRotation`).
    pub(crate) fn turned_by_boat(&mut self, body: f32) {
        self.body_rot = body;
    }

    pub(crate) fn set_held(&mut self, off_hand: bool, stack: Stack) {
        if off_hand {
            self.off_hand = stack;
        } else {
            self.main_hand = stack;
        }
    }

    pub(crate) fn set_chest(&mut self, chest: Chest) {
        self.chest = chest;
    }

    /// What it wears in one of [`WORN_HEAD`] to [`WORN_FEET`].
    pub(crate) fn set_worn(&mut self, place: usize, stack: Stack) {
        self.worn[place] = stack;
    }

    fn using(&self) -> bool {
        self.living_flags & LIVING_USING != 0
    }

    fn held(&self, off_hand: bool) -> &Stack {
        if off_hand {
            &self.off_hand
        } else {
            &self.main_hand
        }
    }

    fn off_hand_used(&self) -> bool {
        self.living_flags & LIVING_OFF_HAND != 0
    }

    /// One field of its data; whether it was one of its.
    pub(crate) fn set_data(
        &mut self,
        f: &AvatarFields,
        field: u8,
        value: &ruthenium_protocol::packets::play::DataValue,
    ) -> bool {
        use ruthenium_protocol::packets::play::DataValue as V;
        let is = |id: Option<u8>| id == Some(field);
        match value {
            V::Byte(b) if is(f.shared_flags) => self.flags = *b as u8,
            V::Byte(b) if is(f.living_flags) => {
                self.living_flags = *b as u8;
                // `onSyncedDataUpdated`.
                if self.using() && self.use_item.is_empty() {
                    self.use_item = self.held(self.off_hand_used()).clone();
                    self.use_remaining = self.use_item.c.use_duration;
                } else if !self.using() {
                    self.use_item = Stack::empty();
                    self.use_remaining = 0;
                }
            }
            V::Byte(b) if is(f.customisation) => self.parts = *b as u8,
            V::Int(n) if is(f.ticks_frozen) => self.frozen_ticks = *n,
            V::Id(n) if is(f.main_hand) => self.left_handed = *n == 0,
            V::OptionalBlockPos(p) if is(f.sleeping_pos) => self.sleeping_pos = *p,
            V::Profile(p) if is(f.profile) => {
                self.skin = Arc::new(SkinSource::Profile((**p).clone()));
            }
            _ => return false,
        }
        true
    }

    /// Its tick, after the entity's interpolation.
    pub(crate) fn tick(&mut self, e: &TickInput) {
        // `ClientAvatarState.tick`.
        self.movement_old = self.movement;
        self.cloak_old = self.cloak;
        for i in 0..3 {
            let d = e.position[i] - self.cloak[i];
            if d > 10.0 || d < -10.0 {
                self.cloak[i] = e.position[i];
                self.cloak_old[i] = e.position[i];
            } else {
                self.cloak[i] += d * 0.25;
            }
        }
        // `baseTick`.
        self.swing.tick();
        self.in_water = e.in_water;
        self.bed = e.bed;
        self.spectator = e.spectator;
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if e.health <= 0.0 && e.simulated {
            self.death_time += 1;
        }
        self.head_rot_old = self.head_rot;
        self.body_rot_old = self.body_rot;
        // `updatingUsingItem`, `updateSwimAmount`.
        if self.using() {
            let held = self.held(self.off_hand_used()).clone();
            if held.same_item(&self.use_item) {
                self.use_item = held;
                self.use_remaining -= 1;
            } else {
                self.use_item = Stack::empty();
                self.use_remaining = 0;
            }
        }
        self.swim_old = self.swim;
        self.swim = if self.visually_swimming(e.pose) {
            (self.swim + 0.09).min(1.0)
        } else {
            (self.swim - 0.09).max(0.0)
        };
        // `aiStep`.
        if !self.player && !e.interpolating {
            self.movement = self.movement.map(|c| c * 0.98);
        }
        if self.head_steps > 0 {
            self.head_rot = mth::rot_lerp_f64(
                1.0 / f64::from(self.head_steps),
                f64::from(self.head_rot),
                f64::from(self.head_target),
            ) as f32;
            self.head_steps -= 1;
        }
        if self.movement_steps > 0 {
            for (c, t) in self.movement.iter_mut().zip(self.movement_target) {
                *c += (t - *c) / f64::from(self.movement_steps);
            }
            self.movement_steps -= 1;
        }
        if !self.player {
            self.walk(e);
        }
        // The body turns the way it goes, or the way it strikes.
        let (dx, dz) = (
            e.position[0] - e.old_position[0],
            e.position[2] - e.old_position[2],
        );
        let mut target = self.body_rot;
        if (dx * dx + dz * dz) as f32 > 0.002_500_000_2 {
            let way = mth::atan2(dz, dx) as f32 * 57.295776 - 90.0;
            let away = (mth::wrap_degrees(e.y_rot) - way).abs();
            target = if 95.0 < away && away < 265.0 {
                way - 180.0
            } else {
                way
            };
        }
        if self.swing.now > 0.0 {
            target = e.y_rot;
        }
        self.body_rot += mth::wrap_degrees(target - self.body_rot) * 0.3;
        let head = mth::wrap_degrees(e.y_rot - self.body_rot);
        if head.abs() > 50.0 {
            self.body_rot += head - head.signum() * 50.0;
        }
        unwrap(e.y_rot, &mut self.old_rot.0);
        unwrap(self.body_rot, &mut self.body_rot_old);
        unwrap(e.x_rot, &mut self.old_rot.1);
        unwrap(self.head_rot, &mut self.head_rot_old);
        self.fall_fly_ticks = if self.flags & FLAG_FALL_FLYING != 0 {
            self.fall_fly_ticks + 1
        } else {
            0
        };
        if self.player {
            self.walk(e);
        }
    }

    /// `calculateEntityAnimation`.
    fn walk(&mut self, e: &TickInput) {
        if e.passenger || e.health <= 0.0 {
            self.walk = Walk::default();
            return;
        }
        let d = |i: usize| e.position[i] - e.old_position[i];
        let distance = (d(0) * d(0) + d(2) * d(2)).sqrt() as f32;
        let w = &mut self.walk;
        w.speed_old = w.speed;
        w.speed += ((distance * 4.0).min(1.0) - w.speed) * 0.4;
        w.position += w.speed;
    }

    fn visually_swimming(&self, pose: i32) -> bool {
        pose == pose::SWIMMING || pose == pose::FALL_FLYING && self.flags & FLAG_FALL_FLYING == 0
    }

    /// `AvatarRenderer.getArmPose` of one hand.
    fn hand_pose(&self, off_hand: bool) -> ArmPose {
        let stack = self.held(off_hand);
        if stack.is_empty() {
            return ArmPose::Empty;
        }
        if self.swing.current.is_none() && stack.c.class == Class::Crossbow && stack.c.charged {
            return ArmPose::CrossbowHold;
        }
        if self.off_hand_used() == off_hand && self.use_remaining > 0 {
            match stack.c.use_animation() {
                animation::BLOCK => return ArmPose::Block,
                animation::BOW => return ArmPose::Bow,
                animation::TRIDENT => return ArmPose::Trident,
                animation::CROSSBOW => return ArmPose::CrossbowCharge,
                animation::SPYGLASS => return ArmPose::Spyglass,
                animation::TOOT_HORN => return ArmPose::TootHorn,
                animation::BRUSH => return ArmPose::Brush,
                animation::SPEAR => return ArmPose::Spear,
                _ => {}
            }
        }
        // `usesSpearPose`: a stab of this hand, or a spear in it. (The game goes by the
        // `spears` tag, whose items are those named so.)
        let stabs = self
            .swing
            .current
            .is_some_and(|s| s.off_hand == off_hand && s.kind == 2);
        if stabs || stack.c.name.ends_with("_spear") {
            ArmPose::Spear
        } else {
            ArmPose::Item
        }
    }

    /// The left and the right arm's poses.
    fn arm_poses(&self) -> (ArmPose, ArmPose) {
        let main = self.hand_pose(false);
        let mut off = self.hand_pose(true);
        if main.two_handed() {
            off = if self.off_hand.is_empty() {
                ArmPose::Empty
            } else {
                ArmPose::Item
            };
        }
        if self.left_handed {
            (main, off)
        } else {
            (off, main)
        }
    }

    /// What drawing needs of it; what is the entity's own is filled in by the caller.
    pub(crate) fn record(&self) -> AvatarRecord {
        let (left_arm, right_arm) = self.arm_poses();
        AvatarRecord {
            skin: self.skin.clone(),
            parts: self.parts,
            left_handed: self.left_handed,
            on_fire: self.flags & FLAG_ON_FIRE != 0,
            invisible: self.flags & FLAG_INVISIBLE != 0,
            fall_flying: self.flags & FLAG_FALL_FLYING != 0,
            spin_attack: self.living_flags & LIVING_SPIN != 0,
            shaking: self.frozen_ticks >= FULLY_FROZEN,
            spectator: self.spectator,
            in_water: self.in_water,
            bed: self.bed,
            upside_down: false,
            hidden: false,
            alive: true,
            old_rot: self.old_rot,
            body_rot: (self.body_rot_old, self.body_rot),
            head_rot: (self.head_rot_old, self.head_rot),
            walk_speed: (self.walk.speed_old, self.walk.speed),
            walk_position: self.walk.position,
            swing: self.swing.current,
            swing_progress: (self.swing.old, self.swing.now),
            swim: (self.swim_old, self.swim),
            hurt: self.hurt_time > 0 || self.death_time > 0,
            death_time: self.death_time,
            fall_fly_ticks: self.fall_fly_ticks,
            movement: (self.movement_old, self.movement),
            cloak: (self.cloak_old, self.cloak),
            chest: self.chest,
            left_arm,
            right_arm,
            using: self
                .using()
                .then(|| (self.ticks_using(), self.off_hand_used())),
            use_remaining: self.use_remaining,
            age: 0,
            scale: 1.0,
            size: [0.6, 1.8],
            eye_height: 1.62,
            vehicle_body: None,
            held: [self.main_hand.clone(), self.off_hand.clone()],
            worn: self.worn.clone(),
        }
    }

    fn ticks_using(&self) -> i32 {
        if self.use_item.is_empty() {
            0
        } else {
            self.use_item.c.use_duration - self.use_remaining
        }
    }

    pub(crate) fn body_rots(&self) -> (f32, f32) {
        (self.body_rot_old, self.body_rot)
    }

    pub(crate) fn is_invisible(&self) -> bool {
        self.flags & FLAG_INVISIBLE != 0
    }

    /// `isDiscrete`: it sneaks.
    pub(crate) fn is_discrete(&self) -> bool {
        self.flags & FLAG_SHIFT_KEY_DOWN != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_protocol::packets::play::DataValue;

    fn player() -> Avatar {
        let skin = SkinSource::Player {
            id: 1,
            textures: None,
        };
        Avatar::new(true, skin, (0.0, 0.0), 0.0, [0.0; 3])
    }

    fn at(position: [f64; 3], old_position: [f64; 3]) -> TickInput {
        TickInput {
            position,
            old_position,
            y_rot: 0.0,
            x_rot: 0.0,
            pose: pose::STANDING,
            health: 20.0,
            passenger: false,
            interpolating: false,
            simulated: true,
            in_water: false,
            bed: None,
            spectator: false,
        }
    }

    const FIELDS: AvatarFields = AvatarFields {
        shared_flags: Some(0),
        ticks_frozen: Some(7),
        living_flags: Some(8),
        sleeping_pos: Some(14),
        main_hand: Some(15),
        customisation: Some(16),
        profile: Some(17),
    };

    #[test]
    fn walking_speeds_up_and_stops_with_death_and_riding() {
        let mut a = player();
        let step = at([0.0, 0.0, 0.2], [0.0; 3]);
        a.tick(&step);
        // min(0.2 * 4, 1) * 0.4.
        assert!((a.walk.speed - 0.32).abs() < 1e-6);
        assert_eq!((a.walk.speed_old, a.walk.position), (0.0, a.walk.speed));
        a.tick(&step);
        assert!((a.walk.speed - 0.512).abs() < 1e-6 && (a.walk.position - 0.832).abs() < 1e-6);
        // Going up alone is not walking.
        let mut up = player();
        up.tick(&at([0.0, 1.0, 0.0], [0.0; 3]));
        assert_eq!(up.walk.speed, 0.0);
        let mut riding = a.clone();
        riding.tick(&TickInput {
            passenger: true,
            ..step
        });
        assert_eq!(riding.walk, Walk::default());
        a.tick(&TickInput {
            health: 0.0,
            ..step
        });
        assert_eq!((a.walk, a.death_time), (Walk::default(), 1));
        a.tick(&TickInput {
            health: 0.0,
            simulated: false,
            ..step
        });
        assert_eq!(a.death_time, 1);
    }

    #[test]
    fn the_body_turns_the_way_it_goes_or_strikes() {
        // East is yaw -90: three tenths of the way there.
        let mut a = player();
        a.tick(&at([0.2, 0.0, 0.0], [0.0; 3]));
        assert!((a.body_rot + 27.0).abs() < 1e-3, "{}", a.body_rot);
        assert_eq!(a.body_rot_old, 0.0);
        // Backwards: it keeps facing forwards.
        let mut a = player();
        a.tick(&at([0.0, 0.0, -0.2], [0.0; 3]));
        assert!(a.body_rot.abs() < 1e-3, "{}", a.body_rot);
        // Too slow to turn.
        let mut a = player();
        a.tick(&at([0.04, 0.0, 0.0], [0.0; 3]));
        assert_eq!(a.body_rot, 0.0);
        // The head is never more than 50 from it.
        let mut a = player();
        a.tick(&TickInput {
            y_rot: 120.0,
            ..at([0.0; 3], [0.0; 3])
        });
        assert!((a.body_rot - 70.0).abs() < 1e-3, "{}", a.body_rot);
        let mut a = player();
        a.tick(&TickInput {
            y_rot: 70.0,
            ..at([0.0; 3], [0.0; 3])
        });
        assert!((a.body_rot - 20.0).abs() < 1e-3, "{}", a.body_rot);
        // Striking turns it to the view.
        let mut a = player();
        a.swing(Swing {
            off_hand: false,
            kind: 1,
            duration: 6,
        });
        let look = TickInput {
            y_rot: 40.0,
            ..at([0.0; 3], [0.0; 3])
        };
        a.tick(&look);
        assert_eq!(a.body_rot, 0.0);
        a.tick(&look);
        assert!((a.body_rot - 12.0).abs() < 1e-3, "{}", a.body_rot);
    }

    #[test]
    fn the_head_takes_three_steps() {
        let mut a = player();
        a.turn_head(90.0);
        let seen: Vec<f32> = (0..4)
            .map(|_| {
                a.tick(&at([0.0; 3], [0.0; 3]));
                a.head_rot
            })
            .collect();
        for (s, want) in seen.iter().zip([30.0, 60.0, 90.0, 90.0]) {
            assert!((s - want).abs() < 1e-3, "{seen:?}");
        }
        // A turn over the seam is taken the short way, and the old angle follows.
        let mut a = player();
        a.head_rot = 170.0;
        a.turn_head(-170.0);
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert!((a.head_rot - 176.667).abs() < 1e-2 && (a.head_rot_old - 170.0).abs() < 1e-3);
    }

    #[test]
    fn a_swing_is_replaced_only_after_its_half() {
        let swing = |off_hand| Swing {
            off_hand,
            kind: 1,
            duration: 6,
        };
        let mut s = SwingState::default();
        s.start(swing(false));
        // Not begun: replaced.
        s.start(swing(true));
        assert!(s.current.unwrap().off_hand);
        let mut seen = Vec::new();
        for _ in 0..4 {
            s.tick();
            seen.push(s.now);
            s.start(swing(false));
        }
        // At its third tick it is still in its first half; at its fourth it is not.
        assert_eq!(seen[..4], [0.0, 1.0 / 6.0, 2.0 / 6.0, 3.0 / 6.0]);
        assert!(!s.current.unwrap().off_hand);
        assert_eq!((s.now, s.ticks), (0.0, 0));
        for _ in 0..8 {
            s.tick();
        }
        assert_eq!((s.current, s.now, s.old), (None, 0.0, 1.0));
    }

    #[test]
    fn hurting_shows_for_ten_ticks() {
        let mut a = player();
        a.damaged();
        assert_eq!(a.walk.speed, 1.5);
        for i in 0..10 {
            assert!(a.record().hurt, "{i}");
            a.tick(&at([0.0; 3], [0.0; 3]));
        }
        assert!(!a.record().hurt);
    }

    #[test]
    fn the_cloak_follows_by_a_quarter_and_jumps_after_ten() {
        let mut a = player();
        a.tick(&at([8.0, 0.0, 20.0], [8.0, 0.0, 20.0]));
        assert_eq!(a.cloak, [2.0, 0.0, 20.0]);
        assert_eq!(a.cloak_old, [0.0, 0.0, 20.0]);
    }

    #[test]
    fn movement_is_eased_into() {
        let mut a = player();
        a.lerp_motion([1.0, 0.0, 0.0], 1);
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert_eq!(a.movement[0], 0.5);
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert_eq!((a.movement[0], a.movement_old[0]), (1.0, 0.5));
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert_eq!(a.movement[0], 1.0);
    }

    #[test]
    fn its_data_is_read() {
        let mut a = player();
        assert!(a.set_data(&FIELDS, 0, &DataValue::Byte(0b1010_0001u8 as i8)));
        assert!(a.set_data(&FIELDS, 16, &DataValue::Byte(0x7E)));
        assert!(a.set_data(&FIELDS, 15, &DataValue::Id(0)));
        assert!(a.set_data(&FIELDS, 7, &DataValue::Int(140)));
        assert!(a.set_data(&FIELDS, 14, &DataValue::OptionalBlockPos(Some([1, 2, 3]))));
        assert!(!a.set_data(&FIELDS, 9, &DataValue::Float(1.0)));
        let r = a.record();
        assert!(r.on_fire && r.invisible && r.fall_flying && r.left_handed && r.shaking);
        assert_eq!((r.parts, a.sleeping_pos), (0x7E, Some([1, 2, 3])));
        let p = ResolvableProfile {
            name: Some("Kai".into()),
            ..ResolvableProfile::default()
        };
        assert!(a.set_data(&FIELDS, 17, &DataValue::Profile(Box::new(p.clone()))));
        assert_eq!(*a.skin, SkinSource::Profile(p));
        // A mannequin shows all its layers until told otherwise, a player none.
        let skin = SkinSource::Profile(ResolvableProfile::default());
        let mannequin = Avatar::new(false, skin, (0.0, 0.0), 0.0, [0.0; 3]);
        assert_eq!(
            (mannequin.record().parts, player().record().parts),
            (0x7F, 0)
        );
    }

    fn stack(item: i32, c: crate::item::Components) -> Stack {
        Stack {
            item,
            count: 1,
            c: Arc::new(c),
            ..Stack::empty()
        }
    }

    #[test]
    fn a_spear_is_held_ready_and_a_stab_is_made_as_with_one() {
        use crate::item::Components;
        let named = |name: &'static str| {
            stack(
                9,
                Components {
                    name,
                    ..Components::default()
                },
            )
        };
        let mut a = player();
        a.set_held(false, named("minecraft:iron_spear"));
        a.set_held(true, named("minecraft:stick"));
        assert_eq!(a.arm_poses(), (ArmPose::Item, ArmPose::Spear));
        // A stab of the other hand, whatever it holds; a whack is not one.
        a.swing(Swing {
            off_hand: true,
            kind: 2,
            duration: 6,
        });
        assert_eq!(a.arm_poses(), (ArmPose::Spear, ArmPose::Spear));
        let mut b = player();
        b.set_held(true, named("minecraft:stick"));
        b.swing(Swing {
            off_hand: true,
            kind: 1,
            duration: 6,
        });
        assert_eq!(b.arm_poses(), (ArmPose::Item, ArmPose::Empty));
        // What is worn and held goes into the record, the main hand first.
        a.set_worn(WORN_LEGS, named("minecraft:iron_leggings"));
        let r = a.record();
        assert_eq!(r.held[0].c.name, "minecraft:iron_spear");
        assert_eq!(r.held[1].c.name, "minecraft:stick");
        assert!(r.worn[WORN_HEAD].is_empty() && !r.worn[WORN_LEGS].is_empty());
        assert_eq!(r.use_remaining, 0);
    }

    #[test]
    fn arms_hold_what_is_in_the_hands() {
        use crate::item::Components;
        let bow = || {
            stack(
                5,
                Components {
                    class: Class::Bow,
                    use_duration: 72000,
                    ..Components::default()
                },
            )
        };
        let mut a = player();
        a.set_held(true, stack(9, Components::default()));
        assert_eq!(a.arm_poses(), (ArmPose::Item, ArmPose::Empty));
        a.set_held(false, bow());
        assert_eq!(a.arm_poses(), (ArmPose::Item, ArmPose::Item));
        // Drawn: both arms, so the other hand only holds.
        a.set_data(&FIELDS, 8, &DataValue::Byte(1));
        assert_eq!(a.arm_poses(), (ArmPose::Item, ArmPose::Bow));
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert_eq!(a.record().using, Some((1, false)));
        a.set_data(&FIELDS, 15, &DataValue::Id(0));
        assert_eq!(a.arm_poses(), (ArmPose::Bow, ArmPose::Item));
        // Even a loaded crossbow.
        a.set_held(
            true,
            stack(
                6,
                Components {
                    class: Class::Crossbow,
                    charged: true,
                    ..Components::default()
                },
            ),
        );
        assert_eq!(a.arm_poses(), (ArmPose::Bow, ArmPose::Item));
        // Another item in the hand ends it.
        a.set_held(false, stack(9, Components::default()));
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert_eq!(a.arm_poses().0, ArmPose::Item);
        assert_eq!(a.record().using, Some((0, false)));
        // A shield in the off hand.
        let shield = stack(
            7,
            Components {
                blocks_attacks: true,
                use_duration: 72000,
                ..Components::default()
            },
        );
        let mut a = player();
        a.set_held(true, shield);
        a.set_data(&FIELDS, 8, &DataValue::Byte(3));
        assert_eq!(a.arm_poses(), (ArmPose::Block, ArmPose::Empty));
        // A loaded crossbow is held up, but not while swinging.
        let crossbow = stack(
            6,
            Components {
                class: Class::Crossbow,
                charged: true,
                ..Components::default()
            },
        );
        let mut a = player();
        a.set_held(false, crossbow);
        assert_eq!(a.arm_poses().1, ArmPose::CrossbowHold);
        a.swing(Swing {
            off_hand: false,
            kind: 1,
            duration: 6,
        });
        assert_eq!(a.arm_poses().1, ArmPose::Item);
    }

    #[test]
    fn default_skins_go_by_the_id() {
        assert_eq!(default_skin(0), 0);
        assert_eq!(default_skin(17), 17);
        // A hash of -1.
        assert_eq!(default_skin(0xFFFF_FFFF), 17);
        assert_eq!(default_skin(1 << 64 | 1 << 32), 0);
        let named = SkinSource::Profile(ResolvableProfile {
            name: Some("Notch".into()),
            ..ResolvableProfile::default()
        });
        assert_eq!(named.id(), 0xb50a_d385_829d_3141_a216_7e7d_7539_ba7f);
        let none = SkinSource::Profile(ResolvableProfile::default());
        assert_eq!(none.id(), 0);
    }

    #[test]
    fn swimming_comes_and_goes() {
        let mut a = player();
        let swim = TickInput {
            pose: pose::SWIMMING,
            ..at([0.0; 3], [0.0; 3])
        };
        for _ in 0..12 {
            a.tick(&swim);
        }
        assert_eq!(a.swim, 1.0);
        a.tick(&at([0.0; 3], [0.0; 3]));
        assert!((a.swim - 0.91).abs() < 1e-6 && a.swim_old == 1.0);
        // Lying without gliding looks like swimming; gliding does not.
        let lie = TickInput {
            pose: pose::FALL_FLYING,
            ..at([0.0; 3], [0.0; 3])
        };
        a.tick(&lie);
        assert_eq!(a.swim, 1.0);
        a.set_data(&FIELDS, 0, &DataValue::Byte(-128));
        a.tick(&lie);
        assert!(a.swim < 1.0);
        assert_eq!(a.record().fall_fly_ticks, 1);
    }
}
