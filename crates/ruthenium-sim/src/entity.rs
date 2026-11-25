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
//! Riding: every passenger ticks after its vehicle (`tickPassenger`), what rides the
//! local player after it, and sits where its vehicle seats it (`positionRider`: the
//! vehicle's passenger point less its own vehicle point; a boat turns what it carries).
//! The server never sends a passenger's position, so this is the only place it gets
//! one. A vehicle the local player controls moves on the client ([`crate::vehicle`])
//! and sends where it is with the player's changes.
//!
//! Not yet: sizes that depend on more than the pose, youth, a slime's size and
//! the scale attribute (a peeking shulker, a puffed pufferfish, a phantom's size) are
//! the type's default. A sitting camel has its sitting size.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use ruthenium_data::v26_3::registries::Item;
use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::EntityTypeData;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::Property;
use ruthenium_protocol::packets::play::{self, DataValue, PositionPath, VecDelta};
use ruthenium_protocol::profile::ResolvableProfile;

use crate::avatar::{Avatar, AvatarFields, AvatarRecord, Chest, SkinSource, Swing, TickInput};
use crate::item::{Items, Stack};
use crate::player::{
    Attribute, DEG_TO_RAD_F, Env, LocalPlayer, Modifier, Riding, VehicleMove, rotate_y,
};
use crate::scoreboard::{NameRecord, NameText, Scoreboard, team_name};
use crate::shape::Aabb;
use crate::vehicle::{Body, Controller, RideKind};

/// `Pose` ordinals.
pub mod pose {
    pub const STANDING: i32 = 0;
    pub const FALL_FLYING: i32 = 1;
    pub const SLEEPING: i32 = 2;
    pub const SWIMMING: i32 = 3;
    pub const SPIN_ATTACK: i32 = 4;
    pub const CROUCHING: i32 = 5;
    pub const DYING: i32 = 7;
    pub const SITTING: i32 = 10;
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

/// Whether the crosshair can pick an entity (`isPickable`, the overrides of its class
/// chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pickable {
    /// `Entity`'s default, and the ender dragon (only its parts are picked).
    No,
    /// While not removed: living entities, boats, minecarts, primed TNT, falling
    /// blocks.
    NotRemoved,
    /// Players: not removed and not spectators.
    Player,
    /// Armor stands: not removed and not markers.
    ArmorStand,
    /// Item frames, paintings and leash knots (`BlockAttachedEntity`), interactions,
    /// end crystals, shulker bullets.
    Always,
    /// Other projectiles: in `#minecraft:redirectable_projectile`.
    Projectile { redirectable: bool },
    /// Arrows and tridents: also not stuck in the ground.
    Arrow { redirectable: bool },
}

/// `#minecraft:redirectable_projectile` as vanilla's data pack has it. Servers can
/// change entity tags, but the client reads only this one for picking, so it is not
/// taken from the tags packet.
const REDIRECTABLE_PROJECTILES: [&str; 3] = [
    "minecraft:fireball",
    "minecraft:wind_charge",
    "minecraft:breeze_wind_charge",
];

impl Pickable {
    fn new(d: &EntityTypeData) -> Pickable {
        let redirectable = REDIRECTABLE_PROJECTILES.contains(&d.key.as_str());
        // Most derived override first.
        if d.is("EnderDragon") {
            Pickable::No
        } else if d.is("ArmorStand") {
            Pickable::ArmorStand
        } else if d.key == "minecraft:player" {
            Pickable::Player
        } else if d.is("LivingEntity")
            || d.is("AbstractBoat")
            || d.is("AbstractMinecart")
            || d.is("PrimedTnt")
            || d.is("FallingBlockEntity")
        {
            Pickable::NotRemoved
        } else if d.is("BlockAttachedEntity")
            || d.is("Interaction")
            || d.is("EndCrystal")
            || d.is("ShulkerBullet")
        {
            Pickable::Always
        } else if d.is("AbstractArrow") {
            Pickable::Arrow { redirectable }
        } else if d.is("Projectile") {
            Pickable::Projectile { redirectable }
        } else {
            Pickable::No
        }
    }
}

/// What the client needs of an entity type.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub key: String,
    pub(crate) size: Size,
    pub(crate) living: bool,
    /// Players and mannequins (`Avatar`): sized by pose.
    avatar: bool,
    player: bool,
    boat: bool,
    /// A sulfur cube.
    sulfur_cube: bool,
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
    pub pickable: Pickable,
    /// A player's attack on it runs `causeExtraKnockback` on the client: it can be
    /// attacked and its `hurtClient` takes the hit. Other players (`RemotePlayer`),
    /// vehicles, shulker bullets and end crystals (whose invulnerability the client
    /// doesn't know). Not living mobs (`hurtClient` is false), nor items and orbs
    /// (`isAttackable` is false) or block-attached entities (`skipAttackInteraction`
    /// is true on the client).
    pub hurt_by_attack: bool,
    /// A `Projectile`: picked with a radius of 1 (`getPickRadius`).
    projectile: bool,
    /// `ArmorStand.DATA_CLIENT_FLAGS`.
    armor_stand_flags_field: Option<u8>,
    /// `AbstractArrow.IN_GROUND`.
    in_ground_field: Option<u8>,
    /// `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`.
    attached_to_field: Option<u8>,
    /// `blocksBuilding`: whether it stops blocks being placed where it is.
    pub blocks_building: BlocksBuilding,
    /// The default size's eye height.
    pub(crate) eye_height: f32,
    /// Where its passengers sit (the n-th at the n-th point, the last for more),
    /// unrotated.
    pub(crate) passenger_points: Vec<[f64; 3]>,
    /// Where it sits on what it rides (the first of its vehicle points), unrotated.
    vehicle_point: [f64; 3],
    /// `Spider.getVehicleAttachmentPoint` (and the cave spider's): on a vehicle no
    /// wider than itself it sits this high, times its scale.
    spider_seat: Option<f64>,
    /// A `Mob` and an `Animal`: how a boat turns it (`clampRotation`).
    mob: bool,
    animal: bool,
    /// `#can_turn_in_boats` (vanilla's list: breezes).
    turns_in_boats: bool,
    /// Villagers and wandering traders sit at a minecart's origin
    /// (`AbstractMinecart.LOWERED_PASSENGER_ATTACHMENT`).
    lowered_in_minecarts: bool,
    /// The kind of vehicle the client moves.
    pub(crate) ride: RideKind,
    /// `AbstractHorse.DATA_ID_FLAGS` and `HappyGhast.STAYS_STILL`.
    horse_flags_field: Option<u8>,
    stays_still_field: Option<u8>,
    /// `#can_float_while_ridden` (vanilla's list).
    pub(crate) float_while_ridden: bool,
    /// The type's default attributes, which the client uses until the server sends
    /// them (it sends only those its own copy has used).
    default_attributes: Vec<(String, f64)>,
    /// `Mob.DATA_MOB_FLAGS_ID`.
    mob_flags_field: Option<u8>,
    /// `AbstractHorse.addPassenger`: a new rider is turned to face as it does.
    snaps_rider: bool,
    /// When a saddle in its saddle slot counts (`isSaddled`: `canUseSlot` and the
    /// saddle's allowed entities).
    pub(crate) saddle_rule: SaddleRule,
    /// `Camel.DASH` and `AbstractNautilus.DASH`.
    dash_field: Option<u8>,
    /// `Camel.LAST_POSE_CHANGE_TICK`.
    pose_tick_field: Option<u8>,
    /// `DATA_BOOST_TIME` of pigs and striders (`ItemBasedSteering`).
    boost_field: Option<u8>,
    /// `Strider.DATA_SUFFOCATING`.
    suffocating_field: Option<u8>,
    /// `TamableAnimal.DATA_FLAGS_ID`.
    tame_flags_field: Option<u8>,
    /// An avatar's own fields.
    avatar_fields: AvatarFields,
    /// `Entity.DATA_CUSTOM_NAME` and `DATA_CUSTOM_NAME_VISIBLE`, and a mannequin's
    /// `DATA_DESCRIPTION`.
    custom_name_field: Option<u8>,
    custom_name_visible_field: Option<u8>,
    description_field: Option<u8>,
}

/// When a saddle counts (`canUseSlot(SADDLE)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaddleRule {
    /// Llamas: a saddle is never for them.
    Never,
    /// Skeleton and zombie horses.
    Always,
    /// Alive and grown: camels, pigs, striders.
    Grown,
    /// Alive, grown and tame: horses, donkeys, mules, nautiluses.
    GrownTamed,
}

/// `#minecraft:can_float_while_ridden` as vanilla's data pack has it.
const FLOAT_WHILE_RIDDEN: [&str; 6] = [
    "minecraft:horse",
    "minecraft:zombie_horse",
    "minecraft:mule",
    "minecraft:donkey",
    "minecraft:camel",
    "minecraft:camel_husk",
];

/// `Entity.blocksBuilding` by type: living entities (players included), end crystals,
/// falling blocks, primed TNT, minecarts and boats do; armor stands unless they are
/// markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlocksBuilding {
    No,
    Yes,
    UnlessMarker,
}

/// The ids of the player's synched-data fields the client reads (from the export, so
/// a field the export lacks is never matched).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerFields {
    pub shared_flags: Option<u8>,
    pub air_supply: Option<u8>,
    pub no_gravity: Option<u8>,
    pub pose: Option<u8>,
    pub ticks_frozen: Option<u8>,
    pub living_flags: Option<u8>,
    pub health: Option<u8>,
    pub absorption: Option<u8>,
    pub score: Option<u8>,
    pub sleeping_pos: Option<u8>,
}

impl PlayerFields {
    fn new(d: &EntityTypeData) -> PlayerFields {
        PlayerFields {
            shared_flags: d.field("Entity.DATA_SHARED_FLAGS_ID"),
            air_supply: d.field("Entity.DATA_AIR_SUPPLY_ID"),
            no_gravity: d.field("Entity.DATA_NO_GRAVITY"),
            pose: d.field("Entity.DATA_POSE"),
            ticks_frozen: d.field("Entity.DATA_TICKS_FROZEN"),
            living_flags: d.field("LivingEntity.DATA_LIVING_ENTITY_FLAGS"),
            health: d.field("LivingEntity.DATA_HEALTH_ID"),
            absorption: d.field("Player.DATA_PLAYER_ABSORPTION_ID"),
            score: d.field("Player.DATA_SCORE_ID"),
            sleeping_pos: d.field("LivingEntity.SLEEPING_POS_ID"),
        }
    }
}

/// Every entity type, by network id.
#[derive(Debug, Clone, Default)]
pub struct EntityTypes {
    types: Vec<Option<TypeInfo>>,
    /// `Entity.DATA_POSE`.
    pose_field: Option<u8>,
    /// `LivingEntity.DATA_HEALTH_ID`.
    health_field: Option<u8>,
    player: PlayerFields,
}

