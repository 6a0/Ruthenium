//! The local player: its movement each tick, ported from vanilla's `LocalPlayer`,
//! `Player`, `LivingEntity` and `Entity` so the same inputs give the same doubles,
//! and the movement packets it sends (`LocalPlayer.sendChanges`).
//!
//! Covered so far: walking, sprinting, sneaking and jumping on land and in the air,
//! block collisions with step-up, block friction and speed and jump factors, bouncing,
//! the push out of suffocating blocks, poses, climbing, water and lava (depth, eyes
//! under the surface, currents, swimming, swim sprinting, jumping and sinking in
//! fluids), entities as solid as blocks (boats, shulkers; [`Env::colliders`]); other
//! entities' pushes come from their ticks ([`crate::entity`]). Flight: the abilities
//! the game mode and the server give, double-tapping jump, flying speeds, landing, and
//! spectators going through everything. The effects that move the player (jump boost,
//! levitation, slow falling, dolphin's grace, blindness stopping a sprint), and the
//! server's copy of the player's entity data, which overrides the client's own (its
//! sprinting flag, pose, no-gravity). Using items: the hands, the use the client
//! predicts and the server's living flags start and stop, the slowdown and the sprint
//! it blocks, food, cooldowns ([`crate::interact`] starts and releases uses). Not yet:
//! elytra, riptide, the scale attribute, vehicles, the world border, and the fence, wall
//! and bounce-suppression tags.

// Negated float comparisons (`!(a < b)`) are kept as the game writes them: they differ
// from the plain comparison when a value is NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ruthenium_data::v26_3::registries::MobEffect;
use ruthenium_javamath::mth;
use ruthenium_protocol::packets::play::{self, DataValue, PlayerPosition};
use ruthenium_protocol::wire;

use crate::blocks::{BlockTags, Blocks, Who};
use crate::effect::{Effect, Effects};
use crate::entity::EntityTypes;
use crate::fluid::FluidInteraction;
use crate::inventory::{Cooldowns, Hand, Inventory};
use crate::item::{Food, Stack};
use crate::shape::{Aabb, Axis, EPSILON, Placed, Shape, collide_all};
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

/// `Pose`, in network id order. The player itself takes standing, crouching and
/// swimming; the others come from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    Standing,
    FallFlying,
    Sleeping,
    Swimming,
    SpinAttack,
    Crouching,
    LongJumping,
    Dying,
    Croaking,
    UsingTongue,
    Sitting,
    Roaring,
    Sniffing,
    Emerging,
    Digging,
    Sliding,
    Shooting,
    Inhaling,
}

impl Pose {
    const ALL: [Pose; 18] = [
        Pose::Standing,
        Pose::FallFlying,
        Pose::Sleeping,
        Pose::Swimming,
        Pose::SpinAttack,
        Pose::Crouching,
        Pose::LongJumping,
        Pose::Dying,
        Pose::Croaking,
        Pose::UsingTongue,
        Pose::Sitting,
        Pose::Roaring,
        Pose::Sniffing,
        Pose::Emerging,
        Pose::Digging,
        Pose::Sliding,
        Pose::Shooting,
        Pose::Inhaling,
    ];

    /// `Pose.BY_ID`: an id out of range is standing.
    pub fn from_id(id: i32) -> Pose {
        usize::try_from(id)
            .ok()
            .and_then(|i| Pose::ALL.get(i))
            .copied()
            .unwrap_or(Pose::Standing)
    }

    pub fn id(self) -> i32 {
        self as i32
    }

    /// By the Java constant's name (`CROUCHING`), as traces record it.
    pub fn from_name(name: &str) -> Option<Pose> {
        const NAMES: [&str; 18] = [
            "STANDING",
            "FALL_FLYING",
            "SLEEPING",
            "SWIMMING",
            "SPIN_ATTACK",
            "CROUCHING",
            "LONG_JUMPING",
            "DYING",
            "CROAKING",
            "USING_TONGUE",
            "SITTING",
            "ROARING",
            "SNIFFING",
            "EMERGING",
            "DIGGING",
            "SLIDING",
            "SHOOTING",
            "INHALING",
        ];
        NAMES.iter().position(|n| *n == name).map(|i| Pose::ALL[i])
    }

    /// Width, height and eye height (`Avatar.POSES`; the poses it doesn't list stand).
    fn dimensions(self) -> (f32, f32, f32) {
        match self {
            Pose::Sleeping => (0.2, 0.2, 0.2),
            Pose::FallFlying | Pose::Swimming | Pose::SpinAttack => (0.6, 0.6, 0.4),
            Pose::Crouching => (0.6, 1.5, 1.27),
            Pose::Dying => (0.2, 0.2, 1.62),
            _ => (0.6, 1.8, 1.62),
        }
    }

    /// Width and height.
    fn size(self) -> (f32, f32) {
        let (w, h, _) = self.dimensions();
        (w, h)
    }

    /// The eye height.
    fn eye_height(self) -> f32 {
        self.dimensions().2
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
    pub(crate) fn new(base: f64, min: f64, max: f64) -> Attribute {
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
    pub water_movement_efficiency: Attribute,
    pub bounciness: Attribute,
    pub block_interaction_range: Attribute,
    pub entity_interaction_range: Attribute,
    /// Attacks per second at full strength (sent by the server, with the held item's
    /// modifiers). The attack damage and knockback attributes aren't sent to clients,
    /// so the client keeps the player's defaults (see [`LocalPlayer::attack`]).
    pub attack_speed: Attribute,
    /// Added to a tool's speed on blocks it is fast on (efficiency).
    pub mining_efficiency: Attribute,
    pub block_break_speed: Attribute,
    /// The mining speed's factor with the eyes under water (aqua affinity).
    pub submerged_mining_speed: Attribute,
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
            water_movement_efficiency: Attribute::new(0.0, 0.0, 1.0),
            bounciness: Attribute::new(0.0, 0.0, 1.0),
            block_interaction_range: Attribute::new(4.5, 0.0, 64.0),
            entity_interaction_range: Attribute::new(3.0, 0.0, 64.0),
            attack_speed: Attribute::new(4.0, 0.0, 1024.0),
            mining_efficiency: Attribute::new(0.0, 0.0, 1024.0),
            block_break_speed: Attribute::new(1.0, 0.0, 1024.0),
            submerged_mining_speed: Attribute::new(0.2, 0.0, 20.0),
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
            "minecraft:water_movement_efficiency" => &mut self.water_movement_efficiency,
            "minecraft:bounciness" => &mut self.bounciness,
            "minecraft:block_interaction_range" => &mut self.block_interaction_range,
            "minecraft:entity_interaction_range" => &mut self.entity_interaction_range,
            "minecraft:attack_speed" => &mut self.attack_speed,
            "minecraft:mining_efficiency" => &mut self.mining_efficiency,
            "minecraft:block_break_speed" => &mut self.block_break_speed,
            "minecraft:submerged_mining_speed" => &mut self.submerged_mining_speed,
            _ => return None,
        })
    }

    /// `assignBaseValues`: each attribute's base value, without modifiers.
    fn base_values(&self) -> Attributes {
        let mut a = self.clone();
        for attribute in [
            &mut a.movement_speed,
            &mut a.jump_strength,
            &mut a.gravity,
            &mut a.step_height,
            &mut a.sneaking_speed,
            &mut a.friction_modifier,
            &mut a.air_drag_modifier,
            &mut a.movement_efficiency,
            &mut a.water_movement_efficiency,
            &mut a.bounciness,
            &mut a.block_interaction_range,
            &mut a.entity_interaction_range,
            &mut a.attack_speed,
            &mut a.mining_efficiency,
            &mut a.block_break_speed,
            &mut a.submerged_mining_speed,
        ] {
            attribute.modifiers.clear();
        }
        a
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

/// What the player may do (`Abilities`): set from the game mode, then by the server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Abilities {
    pub invulnerable: bool,
    pub flying: bool,
    pub may_fly: bool,
    pub instabuild: bool,
    pub may_build: bool,
    pub flying_speed: f32,
    /// What the server says the walking speed is (the field of view follows it; the
    /// movement speed is an attribute).
    pub walking_speed: f32,
}

impl Default for Abilities {
    fn default() -> Abilities {
        Abilities {
            invulnerable: false,
            flying: false,
            may_fly: false,
            instabuild: false,
            may_build: true,
            flying_speed: 0.05,
            walking_speed: 0.1,
        }
    }
}

