//! Which block states are drawn as block entities, from which model, texture and place.
//! (Beds and sign boards are ordinary block models in 26.3: the mesher draws them.)

use std::sync::OnceLock;

use ruthenium_data::v26_3::blocks::{BlockState, STATE_COUNT};
use ruthenium_sim::block::state::{get, get_dir, get_int};
use ruthenium_sim::direction::Direction;

use super::affine::Affine;
use super::model::{Model, Skull};

/// The sixteen dyes' names, in the order of their ids.
pub const DYES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// The colour each dye gives a texture, as 0xRRGGBB.
pub const DYE_COLOURS: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

/// A dye by its name.
pub fn dye(name: &str) -> Option<u8> {
    DYES.iter().position(|d| *d == name).map(|i| i as u8)
}

/// What a chest is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Chest {
    Normal,
    Trapped,
    Ender,
    /// How far it has weathered, 0 to 3.
    Copper(u8),
}

/// A block drawn as a block entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Chest(Chest),
    /// Its dye, if it has one.
    Shulker(Option<u8>),
    Skull {
        skull: Skull,
        wall: bool,
    },
    /// Its cloth's dye.
    Banner {
        dye: u8,
        wall: bool,
    },
}

impl Kind {
    /// By the block's name.
    pub fn of_block(name: &str) -> Option<Kind> {
        let name = name.strip_prefix("minecraft:")?;
        let chest = |c| Some(Kind::Chest(c));
        match name {
            "chest" => return chest(Chest::Normal),
            "trapped_chest" => return chest(Chest::Trapped),
            "ender_chest" => return chest(Chest::Ender),
            "shulker_box" => return Some(Kind::Shulker(None)),
            _ => {}
        }
        match name.strip_prefix("waxed_").unwrap_or(name) {
            "copper_chest" => return chest(Chest::Copper(0)),
            "exposed_copper_chest" => return chest(Chest::Copper(1)),
            "weathered_copper_chest" => return chest(Chest::Copper(2)),
            "oxidized_copper_chest" => return chest(Chest::Copper(3)),
            _ => {}
        }
        if let Some(colour) = name.strip_suffix("_shulker_box") {
            return Some(Kind::Shulker(Some(dye(colour)?)));
        }
        if let Some(rest) = name.strip_suffix("_banner") {
            let (colour, wall) = match rest.strip_suffix("_wall") {
                Some(colour) => (colour, true),
                None => (rest, false),
            };
            return Some(Kind::Banner {
                dye: dye(colour)?,
                wall,
            });
        }
        let rest = name
            .strip_suffix("_skull")
            .or_else(|| name.strip_suffix("_head"))?;
        let (mob, wall) = match rest.strip_suffix("_wall") {
            Some(mob) => (mob, true),
            None => (rest, false),
        };
        let skull = match mob {
            "skeleton" => Skull::Skeleton,
            "wither_skeleton" => Skull::WitherSkeleton,
            "player" => Skull::Player,
            "zombie" => Skull::Zombie,
            "creeper" => Skull::Creeper,
            "piglin" => Skull::Piglin,
            "dragon" => Skull::Dragon,
            // A piston's.
            _ => return None,
        };
        Some(Kind::Skull { skull, wall })
    }

    /// Of a block state.
    pub fn of(state: u32) -> Option<Kind> {
        static KINDS: OnceLock<Vec<Option<Kind>>> = OnceLock::new();
        KINDS
            .get_or_init(|| {
                (0..STATE_COUNT as u32)
                    .map(|id| Kind::of_block(BlockState::from_id(id)?.block().name()))
                    .collect()
            })
            .get(state as usize)
            .copied()
            .flatten()
    }
}

/// Radians of minus an angle in degrees: a model turned to face the way of a yaw.
fn turned(degrees: f32) -> f32 {
    -degrees.to_radians()
}

/// The degrees of a sixteenth of a turn (`rotation`).
fn sixteenths(state: u32) -> f32 {
    get_int(state, "rotation") as f32 * 22.5
}

/// Which half of a chest a state is, as its model and the suffix of its texture.
pub fn chest_half(state: u32) -> (Model, &'static str) {
    match get(state, "type") {
        Some("left") => (Model::ChestLeft, "_left"),
        Some("right") => (Model::ChestRight, "_right"),
        _ => (Model::Chest, ""),
    }
}