impl EntityTypes {
    pub fn new(data: &[EntityTypeData]) -> EntityTypes {
        let mut types = Vec::new();
        let mut pose_field = None;
        let mut health_field = None;
        let mut player = PlayerFields::default();
        for d in data {
            if d.key == "minecraft:player" {
                player = PlayerFields::new(d);
            }
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
                sulfur_cube: d.is("SulfurCube"),
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
                pickable: Pickable::new(d),
                hurt_by_attack: d.key == "minecraft:player"
                    || d.is("VehicleEntity")
                    || d.is("ShulkerBullet")
                    || d.is("EndCrystal"),
                projectile: d.is("Projectile"),
                armor_stand_flags_field: d.field("ArmorStand.DATA_CLIENT_FLAGS"),
                in_ground_field: d.field("AbstractArrow.IN_GROUND"),
                attached_to_field: d.field("FireworkRocketEntity.DATA_ATTACHED_TO_TARGET"),
                blocks_building: if d.is("ArmorStand") {
                    BlocksBuilding::UnlessMarker
                } else if living
                    || d.is("EndCrystal")
                    || d.is("FallingBlockEntity")
                    || d.is("PrimedTnt")
                    || d.is("AbstractMinecart")
                    || d.is("AbstractBoat")
                {
                    BlocksBuilding::Yes
                } else {
                    BlocksBuilding::No
                },
                eye_height: d.eye_height,
                passenger_points: if d.passenger_points.is_empty() {
                    vec![[0.0, f64::from(d.height), 0.0]]
                } else {
                    d.passenger_points.clone()
                },
                vehicle_point: d.vehicle_points.first().copied().unwrap_or_default(),
                spider_seat: if d.is("CaveSpider") {
                    Some(0.21875)
                } else if d.is("Spider") {
                    Some(0.3125)
                } else {
                    None
                },
                mob: d.is("Mob"),
                animal: d.is("Animal"),
                turns_in_boats: d.key == "minecraft:breeze",
                lowered_in_minecarts: d.is("Villager") || d.is("WanderingTrader"),
                ride: if d.is("AbstractBoat") {
                    let raft = d.is("Raft") || d.is("ChestRaft");
                    let chest = d.is("AbstractChestBoat");
                    RideKind::Boat {
                        ride_height: if raft {
                            d.height * 0.8888889
                        } else {
                            d.height / 3.0
                        },
                        x_offset: if chest { 0.15 } else { 0.0 },
                        max_passengers: if chest { 1 } else { 2 },
                    }
                } else if d.is("Camel") {
                    RideKind::Camel
                } else if d.is("AbstractHorse") {
                    // Llamas take no saddle, so no one steers them; they don't rear.
                    RideKind::Horse {
                        rearing: !d.is("Llama"),
                    }
                } else if d.is("HappyGhast") {
                    RideKind::HappyGhast
                } else if d.is("Pig") {
                    RideKind::Pig
                } else if d.is("Strider") {
                    RideKind::Strider
                } else if d.is("AbstractNautilus") {
                    RideKind::Nautilus
                } else {
                    RideKind::None
                },
                horse_flags_field: d.field("AbstractHorse.DATA_ID_FLAGS"),
                stays_still_field: d.field("HappyGhast.STAYS_STILL"),
                float_while_ridden: FLOAT_WHILE_RIDDEN.contains(&d.key.as_str()),
                default_attributes: d.attributes.clone(),
                mob_flags_field: d.field("Mob.DATA_MOB_FLAGS_ID"),
                snaps_rider: d.is("AbstractHorse"),
                saddle_rule: if d.is("Llama") {
                    SaddleRule::Never
                } else if d.is("SkeletonHorse") || d.is("ZombieHorse") {
                    SaddleRule::Always
                } else if d.is("AbstractHorse") && !d.is("Camel") || d.is("AbstractNautilus") {
                    SaddleRule::GrownTamed
                } else {
                    SaddleRule::Grown
                },
                dash_field: d
                    .field("Camel.DASH")
                    .or_else(|| d.field("AbstractNautilus.DASH")),
                pose_tick_field: d.field("Camel.LAST_POSE_CHANGE_TICK"),
                boost_field: d
                    .field("Pig.DATA_BOOST_TIME")
                    .or_else(|| d.field("Strider.DATA_BOOST_TIME")),
                suffocating_field: d.field("Strider.DATA_SUFFOCATING"),
                avatar_fields: AvatarFields {
                    shared_flags: d.field("Entity.DATA_SHARED_FLAGS_ID"),
                    ticks_frozen: d.field("Entity.DATA_TICKS_FROZEN"),
                    living_flags: d.field("LivingEntity.DATA_LIVING_ENTITY_FLAGS"),
                    sleeping_pos: d.field("LivingEntity.SLEEPING_POS_ID"),
                    main_hand: d.field("Avatar.DATA_PLAYER_MAIN_HAND"),
                    customisation: d.field("Avatar.DATA_PLAYER_MODE_CUSTOMISATION"),
                    profile: d.field("Mannequin.DATA_PROFILE"),
                },
                tame_flags_field: d.field("TamableAnimal.DATA_FLAGS_ID"),
                custom_name_field: d.field("Entity.DATA_CUSTOM_NAME"),
                custom_name_visible_field: d.field("Entity.DATA_CUSTOM_NAME_VISIBLE"),
                description_field: d.field("Mannequin.DATA_DESCRIPTION"),
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
            player,
        }
    }

    /// The local player's synched-data fields.
    pub fn player_fields(&self) -> &PlayerFields {
        &self.player
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
    /// The scale changed since its tick last refreshed the size (`refreshDirtyAttributes`
    /// at the end of `LivingEntity.tick`).
    scale_dirty: bool,
    /// `dimensions`: refreshed as the pose, youth or a slime's size change, and as its
    /// tick takes in a new scale.
    pub size: Size,
    /// The scale `size` was refreshed with (the cached dimensions' attachment points).
    size_scale: f32,
    pub bounding_box: Aabb,
    pub vehicle: Option<i32>,
    pub passengers: Vec<i32>,
    pub removed: bool,
    pub tick_count: i32,
    /// An armor stand's marker flag (`CLIENT_FLAG_MARKER`).
    pub marker: bool,
    /// An arrow stuck in a block (`IN_GROUND`).
    pub in_ground: bool,
    /// A firework rocket's entity, the one it carries (`DATA_ATTACHED_TO_TARGET`).
    pub attached_to: Option<i32>,
    /// `VecDeltaCodec`'s base: the last position the server sent.
    codec_base: [f64; 3],
    interpolation: Interpolation,
    /// A remote player that is a spectator goes through everything (`noPhysics`).
    no_physics: bool,
    /// The name scoreboard teams know it by: a player's name, else its UUID.
    scoreboard_name: String,
    section: i64,
    pub(crate) info: TypeInfo,
    /// The physics of a vehicle the client can move.
    pub(crate) body: Option<Box<Body>>,
    /// `isSaddled` and `isWearingBodyArmor` (a sulfur cube's `hasBodyItem`): what its
    /// saddle and body slots hold.
    pub saddled: bool,
    pub body_armor: bool,
    /// `AbstractHorse`'s flags (tame, eating, standing...).
    pub(crate) horse_flags: u8,
    /// `HappyGhast.staysStill`.
    pub(crate) stays_still: bool,
    /// `Mob.isNoAi`: its own AI is off (it doesn't rear on the client).
    pub(crate) no_ai: bool,
    /// A camel's or nautilus's `DASH`.
    pub(crate) dashing: bool,
    /// A camel's `LAST_POSE_CHANGE_TICK`: the game time it last stood up, negated
    /// when it sat down.
    pub(crate) last_pose_change_tick: i64,
    /// A pig's or strider's `DATA_BOOST_TIME`: how long a boost lasts.
    pub(crate) boost_time_total: i32,
    /// A strider's `DATA_SUFFOCATING` (out of the warm).
    pub(crate) suffocating: bool,
    /// `TamableAnimal.isTame`.
    pub(crate) tame: bool,
    /// The sitting bit in the tameable animal flags.
    pub(crate) sitting: bool,
    /// How a player or mannequin holds itself.
    pub(crate) avatar: Option<Box<Avatar>>,
    /// What an avatar is called, apart from its profile's name.
    naming: Option<Box<Naming>>,
}

/// An avatar's names and how far they are seen from.
#[derive(Debug, Clone)]
struct Naming {
    /// `getCustomName` and `isCustomNameVisible`.
    custom_name: Option<Tag>,
    custom_name_visible: bool,
    /// A mannequin's `getDescription`.
    description: Option<Tag>,
    /// `Attributes.NAME_TAG_DISTANCE` and `BELOW_NAME_DISTANCE`.
    distance: f32,
    below_distance: f32,
    /// Counts the changes to the texts.
    rev: u64,
    /// The texts as last put together, and what they were put together from: the teams',
    /// the scoreboard's and the own changes.
    shown: Option<([u64; 3], Arc<NameText>)>,
}

/// `Mannequin.DEFAULT_DESCRIPTION`.
const MANNEQUIN_LABEL: &str = "entity.minecraft.mannequin.label";

fn translatable(key: &str) -> Tag {
    let mut c = ruthenium_protocol::nbt::Compound::new();
    c.put("translate", Tag::String(key.to_owned()));
    Tag::Compound(c)
}

const ATTRIBUTE_SCALE: &str = "minecraft:scale";
const ATTRIBUTE_NAME_TAG_DISTANCE: &str = "minecraft:name_tag_distance";
const ATTRIBUTE_BELOW_NAME_DISTANCE: &str = "minecraft:below_name_distance";

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
        player: Option<&PlayerInfo>,
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
            scale_dirty: false,
            size: info.size,
            size_scale: 1.0,
            bounding_box: info.size.bounding_box([0.0; 3]),
            vehicle: None,
            passengers: Vec::new(),
            removed: false,
            tick_count: 0,
            marker: false,
            in_ground: false,
            attached_to: None,
            codec_base: [0.0; 3],
            interpolation,
            no_physics: info.player,
            scoreboard_name: player.map_or_else(|| uuid_string(p.uuid), |i| i.name.clone()),
            section: 0,
            info: info.clone(),
            body: (info.ride != RideKind::None).then(|| Box::new(Body::new(info.ride))),
            saddled: false,
            body_armor: false,
            horse_flags: 0,
            stays_still: false,
            no_ai: false,
            dashing: false,
            last_pose_change_tick: 0,
            boost_time_total: 0,
            suffocating: false,
            tame: false,
            sitting: false,
            avatar: None,
            naming: info.avatar.then(|| {
                Box::new(Naming {
                    custom_name: None,
                    custom_name_visible: false,
                    description: (!info.player).then(|| translatable(MANNEQUIN_LABEL)),
                    distance: 64.0,
                    below_distance: 10.0,
                    rev: 0,
                    shown: None,
                })
            }),
        };
        if let Some(b) = e.body.as_deref_mut() {
            for (name, base) in &info.default_attributes {
                if let Some(a) = b.attributes.get_mut(name) {
                    a.base = *base;
                }
            }
        }
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
        e.avatar = info.avatar.then(|| {
            let skin = match player {
                Some(i) => SkinSource::Player {
                    id: p.uuid,
                    textures: i.textures.clone(),
                },
                None => SkinSource::Profile(ResolvableProfile::default()),
            };
            let head = mth::unpack_degrees(p.y_head_rot);
            let rot = (e.y_rot, e.x_rot);
            Box::new(Avatar::new(info.player, skin, rot, head, p.movement))
        });
        e
    }

    pub fn info(&self) -> &TypeInfo {
        &self.info
    }

    /// `isSpectator`: a player by its player-list entry
    /// (`AbstractClientPlayer.gameMode`).
    fn is_spectator(&self, players: &PlayerInfos) -> bool {
        self.info.player
            && players
                .get(self.uuid)
                .is_some_and(|i| i.game_mode == play::SPECTATOR)
    }

    /// `isPickable` (`EntitySelector.CAN_BE_PICKED`): a player is a spectator by its
    /// player-list entry (`AbstractClientPlayer.gameMode`).
    pub fn is_pickable(&self, players: &PlayerInfos) -> bool {
        match self.info.pickable {
            Pickable::No => false,
            Pickable::NotRemoved => !self.removed,
            Pickable::Player => !self.removed && !self.is_spectator(players),
            Pickable::ArmorStand => !self.removed && !self.marker,
            Pickable::Always => true,
            Pickable::Projectile { redirectable } => redirectable,
            Pickable::Arrow { redirectable } => redirectable && !self.in_ground,
        }
    }

    /// `getPickRadius` of a pickable entity: 1 for projectiles, else 0.
    pub fn pick_radius(&self) -> f32 {
        if self.info.projectile { 1.0 } else { 0.0 }
    }

    /// `canBePickedFromInside`: not a sulfur cube that has swallowed a block
    /// (`hasBodyItem`).
    pub fn can_be_picked_from_inside(&self) -> bool {
        !(self.info.sulfur_cube && self.body_armor)
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
        } else if t.ride == RideKind::Camel && self.pose == pose::SITTING {
            if self.baby {
                Size::scalable(0.95, 0.425)
            } else {
                Size::scalable(t.size.width, t.size.height - 1.43)
            }
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
    pub(crate) fn refresh_dimensions(&mut self) {
        self.size = self.dimensions();
        self.size_scale = self.scale.value() as f32;
        self.set_pos(self.position);
    }

    /// `getVehicleAttachmentPoint` on a vehicle `vehicle_width` wide, turned with it.
    fn vehicle_attachment(&self, vehicle_width: f32) -> [f64; 3] {
        if let Some(y) = self.info.spider_seat
            && vehicle_width <= self.size.width
        {
            return [0.0, y * f64::from(self.scale.value() as f32), 0.0];
        }
        let t = &self.info;
        // `EntityDimensions.scale`: the points scaled with the size, unless it's fixed.
        let scaled = |p: [f64; 3], size: Size, factor: f32| {
            if size.fixed || factor == 1.0 {
                p
            } else {
                p.map(|c| c * f64::from(factor))
            }
        };
        // `getAttachments`: the cached dimensions', at the scale they were refreshed with.
        let scale = self.size_scale;
        let point = if !t.living {
            t.vehicle_point
        } else if self.pose == pose::SLEEPING {
            [0.0; 3]
        } else if t.avatar {
            // `Avatar.POSES`: 0.6 up where it has the standing dimensions' attachment
            // (crouching too), else at the feet.
            let point = match self.pose {
                pose::FALL_FLYING | pose::SWIMMING | pose::SPIN_ATTACK | pose::DYING => [0.0; 3],
                _ => [0.0, 0.6, 0.0],
            };
            scaled(point, avatar_size(self.pose), scale)
        } else if t.cube_size_field.is_some() {
            let size = t.size.scale(self.cube_size as f32);
            scaled(
                scaled(t.vehicle_point, t.size, self.cube_size as f32),
                size,
                scale,
            )
        } else if self.baby && t.baby_size.is_some() {
            // The baby size's own points aren't exported: the default's, scaled to it.
            let ratio = f64::from(self.size.height / t.size.height);
            t.vehicle_point.map(|c| c * ratio)
        } else if self.baby {
            let size = t.size.scale(t.age_scale);
            scaled(scaled(t.vehicle_point, t.size, t.age_scale), size, scale)
        } else {
            scaled(t.vehicle_point, t.size, scale)
        };
        rotate_y(point, -self.y_rot * DEG_TO_RAD_F)
    }

    /// `setPos`: the position and the box.
    pub(crate) fn set_pos(&mut self, p: [f64; 3]) {
        self.position = p;
        self.bounding_box = self.size.bounding_box(p);
    }

    pub(crate) fn set_y_rot(&mut self, y: f32) {
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
    pub(crate) fn set_rot(&mut self, y: f32, x: f32) {
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
        let rot = (self.y_rot, self.x_rot);
        if let Some(a) = &mut self.avatar {
            a.set_old_rot(rot.0, rot.1);
        }
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
        if let Some(a) = &mut self.avatar
            && a.set_data(&t.avatar_fields, field, value)
        {
            return;
        }
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
        } else if Some(field) == t.armor_stand_flags_field
            && let DataValue::Byte(b) = value
        {
            // `ArmorStand.CLIENT_FLAG_MARKER`.
            self.marker = b & 16 != 0;
        } else if Some(field) == t.in_ground_field
            && let DataValue::Boolean(b) = value
        {
            self.in_ground = *b;
        } else if Some(field) == t.attached_to_field
            && let DataValue::OptionalUnsignedInt(id) = value
        {
            self.attached_to = *id;
        } else if Some(field) == t.horse_flags_field
            && let DataValue::Byte(b) = value
        {
            self.horse_flags = *b as u8;
        } else if Some(field) == t.stays_still_field
            && let DataValue::Boolean(b) = value
        {
            self.stays_still = *b;
        } else if Some(field) == t.mob_flags_field
            && let DataValue::Byte(b) = value
        {
            self.no_ai = b & 1 != 0;
        } else if Some(field) == t.dash_field
            && let DataValue::Boolean(b) = value
        {
            self.dashing = *b;
            self.dash_synced();
        } else if Some(field) == t.pose_tick_field
            && let DataValue::Long(n) = value
        {
            self.last_pose_change_tick = *n;
        } else if Some(field) == t.boost_field
            && let DataValue::Int(n) = value
        {
            self.boost_time_total = *n;
            self.boost_synced();
        } else if Some(field) == t.suffocating_field
            && let DataValue::Boolean(b) = value
        {
            self.suffocating = *b;
        } else if Some(field) == t.tame_flags_field
            && let DataValue::Byte(b) = value
        {
            self.tame = b & 4 != 0;
            self.sitting = b & 1 != 0;
        } else if let Some(n) = &mut self.naming {
            if Some(field) == t.custom_name_field
                && let DataValue::OptionalComponent(name) = value
            {
                n.custom_name = name.clone();
                n.rev += 1;
            } else if Some(field) == t.custom_name_visible_field
                && let DataValue::Boolean(b) = value
            {
                n.custom_name_visible = *b;
            } else if Some(field) == t.description_field
                && let DataValue::OptionalComponent(text) = value
            {
                n.description = text.clone();
                n.rev += 1;
            }
        }
    }
}