impl Abilities {
    /// `GameType.updatePlayerAbilities`: creative may fly, a spectator flies, other
    /// modes stop flying; adventure and spectator may not build.
    pub fn set_game_type(&mut self, game_type: i32) {
        match game_type {
            play::CREATIVE => {
                self.may_fly = true;
                self.instabuild = true;
                self.invulnerable = true;
            }
            play::SPECTATOR => {
                self.may_fly = true;
                self.instabuild = false;
                self.invulnerable = true;
                self.flying = true;
            }
            _ => {
                self.may_fly = false;
                self.instabuild = false;
                self.invulnerable = false;
                self.flying = false;
            }
        }
        self.may_build = !matches!(game_type, play::ADVENTURE | play::SPECTATOR);
    }

    /// `handlePlayerAbilities`: everything but `may_build`.
    pub fn apply(&mut self, flags: u8, flying_speed: f32, walking_speed: f32) {
        self.invulnerable = flags & play::ABILITY_INVULNERABLE != 0;
        self.flying = flags & play::ABILITY_FLYING != 0;
        self.may_fly = flags & play::ABILITY_MAY_FLY != 0;
        self.instabuild = flags & play::ABILITY_INSTABUILD != 0;
        self.flying_speed = flying_speed;
        self.walking_speed = walking_speed;
    }
}

/// The shared flags' bits (`Entity.FLAG_*`).
pub mod flag {
    pub const ON_FIRE: u8 = 1;
    pub const SHIFT_KEY_DOWN: u8 = 1 << 1;
    pub const SPRINTING: u8 = 1 << 3;
    pub const SWIMMING: u8 = 1 << 4;
    pub const INVISIBLE: u8 = 1 << 5;
    pub const GLOWING: u8 = 1 << 6;
    pub const FALL_FLYING: u8 = 1 << 7;
}

/// The player's synched entity data besides its pose, health and the sprinting and
/// swimming flags (which are [`LocalPlayer`]'s own fields): the values the server
/// sends back in `set_entity_data`, which replace the client's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerData {
    /// `DATA_SHARED_FLAGS_ID` without the sprinting and swimming bits ([`flag`]). The
    /// shift bit is the server's: the local player reads its own keys.
    pub flags: u8,
    pub air_supply: i32,
    pub no_gravity: bool,
    pub ticks_frozen: i32,
    /// `DATA_LIVING_ENTITY_FLAGS`: using an item (1), with the off hand (2), riptide
    /// (4).
    pub living_flags: u8,
    pub absorption: f32,
    pub score: i32,
}

impl Default for PlayerData {
    fn default() -> PlayerData {
        PlayerData {
            flags: 0,
            air_supply: 300,
            no_gravity: false,
            ticks_frozen: 0,
            living_flags: 0,
            absorption: 0.0,
            score: 0,
        }
    }
}

/// `handleRespawn`'s `dataToKeep` bits.
pub const KEEP_ATTRIBUTES: u8 = 1;
pub const KEEP_ENTITY_DATA: u8 = 2;

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
    /// The boxes of the entities the player collides with (boats, shulkers:
    /// `canBeCollidedWith`), as solid as blocks.
    pub colliders: &'a [Aabb],
}

impl<'a> Env<'a> {
    fn state(&self, p: [i32; 3]) -> u32 {
        self.level.block(p[0], p[1], p[2])
    }