/// Which way a double chest's other half lies; none for a single one.
pub fn other_half(state: u32) -> Option<Direction> {
    let facing = get_dir(state, "facing")?;
    match get(state, "type")? {
        "left" => Some(facing.clockwise()),
        "right" => Some(facing.counter_clockwise()),
        _ => None,
    }
}

/// Whether `other` is the other half of the double chest `state` is a half of, given that it
/// lies where that would be.
pub fn is_other_half(state: u32, other: u32) -> bool {
    let block = |s| BlockState::from_id(s).map(|s| s.block());
    block(state) == block(other)
        && get(state, "facing") == get(other, "facing")
        && matches!(
            (get(state, "type"), get(other, "type")),
            (Some("left"), Some("right")) | (Some("right"), Some("left"))
        )
}

/// From a model's space to the block's (its corner at the origin).
pub fn transform(kind: Kind, state: u32) -> Affine {
    let facing = get_dir(state, "facing");
    let yaw = || facing.unwrap_or(Direction::South).to_y_rot();
    let i = Affine::IDENTITY;
    match kind {
        Kind::Chest(_) => i
            .translate([0.5, 0.0, 0.5])
            .rotate_y(turned(yaw()))
            .translate([-0.5, 0.0, -0.5]),
        Kind::Shulker(_) => {
            let t = i.translate([0.5; 3]).scale([0.9995; 3]);
            faced(t, facing.unwrap_or(Direction::Up))
                .scale([1.0, -1.0, -1.0])
                .translate([0.0, -1.0, 0.0])
        }
        Kind::Skull { wall: false, .. } => i
            .translate([0.5, 0.0, 0.5])
            .rotate_y(turned(sixteenths(state)))
            .scale([-1.0, -1.0, 1.0]),
        Kind::Skull { wall: true, .. } => {
            let facing = facing.unwrap_or(Direction::North);
            let n = facing.normal().map(|c| c as f32);
            i.translate([0.5 - n[0] * 0.25, 0.25, 0.5 - n[2] * 0.25])
                .rotate_y(turned(facing.opposite().to_y_rot()))
                .scale([-1.0, -1.0, 1.0])
        }
        Kind::Banner { wall, .. } => {
            let angle = if wall { yaw() } else { sixteenths(state) };
            const SIZE: f32 = 2.0 / 3.0;
            i.translate([0.5, 0.0, 0.5])
                .rotate_y(turned(angle))
                .scale([SIZE, -SIZE, -SIZE])
        }
    }
}

/// Turns what points up to point towards `facing` (`Direction.getRotation`).
fn faced(t: Affine, facing: Direction) -> Affine {
    use std::f32::consts::{FRAC_PI_2, PI};
    match facing {
        Direction::Up => t,
        Direction::Down => t.rotate_x(PI),
        Direction::South => t.rotate_x(FRAC_PI_2),
        Direction::North => t.rotate_x(FRAC_PI_2).rotate_z(PI),
        Direction::West => t.rotate_x(FRAC_PI_2).rotate_z(FRAC_PI_2),
        Direction::East => t.rotate_x(FRAC_PI_2).rotate_z(-FRAC_PI_2),
    }
}

/// The phase a banner's cloth waves at.
pub fn banner_phase(pos: [i32; 3], game_time: i64, partial: f32) -> f32 {
    let [x, y, z] = pos;
    let place = x
        .wrapping_mul(7)
        .wrapping_add(y.wrapping_mul(9))
        .wrapping_add(z.wrapping_mul(13));
    let ticks = i64::from(place).wrapping_add(game_time).rem_euclid(100);
    (ticks as f32 + partial) / 100.0
}