/// Whether there is water in a box (`isInWater`, as far as its drawing shows it).
fn touches_water(env: &Env<'_>, b: &Aabb) -> bool {
    let b = b.inflate([-0.001; 3]);
    let range = |i: usize| mth::floor(b.min[i])..mth::ceil(b.max[i]);
    range(0).any(|x| {
        range(1).any(|y| {
            range(2).any(|z| {
                let p = [x, y, z];
                let f = env.fluid(p);
                f.fluid.is_water() && f64::from(y) + f64::from(env.fluid_height(p, f)) > b.min[1]
            })
        })
    })
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

/// A scoreboard team, as far as pushing and names care.
#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub name: String,
    pub collision_rule: i32,
    /// Its members see those of it who are invisible.
    pub see_invisible: bool,
    /// What stands before and after its members' names, as components.
    pub prefix: Tag,
    pub suffix: Tag,
    /// The `ChatFormatting` ordinal its members' names are written in.
    pub colour: Option<i32>,
    /// Who sees its members' names (`visibility`).
    pub name_visibility: i32,
}

/// `Team.Visibility` ids.
pub mod visibility {
    pub const ALWAYS: i32 = 0;
    pub const NEVER: i32 = 1;
    pub const HIDE_FOR_OTHER_TEAMS: i32 = 2;
    pub const HIDE_FOR_OWN_TEAM: i32 = 3;
}

/// The scoreboard's teams and who is on which (`Scoreboard`'s team part).
#[derive(Debug, Clone, Default)]
pub struct Teams {
    teams: HashMap<String, Team>,
    members: HashMap<String, String>,
    rev: u64,
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
        self.rev += 1;
        if method == 0 {
            // `addPlayerTeam` (an existing name is refused by vanilla's scoreboard,
            // which keeps the old team).
            self.teams.entry(name.to_owned()).or_insert(Team {
                name: name.to_owned(),
                collision_rule: rule::ALWAYS,
                see_invisible: true,
                prefix: Tag::String(String::new()),
                suffix: Tag::String(String::new()),
                colour: None,
                name_visibility: visibility::ALWAYS,
            });
        } else if !self.teams.contains_key(name) {
            return;
        }
        if let Some(p) = parameters
            && let Some(t) = self.teams.get_mut(name)
        {
            // Unknown ids read as the first rule.
            t.see_invisible = p.options & 2 != 0;
            t.prefix = p.prefix.clone();
            t.suffix = p.suffix.clone();
            t.colour = p.color;
            t.name_visibility = if (0..=3).contains(&p.name_tag_visibility) {
                p.name_tag_visibility
            } else {
                visibility::ALWAYS
            };
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

    /// Counts the packets taken in: what was made of an older state is out of date.
    pub fn rev(&self) -> u64 {
        self.rev
    }
}

/// What the client knows of a player from the player list (`PlayerInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub name: String,
    pub game_mode: i32,
    /// Its profile's skin.
    pub textures: Option<Property>,
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
                        textures: e.properties.iter().find(|p| p.name == "textures").cloned(),
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