    /// The block shapes an entity (`who`) may meet in `b`.
    fn collisions(&self, b: &Aabb, who: Option<Who>) -> Vec<Placed<'_>> {
        self.level
            .block_collisions_for(self.blocks, b, false, who)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    }

    /// `noCollision`: no block shape and no entity in the way.
    pub(crate) fn no_collision(&self, b: &Aabb) -> bool {
        self.no_collision_for(b, None)
    }

    /// `noCollision(entity, box)`: the same for an entity's own query.
    pub(crate) fn no_collision_for(&self, b: &Aabb, who: Option<Who>) -> bool {
        self.collisions(b, who).is_empty() && self.entity_colliders(b).next().is_none()
    }

    /// `getEntityCollisions`: the colliders touching the area (grown by 1e-7), none
    /// for a vanishing area.
    fn entity_colliders<'b>(&'b self, area: &Aabb) -> impl Iterator<Item = &'a Aabb> + 'b {
        let size = (0..3).map(|i| area.max[i] - area.min[i]).sum::<f64>() / 3.0;
        let grown = area.inflate([1.0E-7; 3]);
        let colliders: &'a [Aabb] = if size < 1.0E-7 { &[] } else { self.colliders };
        colliders.iter().filter(move |c| c.intersects(&grown))
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
    /// The shared flags' sprinting bit (`isSprinting`): set by the client, and by the
    /// server's echo, which leaves the sprint speed modifier as it is.
    pub sprinting: bool,
    /// The shared flags' swimming bit, as set (`isSwimming` also needs the player not
    /// flying and not a spectator).
    pub swimming: bool,
    pub pose: Pose,
    pub attributes: Attributes,
    pub food: i32,
    /// `getHealth` (`set_health` and the entity data).
    pub health: f32,
    /// The client's game mode (`MultiPlayerGameMode.localPlayerMode`): the abilities
    /// follow it, and in spectator mode the player always flies.
    game_type: i32,
    /// `isSpectator`: the player list's game mode for this player, which can lag
    /// behind [`Self::game_type`]. Spectators go through everything.
    pub spectator: bool,
    /// `isCreative`: the player list's game mode for this player is creative.
    pub creative: bool,
    /// The operator level the server gave (entity events 24 to 28: 0 to 4).
    pub permission_level: i32,
    pub abilities: Abilities,
    pub effects: Effects,
    pub data: PlayerData,
    /// Ticks left to double-tap jump (`jumpTriggerTime`).
    jump_trigger_time: i32,
    /// `noPhysics`: moves without collisions (spectators).
    no_physics: bool,
    /// Packets the tick sent before `sendChanges` (abilities), sent first.
    outbox: Vec<Vec<u8>>,
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
    /// The fluids the player's box is in, as last updated.
    fluids: FluidInteraction,
    /// `wasTouchingWater` (`isInWater`).
    in_water: bool,
    /// `LocalPlayer.wasUnderwater`: the eyes were in water at the start of the tick.
    was_underwater: bool,
    first_tick: bool,
    // `sendChanges` state.
    last_sent_keys: Keys,
    last_sent: [f64; 3],
    last_sent_rot: (f32, f32),
    last_on_ground: bool,
    last_horizontal_collision: bool,
    was_sprinting: bool,
    position_reminder: i32,
    /// Where the player was and how it was turned as its last tick began
    /// (`xo`, `yRotO`...): the camera and the crosshair's pick interpolate from them.
    pub old_position: [f64; 3],
    pub old_rot: (f32, f32),
    /// The inventory and the hands.
    pub inventory: Inventory,
    /// `ItemCooldowns`.
    pub cooldowns: Cooldowns,
    /// `FoodData.saturationLevel` (`set_health`, and eating).
    pub saturation: f32,
    /// `LivingEntity.useItem` and `useItemRemaining`; `LocalPlayer.startedUsingItem`
    /// and `usingItemHand` (kept after the use ends, as vanilla keeps it).
    use_item: Stack,
    use_remaining: i32,
    started_using: bool,
    using_hand: Option<Hand>,
    /// `attackStrengthTicker` and `itemSwapTicker`: ticks since the last attack and
    /// since the main hand last changed item.
    attack_strength_ticker: i32,
    item_swap_ticker: i32,
    /// `lastItemInMainHand`: a copy of the main hand as the last tick left it.
    last_item_in_main_hand: Stack,
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
            swimming: false,
            pose: Pose::Standing,
            attributes: Attributes::default(),
            food: 20,
            health: 20.0,
            game_type: play::SURVIVAL,
            spectator: false,
            creative: false,
            permission_level: 0,
            abilities: Abilities::default(),
            effects: Effects::default(),
            data: PlayerData::default(),
            jump_trigger_time: 0,
            no_physics: false,
            outbox: Vec::new(),
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
            fluids: FluidInteraction::default(),
            in_water: false,
            was_underwater: false,
            first_tick: true,
            last_sent_keys: Keys::default(),
            last_sent: [0.0; 3],
            last_sent_rot: (0.0, 0.0),
            last_on_ground: false,
            last_horizontal_collision: false,
            was_sprinting: false,
            position_reminder: 0,
            old_position: [0.0; 3],
            old_rot: (-180.0, 0.0),
            inventory: Inventory::default(),
            cooldowns: Cooldowns::default(),
            saturation: 5.0,
            use_item: Stack::empty(),
            use_remaining: 0,
            started_using: false,
            using_hand: None,
            attack_strength_ticker: 0,
            item_swap_ticker: 0,
            last_item_in_main_hand: Stack::empty(),
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

    /// `handleRespawn`'s new player. With [`KEEP_ENTITY_DATA`] it keeps the entity data,
    /// the motion, the rotation, the last input sent and whether it was sprinting; else it
    /// faces -180 (where it is doesn't matter: the server's teleport places it). With
    /// [`KEEP_ATTRIBUTES`] the attributes and their modifiers, else their base values.
    /// Effects and abilities start afresh; the game mode's are set next.
    pub fn respawned(&self, keep: u8) -> LocalPlayer {
        let mut p = LocalPlayer::new();
        p.entity_id = self.entity_id;
        if keep & KEEP_ENTITY_DATA != 0 {
            p.last_sent_keys = self.last_sent_keys;
            p.was_sprinting = self.sprinting;
            // `getNonDefaultValues`, assigned.
            p.sprinting = self.sprinting;
            p.swimming = self.swimming;
            p.pose = self.pose;
            p.health = self.health;
            p.data = self.data;
            p.delta = self.delta;
            p.y_rot = self.y_rot;
            p.x_rot = self.x_rot;
        }
        p.attributes = if keep & KEEP_ATTRIBUTES != 0 {
            self.attributes.clone()
        } else {
            self.attributes.base_values()
        };
        p.spectator = self.spectator;
        p.creative = self.creative;
        p.set_game_type(self.game_type);
        p
    }

    pub fn game_type(&self) -> i32 {
        self.game_type
    }

    /// `setLocalMode` (and `adjustPlayer`): the game mode and its abilities.
    pub fn set_game_type(&mut self, game_type: i32) {
        self.game_type = game_type;
        self.abilities.set_game_type(game_type);
    }

    /// `onGameModeChanged`: the player list says the player's game mode changed;
    /// becoming a spectator stops it rising or falling.
    pub fn on_game_mode_changed(&mut self, game_mode: i32) {
        if game_mode == play::SPECTATOR {
            self.delta[1] = 0.0;
        }
    }

    /// `Entity.absSnapTo`: moved there, with no interpolation from where it was (the
    /// horizontal position clamped to the world's limits).
    pub fn abs_snap_to(&mut self, pos: [f64; 3]) {
        let x = pos[0].clamp(-3.0e7, 3.0e7);
        let z = pos[2].clamp(-3.0e7, 3.0e7);
        self.position = [x, pos[1], z];
        self.old_position = self.position;
    }

    /// `isEyeInFluid(WATER)`, as the last fluid update left it.
    pub fn is_eye_in_water(&self) -> bool {
        self.fluids.eye_in(true)
    }

    /// `handleUpdateMobEffect`.
    pub fn update_effect(&mut self, e: Effect) {
        self.effects.update(e);
    }

    /// `handleRemoveMobEffect`.
    pub fn remove_effect(&mut self, id: i32) {
        self.effects.remove(id);
    }

    /// `handleSetEntityData` for the local player: the server's values replace the
    /// client's, the sprinting flag without its speed modifier (`assignValues` doesn't go
    /// through `setSprinting`), the pose with its box.
    pub fn set_data(&mut self, values: &[(u8, DataValue)], types: &EntityTypes) {
        let f = types.player_fields();
        for (field, value) in values {
            let field = Some(*field);
            match *value {
                DataValue::Byte(b) if field == f.shared_flags => {
                    let b = b as u8;
                    self.sprinting = b & flag::SPRINTING != 0;
                    self.swimming = b & flag::SWIMMING != 0;
                    self.data.flags = b & !(flag::SPRINTING | flag::SWIMMING);
                }
                DataValue::Int(n) if field == f.air_supply => self.data.air_supply = n,
                DataValue::Boolean(b) if field == f.no_gravity => self.data.no_gravity = b,
                DataValue::Pose(p) if field == f.pose => self.pose = Pose::from_id(p),
                DataValue::Int(n) if field == f.ticks_frozen => self.data.ticks_frozen = n,
                DataValue::Byte(b) if field == f.living_flags => {
                    self.data.living_flags = b as u8;
                    self.on_living_flags();
                }
                DataValue::Float(h) if field == f.health => self.health = h,
                DataValue::Float(a) if field == f.absorption => self.data.absorption = a,
                DataValue::Int(n) if field == f.score => self.data.score = n,
                _ => {}
            }
        }
    }

    /// `isFallFlying`: the server's flag (elytra flight isn't ported).
    pub fn fall_flying(&self) -> bool {
        self.data.flags & flag::FALL_FLYING != 0
    }

    /// `getEyePosition()`: where the eyes are now.
    pub fn eye_position(&self) -> [f64; 3] {
        let p = self.position;
        [p[0], p[1] + f64::from(self.pose.eye_height()), p[2]]
    }

    /// `getEyePosition(1.0)`, as the crosshair's pick takes it: interpolated all the way
    /// from where the last tick began (`Mth.lerp`, which can round).
    pub fn pick_eye_position(&self) -> [f64; 3] {
        let (o, p) = (self.old_position, self.position);
        let lerp = |a: f64, b: f64| a + 1.0 * (b - a);
        [
            lerp(o[0], p[0]),
            lerp(o[1], p[1]) + f64::from(self.pose.eye_height()),
            lerp(o[2], p[2]),
        ]
    }

    /// `getItemInHand`.
    pub fn hand(&self, hand: Hand) -> &Stack {
        self.inventory.hand(hand)
    }

    /// `LocalPlayer.isUsingItem`: the client's own idea, which the server's living
    /// flags start and stop.
    pub fn is_using_item(&self) -> bool {
        self.started_using
    }

    /// `getUsedItemHand`: the hand last used (the main hand before any).
    pub fn used_hand(&self) -> Hand {
        self.using_hand.unwrap_or(Hand::Main)
    }

    /// `getUseItem`.
    pub fn use_item(&self) -> &Stack {
        &self.use_item
    }

    /// `getUseItemRemainingTicks`.
    pub fn use_remaining(&self) -> i32 {
        self.use_remaining
    }

    /// `hasInfiniteMaterials`: creative's instant building.
    pub fn infinite_materials(&self) -> bool {
        self.abilities.instabuild
    }

    /// `isSecondaryUseActive` (`isShiftKeyDown`): sneak held as the last tick read it.
    pub fn secondary_use(&self) -> bool {
        self.keys.shift
    }

    /// `Player.canEat`.
    pub fn can_eat(&self, can_always_eat: bool) -> bool {
        self.abilities.invulnerable || can_always_eat || self.food < 20
    }

    /// `FoodData.eat(FoodProperties)`.
    pub fn eat(&mut self, food: Food) {
        self.food = (food.nutrition + self.food).clamp(0, 20);
        let s = food.saturation + self.saturation;
        // Mth.clamp(float): a NaN passes through.
        let max = self.food as f32;
        self.saturation = if s < 0.0 { 0.0 } else { s.min(max) };
    }

    /// `isSlowDueToUsingItem` (riding isn't ported).
    fn slow_due_to_using_item(&self) -> bool {
        self.started_using && !self.use_item.c.use_effects().can_sprint
    }

    /// `LocalPlayer.startUsingItem`: the item in the hand, if any, unless already using
    /// one.
    pub fn start_using_item(&mut self, hand: Hand) {
        let stack = self.hand(hand).clone();
        if !stack.is_empty() && !self.started_using {
            // LivingEntity.startUsingItem (its own checks pass by now).
            self.use_remaining = stack.c.use_duration;
            self.use_item = stack;
            self.started_using = true;
            self.using_hand = Some(hand);
        }
    }

    /// `LocalPlayer.stopUsingItem`.
    pub fn stop_using_item(&mut self) {
        self.use_item = Stack::empty();
        self.use_remaining = 0;
        self.started_using = false;
    }

    /// `LivingEntity.releaseUsingItem` on the client (the release packet is the
    /// caller's): a crossbow updates once more; a riptide trident's spin isn't ported.
    pub fn release_using_item(&mut self) {
        let held = self.hand(self.used_hand()).clone();
        if !self.use_item.is_empty() && held.same_item(&self.use_item) {
            self.use_item = held;
            if self.use_item.c.use_on_release() {
                self.updating_using_item();
            }
        }
        self.stop_using_item();
    }

    /// `LivingEntity.updatingUsingItem`: the use goes on while the hand holds the same
    /// item, else it stops. The client never finishes it itself.
    fn updating_using_item(&mut self) {
        if !self.started_using {
            return;
        }
        let held = self.hand(self.used_hand());
        if held.same_item(&self.use_item) {
            self.use_item = held.clone();
            self.use_remaining -= 1;
        } else {
            self.stop_using_item();
        }
    }

    /// `completeUsingItem`, on the server's `USE_ITEM_COMPLETE` event: unless the
    /// hand's stack was replaced since the last tick (then it is only released), the
    /// item is finished: eaten, drunk, turned into its remainder.
    pub fn complete_using_item(&mut self) {
        if !self.started_using {
            return;
        }
        let hand = self.used_hand();
        if !self.use_item.same_object(self.hand(hand)) {
            self.release_using_item();
            return;
        }
        if !self.use_item.is_empty() {
            self.finish_using_item(hand);
            self.stop_using_item();
        }
    }

    /// `ItemStack.finishUsingItem` on the stack in a hand: `Item.finishUsingItem`
    /// (consumables are consumed, food eaten) and `applyAfterUseComponentSideEffects`.
    fn finish_using_item(&mut self, hand: Hand) {
        let infinite = self.infinite_materials();
        let before = self.hand(hand).clone();
        if let Some(c) = before.c.consumable {
            let _ = c;
            if let Some(food) = before.c.food {
                self.eat(food);
            }
            self.inventory.hand_mut(hand).consume(1, infinite);
        }
        self.after_use(hand, &before);
    }

    /// `applyAfterUseComponentSideEffects` for the stack now in `hand`, given the stack
    /// as it was before the use: the use remainder, the use cooldown.
    pub fn after_use(&mut self, hand: Hand, before: &Stack) {
        if let Some(remainder) = before.c.remainder.clone() {
            let used = self.hand(hand);
            if !self.infinite_materials() && used.count < before.count {
                let made = remainder.copy();
                if used.is_empty() {
                    self.inventory.set_hand(hand, made);
                } else if !self.inventory.add(made) {
                    // Dropped: the server's inventory update follows.
                }
            }
        }
        if let Some(cd) = &before.c.use_cooldown {
            self.cooldowns.add(&before.cooldown_group(), cd.ticks());
        }
    }

    /// The living flags changed (`onSyncedDataUpdated`): `LivingEntity`'s part keeps
    /// the use item in step, then `LocalPlayer`'s starts or stops using as the server
    /// says.
    fn on_living_flags(&mut self) {
        if self.started_using && self.use_item.is_empty() {
            self.use_item = self.hand(self.used_hand()).clone();
            if !self.use_item.is_empty() {
                self.use_remaining = self.use_item.c.use_duration;
            }
        } else if !self.started_using && !self.use_item.is_empty() {
            self.use_item = Stack::empty();
            self.use_remaining = 0;
        }
        let flags = self.data.living_flags;
        let using = flags & 1 != 0;
        let hand = if flags & 2 != 0 {
            Hand::Off
        } else {
            Hand::Main
        };
        if using && !self.started_using {
            self.start_using_item(hand);
        } else if !using && self.started_using {
            self.stop_using_item();
        }
    }

    /// Applies a teleport (`PositionMoveRotation.calculateAbsolute`, then set without
    /// interpolation).
    pub fn teleport(&mut self, t: &PlayerPosition) {
        (self.position, self.delta, self.y_rot, self.x_rot) = calculate_absolute(
            self.position,
            self.delta,
            self.y_rot,
            self.x_rot,
            t.position,
            t.delta,
            t.y_rot,
            t.x_rot,
            t.relatives,
        );
        // The old position and rotation move with it (no interpolation).
        let (old, _, old_y, old_x) = calculate_absolute(
            self.old_position,
            [0.0; 3],
            self.old_rot.0,
            self.old_rot.1,
            t.position,
            t.delta,
            t.y_rot,
            t.x_rot,
            t.relatives,
        );
        self.old_position = old;
        self.old_rot = (old_y, old_x);
    }

    /// `handleUpdateAttributes`: each attribute's base, and its modifiers replaced by
    /// the server's.
    pub fn apply_attributes(
        &mut self,
        attributes: &[ruthenium_protocol::packets::play::AttributeSnapshot],
    ) {
        for a in attributes {
            let Some(name) = u32::try_from(a.attribute)
                .ok()
                .and_then(ruthenium_data::v26_3::registries::Attribute::from_id)
            else {
                continue;
            };
            if let Some(instance) = self.attributes.get_mut(name.name()) {
                instance.base = a.base;
                instance.modifiers = a
                    .modifiers
                    .iter()
                    .map(|(id, amount, operation)| Modifier {
                        id: id.clone(),
                        amount: *amount,
                        operation: *operation,
                    })
                    .collect();
            }
        }
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

    /// `getCurrentItemAttackStrengthDelay`: ticks from an attack to full strength.
    pub fn attack_strength_delay(&self) -> f32 {
        (1.0 / self.attributes.attack_speed.value() * 20.0) as f32
    }

    /// `getAttackStrengthScale`: how charged an attack would be, `a` ticks on.
    pub fn attack_strength_scale(&self, a: f32) -> f32 {
        mth_clamp(
            (self.attack_strength_ticker as f32 + a) / self.attack_strength_delay(),
            0.0,
            1.0,
        )
    }

    /// `cannotAttackWithItem`: the item needs more charge (`minimum_attack_charge`)
    /// than the attack would have, `tolerance` ticks on.
    pub fn cannot_attack_with_item(&self, stack: &Stack, tolerance: i32) -> bool {
        let required = stack.c.minimum_attack_charge;
        let optimistic =
            (self.attack_strength_ticker + tolerance) as f32 / self.attack_strength_delay();
        required > 0.0 && optimistic < required
    }

    /// `resetAttackStrengthTicker`.
    pub fn reset_attack_strength_ticker(&mut self) {
        self.attack_strength_ticker = 0;
        self.item_swap_ticker = 0;
    }

    /// `onAttack` (a spear's jab): only the attack strength starts over.
    pub fn on_attack(&mut self) {
        self.attack_strength_ticker = 0;
    }

    /// `Player.attack` as it runs on the client, after `MultiPlayerGameMode.attack`
    /// sent the packet. Damage, sounds, particles and the target's own knockback are
    /// left to the server; what the client does to itself is `causeExtraKnockback`'s
    /// slowdown.
    ///
    /// `hurts` is whether the target can be attacked (`cannotAttack` false) and its
    /// `hurtClient` takes the hit: on the client that holds for other players,
    /// vehicles, shulker bullets and end crystals, not for mobs. The client's attack
    /// damage is the player's base of 1 and its attack knockback 0 (neither attribute
    /// is sent to clients, and only the server applies equipment and effect
    /// modifiers), so the damage is never zero and the knockback is the sprint's alone:
    /// a sprinting hit at more than 0.9 strength slows the attacker to 0.6 across and
    /// ends the sprint. `onAttack`'s reset is the caller's (`resetAttackStrengthTicker`
    /// follows at once).
    pub fn attack(&mut self, hurts: bool) {
        let scale = self.attack_strength_scale(0.5);
        if !hurts {
            return;
        }
        let full_strength = scale > 0.9;
        let knockback_attack = self.sprinting && full_strength;
        // `getKnockback`: the attribute halved, on the client.
        let knockback = 0.0f32 / 2.0 + if knockback_attack { 0.5 } else { 0.0 };
        if knockback > 0.0 {
            self.delta = [
                self.delta[0] * 0.6,
                self.delta[1] * 1.0,
                self.delta[2] * 0.6,
            ];
            self.set_sprinting(false);
        }
    }

    /// `Player.hasEnoughFoodToDoExhaustiveManoeuvres`.
    fn enough_food(&self) -> bool {
        self.food > 6 || self.abilities.may_fly
    }

    /// `Player.isSwimming`: the flag, unless flying or a spectator.
    fn is_swimming(&self) -> bool {
        !self.abilities.flying && !self.spectator && self.swimming
    }

    /// `Player.isAffectedByFluids` (and `isPushedByFluid`): not while flying.
    fn affected_by_fluids(&self) -> bool {
        !self.abilities.flying
    }

    /// `getGravity`: none with the no-gravity flag.
    fn gravity(&self) -> f64 {
        if self.data.no_gravity {
            0.0
        } else {
            self.attributes.gravity.value()
        }
    }

    /// `LivingEntity.getEffectiveGravity`: slow falling caps it at 0.01 while falling.
    fn effective_gravity(&self) -> f64 {
        let falling = self.delta[1] <= 0.0;
        if falling && self.effects.has(MobEffect::SlowFalling) {
            self.gravity().min(0.01)
        } else {
            self.gravity()
        }
    }

    /// `Player.getFlyingSpeed`: the speed in the air, faster flying.
    fn flying_speed(&self) -> f32 {
        if self.abilities.flying {
            if self.sprinting {
                self.abilities.flying_speed * 2.0
            } else {
                self.abilities.flying_speed
            }
        } else if self.sprinting {
            0.025999999
        } else {
            0.02
        }
    }

    /// `LivingEntity.getJumpBoostPower`.
    fn jump_boost_power(&self) -> f32 {
        match self.effects.get(MobEffect::JumpBoost) {
            Some(e) => 0.1 * (e.amplifier as f32 + 1.0),
            None => 0.0,
        }
    }

    /// `LocalPlayer.onUpdateAbilities`: tells the server whether the player flies.
    fn on_update_abilities(&mut self) {
        self.outbox
            .push(play::player_abilities(self.abilities.flying));
    }

    fn can_fit(&self, env: &Env<'_>, pose: Pose) -> bool {
        env.no_collision_for(
            &pose.bounding_box(self.position).inflate([-EPSILON; 3]),
            Some(self.who()),
        )
    }

    /// `isMovingSlowly`: crouching, or crawling (the swimming pose out of water).
    fn moving_slowly(&self) -> bool {
        self.crouching || self.pose == Pose::Swimming && !self.in_water
    }

    /// `isInWater`.
    pub fn in_water(&self) -> bool {
        self.in_water
    }

    /// `isInLava` (never on the first tick).
    pub fn in_lava(&self) -> bool {
        !self.first_tick && self.fluids.is_in(false)
    }

    /// `isInShallowWater`, with `LocalPlayer.isUnderWater`.
    fn in_shallow_water(&self) -> bool {
        self.in_water && !self.was_underwater
    }

    /// `LocalPlayer.isSprintingPossible` (not riding): not blind (`isMobilityRestricted`),
    /// fed enough, and not in shallow water unless allowed.
    fn sprinting_possible(&self, allowed_in_shallow_water: bool) -> bool {
        !self.effects.has(MobEffect::Blindness)
            && self.enough_food()
            && (allowed_in_shallow_water || !self.in_shallow_water())
    }

    fn eye_y(&self) -> f64 {
        self.position[1] + f64::from(self.pose.eye_height())
    }

    /// `getFluidJumpThreshold`.
    fn fluid_jump_threshold(&self) -> f64 {
        if f64::from(self.pose.eye_height()) < 0.4 {
            0.0
        } else {
            0.4
        }
    }

    fn has_forward_impulse(&self) -> bool {
        self.move_vector.1 > 1.0E-5
    }

    /// One client tick of the player (`LocalPlayer.tick`, only called once the player
    /// has loaded into the level). `keys` are the movement keys held.
    pub fn tick(&mut self, env: &Env<'_>, keys: Keys) {
        // Level.tickNonPassenger: setOldPosAndRot.
        self.old_position = self.position;
        self.old_rot = (self.y_rot, self.x_rot);
        // Player.tick: spectators go through everything and are never on the ground;
        // updateIsUnderwater, from the fluids as last updated.
        self.no_physics = self.spectator;
        if self.spectator {
            self.on_ground = false;
        }
        self.was_underwater = self.fluids.eye_in(true);
        self.base_tick(env);
        // LivingEntity.tick.
        self.updating_using_item();
        self.ai_step(env, keys);
        let clamp = |v: f64| v.clamp(-2.9999999E7, 2.9999999E7);
        self.position[0] = clamp(self.position[0]);
        self.position[2] = clamp(self.position[2]);
        // The attack strength builds up; a new item in the main hand starts it over.
        self.attack_strength_ticker += 1;
        self.item_swap_ticker += 1;
        let main = self.inventory.hand(Hand::Main).clone();
        if !self.last_item_in_main_hand.matches(&main) {
            if !self.last_item_in_main_hand.same_item(&main) {
                self.reset_attack_strength_ticker();
            }
            self.last_item_in_main_hand = main.copy();
        }
        self.cooldowns.tick();
        self.update_pose(env);
    }

    /// `Entity.baseTick`'s movement parts: the fluids, swimming, lava halving the fall.
    fn base_tick(&mut self, env: &Env<'_>) {
        self.update_fluid_interaction(env);
        self.update_swimming(env);
        if self.in_lava() {
            self.fall_distance *= 0.5;
        }
        self.first_tick = false;
        // LivingEntity.baseTick: tickEffects.
        self.effects.tick();
    }

    /// `Entity.updateFluidInteraction`: what the box is in, the fall reset in water,
    /// and the currents (players not flying are pushed by fluids).
    fn update_fluid_interaction(&mut self, env: &Env<'_>) {
        let pushed = self.affected_by_fluids();
        let b = self.bounding_box();
        let eye = (
            mth::floor(self.position[0]),
            self.eye_y(),
            mth::floor(self.position[2]),
        );
        self.fluids.update(env, &b, eye, !pushed);
        let in_water = self.fluids.is_in(true);
        let in_lava = self.fluids.is_in(false);
        if in_water {
            self.fall_distance = 0.0;
        }
        self.in_water = in_water;
        if !pushed {
            return;
        }
        if in_water {
            self.fluids.apply_current(true, &mut self.delta, 0.014);
        }
        if in_lava {
            let scale = if env.level.fast_lava() {
                0.007
            } else {
                0.0023333333333333335
            };
            self.fluids.apply_current(false, &mut self.delta, scale);
        }
    }

    /// `Player.updateSwimming`: never while flying; else `Entity.updateSwimming` keeps
    /// swimming while sprinting in water and starts with the eyes under water (as the
    /// tick began) in a water block.
    fn update_swimming(&mut self, env: &Env<'_>) {
        if self.abilities.flying {
            self.swimming = false;
            return;
        }
        self.swimming = if self.is_swimming() {
            self.sprinting && self.in_water
        } else {
            self.sprinting
                && self.was_underwater
                && env.fluid(self.block_position()).fluid.is_water()
        };
    }

    /// `Player.updatePlayerPose` (sleeping, elytra and riptide poses aside): a
    /// spectator takes the pose it wants.
    fn update_pose(&mut self, env: &Env<'_>) {
        if self.can_fit(env, Pose::Swimming) {
            let desired = if self.is_swimming() {
                Pose::Swimming
            } else if self.keys.shift && !self.abilities.flying {
                Pose::Crouching
            } else {
                Pose::Standing
            };
            self.pose = if self.spectator || self.can_fit(env, desired) {
                desired
            } else if self.can_fit(env, Pose::Crouching) {
                Pose::Crouching
            } else {
                Pose::Swimming
            };
        }
    }

    /// `LocalPlayer.aiStep` (not riding, no auto-jump, no elytra).
    fn ai_step(&mut self, env: &Env<'_>, keys: Keys) {
        if self.sprint_trigger_time > 0 {
            self.sprint_trigger_time -= 1;
        }
        let was_jumping = self.keys.jump;
        let was_shift = self.keys.shift;
        let had_forward = self.has_forward_impulse();
        self.crouching = !self.abilities.flying
            && !self.is_swimming()
            && self.can_fit(env, Pose::Crouching)
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
        if !self.no_physics {
            let w = f64::from(self.pose.size().0) * 0.35;
            let (px, pz) = (self.position[0], self.position[2]);
            for (x, z) in [
                (px - w, pz + w),
                (px - w, pz - w),
                (px + w, pz - w),
                (px + w, pz + w),
            ] {
                self.move_towards_closest_space(env, x, z);
            }
        }
        let slow_using = self.slow_due_to_using_item();
        if was_shift || slow_using || keys.backward {
            self.sprint_trigger_time = 0;
        }
        let flying = self.abilities.flying;
        let can_start_sprinting = !self.sprinting
            && self.has_forward_impulse()
            && self.sprinting_possible(flying)
            && !slow_using
            && (!self.fall_flying() || self.was_underwater)
            && (!self.moving_slowly() || self.was_underwater);
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
            let stop = if self.is_swimming() {
                // shouldStopSwimSprinting.
                !self.sprinting_possible(true)
                    || !self.in_water
                    || !self.has_forward_impulse() && !self.on_ground && !keys.shift
            } else {
                // shouldStopRunSprinting.
                !self.sprinting_possible(self.abilities.flying)
                    || !self.has_forward_impulse()
                    || self.horizontal_collision && !self.minor_horizontal_collision
            };
            if stop {
                self.set_sprinting(false);
            }
        }
        // Creative flight: a spectator always flies; otherwise a second press of jump
        // within 7 ticks of the first toggles it.
        if self.abilities.may_fly {
            if self.game_type == play::SPECTATOR {
                if !self.abilities.flying {
                    self.abilities.flying = true;
                    self.on_update_abilities();
                }
            } else if !was_jumping && keys.jump {
                if self.jump_trigger_time == 0 {
                    self.jump_trigger_time = 7;
                } else if !self.is_swimming() {
                    self.abilities.flying = !self.abilities.flying;
                    if self.abilities.flying && self.on_ground {
                        self.jump_from_ground(env);
                    }
                    self.on_update_abilities();
                    self.jump_trigger_time = 0;
                }
            }
        }
        // goDownInWater.
        if self.in_water && keys.shift && self.affected_by_fluids() {
            self.delta[1] += -0.03999999910593033;
        }
        // Flying up and down.
        if self.abilities.flying {
            let ya = i32::from(keys.jump) - i32::from(keys.shift);
            if ya != 0 {
                self.delta[1] += f64::from(ya as f32 * self.abilities.flying_speed * 3.0);
            }
        }
        // Player.aiStep.
        if self.jump_trigger_time > 0 {
            self.jump_trigger_time -= 1;
        }
        if self.abilities.flying {
            self.fall_distance = 0.0;
        }
        self.living_ai_step(env);
        // Landing ends creative flight.
        if self.on_ground && self.abilities.flying && self.game_type != play::SPECTATOR {
            self.abilities.flying = false;
            self.on_update_abilities();
        }
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
            .block_collisions_for(env.blocks, &area, true, Some(self.who()))
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
        // isImmobile: dead (sleeping isn't ported).
        if self.health <= 0.0 {
            self.jumping = false;
            self.xxa = 0.0;
            self.zza = 0.0;
        }
        if self.jumping && self.affected_by_fluids() {
            let in_lava = self.in_lava();
            let height = self.fluids.height(!in_lava);
            let in_water_with_height = self.in_water && height > 0.0;
            let threshold = self.fluid_jump_threshold();
            if !in_water_with_height || self.on_ground && !(height > threshold) {
                if !in_lava || self.on_ground && self.fluids.height(false) <= threshold {
                    if (self.on_ground || in_water_with_height && height <= threshold)
                        && self.no_jump_delay == 0
                    {
                        self.jump_from_ground(env);
                        self.no_jump_delay = 10;
                    }
                } else {
                    // jumpInLiquid(LAVA).
                    self.delta[1] += 0.03999999910593033;
                }
            } else {
                // jumpInLiquid(WATER).
                self.delta[1] += 0.03999999910593033;
            }
        } else {
            self.no_jump_delay = 0;
        }
        if self.effects.has(MobEffect::SlowFalling) || self.effects.has(MobEffect::Levitation) {
            self.fall_distance = 0.0;
        }
        self.travel(env);
    }

    /// `Player.travel`: swimming follows the look, and flying damps the vertical
    /// motion; `LivingEntity.travel` in between.
    fn travel(&mut self, env: &Env<'_>) {
        if self.is_swimming() {
            // getLookAngle().y.
            let look_y = f64::from(-mth::sin(f64::from(self.x_rot * DEG_TO_RAD_F)));
            let multiplier = if look_y < -0.2 { 0.085 } else { 0.06 };
            let above = [
                mth::floor(self.position[0]),
                mth::floor(self.position[1] + 1.0 - 0.1),
                mth::floor(self.position[2]),
            ];
            if look_y <= 0.0 || self.jumping || !env.fluid(above).is_empty() {
                self.delta[1] += (look_y - self.delta[1]) * multiplier;
            }
        }
        if self.abilities.flying {
            let original_y = self.delta[1];
            self.living_travel(env);
            self.delta[1] = original_y * 0.6;
        } else {
            self.living_travel(env);
        }
    }

    /// `LivingEntity.travel` (elytra flight isn't ported).
    fn living_travel(&mut self, env: &Env<'_>) {
        // shouldTravelInFluid: players never stand on fluids.
        if (self.in_water || self.in_lava()) && self.affected_by_fluids() {
            self.travel_in_fluid(env);
        } else {
            self.travel_in_air(env);
        }
    }

    /// `LivingEntity.travelInFluid`.
    fn travel_in_fluid(&mut self, env: &Env<'_>) {
        let is_falling = self.delta[1] <= 0.0;
        let old_y = self.position[1];
        let gravity = self.effective_gravity();
        if self.in_water {
            self.travel_in_water(env, gravity, is_falling, old_y);
        } else {
            self.travel_in_lava(env, gravity, is_falling, old_y);
        }
    }

    /// `LivingEntity.travelInWater`.
    fn travel_in_water(&mut self, env: &Env<'_>, gravity: f64, is_falling: bool, old_y: f64) {
        let mut slow_down: f32 = if self.sprinting { 0.9 } else { 0.8 };
        let mut speed: f32 = 0.02;
        let mut water_walker = self.attributes.water_movement_efficiency.value() as f32;
        if !self.on_ground {
            water_walker *= 0.5;
        }
        if water_walker > 0.0 {
            slow_down += (0.54600006 - slow_down) * water_walker;
            let own = self.attributes.movement_speed.value() as f32;
            speed += (own - speed) * water_walker;
        }
        if self.effects.has(MobEffect::DolphinsGrace) {
            slow_down = 0.96;
        }
        self.move_relative(speed);
        self.do_move(env, self.delta);
        let mut m = self.delta;
        if self.horizontal_collision && self.on_climbable(env) {
            m[1] = 0.2;
        }
        let s = f64::from(slow_down);
        let m = [m[0] * s, m[1] * 0.800000011920929, m[2] * s];
        self.delta = self.fluid_falling_adjusted(gravity, is_falling, m);
        self.jump_out_of_fluid(env, old_y);
    }

    /// `LivingEntity.travelInLava`.
    fn travel_in_lava(&mut self, env: &Env<'_>, gravity: f64, is_falling: bool, old_y: f64) {
        self.move_relative(0.02);
        self.do_move(env, self.delta);
        let d = self.delta;
        if self.fluids.height(false) <= self.fluid_jump_threshold() {
            let m = [d[0] * 0.5, d[1] * 0.800000011920929, d[2] * 0.5];
            self.delta = self.fluid_falling_adjusted(gravity, is_falling, m);
        } else {
            self.delta = [d[0] * 0.5, d[1] * 0.5, d[2] * 0.5];
        }
        if gravity != 0.0 {
            self.delta[1] += -gravity / 4.0;
        }
        self.jump_out_of_fluid(env, old_y);
    }

    /// `LivingEntity.getFluidFallingAdjustedMovement`.
    fn fluid_falling_adjusted(&self, gravity: f64, is_falling: bool, m: [f64; 3]) -> [f64; 3] {
        if gravity != 0.0 && !self.sprinting {
            let y = if is_falling
                && (m[1] - 0.005).abs() >= 0.003
                && (m[1] - gravity / 16.0).abs() < 0.003
            {
                -0.003
            } else {
                m[1] - gravity / 16.0
            };
            [m[0], y, m[2]]
        } else {
            m
        }
    }

    /// `LivingEntity.jumpOutOfFluid`: against a wall with room above, hop out.
    fn jump_out_of_fluid(&mut self, env: &Env<'_>, old_y: f64) {
        let m = self.delta;
        if self.horizontal_collision
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
    }

    /// `Entity.isFree`: no collision and no fluid in the box moved by `d`.
    fn is_free(&self, env: &Env<'_>, d: [f64; 3]) -> bool {
        let b = self.bounding_box().moved(d);
        env.no_collision_for(&b, Some(self.who())) && !env.contains_any_liquid(&b)
    }

    /// `LocalPlayer.modifyInput`.
    fn modify_input(&self, (x, y): (f32, f32)) -> (f32, f32) {
        if x * x + y * y == 0.0 {
            return (x, y);
        }
        let (mut x, mut y) = (x * 0.98, y * 0.98);
        if self.started_using {
            let f = self.use_item.c.use_effects().speed_multiplier;
            x *= f;
            y *= f;
        }
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
            self.attributes.jump_strength.value() as f32 * 1.0 * self.block_jump_factor(env)
                + self.jump_boost_power();
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
        } else {
            self.flying_speed()
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
            // LocalPlayer.isSuppressingSlidingDownLadder.
            if d[1] < 0.0 && !in_scaffolding && !self.abilities.flying && self.keys.shift {
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
        let movement_y = if let Some(levitation) = self.effects.get(MobEffect::Levitation) {
            movement[1] + (0.05 * f64::from(levitation.amplifier + 1) - movement[1]) * 0.2
        } else if env.has_chunk_at(below) {
            movement[1] - self.effective_gravity()
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
    /// trapdoor above a ladder facing the same way; never while flying or for a
    /// spectator.
    pub(crate) fn on_climbable(&self, env: &Env<'_>) -> bool {
        if self.abilities.flying || self.spectator {
            return false;
        }
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

    /// `Entity.getBlockSpeedFactor` (with `LivingEntity`'s movement efficiency lerp);
    /// none while flying.
    fn block_speed_factor(&self, env: &Env<'_>) -> f32 {
        if self.abilities.flying || self.fall_flying() {
            return 1.0;
        }
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

    /// `Entity.move` for the player moving itself; a spectator just goes there.
    fn do_move(&mut self, env: &Env<'_>, delta: [f64; 3]) {
        if self.no_physics {
            self.position = [
                self.position[0] + delta[0],
                self.position[1] + delta[1],
                self.position[2] + delta[2],
            ];
            self.horizontal_collision = false;
            self.vertical_collision = false;
            self.vertical_collision_below = false;
            self.minor_horizontal_collision = false;
            return;
        }
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
        // LivingEntity.checkFallDamage: out of water the fluids are updated again
        // (currents and all), then Entity.checkFallDamage.
        if !self.in_water {
            self.update_fluid_interaction(env);
        }
        if !self.in_water && movement[1] < 0.0 {
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
            let gravity = self.effective_gravity();
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
        for (p, _) in env
            .level
            .block_collisions_for(env.blocks, area, false, Some(self.who()))
        {
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
        if self.abilities.flying
            || delta[1] > 0.0
            || !self.keys.shift
            || !self.above_ground(env, max_down)
        {
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
        env.no_collision_for(
            &Aabb::new(
                [
                    b.min[0] + EPSILON + dx,
                    b.min[1] - min_height - EPSILON,
                    b.min[2] + EPSILON + dz,
                ],
                [b.max[0] - EPSILON + dx, b.min[1], b.max[2] - EPSILON + dz],
            ),
            Some(self.who()),
        )
    }

    /// The local player as a collision query's entity: the bottom of its box now,
    /// and whether it descends (`isDescending` is `isShiftKeyDown`: sneak held).
    fn who(&self) -> Who {
        Who {
            bottom: self.position[1],
            descending: self.keys.shift,
        }
    }

    /// `Entity.collide`: block collisions, then stepping up.
    fn collide(&self, env: &Env<'_>, movement: [f64; 3]) -> [f64; 3] {
        let b = self.bounding_box();
        let step = self.attributes.step_height.value() as f32;
        // Entities first, found once for the move and the step up (`Shapes.create` of
        // each box).
        let entity_shapes: Vec<Shape> = env
            .entity_colliders(&b.expand_towards(movement).expand_towards([
                0.0,
                f64::from(step),
                0.0,
            ]))
            .map(box_shape)
            .collect();
        let with_entities = |blocks| entities_then(&entity_shapes, blocks);
        let len = movement[0] * movement[0] + movement[1] * movement[1] + movement[2] * movement[2];
        let result = if len == 0.0 {
            movement
        } else {
            let shapes =
                with_entities(env.collisions(&b.expand_towards(movement), Some(self.who())));
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
            let shapes = with_entities(env.collisions(&area, Some(self.who())));
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
        let mut out = std::mem::take(&mut self.outbox);
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
/// `PositionMoveRotation.calculateAbsolute`: the current position, motion and
/// rotation changed by a teleport's, each value absolute or relative as `relatives`
/// says (`Relative` bits); the motion turns with the rotation under `ROTATE_DELTA`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn calculate_absolute(
    position: [f64; 3],
    delta: [f64; 3],
    y_rot: f32,
    x_rot: f32,
    change: [f64; 3],
    change_delta: [f64; 3],
    change_y: f32,
    change_x: f32,
    relatives: u32,
) -> ([f64; 3], [f64; 3], f32, f32) {
    let rel = |bit: u32| relatives & bit != 0;
    let off = |bit: u32, v: f64| if rel(bit) { v } else { 0.0 };
    let new_position = [
        off(X, position[0]) + change[0],
        off(Y, position[1]) + change[1],
        off(Z, position[2]) + change[2],
    ];
    let new_y = (if rel(Y_ROT) { y_rot } else { 0.0 }) + change_y;
    let new_x = ((if rel(X_ROT) { x_rot } else { 0.0 }) + change_x).clamp(-90.0, 90.0);
    let mut moving = delta;
    if rel(ROTATE_DELTA) {
        let dy = y_rot - new_y;
        let dx = x_rot - new_x;
        moving = rotate_x(moving, (f64::from(dx) * DEGREES_TO_RADIANS) as f32);
        moving = rotate_y(moving, (f64::from(dy) * DEGREES_TO_RADIANS) as f32);
    }
    let d = |cur: f64, change: f64, bit: u32| if rel(bit) { cur + change } else { change };
    let new_delta = [
        d(moving[0], change_delta[0], DELTA_X),
        d(moving[1], change_delta[1], DELTA_Y),
        d(moving[2], change_delta[2], DELTA_Z),
    ];
    (new_position, new_delta, new_y, new_x)
}

/// The entity shapes, then the block shapes (`collideBoundingBox`'s order).
fn entities_then<'s>(entities: &'s [Shape], blocks: Vec<Placed<'s>>) -> Vec<Placed<'s>> {
    let mut all: Vec<Placed<'s>> = entities
        .iter()
        .map(|shape| Placed {
            shape,
            offset: None,
            pos: [0.0; 3],
        })
        .collect();
    all.extend(blocks);
    all
}

/// `Shapes.create` of an entity's box: one full cell spanning it.
fn box_shape(b: &Aabb) -> Shape {
    Shape::new(&ruthenium_prepare::exporters::ShapeData {
        coords: [
            vec![b.min[0], b.max[0]],
            vec![b.min[1], b.max[1]],
            vec![b.min[2], b.max[2]],
        ],
        full: vec![true],
    })
}

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

    /// Air, and stone (state 1) where the tests put it, in one chunk at the origin.
    fn world() -> (Level, Blocks, BlockTags) {
        use ruthenium_prepare::exporters::{BlockPhysics, ShapeData, StatePhysics};
        let state = |shape, flags| StatePhysics {
            shape,
            flags,
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            bounce: 0.0,
            fluid: 0,
            sturdy: 0,
            offset: None,
            outline: shape,
            interaction_shape: 0,
            interaction: 0,
            destroy_time: 0.0,
        };
        let blocks = Blocks::new(&BlockPhysics {
            shapes: vec![
                ShapeData {
                    coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
                    full: vec![false],
                },
                ShapeData {
                    coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
                    full: vec![true],
                },
            ],
            states: vec![state(0, 0), state(1, 0)],
        });
        let mut level = Level::new(-64, 384);
        level.add_empty_chunk(0, 0);
        // A stone floor under y = -60.
        for x in 0..16 {
            for z in 0..16 {
                level.set_block(x, -61, z, 1);
            }
        }
        (level, blocks, BlockTags::new())
    }

    fn standing(env: &Env<'_>, game_type: i32) -> LocalPlayer {
        let mut p = LocalPlayer::new();
        p.position = [8.5, -60.0, 8.5];
        p.set_game_type(game_type);
        p.stand(env);
        p
    }

    const JUMP: Keys = Keys {
        forward: false,
        backward: false,
        left: false,
        right: false,
        jump: true,
        shift: false,
        sprint: false,
    };
    const SHIFT: Keys = Keys {
        shift: true,
        jump: false,
        ..JUMP
    };
    const NONE: Keys = Keys {
        jump: false,
        ..JUMP
    };

    #[test]
    fn creative_flight_starts_on_a_double_tap_and_ends_on_landing() {
        let (level, blocks, tags) = world();
        let env = Env {
            level: &level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
        };
        // Survival never flies.
        let mut p = standing(&env, play::SURVIVAL);
        for keys in [JUMP, NONE, JUMP] {
            p.tick(&env, keys);
        }
        assert!(!p.abilities.flying);

        let mut p = standing(&env, play::CREATIVE);
        p.tick(&env, JUMP);
        // The first press jumps as usual and opens the window.
        assert!(!p.abilities.flying && !p.on_ground);
        p.send_changes();
        p.tick(&env, NONE);
        p.tick(&env, JUMP);
        assert!(p.abilities.flying);
        let id = ruthenium_data::v26_3::packets::play::Serverbound::PlayerAbilities.id() as u8;
        assert_eq!(p.send_changes()[0], [id, 2]);
        // Holding jump rises by three times the flying speed a tick, damped.
        let y = p.position[1];
        p.tick(&env, JUMP);
        assert!(p.position[1] > y);
        // Sinking onto the floor ends the flight.
        for _ in 0..60 {
            p.tick(&env, SHIFT);
            if !p.abilities.flying {
                break;
            }
        }
        assert!(p.on_ground && !p.abilities.flying);
        assert_eq!(p.send_changes()[0], [id, 0]);
    }

    #[test]
    fn spectators_fly_through_blocks() {
        let (level, blocks, tags) = world();
        let env = Env {
            level: &level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
        };
        let mut p = standing(&env, play::SPECTATOR);
        p.spectator = true;
        assert!(p.abilities.flying && !p.abilities.may_build);
        for _ in 0..10 {
            p.tick(&env, SHIFT);
        }
        // Down through the floor, standing up (no crouching while flying), never on
        // the ground.
        assert!(p.position[1] < -61.0, "{}", p.position[1]);
        assert_eq!(p.pose, Pose::Standing);
        assert!(!p.on_ground && !p.vertical_collision);
    }

    fn player_types() -> EntityTypes {
        use ruthenium_prepare::exporters::EntityTypeData;
        let fields = [
            ("Entity.DATA_SHARED_FLAGS_ID", 0),
            ("Entity.DATA_AIR_SUPPLY_ID", 1),
            ("Entity.DATA_NO_GRAVITY", 5),
            ("Entity.DATA_POSE", 6),
            ("LivingEntity.DATA_HEALTH_ID", 9),
            ("Player.DATA_SCORE_ID", 16),
        ];
        EntityTypes::new(&[EntityTypeData {
            id: 155,
            key: "minecraft:player".into(),
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            fixed: false,
            update_interval: 2,
            classes: ["Player", "Avatar", "LivingEntity", "Entity"]
                .map(str::to_owned)
                .to_vec(),
            fields: fields.map(|(n, i)| (n.to_owned(), i, 0)).to_vec(),
            baby: None,
        }])
    }

    #[test]
    fn the_servers_entity_data_overrides_the_clients() {
        let types = player_types();
        let mut p = LocalPlayer::new();
        p.set_sprinting(true);
        // The echo of an old "not sprinting" clears the flag but not the speed boost.
        p.set_data(&[(0, DataValue::Byte(flag::ON_FIRE as i8))], &types);
        assert!(!p.sprinting);
        assert_eq!(p.data.flags, flag::ON_FIRE);
        assert_eq!(p.attributes.movement_speed.modifiers.len(), 1);
        p.set_data(
            &[
                (6, DataValue::Pose(Pose::Crouching.id())),
                (5, DataValue::Boolean(true)),
                (1, DataValue::Int(120)),
                (9, DataValue::Float(7.5)),
                (16, DataValue::Int(3)),
            ],
            &types,
        );
        assert_eq!(p.pose, Pose::Crouching);
        assert_eq!(p.bounding_box().max[1], 1.5);
        assert_eq!(p.effective_gravity(), 0.0);
        assert_eq!((p.data.air_supply, p.health, p.data.score), (120, 7.5, 3));
        // An unknown field or a wrong kind changes nothing.
        p.set_data(&[(6, DataValue::Int(3)), (40, DataValue::Int(1))], &types);
        assert_eq!(p.pose, Pose::Crouching);
    }

    #[test]
    fn effects_change_jumps_and_falls() {
        let mut p = LocalPlayer::new();
        assert_eq!(p.jump_boost_power(), 0.0);
        p.update_effect(Effect {
            id: MobEffect::JumpBoost.id() as i32,
            amplifier: 1,
            duration: 100,
            flags: 0,
        });
        assert_eq!(p.jump_boost_power(), 0.2);
        p.update_effect(Effect {
            id: MobEffect::SlowFalling.id() as i32,
            amplifier: 0,
            duration: 100,
            flags: 0,
        });
        // Slow falling caps gravity only while falling.
        p.delta[1] = -0.5;
        assert_eq!(p.effective_gravity(), 0.01);
        p.delta[1] = 0.1;
        assert_eq!(p.effective_gravity(), 0.08);
        assert!(p.sprinting_possible(false));
        p.update_effect(Effect {
            id: MobEffect::Blindness.id() as i32,
            amplifier: 0,
            duration: 100,
            flags: 0,
        });
        assert!(!p.sprinting_possible(true));
        p.remove_effect(MobEffect::Blindness.id() as i32);
        assert!(p.sprinting_possible(false));
    }

    #[test]
    fn abilities_follow_the_game_mode_and_the_server() {
        let mut a = Abilities::default();
        a.set_game_type(play::CREATIVE);
        assert!(a.may_fly && a.instabuild && a.invulnerable && !a.flying && a.may_build);
        a.flying = true;
        a.set_game_type(play::SURVIVAL);
        assert!(!a.may_fly && !a.flying && a.may_build);
        a.set_game_type(play::ADVENTURE);
        assert!(!a.may_build);
        a.set_game_type(play::SPECTATOR);
        assert!(a.flying && a.may_fly && !a.instabuild && !a.may_build);
        a.apply(play::ABILITY_MAY_FLY, 0.1, 0.2);
        assert!(a.may_fly && !a.flying && !a.invulnerable && !a.may_build);
        assert_eq!((a.flying_speed, a.walking_speed), (0.1, 0.2));
    }

    #[test]
    fn a_respawn_keeps_what_the_server_says() {
        let mut p = LocalPlayer::new();
        p.entity_id = 7;
        p.set_game_type(play::CREATIVE);
        p.abilities.flying = true;
        p.set_sprinting(true);
        p.delta = [0.1, 0.2, 0.3];
        p.y_rot = 45.0;
        p.health = 6.0;
        p.attributes.gravity.base = 0.05;
        p.update_effect(Effect {
            id: MobEffect::Speed.id() as i32,
            amplifier: 0,
            duration: 100,
            flags: 0,
        });
        let fresh = p.respawned(0);
        assert_eq!(fresh.entity_id, 7);
        assert!(!fresh.sprinting && !fresh.was_sprinting && !fresh.abilities.flying);
        assert!(fresh.abilities.may_fly);
        assert_eq!(
            (fresh.delta, fresh.y_rot, fresh.health),
            ([0.0; 3], -180.0, 20.0)
        );
        assert_eq!(fresh.attributes.gravity.base, 0.05);
        assert!(fresh.attributes.movement_speed.modifiers.is_empty());
        assert!(fresh.effects.is_empty());
        let kept = p.respawned(KEEP_ENTITY_DATA | KEEP_ATTRIBUTES);
        assert!(kept.sprinting && kept.was_sprinting);
        assert_eq!((kept.delta, kept.y_rot, kept.health), (p.delta, 45.0, 6.0));
        assert_eq!(kept.attributes, p.attributes);
        assert!(kept.effects.is_empty());
    }

    #[test]
    fn poses_by_id_and_name() {
        assert_eq!(Pose::from_id(5), Pose::Crouching);
        assert_eq!(Pose::from_id(99), Pose::Standing);
        assert_eq!(Pose::from_name("SPIN_ATTACK"), Some(Pose::SpinAttack));
        assert_eq!(Pose::Dying.size(), (0.2, 0.2));
        assert_eq!(Pose::FallFlying.eye_height(), 0.4);
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

    #[test]
    fn a_charged_sprinting_hit_slows_the_attacker() {
        let mut p = LocalPlayer::new();
        p.set_sprinting(true);
        p.delta = [0.25, -0.08, -0.5];
        // A fist charges in 5 ticks; more than 0.9 (half a tick added) needs all 5.
        assert_eq!(p.attack_strength_delay(), 5.0);
        p.attack_strength_ticker = 4;
        p.attack(true);
        assert!(p.sprinting);
        assert_eq!(p.delta, [0.25, -0.08, -0.5]);
        p.attack_strength_ticker = 5;
        // A mob's `hurtClient` refuses the hit: nothing changes on the client.
        p.attack(false);
        assert!(p.sprinting);
        p.attack(true);
        assert!(!p.sprinting);
        assert!(p.attributes.movement_speed.modifiers.is_empty());
        assert_eq!(p.delta, [0.25 * 0.6, -0.08, -0.5 * 0.6]);
        // Not sprinting any more: the next hit leaves the motion alone.
        let before = p.delta;
        p.attack(true);
        assert_eq!(p.delta, before);
        p.reset_attack_strength_ticker();
        assert_eq!(p.attack_strength_scale(0.0), 0.0);
        assert_eq!(p.attack_strength_scale(2.5), 0.5);
    }
}