/// A chest's texture below `textures/entity/chest`, without the half's suffix.
pub fn chest_texture(chest: Chest, christmas: bool) -> &'static str {
    match chest {
        Chest::Copper(0) => "copper",
        Chest::Copper(1) => "copper_exposed",
        Chest::Copper(2) => "copper_weathered",
        Chest::Copper(_) => "copper_oxidized",
        Chest::Ender => "ender",
        Chest::Normal | Chest::Trapped if christmas => "christmas",
        Chest::Trapped => "trapped",
        Chest::Normal => "normal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str, properties: &[(&str, &str)]) -> u32 {
        (0..STATE_COUNT as u32)
            .find(|&id| {
                let s = BlockState::from_id(id).unwrap();
                s.block().name() == name && properties.iter().all(|(p, v)| get(id, p) == Some(*v))
            })
            .unwrap()
    }

    #[test]
    fn kinds_go_by_the_blocks_names() {
        let of = |n: &str| Kind::of_block(&format!("minecraft:{n}"));
        assert_eq!(of("chest"), Some(Kind::Chest(Chest::Normal)));
        assert_eq!(
            of("waxed_weathered_copper_chest"),
            Some(Kind::Chest(Chest::Copper(2)))
        );
        assert_eq!(of("light_blue_shulker_box"), Some(Kind::Shulker(Some(3))));
        assert_eq!(
            of("black_wall_banner"),
            Some(Kind::Banner {
                dye: 15,
                wall: true
            })
        );
        assert_eq!(
            of("wither_skeleton_wall_skull"),
            Some(Kind::Skull {
                skull: Skull::WitherSkeleton,
                wall: true
            })
        );
        assert_eq!(of("piston_head"), None);
        assert_eq!(of("white_bed"), None);
        assert_eq!(of("oak_sign"), None);
        // Every state of every block there is: 4 + 8 chests... counted by block.
        let mut blocks: Vec<&str> = (0..STATE_COUNT as u32)
            .filter(|&id| Kind::of(id).is_some())
            .map(|id| BlockState::from_id(id).unwrap().block().name())
            .collect();
        blocks.dedup();
        // 3 chests and 8 copper ones, 17 shulker boxes, 14 skulls, 32 banners.
        assert_eq!(blocks.len(), 3 + 8 + 17 + 14 + 32);
    }

    #[test]
    fn transforms_are_the_games() {
        use super::super::golden::TRANSFORMS;
        for (kind, key, want) in TRANSFORMS {
            let (block, property) = match kind {
                "chest" => ("minecraft:ender_chest", "facing"),
                "shulker" => ("minecraft:red_shulker_box", "facing"),
                "skull" => ("minecraft:zombie_head", "rotation"),
                "wall_skull" => ("minecraft:zombie_wall_head", "facing"),
                "banner" => ("minecraft:white_banner", "rotation"),
                "wall_banner" => ("minecraft:white_wall_banner", "facing"),
                other => panic!("{other}"),
            };
            let value = if property == "rotation" {
                key.to_string()
            } else {
                Direction::from_id(key as u8).unwrap().name().to_owned()
            };
            let state = state(block, &[(property, &value)]);
            let got = transform(Kind::of(state).unwrap(), state);
            for (g, w) in got.0.as_flattened().iter().zip(want) {
                assert!(
                    (g - w).abs() < 1e-6,
                    "{kind} {value}: {got:?}, not {want:?}"
                );
            }
        }
    }

    #[test]
    fn halves_find_each_other() {
        let left = state("minecraft:chest", &[("facing", "north"), ("type", "left")]);
        let right = state("minecraft:chest", &[("facing", "north"), ("type", "right")]);
        let single = state(
            "minecraft:chest",
            &[("facing", "north"), ("type", "single")],
        );
        assert_eq!(other_half(left), Some(Direction::East));
        assert_eq!(other_half(right), Some(Direction::West));
        assert_eq!(other_half(single), None);
        assert!(is_other_half(left, right) && is_other_half(right, left));
        assert!(!is_other_half(left, left) && !is_other_half(left, single));
        let away = state("minecraft:chest", &[("facing", "south"), ("type", "right")]);
        let trapped = state(
            "minecraft:trapped_chest",
            &[("facing", "north"), ("type", "right")],
        );
        assert!(!is_other_half(left, away) && !is_other_half(left, trapped));
        assert_eq!(chest_half(left), (Model::ChestLeft, "_left"));
        assert_eq!(chest_half(single), (Model::Chest, ""));
        let ender = state("minecraft:ender_chest", &[]);
        assert_eq!((other_half(ender), chest_half(ender).1), (None, ""));
    }

    #[test]
    fn banners_wave_by_their_place_and_the_time() {
        assert_eq!(banner_phase([0, 0, 0], 25, 0.5), 0.255);
        // Never negative: the remainder is a floored one.
        assert_eq!(banner_phase([-1, 0, 0], 0, 0.0), 0.93);
        assert_eq!(banner_phase([1, 1, 1], 100, 0.0), 0.29);
    }

    #[test]
    fn christmas_chests_are_only_the_plain_ones() {
        assert_eq!(chest_texture(Chest::Trapped, true), "christmas");
        assert_eq!(chest_texture(Chest::Ender, true), "ender");
        assert_eq!(chest_texture(Chest::Copper(3), true), "copper_oxidized");
        assert_eq!(chest_texture(Chest::Trapped, false), "trapped");
    }
}