    /// The game mode an update gives `uuid`'s entry, if it changes it (a new entry
    /// starts in survival). Ask before applying the update.
    pub fn changed_game_mode(
        &self,
        uuid: u128,
        actions: u8,
        entries: &[play::PlayerInfoEntry],
    ) -> Option<i32> {
        let e = entries.iter().find(|e| e.uuid == uuid)?;
        let new = e.game_mode?;
        let old = match self.players.get(&uuid) {
            Some(p) => p.game_mode,
            None if actions & play::PLAYER_INFO_ADD != 0 && e.name.is_some() => 0,
            None => return None,
        };
        (old != new).then_some(new)
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
    /// `isAlive`: still in the level (a dying player is, for 20 ticks).
    pub alive: bool,
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
    /// The level's game time (`getGameTime`).
    pub game_time: i64,
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
    /// The level's game time as the entities tick (`getGameTime`): a camel's pose
    /// changes are timed by it.
    game_time: i64,
    /// The items in the local player's hands (`isHolding`): a pig or strider follows
    /// a rider holding its item on a stick.
    local_holding: [i32; 2],
    /// How many chunks from the player the server ticks.
    pub simulation_distance: i32,
}

impl Entities {
    /// A new level's entities with the local player in it (`addEntity` of the player:
    /// it always ticks).
    pub fn new(local_id: i32, minecart_improvements: bool) -> Entities {
        Entities {
            ticking: vec![local_id],
            local_id: Some(local_id),
            minecart_improvements,
            simulation_distance: i32::MAX,
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

    /// The entities whose box intersects `b`, in the order `Level.getEntities(except,
    /// box, ...)` visits them (`EntitySectionStorage.getEntities`): sections near the
    /// box by x, then by their key's order within each x (y 0 upwards, then the
    /// negative ones, then z), each section's entities in the order they arrived. The
    /// local player isn't among them.
    pub fn in_box(&self, b: &Aabb) -> Vec<&Entity> {
        let section = |v: f64| mth::floor(v) >> 4;
        let (x0, y0, z0) = (
            section(b.min[0] - 2.0),
            section(b.min[1] - 4.0),
            section(b.min[2] - 2.0),
        );
        let (x1, y1, z1) = (
            section(b.max[0] + 2.0),
            section(b.max[1] + 0.0),
            section(b.max[2] + 2.0),
        );
        let mut out = Vec::new();
        for x in x0..=x1 {
            let range = pack_section(x, 0, 0)..=pack_section(x, -1, -1);
            for (&key, ids) in self.sections.range(range) {
                let y = (key << 44 >> 44) as i32;
                let z = (key << 22 >> 42) as i32;
                if y < y0 || y > y1 || z < z0 || z > z1 {
                    continue;
                }
                for id in ids {
                    if let Some(e) = self.entities.get(id)
                        && e.bounding_box.intersects(b)
                    {
                        out.push(e);
                    }
                }
            }
        }
        out
    }

    /// What `Level.getEntities(player, b)` finds for the local player: the entities
    /// touching `b` in order, spectators aside. (The ender dragon's parts aren't
    /// tracked.)
    pub fn touching(&self, b: &Aabb, players: &PlayerInfos) -> Vec<&Entity> {
        let mut found = self.in_box(b);
        found.retain(|e| !e.removed && !e.is_spectator(players));
        found
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
        let player = if info.player {
            match players.get(p.uuid) {
                Some(i) => Some(i),
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
        let e = Entity::new(p, info, player, self.minecart_improvements);
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

    /// `handleSetEntityPassengersPacket`: the vehicle's passengers replaced. When this
    /// mounts the local player, the turn it takes: a boat's heading (when it wasn't on
    /// it already), or a horse's view (`AbstractHorse.addPassenger`, each time).
    pub fn set_passengers(
        &mut self,
        vehicle: i32,
        passengers: &[i32],
    ) -> Option<(f32, Option<f32>)> {
        if !self.exists(vehicle) {
            return None;
        }
        let was_mounted = self.carries_local(vehicle);
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
        if self.local_id.is_none() || self.local_vehicle != Some(vehicle) {
            return None;
        }
        let v = self.entities.get(&vehicle)?;
        match v.info.ride {
            _ if v.info.snaps_rider => Some((v.y_rot, Some(v.x_rot))),
            RideKind::Boat { .. } if !was_mounted => Some((v.y_rot, None)),
            _ => None,
        }
    }

    /// `getRootVehicle`: the bottom of the stack `id` rides in (itself when it
    /// rides nothing); the local player's too.
    pub fn root_vehicle(&self, mut id: i32) -> i32 {
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
        if !self.entities.contains_key(&id) {
            return;
        }
        let rot = rotation.map(|(y, x)| (mth::unpack_degrees(y), mth::unpack_degrees(x)));
        if self.controlled_by_local(id) {
            // The client moves it: only the codec's base follows.
            let e = self.entities.get_mut(&id).expect("entity");
            if let Some(d) = delta {
                let path = decode_delta(&mut e.codec_base, d);
                e.codec_base = path.end();
            }
            return;
        }
        let e = self.entities.get_mut(&id).expect("entity");
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
        player: &mut LocalPlayer,
    ) {
        let ticking = self.ticking.contains(&id);
        let controlled = self.controlled_by_local(id);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let end = path.end();
        e.codec_base = end;
        if controlled {
            return;
        }
        let too_far = length_sqr(sub(e.position, end)) > 4096.0;
        if ticking && !too_far {
            e.move_or_interpolate_to(Some(path), y_rot, x_rot, true);
        } else {
            e.snap_to(end, y_rot, x_rot);
        }
        let interpolating = e.is_interpolating();
        // Its rider put where it now is.
        if !interpolating && self.carries_local(id) {
            self.position_riders_of(id, player);
            player.old_position = player.position;
            player.old_rot = (player.y_rot, player.x_rot);
        }
        if let Some(e) = self.entities.get_mut(&id) {
            e.on_ground = on_ground;
        }
        self.moved(id);
    }

    /// `positionRider` of the local player by `id` when it rides it, directly or on
    /// something that does.
    fn position_riders_of(&self, id: i32, player: &mut LocalPlayer) {
        if let Some(v) = self.local_vehicle
            && (v == id || self.carries_local(id))
        {
            self.position_local_rider(v, player);
        }
    }

    /// `handleMoveVehicle`: where the server has the vehicle the player controls. A
    /// difference beyond 1e-5 snaps it there; either way the client answers with
    /// where it now has it.
    pub fn move_vehicle(&mut self, position: [f64; 3], y_rot: f32, x_rot: f32) -> Option<Vec<u8>> {
        let local = self.local_id?;
        let root = self.root_vehicle(local);
        if root == local || !self.controlled_by_local(root) {
            return None;
        }
        let e = self.entities.get_mut(&root)?;
        let current = e.client_pos_rot().position;
        if length_sqr(sub(position, current)).sqrt() > 9.999999747378752E-6 {
            e.cancel();
            // absSnapTo.
            let p = [
                position[0].clamp(-3.0E7, 3.0E7),
                position[1],
                position[2].clamp(-3.0E7, 3.0E7),
            ];
            e.set_pos(p);
            e.set_y_rot(y_rot % 360.0);
            e.set_x_rot(x_rot.clamp(-90.0, 90.0) % 360.0);
            e.old_position = p;
        }
        let at = e.client_pos_rot();
        let packet = play::move_vehicle(at.position, at.y_rot, at.x_rot, e.on_ground);
        self.moved(root);
        Some(packet)
    }

    /// `handleEntityEvent` for another entity: a boat's bubble column events.
    pub fn entity_event(&mut self, id: i32, event: u8) {
        let controlled = self.controlled_by_local(id);
        if let Some(e) = self.entities.get_mut(&id)
            && e.info.boat
            && e.body.is_some()
        {
            e.boat_event(event, controlled);
        }
    }

    /// `handleSetEquipment`: what the saddle and body slots hold (`isSaddled`,
    /// `isWearingBodyArmor`: a saddle, a harness, a block a sulfur cube swallowed).
    pub fn set_equipment(&mut self, id: i32, slots: &[(u8, ruthenium_protocol::item::ItemStack)]) {
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        for (slot, stack) in slots {
            let name = (stack.count > 0)
                .then(|| u32::try_from(stack.item).ok())
                .flatten()
                .and_then(ruthenium_data::v26_3::registries::Item::from_id)
                .map(|i| i.name());
            match *slot {
                play::EQUIPMENT_SADDLE => e.saddled = name == Some("minecraft:saddle"),
                play::EQUIPMENT_BODY => {
                    e.body_armor = name.is_some_and(|n| n.ends_with("_harness"))
                        || name.is_some() && !matches!(e.info.ride, RideKind::HappyGhast)
                }
                _ => {}
            }
        }
    }

    /// `handleTeleportEntity` (`setValuesFromPositionPacket`, interpolating unless it
    /// is too far).
    pub fn teleport(
        &mut self,
        id: i32,
        change: &play::PositionChange,
        on_ground: bool,
        player: &mut LocalPlayer,
    ) -> Option<Vec<u8>> {
        let ticking = self.ticking.contains(&id);
        let controlled = self.controlled_by_local(id);
        let relative = change.relatives & 7 != 0;
        let interpolate = ticking || !controlled || relative;
        let play::PositionChange {
            position,
            delta,
            y_rot,
            x_rot,
            relatives,
        } = *change;
        let e = self.entities.get_mut(&id)?;
        let current = e.client_pos_rot();
        // `PositionMoveRotation.of` takes `getKnownMovement`: the controlling
        // player's movement for a vehicle it steers.
        let known = if controlled && e.alive() {
            player.delta
        } else {
            e.delta
        };
        let (pos, new_delta, y, x) = crate::player::calculate_absolute(
            current.position,
            known,
            current.y_rot,
            current.x_rot,
            position,
            delta,
            y_rot,
            x_rot,
            relatives,
        );
        let interpolated = interpolate && length_sqr(sub(current.position, pos)) <= 4096.0;
        if interpolated {
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
        // Not interpolated: its rider follows at once, and a vehicle the player
        // controls says where it now is.
        if !interpolated && self.carries_local(id) {
            self.position_riders_of(id, player);
            player.old_position = player.position;
            player.old_rot = (player.y_rot, player.x_rot);
            if controlled {
                let e = self.entities.get(&id)?;
                let at = e.client_pos_rot();
                return Some(play::move_vehicle(
                    at.position,
                    at.y_rot,
                    at.x_rot,
                    e.on_ground,
                ));
            }
        }
        None
    }

    /// `handleSetEntityMotion` for another entity (`lerpMotion`; remote players ease
    /// into it, which only shows in how they're drawn).
    pub fn set_motion(&mut self, id: i32, movement: [f64; 3]) {
        if let Some(e) = self.entities.get_mut(&id) {
            e.delta = movement;
            if let (Some(a), Follow::Stepped(interval)) = (&mut e.avatar, e.info.follow) {
                a.lerp_motion(movement, interval);
            }
            // `AbstractArrow.lerpMotion`: moving again, so out of the ground.
            if matches!(e.info.pickable, Pickable::Arrow { .. }) && length_sqr(movement) > 0.0 {
                e.in_ground = false;
            }
        }
    }

    /// The avatar of `id`, if it is one.
    fn avatar(&mut self, id: i32) -> Option<&mut Avatar> {
        self.entities.get_mut(&id)?.avatar.as_deref_mut()
    }

    /// `handleRotateMob`.
    pub fn rotate_head(&mut self, id: i32, y_head_rot: i8) {
        if let Some(a) = self.avatar(id) {
            a.turn_head(mth::unpack_degrees(y_head_rot));
        }
    }

    /// `handleDamageEvent`.
    pub fn damaged(&mut self, id: i32) {
        if let Some(a) = self.avatar(id) {
            a.damaged();
        }
    }

    /// `handleHurtAnimation`.
    pub fn hurt(&mut self, id: i32) {
        if let Some(a) = self.avatar(id) {
            a.hurt();
        }
    }

    /// `handleSwingAnimation`.
    pub fn swing(&mut self, id: i32, swing: Swing) {
        if let Some(a) = self.avatar(id) {
            a.swing(swing);
        }
    }

    /// `handleSetEquipment` for what an avatar holds and wears, as far as it shows.
    pub fn set_worn(
        &mut self,
        id: i32,
        slots: &[(u8, ruthenium_protocol::item::ItemStack)],
        items: &Items,
    ) {
        let Some(a) = self.avatar(id) else {
            return;
        };
        for (slot, stack) in slots {
            let stack = if stack.count > 0 {
                items.stack(stack)
            } else {
                Stack::empty()
            };
            match *slot {
                play::EQUIPMENT_MAINHAND => a.set_held(false, stack),
                play::EQUIPMENT_OFFHAND => a.set_held(true, stack),
                play::EQUIPMENT_CHEST => {
                    let name = u32::try_from(stack.item()).ok().and_then(Item::from_id);
                    a.set_chest(match name.map_or("", |i| i.name()) {
                        "minecraft:elytra" => Chest::Wings,
                        n if n.ends_with("_chestplate") => Chest::Armour,
                        _ => Chest::Bare,
                    });
                    a.set_worn(crate::avatar::WORN_CHEST, stack);
                }
                play::EQUIPMENT_HEAD => a.set_worn(crate::avatar::WORN_HEAD, stack),
                play::EQUIPMENT_LEGS => a.set_worn(crate::avatar::WORN_LEGS, stack),
                play::EQUIPMENT_FEET => a.set_worn(crate::avatar::WORN_FEET, stack),
                _ => {}
            }
        }
    }

    /// What drawing needs of the avatar `e`, for a viewer who is a spectator or not and
    /// has this scoreboard name.
    pub fn avatar_record(
        &self,
        e: &Entity,
        teams: &Teams,
        spectator: bool,
        viewer: &str,
    ) -> Option<Box<AvatarRecord>> {
        let a = e.avatar.as_deref()?;
        let scale = e.scale.value() as f32;
        let eyes = match e.pose {
            pose::SLEEPING => 0.2,
            pose::FALL_FLYING | pose::SWIMMING | pose::SPIN_ATTACK => 0.4 * scale,
            pose::CROUCHING => 1.27 * scale,
            pose::DYING => 1.62,
            _ => 1.62 * scale,
        };
        let sees = spectator
            || teams.team_of(&e.scoreboard_name).is_some_and(|t| {
                t.see_invisible && teams.team_of(viewer).is_some_and(|v| v.name == t.name)
            });
        let vehicle = e.vehicle.and_then(|v| self.entities.get(&v));
        let mut r = a.record();
        r.hidden = a.is_invisible() && !sees;
        r.alive = !e.removed && e.health > 0.0;
        r.upside_down = e.info.player
            && r.parts & 1 != 0
            && matches!(e.scoreboard_name.as_str(), "Dinnerbone" | "Grumm");
        r.age = e.tick_count;
        r.scale = scale;
        r.size = [e.size.width, e.size.height];
        r.eye_height = eyes;
        r.vehicle_body = vehicle.filter(|v| v.info.living).map(|v| {
            v.avatar
                .as_deref()
                .map_or((v.y_rot, v.y_rot), Avatar::body_rots)
        });
        Some(Box::new(r))
    }

    /// Puts together again the names that are out of date: before a frame is published.
    pub fn refresh_names(&mut self, teams: &Teams, scores: &Scoreboard) {
        for e in self.entities.values_mut() {
            let Some(n) = &mut e.naming else {
                continue;
            };
            let stamp = [teams.rev(), scores.rev(), n.rev];
            if n.shown.as_ref().is_some_and(|s| s.0 == stamp) {
                continue;
            }
            // `getName`: a player's is its profile's, another's the one it was given or
            // its type's.
            let own = if e.info.player {
                Tag::String(e.scoreboard_name.clone())
            } else {
                n.custom_name.clone().unwrap_or_else(|| {
                    translatable(&format!("entity.{}", e.info.key.replace(':', ".")))
                })
            };
            let name = match teams.team_of(&e.scoreboard_name) {
                Some(t) => team_name(&t.prefix, own, &t.suffix, t.colour),
                None => own,
            };
            let below = if e.info.player {
                scores.below_name(&e.scoreboard_name)
            } else {
                n.description.clone()
            };
            let text = NameText { name, below };
            // The same one as before if nothing of it changed: what is drawn from it is
            // kept by it.
            let text = match n.shown.take() {
                Some((_, old)) if *old == text => old,
                _ => Arc::new(text),
            };
            n.shown = Some((stamp, text));
        }
    }

    /// The name over the avatar `e` as [`Self::refresh_names`] left it, if a viewer who is
    /// a spectator or not and has this scoreboard name is shown it
    /// (`LivingEntityRenderer.shouldShowName`, `AvatarRenderer.shouldShowName`). How far
    /// it is seen from is left to the one who draws.
    pub fn name_record(
        &self,
        e: &Entity,
        teams: &Teams,
        spectator: bool,
        viewer: &str,
    ) -> Option<NameRecord> {
        let a = e.avatar.as_deref()?;
        let n = e.naming.as_deref()?;
        let team = teams.team_of(&e.scoreboard_name);
        let own = teams.team_of(viewer);
        let same = |t: &Team| own.is_some_and(|o| o.name == t.name);
        // `isInvisibleTo`.
        let visible =
            !a.is_invisible() || spectator || team.is_some_and(|t| t.see_invisible && same(t));
        let shown = match team {
            Some(t) => match t.name_visibility {
                visibility::NEVER => false,
                visibility::HIDE_FOR_OTHER_TEAMS if own.is_some() => {
                    same(t) && (t.see_invisible || visible)
                }
                visibility::HIDE_FOR_OWN_TEAM if own.is_some() => !same(t) && visible,
                _ => visible,
            },
            // (The heads-up display cannot be hidden yet.)
            None => visible && e.passengers.is_empty(),
        };
        // A player's name is always there to show; another's only if it is said to be.
        // (Nor while the crosshair is on it: what it is on is not kept.)
        if !shown || !(e.info.player || n.custom_name.is_some() && n.custom_name_visible) {
            return None;
        }
        Some(NameRecord {
            text: n.shown.as_ref()?.1.clone(),
            discrete: a.is_discrete(),
            distance: n.distance,
            below_distance: n.below_distance,
            offset: if e.info.player && e.scoreboard_name == "deadmau5" {
                -10
            } else {
                0
            },
        })
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
            if let (Some(n), Some(b)) = (name, e.body.as_deref_mut())
                && let Some(attr) = b.attributes.get_mut(n)
            {
                attr.base = a.base;
                attr.modifiers = a
                    .modifiers
                    .iter()
                    .map(|(id, amount, operation)| Modifier {
                        id: id.clone(),
                        amount: *amount,
                        operation: *operation,
                    })
                    .collect();
            }
            if let Some(n) = &mut e.naming
                && matches!(
                    name,
                    Some(ATTRIBUTE_NAME_TAG_DISTANCE | ATTRIBUTE_BELOW_NAME_DISTANCE)
                )
            {
                // Both have the same range.
                let mut attribute = Attribute::new(a.base, 0.0, 512.0);
                attribute.modifiers = a
                    .modifiers
                    .iter()
                    .map(|(id, amount, operation)| Modifier {
                        id: id.clone(),
                        amount: *amount,
                        operation: *operation,
                    })
                    .collect();
                let value = attribute.value() as f32;
                if name == Some(ATTRIBUTE_NAME_TAG_DISTANCE) {
                    n.distance = value;
                } else {
                    n.below_distance = value;
                }
            }
            if name == Some(ATTRIBUTE_SCALE) {
                // Dirty as `setBaseValue`, `removeModifiers` and the adds mark it.
                e.scale_dirty |= e.scale.base != a.base
                    || !e.scale.modifiers.is_empty()
                    || !a.modifiers.is_empty();
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
            }
        }
    }

    /// `ClientLevel.tickEntities`: every entity that ticks, in order, the local player
    /// in its place (`tick_player` runs its tick, among the entities as they are then
    /// and the boxes it collides with), each vehicle's passengers right after
    /// it.
    pub fn tick(
        &mut self,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        mut tick_player: impl FnMut(&mut LocalPlayer, &Entities, &[Aabb]),
    ) {
        // The list as it was when the iteration began (`EntityTickList` copies it if
        // it changes meanwhile).
        let order = self.ticking.clone();
        self.game_time = cx.game_time;
        self.local_holding = player.holding();
        let speed = if cx.tick_rate > 20.0 {
            cx.tick_rate / 20.0
        } else {
            1.0
        };
        for id in order {
            if Some(id) == self.local_id {
                // A passenger ticks after its vehicle (`tickPassenger`).
                if self.local_vehicle.is_some() {
                    continue;
                }
                self.sync_riding(player);
                let colliders = self.colliders(player);
                tick_player(player, self, &colliders);
                self.tick_local_passengers(cx, player, speed, &mut tick_player);
                continue;
            }
            let Some(e) = self.entities.get(&id) else {
                continue;
            };
            // Frozen: only players (and what carries one) tick.
            let frozen = !cx.runs_normally && !e.info.player && !self.carries_local(id);
            if e.removed || e.vehicle.is_some() || frozen {
                continue;
            }
            self.tick_one(id, cx, player, speed, false, &mut tick_player);
        }
    }

    /// Whether the local player rides `id`, directly or not.
    fn carries_local(&self, id: i32) -> bool {
        let mut v = self.local_vehicle;
        while let Some(x) = v {
            if x == id {
                return true;
            }
            v = self.vehicle_of(x);
        }
        false
    }

    /// The local player's vehicle (`getVehicle`).
    pub fn local_vehicle(&self) -> Option<i32> {
        self.local_vehicle
    }

    /// Whether the local player controls `id` (`isLocalInstanceAuthoritative`): it
    /// is the first passenger of a vehicle a player steers: a boat; a saddled horse,
    /// camel or nautilus; a harnessed happy ghast that isn't holding still; a saddled
    /// pig or strider while the player holds its item on a stick.
    pub fn controlled_by_local(&self, id: i32) -> bool {
        let (Some(local), Some(e)) = (self.local_id, self.entities.get(&id)) else {
            return false;
        };
        if e.passengers.first() != Some(&local) {
            return false;
        }
        let holding = |item: Item| self.local_holding.contains(&(item.id() as i32));
        match e.info.ride {
            RideKind::Boat { .. } => true,
            RideKind::Horse { .. } | RideKind::Camel | RideKind::Nautilus => e.is_saddled(),
            RideKind::HappyGhast => e.body_armor && !e.stays_still,
            RideKind::Pig => e.is_saddled() && holding(Item::CarrotOnAStick),
            RideKind::Strider => e.is_saddled() && holding(Item::WarpedFungusOnAStick),
            RideKind::None => false,
        }
    }

    /// Tells the player what it rides ([`LocalPlayer::riding`]); getting off frees its
    /// hands (`LocalPlayer.removeVehicle`).
    pub fn sync_riding(&mut self, player: &mut LocalPlayer) {
        self.local_holding = player.holding();
        let riding = self
            .local_vehicle
            .and_then(|v| self.entities.get(&v))
            .map(|e| {
                let controls = self.controlled_by_local(e.id);
                Riding {
                    vehicle: e.id,
                    controls,
                    jumpable: controls && e.can_jump(self.game_time),
                    jump_cooldown: e.jump_cooldown(),
                    can_sprint: controls && e.info.ride == RideKind::Camel,
                    fluids_above: match (&e.info.ride, &e.body) {
                        (RideKind::Boat { .. }, Some(b)) if !b.is_under_water() => {
                            Some(e.bounding_box.max[1])
                        }
                        _ => None,
                    },
                }
            });
        if riding.is_none() && player.riding.is_some() {
            player.hands_busy = false;
        }
        player.riding = riding;
    }

    /// Where the vehicle the player controls is, for `sendChanges` (none unless the
    /// player controls the bottom of what it rides).
    pub fn vehicle_move(&self) -> Option<VehicleMove> {
        let local = self.local_id?;
        let root = self.root_vehicle(local);
        if root == local || !self.controlled_by_local(root) {
            return None;
        }
        let e = self.entities.get(&root)?;
        let at = e.client_pos_rot();
        Some(VehicleMove {
            position: at.position,
            y_rot: at.y_rot,
            x_rot: at.x_rot,
            on_ground: e.on_ground,
        })
    }

    /// The vehicle the player controls (the bottom of what it rides) taken out to be
    /// pushed by pistons, with the boxes it collides with; give it back with
    /// [`Self::put_back`].
    pub fn take_controlled_vehicle(&mut self, player: &LocalPlayer) -> Option<(Entity, Vec<Aabb>)> {
        let local = self.local_id?;
        let root = self.root_vehicle(local);
        if root == local || !self.controlled_by_local(root) {
            return None;
        }
        let colliders = self.vehicle_colliders(root, player);
        let e = self.entities.get(&root)?.clone();
        Some((e, colliders))
    }

    /// Puts back a vehicle [`Self::take_controlled_vehicle`] gave.
    pub fn put_back(&mut self, e: Entity) {
        let id = e.id;
        if let Some(slot) = self.entities.get_mut(&id) {
            *slot = e;
        }
        self.moved(id);
    }

    /// `tickPassenger` for the local player: `rideTick` (at rest, its tick, then put
    /// where the vehicle seats it), then `LocalPlayer.rideTick` (a boat takes its keys,
    /// which busy its hands) and a horse's charged jump let go.
    fn ride_tick_local(
        &mut self,
        vehicle: i32,
        player: &mut LocalPlayer,
        tick_player: &mut dyn FnMut(&mut LocalPlayer, &Entities, &[Aabb]),
    ) {
        if !player.is_alive() {
            // `isRemoved`: off the vehicle.
            if let Some(local) = self.local_id {
                self.stop_riding(local, Some(vehicle));
            }
            self.sync_riding(player);
            return;
        }
        self.sync_riding(player);
        player.delta = [0.0; 3];
        let colliders = self.colliders(player);
        tick_player(player, self, &colliders);
        self.position_local_rider(vehicle, player);
        player.fall_distance = 0.0;
        player.hands_busy = false;
        let controls = self.controlled_by_local(vehicle);
        if let Some(e) = self.entities.get_mut(&vehicle) {
            let keys = player.keys();
            if controls
                && let Some(b) = e.body.as_deref_mut()
                && matches!(b.kind, RideKind::Boat { .. })
            {
                b.input = [keys.left, keys.right, keys.forward, keys.backward];
                player.hands_busy = keys.left || keys.right || keys.forward || keys.backward;
            }
            if let Some(amount) = player.riding_jump.take()
                && controls
            {
                e.on_player_jump(amount);
            }
        }
    }

    /// `positionRider`: the local player where its vehicle seats it (its passenger
    /// point, less the player's own vehicle point); a boat turns the player with it and
    /// keeps it facing within 105 degrees of its heading.
    pub fn position_local_rider(&self, vehicle: i32, player: &mut LocalPlayer) {
        let (Some(local), Some(v)) = (self.local_id, self.entities.get(&vehicle)) else {
            return;
        };
        let Some(index) = v.passengers.iter().position(|&p| p == local) else {
            return;
        };
        let attach = self.passenger_point(v, index, false);
        let own = rotate_y(player.vehicle_attachment(), -player.y_rot * DEG_TO_RAD_F);
        player.position = [
            v.position[0] + attach[0] - own[0],
            v.position[1] + attach[1] - own[1],
            v.position[2] + attach[2] - own[2],
        ];
        if let Some(b) = v.body.as_deref()
            && matches!(b.kind, RideKind::Boat { .. })
        {
            let y = player.y_rot + b.delta_rotation;
            if y.is_finite() {
                player.y_rot = y;
            }
            player.head_y_rot += b.delta_rotation;
            let delta = mth::wrap_degrees(player.y_rot - v.y_rot);
            let target = delta.clamp(-105.0, 105.0);
            let y = player.y_rot + target - delta;
            if y.is_finite() {
                player.y_rot = y;
            }
        }
    }

    /// `positionRider` for another passenger: seated as the local player is, on its own
    /// vehicle point; a boat turns a mob to face as it does (an animal sideways when the
    /// boat is full) and keeps anything else within 105 degrees of its heading. The
    /// vehicle may be the local player.
    fn position_rider(&mut self, vehicle: i32, passenger: i32, player: &LocalPlayer) {
        let Some(p) = self.entities.get(&passenger) else {
            return;
        };
        let (seat, vehicle_width, boat_turn) = if Some(vehicle) == self.local_id {
            if !self.local_passengers.contains(&passenger) {
                return;
            }
            // An avatar's passenger point: the top of its box.
            let seat = rotate_y(
                [0.0, player.passenger_height(), 0.0],
                -player.y_rot * DEG_TO_RAD_F,
            );
            (
                [
                    player.position[0] + seat[0],
                    player.position[1] + seat[1],
                    player.position[2] + seat[2],
                ],
                player.width(),
                None,
            )
        } else {
            let Some(v) = self.entities.get(&vehicle) else {
                return;
            };
            let Some(index) = v.passengers.iter().position(|&x| x == passenger) else {
                return;
            };
            let a = self.passenger_point(v, index, p.info.lowered_in_minecarts);
            // `AbstractBoat.calculatePassengerBodyYRot`.
            let boat_turn = match v.info.ride {
                RideKind::Boat { max_passengers, .. } if !p.info.turns_in_boats => {
                    Some(if p.info.animal && v.passengers.len() == max_passengers {
                        v.y_rot + if passenger % 2 == 0 { 90.0 } else { 270.0 }
                    } else {
                        v.y_rot
                    })
                }
                _ => None,
            };
            (
                [
                    v.position[0] + a[0],
                    v.position[1] + a[1],
                    v.position[2] + a[2],
                ],
                v.size.width,
                boat_turn,
            )
        };
        let own = p.vehicle_attachment(vehicle_width);
        let p = self.entities.get_mut(&passenger).expect("passenger");
        p.set_pos([seat[0] - own[0], seat[1] - own[1], seat[2] - own[2]]);
        // `clampRotation` (a remote passenger is never the client's to turn by the
        // boat's `deltaRotation`).
        if let Some(body) = boat_turn {
            if p.info.mob {
                p.set_y_rot(body);
            } else {
                let delta = mth::wrap_degrees(p.y_rot - body);
                let target = delta.clamp(-105.0, 105.0);
                p.set_y_rot(p.y_rot + target - delta);
            }
            if let Some(a) = &mut p.avatar {
                a.turned_by_boat(body);
            }
        }
        self.moved(passenger);
    }

    /// `getPassengerAttachmentPoint` of `v` for its `index`-th passenger, turned with
    /// it; `lowered`: a passenger a minecart seats at its origin.
    fn passenger_point(&self, v: &Entity, index: usize, lowered: bool) -> [f64; 3] {
        let turn = -v.y_rot * DEG_TO_RAD_F;
        if lowered && v.info.minecart {
            return [0.0; 3];
        }
        // `LivingEntity.getPassengerRidingPosition` takes the dimensions and the scale
        // the vehicle has now, not the cached ones.
        let size = v.dimensions();
        if v.info.cube_size_field.is_some() {
            // `AbstractCubeMob`: its height less a 64th per size.
            let scale = v.scale.value() as f32;
            let y = f64::from(size.height) - 0.015625 * f64::from(v.cube_size) * f64::from(scale);
            return [0.0, y, 0.0];
        }
        match v.info.ride {
            RideKind::Boat {
                ride_height,
                x_offset,
                ..
            } => {
                let offset = if v.passengers.len() > 1 {
                    if index == 0 { 0.2 } else { -0.6 }
                } else {
                    x_offset
                };
                rotate_y([0.0, f64::from(ride_height), f64::from(offset)], turn)
            }
            RideKind::Camel => v.camel_passenger_point(index, self.game_time),
            _ => {
                let points = &v.info.passenger_points;
                let scale = f64::from(size.height / v.info.size.height);
                let p = points[index.min(points.len() - 1)].map(|c| c * scale);
                let mut a = rotate_y(p, turn);
                // `AbstractHorse`: lifted as it rears (a llama keeps the default).
                if let Some(b) = v.body.as_deref()
                    && matches!(b.kind, RideKind::Horse { rearing: true })
                {
                    let s = f64::from(b.stand_anim_o);
                    let stand = rotate_y([0.0, 0.15 * s, -0.7 * s], turn);
                    a = [a[0] + stand[0], a[1] + stand[1], a[2] + stand[2]];
                }
                if v.info.ride == RideKind::Strider {
                    a[1] += v.strider_bob();
                }
                a
            }
        }
    }

    /// The boxes a vehicle collides with as it moves (`getEntityCollisions`): a boat
    /// with what can be collided with or pushed, a living one with what can be collided
    /// with; not what rides with it.
    fn vehicle_colliders(&self, id: i32, player: &LocalPlayer) -> Vec<Aabb> {
        let Some(v) = self.entities.get(&id) else {
            return Vec::new();
        };
        let boat = matches!(v.info.ride, RideKind::Boat { .. });
        let root = self.root_vehicle(id);
        let mut out = Vec::new();
        for e in self.entities.values() {
            if e.id == id || e.removed || self.root_vehicle(e.id) == root {
                continue;
            }
            let collided = match e.info.collider {
                Collider::No => false,
                Collider::Always => true,
                Collider::Alive => e.health > 0.0,
                Collider::HappyGhast => {
                    !e.baby
                        && e.health > 0.0
                        && (e.stays_still
                            || !e.passengers.is_empty() && v.info.ride == RideKind::HappyGhast)
                }
            };
            // `isPushable`: living things alive (not spectators), boats and minecarts.
            let pushable = boat
                && if e.info.living {
                    e.health > 0.0 && !e.no_physics && !e.info.key.ends_with("armor_stand")
                } else {
                    e.info.boat || e.info.minecart
                };
            if collided || pushable {
                out.push(e.bounding_box);
            }
        }
        // The local player, when it doesn't ride with it.
        if boat
            && let Some(local) = self.local_id
            && self.root_vehicle(local) != root
            && player.is_alive()
            && !player.spectator
        {
            out.push(player.bounding_box());
        }
        out
    }

    /// `tickNonPassenger` / `tickPassenger`: the entity's tick, then its passengers'.
    fn tick_one(
        &mut self,
        id: i32,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        speed: f32,
        riding: bool,
        tick_player: &mut dyn FnMut(&mut LocalPlayer, &Entities, &[Aabb]),
    ) {
        let free = |b: &Aabb| cx.env.no_collision(b);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        // `commonTick`.
        e.old_position = e.position;
        let rot = (e.y_rot, e.x_rot);
        if let Some(a) = &mut e.avatar {
            a.set_old_rot(rot.0, rot.1);
        }
        e.interpolate(speed, &free);
        e.tick_count += 1;
        if riding {
            e.delta = [0.0; 3];
            if let Some(a) = &mut e.avatar {
                a.set_movement([0.0; 3]);
            }
        }
        self.moved(id);
        self.entity_tick(id, cx, player);
        self.avatar_tick(id, cx, player, riding);
        // `rideTick`: seated where its vehicle says; a living one's fall forgotten.
        if riding && let Some(vehicle) = self.vehicle_of(id) {
            self.position_rider(vehicle, id, player);
            if let Some(b) = self
                .entities
                .get_mut(&id)
                .and_then(|e| e.body.as_deref_mut())
            {
                b.fall_distance = 0.0;
            }
        }
        let passengers = self
            .entities
            .get(&id)
            .map(|e| e.passengers.clone())
            .unwrap_or_default();
        for p in passengers {
            if Some(p) == self.local_id {
                if self.local_vehicle == Some(id) {
                    self.ride_tick_local(id, player, tick_player);
                    self.tick_local_passengers(cx, player, speed, tick_player);
                }
                continue;
            }
            let ok = self
                .entities
                .get(&p)
                .is_some_and(|e| !e.removed && e.vehicle == Some(id));
            if ok && self.ticking.contains(&p) {
                self.tick_one(p, cx, player, speed, true, tick_player);
            }
        }
    }

    /// An avatar's part of its tick: what only its drawing shows.
    fn avatar_tick(
        &mut self,
        id: i32,
        cx: &TickContext<'_, '_>,
        player: &LocalPlayer,
        riding: bool,
    ) {
        let simulation_distance = self.simulation_distance;
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let interpolating = e.is_interpolating();
        let spectator = e.is_spectator(cx.players);
        let Some(a) = &mut e.avatar else {
            return;
        };
        let chunk = |p: [f64; 3], i: usize| mth::floor(p[i]) >> 4;
        let away = [0, 2]
            .map(|i| (chunk(e.position, i) - chunk(player.position, i)).abs())
            .into_iter()
            .max();
        let bed = a.sleeping_pos.and_then(|p| {
            let state = cx.env.state(p);
            let block = ruthenium_data::v26_3::blocks::BlockState::from_id(state)?.block();
            block.name().ends_with("_bed").then_some(())?;
            crate::block::state::get_dir(state, "facing")
        });
        a.tick(&TickInput {
            position: e.position,
            old_position: e.old_position,
            y_rot: e.y_rot,
            x_rot: e.x_rot,
            pose: e.pose,
            health: e.health,
            passenger: riding,
            interpolating,
            simulated: away.is_some_and(|d| d <= simulation_distance),
            in_water: touches_water(cx.env, &e.bounding_box),
            bed,
            spectator,
        });
        // `isSleeping`.
        if a.sleeping_pos.is_some() {
            e.x_rot = 0.0;
        }
    }

    /// `tickPassenger` for what rides the local player, after the player's tick.
    fn tick_local_passengers(
        &mut self,
        cx: &TickContext<'_, '_>,
        player: &mut LocalPlayer,
        speed: f32,
        tick_player: &mut dyn FnMut(&mut LocalPlayer, &Entities, &[Aabb]),
    ) {
        let Some(local) = self.local_id else {
            return;
        };
        for p in self.local_passengers.clone() {
            let ok = self
                .entities
                .get(&p)
                .is_some_and(|e| !e.removed && e.vehicle == Some(local));
            if ok && self.ticking.contains(&p) {
                self.tick_one(p, cx, player, speed, true, tick_player);
            }
        }
    }

    /// `Entity.tick`, for what it does to the local player.
    fn entity_tick(&mut self, id: i32, cx: &TickContext<'_, '_>, player: &mut LocalPlayer) {
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
        // A vehicle's own tick: the one the local player controls moves.
        if self.entities[&id].body.is_some() {
            let controller = self.controlled_by_local(id).then(|| {
                let (xxa, zza, jumping) = player.controller_input();
                Controller {
                    xxa,
                    zza,
                    jumping,
                    sprinting: player.sprinting,
                    y_rot: player.y_rot,
                    x_rot: player.x_rot,
                }
            });
            let colliders = self.vehicle_colliders(id, player);
            let env = Env {
                colliders: &colliders,
                ..*cx.env
            };
            // `Strider.tick`: one riding a strider that isn't cold is kept warm.
            let on_warm_strider = self
                .vehicle_of(id)
                .and_then(|v| self.entities.get(&v))
                .is_some_and(|v| v.info.ride == RideKind::Strider && !v.suffocating);
            let mut out = Vec::new();
            if let Some(e) = self.entities.get_mut(&id) {
                e.vehicle_tick(&env, controller, cx.game_time, on_warm_strider, &mut out);
            }
            for p in out {
                player.queue(p);
            }
            self.moved(id);
        }
        let e = &self.entities[&id];
        // `FireworkRocketEntity.tick`: a rocket attached to the local player boosts it
        // while it glides (the rocket finds the player while it is in the level).
        if e.attached_to.is_some()
            && e.attached_to == self.local_id
            && cx.local.alive
            && player.fall_flying()
        {
            player.rocket_boost();
        }
        if e.info.living && !e.info.no_push {
            // `pushEntities` → `getPushableEntities` → `doPush` → `LocalPlayer.push`,
            // which a sleeping player ignores.
            if player.bounding_box().intersects(&e.bounding_box)
                && self.pushable_by(e, cx, player)
                && !player.is_sleeping()
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
        // The end of `LivingEntity.tick`: `refreshDirtyAttributes` takes in a new scale.
        if let Some(e) = self.entities.get_mut(&id)
            && e.info.living
            && std::mem::take(&mut e.scale_dirty)
        {
            e.refresh_dimensions();
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

    /// `LivingEntity.isPushable` for the local player: alive (still in the level, even
    /// while dying), not a spectator, not climbing.
    fn player_pushable(&self, cx: &TickContext<'_, '_>, player: &LocalPlayer) -> bool {
        cx.local.alive && !cx.local.spectator && !player.on_climbable(cx.env)
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
    const PIG: i32 = 4;
    const STRIDER: i32 = 5;
    const REMOTE_PLAYER: i32 = 6;
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
            passenger_points: vec![[0.0, f64::from(size.1), 0.0]],
            vehicle_points: vec![[0.0; 3]],
            attributes: Vec::new(),
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
            data(
                3,
                "minecraft:horse",
                &[
                    "Horse",
                    "AbstractHorse",
                    "Animal",
                    "Mob",
                    "LivingEntity",
                    "Entity",
                ],
                (1.3964844, 1.6),
                &[("AbstractHorse.DATA_ID_FLAGS", 17)],
            ),
            data(
                4,
                "minecraft:pig",
                &["Pig", "Animal", "Mob", "LivingEntity", "Entity"],
                (0.9, 0.9),
                &[],
            ),
            data(
                5,
                "minecraft:strider",
                &["Strider", "Animal", "Mob", "LivingEntity", "Entity"],
                (0.9, 1.7),
                &[
                    ("Mob.DATA_MOB_FLAGS_ID", 15),
                    ("Strider.DATA_SUFFOCATING", 18),
                ],
            ),
            data(
                6,
                "minecraft:player",
                &["Player", "Avatar", "LivingEntity", "Entity"],
                (0.6, 1.8),
                &[("Entity.DATA_SHARED_FLAGS_ID", 0)],
            ),
            data(
                7,
                "minecraft:mannequin",
                &["Mannequin", "Avatar", "LivingEntity", "Entity"],
                (0.6, 1.8),
                &[
                    ("Entity.DATA_SHARED_FLAGS_ID", 0),
                    ("Entity.DATA_CUSTOM_NAME", 2),
                    ("Entity.DATA_CUSTOM_NAME_VISIBLE", 3),
                    ("Mannequin.DATA_DESCRIPTION", 19),
                ],
            ),
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
                outline: 0,
                interaction_shape: 0,
                interaction: 0,
                destroy_time: 0.0,
                light_dampening: 0,
                light_emission: 0,
                light_shape: false,
                occlusion: [0; 6],
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
            others: None,
        };
        let players = PlayerInfos::default();
        let cx = TickContext {
            env: &env,
            teams,
            players: &players,
            local: Local {
                name: "Developer",
                spectator: false,
                alive: player.is_alive(),
            },
            tick_rate: 20.0,
            runs_normally: true,
            game_time: 0,
        };
        e.tick(&cx, player, |_, _, _| {});
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
        // A dying player is still pushed; once out of the level it isn't.
        let mut p = player_at([0.5, -60.0, 0.5]);
        p.health = 0.0;
        tick(&mut e, &Teams::default(), &mut p);
        assert_eq!(p.delta, [x, 0.0, z]);
        let mut p = player_at([0.5, -60.0, 0.5]);
        p.health = 0.0;
        p.removed = true;
        tick(&mut e, &Teams::default(), &mut p);
        assert_eq!(p.delta, [0.0; 3]);
    }

    #[test]
    fn a_riptide_spin_hits_what_it_touches() {
        let (level, blocks, tags) = world();
        let mut players = PlayerInfos::default();
        players.update(
            play::PLAYER_INFO_ADD,
            &[play::PlayerInfoEntry {
                uuid: 1007,
                name: Some("Alex".into()),
                properties: Vec::new(),
                game_mode: None,
            }],
        );
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        let other = play::AddEntity {
            entity_id: 7,
            uuid: 1007,
            entity_type: REMOTE_PLAYER,
            position: [8.0, -60.0, 8.0],
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: 0,
        };
        e.add(&other, &types(), &players);
        let spin = |e: &Entities, players: &PlayerInfos| {
            let env = Env {
                level: &level,
                blocks: &blocks,
                tags: &tags,
                colliders: &[],
                others: Some(crate::player::Others {
                    entities: e,
                    players,
                }),
            };
            // A full-strength sprinting spin that went over x 6 to 10 this tick, into a
            // wall.
            let mut p = player_at([6.0, -60.0, 8.0]);
            let before = p.bounding_box();
            p.position[0] = 10.0;
            p.delta = [2.0, 0.5, -1.0];
            p.horizontal_collision = true;
            p.attack_strength_ticker = 50;
            p.item_swap_ticker = 50;
            p.auto_spin_attack_ticks = 10;
            p.sprinting = true;
            p.check_auto_spin_attack(&env, &before);
            p
        };
        // Another player takes the hit: the sprint slows the attacker before the bounce.
        let p = spin(&e, &players);
        assert_eq!(p.delta, [2.0 * 0.6 * -0.2, 0.5 * -0.2, -0.6 * -0.2]);
        assert!(!p.sprinting);
        assert_eq!(p.auto_spin_attack_ticks, 0);
        // `onAttack` starts only the attack strength over.
        assert_eq!((p.attack_strength_ticker, p.item_swap_ticker), (0, 50));
        // A spectator isn't touched: the wall ends the spin.
        players.update(
            play::PLAYER_INFO_GAME_MODE,
            &[play::PlayerInfoEntry {
                uuid: 1007,
                name: None,
                properties: Vec::new(),
                game_mode: Some(play::SPECTATOR),
            }],
        );
        let p = spin(&e, &players);
        assert_eq!((p.delta, p.sprinting), ([2.0, 0.5, -1.0], true));
        assert_eq!(
            (p.auto_spin_attack_ticks, p.attack_strength_ticker),
            (0, 50)
        );
        // A boat isn't hit, but the spin goes on against the wall.
        add(&mut e, 8, BOAT, [8.0, -60.0, 8.0]);
        let p = spin(&e, &players);
        assert_eq!((p.delta, p.auto_spin_attack_ticks), ([2.0, 0.5, -1.0], 10));
        assert_eq!(p.attack_strength_ticker, 50);
        // A mob is hit and bounced off, but doesn't slow the sprint.
        add(&mut e, 9, ZOMBIE, [8.0, -60.0, 8.0]);
        let p = spin(&e, &players);
        assert_eq!(p.delta, [2.0 * -0.2, 0.5 * -0.2, 0.2]);
        assert!(p.sprinting);
        assert_eq!((p.auto_spin_attack_ticks, p.attack_strength_ticker), (0, 0));
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
                display_name: Tag::String(String::new()),
                prefix: Tag::String(String::new()),
                suffix: Tag::String(String::new()),
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
        e.teleport(5, &away, true, &mut LocalPlayer::new());
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
    #[test]
    fn a_boat_seats_its_rider_and_holds_its_look_within_105_degrees() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, BOAT, [0.5, 0.0, 0.5]);
        let mut player = player_at([3.0, 0.0, 0.5]);
        assert_eq!(e.set_passengers(5, &[PLAYER_ID]), Some((0.0, None)));
        e.sync_riding(&mut player);
        assert!(e.controlled_by_local(5));
        assert!(player.riding.is_some_and(|r| r.controls && !r.jumpable));
        player.y_rot = 150.0;
        tick(&mut e, &Teams::default(), &mut player);
        let boat = e.get(5).unwrap();
        // In the air it falls by its gravity, and the rider sits a third of its height
        // up, less the player's own 0.6.
        assert_eq!(boat.delta[1], -0.04);
        assert_eq!(
            player.position,
            [
                0.5,
                boat.position[1] + f64::from(0.5625f32 / 3.0) - 0.6,
                0.5
            ]
        );
        assert_eq!(player.y_rot, 105.0);
        // It sends where it is after the rider's rotation.
        let sent = player.send_changes_riding(e.vehicle_move());
        assert_eq!(
            sent.last(),
            Some(&play::move_vehicle(boat.position, 0.0, 0.0, false))
        );
    }

    #[test]
    fn paddling_pushes_the_boat_along_its_heading() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, BOAT, [0.5, 0.0, 0.5]);
        let mut player = player_at([0.5, 0.0, 0.5]);
        e.set_passengers(5, &[PLAYER_ID]);
        e.entities
            .get_mut(&5)
            .unwrap()
            .body
            .as_deref_mut()
            .unwrap()
            .input = [false, false, true, false];
        tick(&mut e, &Teams::default(), &mut player);
        let boat = e.get(5).unwrap();
        // Facing south (yaw 0): 0.04 forward, then the air's 0.9 a tick.
        assert!(boat.delta[0].abs() < 1e-9);
        assert!(boat.position[2] > 0.5);
        // Both paddles go, and the packet goes before the rider's.
        let sent = player.send_changes_riding(e.vehicle_move());
        assert_eq!(sent[0], play::paddle_boat(true, true));
    }

    #[test]
    fn a_horse_is_steered_only_with_a_saddle() {
        let mut e = Entities::new(PLAYER_ID, false);
        add(&mut e, 7, 3, [0.5, 0.0, 0.5]);
        let mut player = player_at([0.5, 0.0, 0.5]);
        e.set_passengers(7, &[PLAYER_ID]);
        e.sync_riding(&mut player);
        assert!(!e.controlled_by_local(7));
        assert!(e.vehicle_move().is_none());
        assert!(player.riding.is_some_and(|r| !r.controls));
        let saddle = ruthenium_protocol::item::ItemStack {
            item: ruthenium_data::v26_3::registries::Item::Saddle.id() as i32,
            count: 1,
            patch: Default::default(),
        };
        e.set_equipment(7, &[(play::EQUIPMENT_SADDLE, saddle)]);
        // Only a tame horse can wear it.
        e.sync_riding(&mut player);
        assert!(!e.controlled_by_local(7));
        e.set_data(7, &[(17, DataValue::Byte(2))], &types());
        e.sync_riding(&mut player);
        assert!(e.controlled_by_local(7));
        assert!(player.riding.is_some_and(|r| r.controls && r.jumpable));
        // Off it, the hands are free again.
        player.hands_busy = true;
        e.set_passengers(7, &[]);
        e.sync_riding(&mut player);
        assert!(player.riding.is_none() && !player.hands_busy);
    }

    #[test]
    fn passengers_sit_where_their_vehicle_seats_them() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 8, PIG, [0.5, -60.0, 0.5]);
        add(&mut e, 9, ZOMBIE, [3.5, -60.0, 3.5]);
        e.set_passengers(8, &[9]);
        let mut player = player_at([5.5, -60.0, 5.5]);
        tick(&mut e, &Teams::default(), &mut player);
        let seat = -60.0 + f64::from(0.9f32);
        assert_eq!(e.get(9).unwrap().position, [0.5, seat, 0.5]);
        // It follows the pig the next tick.
        e.entities.get_mut(&8).unwrap().set_pos([1.5, -60.0, 0.5]);
        tick(&mut e, &Teams::default(), &mut player);
        assert_eq!(e.get(9).unwrap().position, [1.5, seat, 0.5]);
        // On the local player's head, after the player's tick.
        e.set_passengers(8, &[]);
        e.set_passengers(PLAYER_ID, &[9]);
        tick(&mut e, &Teams::default(), &mut player);
        let head = -60.0 + f64::from(1.8f32);
        assert_eq!(e.get(9).unwrap().position, [5.5, head, 5.5]);
        // A boat turns a mob it carries to its heading.
        e.set_passengers(PLAYER_ID, &[]);
        add(&mut e, 10, BOAT, [0.5, -60.0, 3.5]);
        e.entities.get_mut(&10).unwrap().y_rot = 30.0;
        e.set_passengers(10, &[9]);
        tick(&mut e, &Teams::default(), &mut player);
        assert_eq!(e.get(9).unwrap().y_rot, 30.0);
    }

    #[test]
    fn a_strider_on_a_warm_strider_is_kept_warm() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 8, STRIDER, [0.5, -60.0, 0.5]);
        add(&mut e, 9, STRIDER, [0.5, -58.3, 0.5]);
        e.set_passengers(8, &[9]);
        let mut player = player_at([5.5, -60.0, 5.5]);
        tick(&mut e, &Teams::default(), &mut player);
        let cold = |e: &Entities, id| e.get(id).unwrap().suffocating;
        assert!(cold(&e, 8) && cold(&e, 9), "out of the warm");
        // The bottom one warm (its AI off, so the server's flag stands): so is the top.
        e.set_data(
            8,
            &[(15, DataValue::Byte(1)), (18, DataValue::Boolean(false))],
            &types(),
        );
        tick(&mut e, &Teams::default(), &mut player);
        assert!(!cold(&e, 8) && !cold(&e, 9));
        e.set_data(8, &[(18, DataValue::Boolean(true))], &types());
        tick(&mut e, &Teams::default(), &mut player);
        assert!(cold(&e, 9));
    }

    #[test]
    fn a_pig_follows_a_rider_holding_a_carrot_on_a_stick() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 8, PIG, [0.5, 0.0, 0.5]);
        let mut player = player_at([0.5, 0.0, 0.5]);
        e.set_passengers(8, &[PLAYER_ID]);
        let saddle = ruthenium_protocol::item::ItemStack {
            item: Item::Saddle.id() as i32,
            count: 1,
            patch: Default::default(),
        };
        e.set_equipment(8, &[(play::EQUIPMENT_SADDLE, saddle)]);
        e.sync_riding(&mut player);
        assert!(!e.controlled_by_local(8), "not without the stick");
        let selected = player.inventory.selected as usize;
        let stick = crate::item::Items::default().new_stack(Item::CarrotOnAStick.id() as i32, 1);
        player.inventory.set(selected, stick);
        e.sync_riding(&mut player);
        assert!(e.controlled_by_local(8));
        assert!(
            player
                .riding
                .is_some_and(|r| r.controls && !r.jumpable && !r.can_sprint)
        );
        // It walks forward along the rider's look (south at yaw 0), with no keys held.
        player.y_rot = 0.0;
        tick(&mut e, &Teams::default(), &mut player);
        let pig = e.get(8).unwrap();
        assert!(pig.position[2] > 0.5);
        assert_eq!(pig.position[0], 0.5);
    }

    /// A player in the player list and the level.
    fn add_player(e: &mut Entities, id: i32, name: &str, position: [f64; 3]) {
        let mut players = PlayerInfos::default();
        let uuid = 1000 + id as u128;
        let textures = Property {
            name: "textures".into(),
            value: "e30=".into(),
            signature: None,
        };
        players.update(
            play::PLAYER_INFO_ADD,
            &[play::PlayerInfoEntry {
                uuid,
                name: Some(name.into()),
                properties: vec![
                    Property {
                        name: "other".into(),
                        value: "e31=".into(),
                        signature: None,
                    },
                    textures,
                ],
                game_mode: None,
            }],
        );
        let p = play::AddEntity {
            entity_id: id,
            uuid,
            entity_type: REMOTE_PLAYER,
            position,
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 64,
            data: 0,
        };
        e.add(&p, &types(), &players);
    }

    const MANNEQUIN: i32 = 7;

    fn text(s: &str) -> Tag {
        Tag::String(s.to_owned())
    }

    fn team(visibility: i32, options: u8) -> play::TeamParameters {
        play::TeamParameters {
            display_name: text(""),
            prefix: text("["),
            suffix: text("]"),
            name_tag_visibility: visibility,
            collision_rule: 0,
            color: Some(12),
            options,
        }
    }

    /// The name over `id` as the player "Developer" is shown it.
    fn name(e: &mut Entities, id: i32, teams: &Teams, spectator: bool) -> Option<NameRecord> {
        e.refresh_names(teams, &Scoreboard::default());
        e.name_record(e.get(id).unwrap(), teams, spectator, "Developer")
    }

    #[test]
    fn a_player_is_named_as_its_team_names_it() {
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let mut teams = Teams::default();
        let plain = name(&mut e, 9, &teams, false).unwrap();
        assert_eq!(plain.text.name, text("Kai"));
        assert_eq!((plain.distance, plain.below_distance), (64.0, 10.0));
        assert!(!plain.discrete && plain.offset == 0 && plain.text.below.is_none());
        // Nothing changed: the same text, not another like it.
        let again = name(&mut e, 9, &teams, false).unwrap();
        assert!(Arc::ptr_eq(&plain.text, &again.text));
        teams.apply("red", 0, Some(&team(0, 0)), &["Kai".to_owned()]);
        let named = name(&mut e, 9, &teams, false).unwrap();
        let c = named.text.name.as_compound().unwrap();
        assert_eq!(c.string("color"), Some("red"));
        assert_eq!(c.list("extra"), [text("["), text("Kai"), text("]")]);
        // A packet that changes nothing of it leaves the text the one it was.
        teams.apply("blue", 0, Some(&team(0, 0)), &[]);
        let after = name(&mut e, 9, &teams, false).unwrap();
        assert!(Arc::ptr_eq(&named.text, &after.text));
        // Sneaking, and the ears of one player.
        e.set_data(9, &[(0, DataValue::Byte(2))], &types());
        assert!(name(&mut e, 9, &teams, false).unwrap().discrete);
        add_player(&mut e, 10, "deadmau5", [5.5, -60.0, 5.5]);
        assert_eq!(name(&mut e, 10, &teams, false).unwrap().offset, -10);
    }

    #[test]
    fn a_score_stands_below_a_players_name() {
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let teams = Teams::default();
        let mut scores = Scoreboard::default();
        scores.set_objective("hp", 0, Some(text("HP")), None);
        scores.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        scores.set_score("Kai", "hp", 14, None);
        e.refresh_names(&teams, &scores);
        let shown = e
            .name_record(e.get(9).unwrap(), &teams, false, "Developer")
            .unwrap();
        assert_eq!(shown.text.below, scores.below_name("Kai"));
        assert!(shown.text.below.is_some());
        scores.reset_score("Kai", None);
        e.refresh_names(&teams, &scores);
        let shown = e
            .name_record(e.get(9).unwrap(), &teams, false, "Developer")
            .unwrap();
        assert_eq!(shown.text.below, None);
    }

    #[test]
    fn teams_say_who_sees_a_name() {
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let kai = ["Kai".to_owned()];
        let me = ["Developer".to_owned()];
        let seen = |e: &mut Entities, teams: &Teams| name(e, 9, teams, false).is_some();
        // Its team alone decides while the viewer has none.
        for (visibility, shown) in [(0, true), (1, false), (2, true), (3, true), (9, true)] {
            let mut teams = Teams::default();
            teams.apply("red", 0, Some(&team(visibility, 0)), &kai);
            assert_eq!(seen(&mut e, &teams), shown, "{visibility}");
        }
        // With the viewer on the same team, and on another.
        for (visibility, same, other) in [(0, true, true), (2, true, false), (3, false, true)] {
            let mut teams = Teams::default();
            teams.apply("red", 0, Some(&team(visibility, 0)), &kai);
            teams.apply("blue", 0, Some(&team(0, 0)), &me);
            assert_eq!(seen(&mut e, &teams), other, "{visibility} from another");
            teams.apply("red", 3, None, &me);
            assert_eq!(seen(&mut e, &teams), same, "{visibility} from its own");
        }
        // The invisible: not named, but for spectators and for a team that sees its own.
        e.set_data(9, &[(0, DataValue::Byte(0x20))], &types());
        let mut teams = Teams::default();
        assert!(!seen(&mut e, &teams));
        assert!(name(&mut e, 9, &teams, true).is_some());
        teams.apply("red", 0, Some(&team(0, 2)), &kai);
        assert!(!seen(&mut e, &teams));
        teams.apply("red", 3, None, &me);
        assert!(seen(&mut e, &teams));
        teams.apply("red", 2, Some(&team(0, 0)), &[]);
        assert!(!seen(&mut e, &teams));
        // Hidden from other teams, a team that sees its own shows them regardless.
        teams.apply("red", 2, Some(&team(2, 2)), &[]);
        assert!(seen(&mut e, &teams));
        teams.apply("red", 2, Some(&team(3, 2)), &[]);
        assert!(!seen(&mut e, &teams));
    }

    #[test]
    fn one_who_carries_another_has_no_name_shown() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        add(&mut e, 8, ZOMBIE, [5.5, -60.0, 5.5]);
        let mut teams = Teams::default();
        assert!(name(&mut e, 9, &teams, false).is_some());
        e.set_passengers(9, &[8]);
        assert!(name(&mut e, 9, &teams, false).is_none());
        // A team asks no such thing.
        teams.apply("red", 0, Some(&team(0, 0)), &["Kai".to_owned()]);
        assert!(name(&mut e, 9, &teams, false).is_some());
    }

    #[test]
    fn a_mannequin_shows_the_name_it_was_given_if_told_to() {
        let mut e = Entities::new(PLAYER_ID, false);
        let p = play::AddEntity {
            entity_id: 9,
            uuid: 1009,
            entity_type: MANNEQUIN,
            position: [5.5, -60.0, 5.5],
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: 0,
        };
        e.add(&p, &types(), &PlayerInfos::default());
        let teams = Teams::default();
        assert!(name(&mut e, 9, &teams, false).is_none());
        // Told to show a name it does not have.
        e.set_data(9, &[(3, DataValue::Boolean(true))], &types());
        assert!(name(&mut e, 9, &teams, false).is_none());
        let given = DataValue::OptionalComponent(Some(text("Guard")));
        e.set_data(9, &[(2, given)], &types());
        let shown = name(&mut e, 9, &teams, false).unwrap();
        assert_eq!(shown.text.name, text("Guard"));
        // Below it what it is described as: a label until it is given another or none.
        let label = shown.text.below.as_ref().unwrap().as_compound().unwrap();
        assert_eq!(label.string("translate"), Some(MANNEQUIN_LABEL));
        let described = DataValue::OptionalComponent(Some(text("of the gate")));
        e.set_data(9, &[(19, described)], &types());
        let shown = name(&mut e, 9, &teams, false).unwrap();
        assert_eq!(shown.text.below, Some(text("of the gate")));
        e.set_data(9, &[(19, DataValue::OptionalComponent(None))], &types());
        assert_eq!(name(&mut e, 9, &teams, false).unwrap().text.below, None);
        e.set_data(9, &[(3, DataValue::Boolean(false))], &types());
        assert!(name(&mut e, 9, &teams, false).is_none());
        // Its team is the one of its id.
        let mut teams = Teams::default();
        teams.apply("red", 0, Some(&team(0, 0)), &[uuid_string(1009)]);
        e.set_data(9, &[(3, DataValue::Boolean(true))], &types());
        let shown = name(&mut e, 9, &teams, false).unwrap();
        let c = shown.text.name.as_compound().unwrap();
        assert_eq!(c.list("extra")[1], text("Guard"));
    }

    #[test]
    fn names_are_seen_from_as_far_as_the_attributes_say() {
        use ruthenium_data::v26_3::registries::Attribute as Registered;
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let id = |name: &str| {
            (0..200)
                .find(|i| Registered::from_id(*i).is_some_and(|a| a.name() == name))
                .unwrap() as i32
        };
        let snapshot = |name: &str, base: f64, modifiers| play::AttributeSnapshot {
            attribute: id(name),
            base,
            modifiers,
        };
        e.update_attributes(
            9,
            &[
                snapshot("minecraft:name_tag_distance", 20.0, Vec::new()),
                snapshot(
                    "minecraft:below_name_distance",
                    10.0,
                    vec![("a".to_owned(), 1.0, 2), ("b".to_owned(), 6.0, 0)],
                ),
            ],
        );
        let shown = name(&mut e, 9, &Teams::default(), false).unwrap();
        assert_eq!((shown.distance, shown.below_distance), (20.0, 32.0));
        // Never farther than the attribute goes.
        e.update_attributes(
            9,
            &[snapshot("minecraft:name_tag_distance", 9000.0, Vec::new())],
        );
        let shown = name(&mut e, 9, &Teams::default(), false).unwrap();
        assert_eq!(shown.distance, 512.0);
    }

    fn record(e: &Entities, id: i32, teams: &Teams, spectator: bool) -> Option<AvatarRecord> {
        let found = e.get(id).unwrap();
        e.avatar_record(found, teams, spectator, "Developer")
            .map(|r| *r)
    }

    #[test]
    fn a_players_pose_is_kept_for_drawing() {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        add(&mut e, 8, ZOMBIE, [8.5, -60.0, 5.5]);
        let mut player = player_at([0.5, -60.0, 0.5]);
        let none = Teams::default();
        assert!(record(&e, 8, &none, false).is_none());
        let r = record(&e, 9, &none, false).unwrap();
        let SkinSource::Player { id, textures } = &*r.skin else {
            panic!("{:?}", r.skin);
        };
        assert_eq!(
            (*id, textures.as_ref().unwrap().value.as_str()),
            (1009, "e30=")
        );
        // It faces the way its head does.
        assert_eq!((r.head_rot, r.body_rot), ((90.0, 90.0), (90.0, 90.0)));
        assert!((r.eye_height - 1.62).abs() < 1e-6);
        e.rotate_head(9, 0);
        e.damaged(9);
        e.swing(
            9,
            Swing {
                off_hand: true,
                kind: 1,
                duration: 6,
            },
        );
        tick(&mut e, &none, &mut player);
        let r = record(&e, 9, &none, false).unwrap();
        assert!(r.hurt && r.swing.is_some_and(|s| s.off_hand));
        assert!((r.head_rot.1 - 60.0).abs() < 1e-3, "{:?}", r.head_rot);
        assert_eq!(r.age, 1);
        // Nothing of the kind for others, and nothing at all for those not there.
        e.rotate_head(8, 0);
        e.hurt(77);
        // Dead, and out of the server's reach: its death waits.
        e.entities.get_mut(&9).unwrap().health = 0.0;
        e.simulation_distance = -1;
        tick(&mut e, &none, &mut player);
        let r = record(&e, 9, &none, false).unwrap();
        assert!(!r.alive && r.death_time == 0);
        e.simulation_distance = 4;
        tick(&mut e, &none, &mut player);
        assert_eq!(record(&e, 9, &none, false).unwrap().death_time, 1);
    }

    #[test]
    fn what_a_player_holds_and_wears_shows() {
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let stack = |item: Item| ruthenium_protocol::item::ItemStack {
            item: item.id() as i32,
            count: 1,
            patch: Default::default(),
        };
        let items = Items::default();
        let teams = Teams::default();
        let wear = |e: &mut Entities, slot, s| {
            e.set_worn(9, &[(slot, s)], &items);
            record(e, 9, &teams, false).unwrap()
        };
        let r = wear(&mut e, play::EQUIPMENT_OFFHAND, stack(Item::Stone));
        assert_eq!(
            (r.left_arm, r.right_arm),
            (crate::avatar::ArmPose::Item, crate::avatar::ArmPose::Empty)
        );
        assert_eq!(
            wear(&mut e, play::EQUIPMENT_CHEST, stack(Item::IronChestplate)).chest,
            Chest::Armour
        );
        assert_eq!(
            wear(&mut e, play::EQUIPMENT_CHEST, stack(Item::Elytra)).chest,
            Chest::Wings
        );
        let none = ruthenium_protocol::item::ItemStack {
            count: 0,
            ..stack(Item::Elytra)
        };
        assert_eq!(wear(&mut e, play::EQUIPMENT_CHEST, none).chest, Chest::Bare);
    }

    #[test]
    fn the_invisible_are_seen_by_their_team_and_by_spectators() {
        let mut e = Entities::new(PLAYER_ID, false);
        add_player(&mut e, 9, "Kai", [5.5, -60.0, 5.5]);
        let mut teams = Teams::default();
        let team = |options| play::TeamParameters {
            display_name: Tag::String(String::new()),
            prefix: Tag::String(String::new()),
            suffix: Tag::String(String::new()),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: None,
            options,
        };
        let kai = ["Kai".to_owned()];
        teams.apply("red", 0, Some(&team(2)), &kai);
        assert!(!record(&e, 9, &teams, false).unwrap().hidden);
        e.set_data(9, &[(0, DataValue::Byte(0x20))], &types());
        let r = record(&e, 9, &teams, false).unwrap();
        assert!(r.invisible && r.hidden);
        assert!(!record(&e, 9, &teams, true).unwrap().hidden);
        teams.apply("red", 3, None, &["Developer".to_owned()]);
        assert!(!record(&e, 9, &teams, false).unwrap().hidden);
        // Not if the team does not see its own.
        teams.apply("red", 2, Some(&team(1)), &[]);
        assert!(record(&e, 9, &teams, false).unwrap().hidden);
        // Another team's.
        teams.apply("red", 2, Some(&team(2)), &[]);
        teams.apply("blue", 0, Some(&team(2)), &["Developer".to_owned()]);
        assert!(record(&e, 9, &teams, false).unwrap().hidden);
    }
}
