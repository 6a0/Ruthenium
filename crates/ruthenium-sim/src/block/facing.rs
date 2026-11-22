//! Blocks that face the player or hang on a face: dispensers, furnaces, pumpkins, terracotta, anvils, workstations, hoppers, observers, crafters, sculk sensors and shriekers, shelves, statues, vaults, decorated pots, bookshelves, beehives, campfires, bells, grindstones, wall-or-floor blocks (levers and buttons share their placement), lecterns, skulls, banners and signs.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Most of these blocks keep what they hold in a block entity, which this client
//! doesn't track. The vanilla client always has one for them (made when the block
//! is set), so the ports below act as if it is there, with nothing known of what it
//! holds; where an answer would depend on its contents, the doc comment says what
//! the port assumes.

#![allow(unused_variables)]

use super::{Behaviour, Class, Pos, Support, Update, Use, state};
use crate::direction::Direction;
use crate::fluid::{Fluid, FluidState};
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::item::Stack;
use crate::place::PlaceContext;
use crate::player::LocalPlayer;
use crate::shape::Axis;

/// The (class, behaviour) pairs this group answers for; every pair the export lists
/// must be in some group's list (see [`super`]'s coverage test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PORTED: &[(Class, Behaviour)] = &[
    (Class::DispenserBlock, Behaviour::GetStateForPlacement),
    (Class::DispenserBlock, Behaviour::UseWithoutItem),
    (Class::AbstractFurnaceBlock, Behaviour::GetStateForPlacement),
    (Class::AbstractFurnaceBlock, Behaviour::UseWithoutItem),
    (Class::CarvedPumpkinBlock, Behaviour::GetStateForPlacement),
    (
        Class::GlazedTerracottaBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::AnvilBlock, Behaviour::GetStateForPlacement),
    (Class::AnvilBlock, Behaviour::UseWithoutItem),
    (Class::LoomBlock, Behaviour::GetStateForPlacement),
    (Class::LoomBlock, Behaviour::UseWithoutItem),
    (Class::BarrelBlock, Behaviour::GetStateForPlacement),
    (Class::BarrelBlock, Behaviour::UseWithoutItem),
    (Class::StonecutterBlock, Behaviour::GetStateForPlacement),
    (Class::StonecutterBlock, Behaviour::UseWithoutItem),
    (Class::EndPortalFrameBlock, Behaviour::GetStateForPlacement),
    (Class::HopperBlock, Behaviour::GetStateForPlacement),
    (Class::HopperBlock, Behaviour::UseWithoutItem),
    (Class::ObserverBlock, Behaviour::GetStateForPlacement),
    (Class::ObserverBlock, Behaviour::UpdateShape),
    (Class::CrafterBlock, Behaviour::GetStateForPlacement),
    (Class::CrafterBlock, Behaviour::UseWithoutItem),
    (Class::CrafterBlock, Behaviour::SetPlacedBy),
    (Class::DriedGhastBlock, Behaviour::GetStateForPlacement),
    (Class::DriedGhastBlock, Behaviour::UpdateShape),
    (Class::DriedGhastBlock, Behaviour::SetPlacedBy),
    (Class::CreakingHeartBlock, Behaviour::GetStateForPlacement),
    (Class::CreakingHeartBlock, Behaviour::UpdateShape),
    (Class::PotentSulfurBlock, Behaviour::GetStateForPlacement),
    (Class::PotentSulfurBlock, Behaviour::UpdateShape),
    (Class::SculkSensorBlock, Behaviour::GetStateForPlacement),
    (Class::SculkSensorBlock, Behaviour::UpdateShape),
    (
        Class::CalibratedSculkSensorBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::SculkShriekerBlock, Behaviour::GetStateForPlacement),
    (Class::SculkShriekerBlock, Behaviour::UpdateShape),
    (Class::ShelfBlock, Behaviour::GetStateForPlacement),
    (Class::ShelfBlock, Behaviour::UpdateShape),
    (Class::ShelfBlock, Behaviour::UseItemOn),
    (
        Class::CopperGolemStatueBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::CopperGolemStatueBlock, Behaviour::UpdateShape),
    (Class::CopperGolemStatueBlock, Behaviour::UseItemOn),
    (
        Class::WeatheringCopperGolemStatueBlock,
        Behaviour::UseItemOn,
    ),
    (Class::VaultBlock, Behaviour::GetStateForPlacement),
    (Class::VaultBlock, Behaviour::UseItemOn),
    (Class::DecoratedPotBlock, Behaviour::GetStateForPlacement),
    (Class::DecoratedPotBlock, Behaviour::UpdateShape),
    (Class::DecoratedPotBlock, Behaviour::UseItemOn),
    (Class::DecoratedPotBlock, Behaviour::UseWithoutItem),
    (
        Class::ChiseledBookShelfBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::ChiseledBookShelfBlock, Behaviour::UseItemOn),
    (Class::ChiseledBookShelfBlock, Behaviour::UseWithoutItem),
    (Class::BeehiveBlock, Behaviour::GetStateForPlacement),
    (Class::BeehiveBlock, Behaviour::UpdateShape),
    (Class::BeehiveBlock, Behaviour::UseItemOn),
    (Class::CampfireBlock, Behaviour::GetStateForPlacement),
    (Class::CampfireBlock, Behaviour::UpdateShape),
    (Class::CampfireBlock, Behaviour::UseItemOn),
    (Class::BellBlock, Behaviour::GetStateForPlacement),
    (Class::BellBlock, Behaviour::UpdateShape),
    (Class::BellBlock, Behaviour::CanSurvive),
    (Class::BellBlock, Behaviour::UseWithoutItem),
    (Class::GrindstoneBlock, Behaviour::CanSurvive),
    (Class::GrindstoneBlock, Behaviour::UseWithoutItem),
    (
        Class::FaceAttachedHorizontalDirectionalBlock,
        Behaviour::GetStateForPlacement,
    ),
    (
        Class::FaceAttachedHorizontalDirectionalBlock,
        Behaviour::UpdateShape,
    ),
    (
        Class::FaceAttachedHorizontalDirectionalBlock,
        Behaviour::CanSurvive,
    ),
    (Class::LecternBlock, Behaviour::GetStateForPlacement),
    (Class::LecternBlock, Behaviour::UseItemOn),
    (Class::LecternBlock, Behaviour::UseWithoutItem),
    (Class::SkullBlock, Behaviour::GetStateForPlacement),
    (Class::WallSkullBlock, Behaviour::GetStateForPlacement),
    (Class::WitherSkullBlock, Behaviour::SetPlacedBy),
    (Class::WitherWallSkullBlock, Behaviour::SetPlacedBy),
    (Class::BannerBlock, Behaviour::GetStateForPlacement),
    (Class::BannerBlock, Behaviour::UpdateShape),
    (Class::BannerBlock, Behaviour::CanSurvive),
    (Class::WallBannerBlock, Behaviour::GetStateForPlacement),
    (Class::WallBannerBlock, Behaviour::UpdateShape),
    (Class::WallBannerBlock, Behaviour::CanSurvive),
    (Class::StandingSignBlock, Behaviour::GetStateForPlacement),
    (Class::StandingSignBlock, Behaviour::UpdateShape),
    (Class::StandingSignBlock, Behaviour::CanSurvive),
    (Class::WallSignBlock, Behaviour::GetStateForPlacement),
    (Class::WallSignBlock, Behaviour::UpdateShape),
    (Class::WallSignBlock, Behaviour::CanSurvive),
    (
        Class::CeilingHangingSignBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::CeilingHangingSignBlock, Behaviour::UpdateShape),
    (Class::CeilingHangingSignBlock, Behaviour::CanSurvive),
    (Class::CeilingHangingSignBlock, Behaviour::UseItemOn),
    (Class::WallHangingSignBlock, Behaviour::GetStateForPlacement),
    (Class::WallHangingSignBlock, Behaviour::UpdateShape),
    (Class::WallHangingSignBlock, Behaviour::UseItemOn),
    (Class::SignBlock, Behaviour::UseItemOn),
    (Class::SignBlock, Behaviour::UseWithoutItem),
    (Class::SignBlock, Behaviour::SetPlacedBy),
];

/// `getStateForPlacement` (`Some(None)`: the block can't be placed there).
pub(super) fn state_for_placement(
    c: Class,
    w: &World<'_>,
    ctx: &PlaceContext,
    default: u32,
) -> Option<Option<u32>> {
    let pos = ctx.clicked_pos();
    // Most of these blocks turn their front to the player.
    let to_player = ctx.horizontal_direction().opposite();
    let placed = match c {
        Class::DispenserBlock | Class::BarrelBlock => {
            facing(default, ctx.nearest_looking_direction().opposite())
        }
        // The observer's face points where the player looks (watching what they
        // see), its output back at them.
        Class::ObserverBlock => facing(default, ctx.nearest_looking_direction()),
        Class::AbstractFurnaceBlock
        | Class::CarvedPumpkinBlock
        | Class::GlazedTerracottaBlock
        | Class::LoomBlock
        | Class::StonecutterBlock
        | Class::VaultBlock
        | Class::ChiseledBookShelfBlock
        | Class::BeehiveBlock => facing(default, to_player),
        Class::EndPortalFrameBlock => state::set_bool(facing(default, to_player), "eye", false),
        Class::AnvilBlock => facing(default, ctx.horizontal_direction().clockwise()),
        Class::HopperBlock => hopper_placement(ctx, default),
        Class::CrafterBlock => crafter_placement(w, ctx, default),
        Class::DriedGhastBlock | Class::CopperGolemStatueBlock => {
            waterlogged_by(w, pos, facing(default, to_player))
        }
        Class::CreakingHeartBlock => creaking_heart_placement(w, ctx, default),
        Class::PotentSulfurBlock => potent_sulfur_state(w, default, pos),
        Class::SculkSensorBlock | Class::SculkShriekerBlock => waterlogged_by(w, pos, default),
        Class::CalibratedSculkSensorBlock => {
            facing(waterlogged_by(w, pos, default), ctx.horizontal_direction())
        }
        Class::ShelfBlock => {
            let s = facing(default, to_player);
            let s = state::set_bool(s, "powered", w.has_neighbor_signal(pos));
            waterlogged_by(w, pos, s)
        }
        Class::DecoratedPotBlock => {
            let s = facing(default, ctx.horizontal_direction());
            state::set_bool(waterlogged_by(w, pos, s), "cracked", false)
        }
        Class::CampfireBlock => campfire_placement(w, ctx, default),
        Class::BellBlock => return Some(bell_placement(w, ctx, default)),
        Class::FaceAttachedHorizontalDirectionalBlock => {
            return Some(face_attached_placement(w, ctx, default));
        }
        // A lectern placed by a client never starts with a book: only the server
        // reads a book from the item's block entity data.
        Class::LecternBlock => state::set_bool(facing(default, to_player), "has_book", false),
        Class::SkullBlock => {
            let s = skull_powered(w, pos, default);
            state::set_int(s, "rotation", rotation_segment(ctx.rotation()))
        }
        Class::WallSkullBlock => return Some(wall_skull_placement(w, ctx, default)),
        Class::BannerBlock => state::set_int(
            default,
            "rotation",
            rotation_segment(ctx.rotation() + 180.0),
        ),
        Class::WallBannerBlock => return Some(wall_placement(w, ctx, default)),
        Class::StandingSignBlock => {
            let s = state::set_int(
                default,
                "rotation",
                rotation_segment(ctx.rotation() + 180.0),
            );
            waterlogged_by(w, pos, s)
        }
        Class::WallSignBlock => {
            return Some(wall_placement(w, ctx, default).map(|s| waterlogged_by(w, pos, s)));
        }
        Class::CeilingHangingSignBlock => ceiling_hanging_sign_placement(w, ctx, default),
        Class::WallHangingSignBlock => return Some(wall_hanging_sign_placement(w, ctx, default)),
        _ => return None,
    };
    Some(Some(placed))
}

/// `updateShape`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    let s = u.state;
    let unless_unsupported = |towards: Direction| {
        if u.direction == towards && !super::can_survive(w, s, u.pos) {
            state::AIR
        } else {
            s
        }
    };
    Some(match c {
        // These only schedule ticks (water flowing in, an observer's pulse, a
        // creaking heart's check) or tell a block entity, none of which the client
        // does; the block stays as it is.
        Class::ObserverBlock
        | Class::DriedGhastBlock
        | Class::CreakingHeartBlock
        | Class::SculkSensorBlock
        | Class::SculkShriekerBlock
        | Class::ShelfBlock
        | Class::CopperGolemStatueBlock
        | Class::DecoratedPotBlock
        | Class::BeehiveBlock
        | Class::SignBlock => s,
        Class::PotentSulfurBlock => potent_sulfur_state(w, s, u.pos),
        // `CampfireBlock.updateShape`: a hay bale below makes it a signal fire.
        Class::CampfireBlock if u.direction == Direction::Down => {
            state::set_bool(s, "signal_fire", is_smoke_source(u.neighbour))
        }
        Class::CampfireBlock => s,
        Class::BellBlock => bell_update(w, u),
        Class::FaceAttachedHorizontalDirectionalBlock => {
            unless_unsupported(attached_towards(s).opposite())
        }
        Class::BannerBlock | Class::StandingSignBlock => unless_unsupported(Direction::Down),
        Class::WallBannerBlock | Class::WallSignBlock => {
            unless_unsupported(facing_of(s).opposite())
        }
        Class::CeilingHangingSignBlock => unless_unsupported(Direction::Up),
        // `WallHangingSignBlock.updateShape`: a neighbour along the sign's beam (to
        // either side) may take its hold away.
        Class::WallHangingSignBlock => {
            let beam = facing_of(s).clockwise().axis();
            if u.direction.axis() == beam && !super::can_survive(w, s, u.pos) {
                state::AIR
            } else {
                s
            }
        }
        _ => return None,
    })
}

/// `canSurvive`.
pub(super) fn can_survive(c: Class, w: &World<'_>, state: u32, pos: Pos) -> Option<bool> {
    Some(match c {
        Class::BellBlock => bell_can_survive(w, state, pos),
        // `GrindstoneBlock.canSurvive`: a grindstone stays anywhere.
        Class::GrindstoneBlock => true,
        Class::FaceAttachedHorizontalDirectionalBlock => {
            can_attach(w, pos, attached_towards(state).opposite())
        }
        // `BannerBlock`, `StandingSignBlock.canSurvive`: something solid below.
        Class::BannerBlock | Class::StandingSignBlock => {
            w.blocks.solid(w.get(Direction::Down.step(pos)))
        }
        // `WallBannerBlock`, `WallSignBlock.canSurvive`: something solid behind.
        Class::WallBannerBlock | Class::WallSignBlock => {
            w.blocks.solid(w.get(facing_of(state).opposite().step(pos)))
        }
        // `CeilingHangingSignBlock.canSurvive`: the block above holds it by the
        // centre of its bottom face.
        Class::CeilingHangingSignBlock => {
            w.face_sturdy(Direction::Up.step(pos), Direction::Down, Support::Center)
        }
        _ => return None,
    })
}

/// `canBeReplaced(state, context)`.
pub(super) fn can_be_replaced(
    c: Class,
    w: &World<'_>,
    state: u32,
    ctx: &PlaceContext,
) -> Option<bool> {
    None
}

/// `canBeReplaced(state, fluid)` (`water`: the fluid is water, else lava).
pub(super) fn can_be_replaced_by_fluid(
    c: Class,
    w: &World<'_>,
    state: u32,
    water: bool,
) -> Option<bool> {
    None
}

/// `useItemOn`.
pub(super) fn use_item_on(
    c: Class,
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    u: &Use<'_>,
) -> Option<Outcome> {
    let stack = p.hand(u.hand).clone();
    Some(match c {
        Class::ShelfBlock => shelf_use(p, u),
        Class::CopperGolemStatueBlock => {
            if w.item_is(&stack, "minecraft:axes") {
                Outcome::Pass
            } else {
                next_statue_pose(w, u)
            }
        }
        Class::WeatheringCopperGolemStatueBlock => weathering_statue_use(w, &stack, u),
        // `VaultBlock.useItemOn`: an active vault takes any item as a try at its
        // key, which the server judges.
        Class::VaultBlock => {
            if !stack.is_empty() && state::get(u.state, "vault_state") == Some("active") {
                Outcome::Success
            } else {
                Outcome::TryEmptyHand
            }
        }
        // `DecoratedPotBlock.useItemOn`: the client takes every click; whether the
        // item goes in is the server's.
        Class::DecoratedPotBlock => Outcome::Success,
        Class::ChiseledBookShelfBlock => bookshelf_use_item(w, &stack, u),
        Class::BeehiveBlock => beehive_use(w, p, &stack, u),
        Class::CampfireBlock => campfire_use(w, &stack, u),
        Class::LecternBlock => lectern_use_item(w, &stack, u),
        Class::CeilingHangingSignBlock => {
            if stack.c.is("HangingSignItem") && u.hit.direction == Direction::Down {
                Outcome::Pass
            } else {
                sign_use_item()
            }
        }
        Class::WallHangingSignBlock => {
            let edited_side = u.hit.direction.axis() == facing_of(u.state).axis();
            if stack.c.is("HangingSignItem") && !edited_side {
                Outcome::Pass
            } else {
                sign_use_item()
            }
        }
        Class::SignBlock => sign_use_item(),
        _ => return None,
    })
}

/// `useWithoutItem`.
pub(super) fn use_without_item(
    c: Class,
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    u: &Use<'_>,
) -> Option<Outcome> {
    Some(match c {
        // Containers and workstations: the server opens the menu; the client only
        // takes the click.
        Class::DispenserBlock
        | Class::AbstractFurnaceBlock
        | Class::AnvilBlock
        | Class::LoomBlock
        | Class::BarrelBlock
        | Class::StonecutterBlock
        | Class::HopperBlock
        | Class::CrafterBlock
        | Class::GrindstoneBlock => Outcome::Success,
        // `DecoratedPotBlock.useWithoutItem`: the pot wobbles (its block entity's
        // animation) and the click is taken.
        Class::DecoratedPotBlock => Outcome::Success,
        // `ChiseledBookShelfBlock.useWithoutItem`: a click on a slot of the front is
        // taken (the server hands the book over, if there is one); elsewhere it
        // passes.
        Class::ChiseledBookShelfBlock => {
            if hit_slot(u, 2, 3).is_some() {
                Outcome::Success
            } else {
                Outcome::Pass
            }
        }
        Class::BellBlock => {
            if bell_proper_hit(u) {
                Outcome::Success
            } else {
                Outcome::Pass
            }
        }
        // `LecternBlock.useWithoutItem`: the server opens a lectern's book; the click
        // is taken with or without one.
        Class::LecternBlock => Outcome::Success,
        // `SignBlock.useWithoutItem`: editing and click commands are the server's
        // (it opens the editor); the client takes the click.
        Class::SignBlock => Outcome::Success,
        _ => return None,
    })
}

/// `updateIndirectNeighbourShapes`.
pub(super) fn update_indirect_neighbour_shapes(
    c: Class,
    w: &mut World<'_>,
    state: u32,
    pos: Pos,
    flags: u32,
    limit: i32,
) -> Option<()> {
    None
}

/// `setPlacedBy`.
pub(super) fn set_placed_by(
    c: Class,
    w: &mut World<'_>,
    p: &LocalPlayer,
    state: u32,
    pos: Pos,
    hand: Hand,
) -> Option<()> {
    match c {
        // A triggered crafter schedules its tick, a dried ghast plays its sound, a
        // wither skull looks for a wither to build, a sign opens its editor: all on
        // the server, nothing on the client.
        Class::CrafterBlock
        | Class::DriedGhastBlock
        | Class::WitherSkullBlock
        | Class::WitherWallSkullBlock
        | Class::SignBlock => Some(()),
        _ => None,
    }
}

/// `attack` (a left click on the block as breaking starts).
pub(super) fn attack(
    c: Class,
    w: &mut World<'_>,
    p: &LocalPlayer,
    state: u32,
    pos: Pos,
) -> Option<()> {
    None
}

/// `mayPlaceOn`: whether a plant may stand on `below` (the block under it, at
/// `below_pos`).
pub(super) fn may_place_on(
    c: Class,
    w: &World<'_>,
    plant: u32,
    below: u32,
    below_pos: Pos,
) -> Option<bool> {
    None
}

/// `isValidBonemealTarget`.
pub(super) fn is_valid_bonemeal_target(
    c: Class,
    w: &World<'_>,
    state: u32,
    pos: Pos,
) -> Option<bool> {
    None
}

/// `WallHangingSignBlock.canPlace`: a wall hanging sign has something to hang from
/// at `pos`: on one side or the other along its beam, a block whose facing face is
/// full, or another wall hanging sign lying along the same line.
pub(super) fn wall_hanging_sign_can_place(w: &World<'_>, state: u32, pos: Pos) -> bool {
    let f = facing_of(state);
    let (cw, ccw) = (f.clockwise(), f.counter_clockwise());
    hanging_sign_holds_to(w, f, cw.step(pos), ccw) || hanging_sign_holds_to(w, f, ccw.step(pos), cw)
}

/// `WallHangingSignBlock.canAttachTo`: whether the block at `at` holds a wall
/// hanging sign facing `f` by its `face`.
fn hanging_sign_holds_to(w: &World<'_>, f: Direction, at: Pos, face: Direction) -> bool {
    let s = w.get(at);
    if w.tags.is(s, "minecraft:wall_hanging_signs") {
        return state::get_dir(s, "facing").is_some_and(|d| d.axis() == f.axis());
    }
    w.face_sturdy(at, face, Support::Full)
}

// Placement.

/// A state with its `facing` property set.
fn facing(s: u32, d: Direction) -> u32 {
    state::set_dir(s, "facing", d)
}

/// A state's `facing` (north when it has none).
fn facing_of(s: u32) -> Direction {
    state::get_dir(s, "facing").unwrap_or(Direction::North)
}

/// `FluidState.is(Fluids.WATER)` where the block goes: a water source (not flowing
/// water) is there.
fn water_source_at(w: &World<'_>, pos: Pos) -> bool {
    w.fluid(pos).fluid == Fluid::Water
}

/// The state waterlogged when it replaces a water source.
fn waterlogged_by(w: &World<'_>, pos: Pos, s: u32) -> u32 {
    state::set_bool(s, "waterlogged", water_source_at(w, pos))
}

/// `HopperBlock.getStateForPlacement`: the spout points into the clicked face, or
/// down when the click was on a top or bottom face; it starts enabled.
fn hopper_placement(ctx: &PlaceContext, default: u32) -> u32 {
    let into = ctx.clicked_face().opposite();
    let spout = if into.axis() == Axis::Y {
        Direction::Down
    } else {
        into
    };
    state::set_bool(facing(default, spout), "enabled", true)
}

/// `CrafterBlock.getStateForPlacement`: the front faces back along the player's
/// nearest view direction; its top is up for a front facing sideways, and for a
/// front facing up or down, the player's horizontal facing (away from the player for
/// a front facing up, towards them for one facing down). It starts triggered if
/// powered.
fn crafter_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let front = ctx.nearest_looking_direction().opposite();
    let top = match front {
        Direction::Down => ctx.horizontal_direction().opposite(),
        Direction::Up => ctx.horizontal_direction(),
        _ => Direction::Up,
    };
    let orientation = format!("{}_{}", front.name(), top.name());
    let s = state::set(default, "orientation", &orientation);
    state::set_bool(s, "triggered", w.has_neighbor_signal(ctx.clicked_pos()))
}

/// `CreakingHeartBlock.getStateForPlacement`: the heart's axis follows the
/// clicked face; an uprooted heart placed between two pale oak logs lying along that
/// axis wakes up.
///
/// Whether it wakes awake or dormant depends on the `creaking_active` environment
/// attribute (night in the overworld), which needs the level's time of day; this
/// client doesn't track that, so the port takes it as inactive (dormant), and the
/// server's state follows.
fn creaking_heart_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let axis = ctx.clicked_face().axis();
    let s = state::set(default, "axis", axis_name(axis));
    let pos = ctx.clicked_pos();
    let logs_along = [true, false].into_iter().all(|positive| {
        let n = w.get(Direction::from_axis(axis, positive).step(pos));
        w.tags.is(n, "minecraft:pale_oak_logs") && state::get(n, "axis") == Some(axis_name(axis))
    });
    if logs_along && state::get(s, "creaking_heart_state") == Some("uprooted") {
        state::set(s, "creaking_heart_state", "dormant")
    } else {
        s
    }
}

fn axis_name(a: Axis) -> &'static str {
    match a {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// `PotentSulfurBlock`'s state for where it is (placing and every neighbour
/// change): dry without a water source right above; with one, a block below tagged
/// for continuous eruptions (that isn't flowing fluid) makes it erupt continuously,
/// one tagged for periodic eruptions makes it a dormant geyser (one already erupting
/// stays so), anything else leaves it wet. (The periodic geyser's countdown is its
/// block entity's.)
fn potent_sulfur_state(w: &World<'_>, s: u32, pos: Pos) -> u32 {
    let set = |v: &str| state::set(s, "potent_sulfur_state", v);
    if w.fluid(Direction::Up.step(pos)).fluid != Fluid::Water {
        return set("dry");
    }
    let below = w.get(Direction::Down.step(pos));
    let still = {
        let f = w.blocks.fluid(below);
        f.is_empty() || f.is_source()
    };
    if still
        && w.tags
            .is(below, "minecraft:causes_continuous_geyser_eruptions")
    {
        return set("continuous");
    }
    if still
        && w.tags
            .is(below, "minecraft:causes_periodic_geyser_eruptions")
    {
        if state::get(s, "potent_sulfur_state") == Some("erupting") {
            return s;
        }
        return set("dormant");
    }
    set("wet")
}

/// `CampfireBlock.getStateForPlacement`: lit unless it goes into a water source
/// (then waterlogged), a signal fire over a hay bale, facing the way the player
/// faces.
fn campfire_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let water = water_source_at(w, pos);
    let s = state::set_bool(default, "waterlogged", water);
    let s = state::set_bool(
        s,
        "signal_fire",
        is_smoke_source(w.get(Direction::Down.step(pos))),
    );
    let s = state::set_bool(s, "lit", !water);
    facing(s, ctx.horizontal_direction())
}

/// `CampfireBlock.isSmokeSource`: a hay bale.
fn is_smoke_source(s: u32) -> bool {
    state::is(s, "minecraft:hay_block")
}

/// `AbstractSkullBlock.getStateForPlacement`: powered if a neighbour powers it.
fn skull_powered(w: &World<'_>, pos: Pos, s: u32) -> u32 {
    state::set_bool(s, "powered", w.has_neighbor_signal(pos))
}

/// `WallSkullBlock.getStateForPlacement`: of the horizontal directions the player
/// looks along, nearest first, the first whose next block can't be built over takes
/// the skull's back; none, and it can't go on a wall.
fn wall_skull_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let s = skull_powered(w, pos, default);
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal())
        .find(|d| !super::can_be_replaced(w, w.get(d.step(pos)), ctx))
        .map(|d| facing(s, d.opposite()))
}

/// `WallBannerBlock`, `WallSignBlock.getStateForPlacement`: of the horizontal
/// directions the player looks along, nearest first, the first the block may hang
/// against (its back that way).
fn wall_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal())
        .map(|d| facing(default, d.opposite()))
        .find(|&s| super::can_survive(w, s, pos))
}

/// `WallHangingSignBlock.getStateForPlacement`: of the horizontal directions the
/// player looks along, nearest first, leaving out the clicked face's axis, the first
/// facing (the sign's face towards the player) it may stay at and hang from.
fn wall_hanging_sign_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let clicked_axis = ctx.clicked_face().axis();
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal() && d.axis() != clicked_axis)
        .map(|d| facing(default, d.opposite()))
        .find(|&s| super::can_survive(w, s, pos) && wall_hanging_sign_can_place(w, s, pos))
        .map(|s| waterlogged_by(w, pos, s))
}

/// `CeilingHangingSignBlock.getStateForPlacement`. A sign hangs by its two chains
/// facing the way the player faces (its rotation snapped to that direction) when the
/// bottom of the block above is full; otherwise, or when sneaking, it hangs by one
/// chain (`attached`) at any of the 16 rotations. Under another hanging sign lying
/// along the player's facing (and not sneaking), it always hangs by two chains, to
/// make a column.
fn ceiling_hanging_sign_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let above = w.get(Direction::Up.step(pos));
    let facing_dir = Direction::from_y_rot(f64::from(ctx.rotation()));
    let sneaking = ctx.secondary_use();
    let mut one_chain = !w.blocks.collision_face_full(above, Direction::Down) || sneaking;
    if !sneaking && w.tags.is(above, "minecraft:all_hanging_signs") {
        let above_dir = match state::get_dir(above, "facing") {
            Some(d) => Some(d),
            None if state::has(above, "rotation") => {
                segment_direction(state::get_int(above, "rotation"))
            }
            None => None,
        };
        if above_dir.is_some_and(|d| d.axis() == facing_dir.axis()) {
            one_chain = false;
        }
    }
    let rotation = if one_chain {
        rotation_segment(ctx.rotation() + 180.0)
    } else {
        direction_segment(facing_dir.opposite())
    };
    let s = state::set_bool(default, "attached", one_chain);
    let s = state::set_int(s, "rotation", rotation);
    waterlogged_by(w, pos, s)
}

/// `RotationSegment.convertToSegment(float)`: a yaw in sixteenths of a turn,
/// rounded to the nearest (halves up) and wrapped into 0 to 15.
fn rotation_segment(degrees: f32) -> i32 {
    // `SegmentedAnglePrecision(4)`: 16 / 360 in floats, `Math.round`, masked.
    let turns = degrees * (16.0_f32 / 360.0);
    let whole = turns.floor();
    let rounded = if turns - whole >= 0.5 {
        whole + 1.0
    } else {
        whole
    };
    (rounded as i32) & 15
}

/// `RotationSegment.convertToSegment(Direction)`: four times the 2D data value
/// (south 0, west 4, north 8, east 12); up and down are 0.
fn direction_segment(d: Direction) -> i32 {
    if d.is_horizontal() {
        d.data_2d() << 2
    } else {
        0
    }
}

/// `RotationSegment.convertToDirection`: the four rotations that are a direction.
fn segment_direction(segment: i32) -> Option<Direction> {
    match segment {
        0 => Some(Direction::North),
        4 => Some(Direction::East),
        8 => Some(Direction::South),
        12 => Some(Direction::West),
        _ => None,
    }
}

// Wall, floor and ceiling blocks: levers, buttons, grindstones and bells.

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: where the block
/// points away from its support: up on a floor, down from a ceiling, its facing on a
/// wall.
fn attached_towards(s: u32) -> Direction {
    match state::get(s, "face") {
        Some("ceiling") => Direction::Down,
        Some("floor") => Direction::Up,
        _ => facing_of(s),
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.canAttach`: the neighbour towards
/// `towards` has a full face turned to `pos`.
fn can_attach(w: &World<'_>, pos: Pos, towards: Direction) -> bool {
    w.face_sturdy(towards.step(pos), towards.opposite(), Support::Full)
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`: for each
/// direction the player looks along, nearest first (the clicked face's first when
/// placing against it): looking up puts it on the ceiling, down on the floor (both
/// facing the way the player faces), sideways on the wall ahead (facing back at the
/// player); the first that may stay is taken.
fn face_attached_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    ctx.nearest_looking_directions()
        .into_iter()
        .map(|d| match d {
            Direction::Up | Direction::Down => {
                let face = if d == Direction::Up {
                    "ceiling"
                } else {
                    "floor"
                };
                facing(
                    state::set(default, "face", face),
                    ctx.horizontal_direction(),
                )
            }
            _ => facing(state::set(default, "face", "wall"), d.opposite()),
        })
        .find(|&s| super::can_survive(w, s, pos))
}

/// `BellBlock.getStateForPlacement`. A click on a top or bottom face stands the
/// bell on the floor or hangs it from the ceiling, facing the way the player faces.
/// A click on a side hangs it on that wall (facing it), between two walls when both
/// sides along that line are full; failing that, it stands on the floor if the block
/// below is full on top, else hangs from the ceiling. Whichever is tried must be able
/// to stay.
fn bell_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let face = ctx.clicked_face();
    let pos = ctx.clicked_pos();
    let survives = |s: u32| super::can_survive(w, s, pos).then_some(s);
    if face.axis() == Axis::Y {
        let attachment = if face == Direction::Down {
            "ceiling"
        } else {
            "floor"
        };
        let s = state::set(default, "attachment", attachment);
        return survives(facing(s, ctx.horizontal_direction()));
    }
    let full_towards = |d: Direction| w.face_sturdy(d.step(pos), d.opposite(), Support::Full);
    let between = match face.axis() {
        Axis::X => full_towards(Direction::West) && full_towards(Direction::East),
        _ => full_towards(Direction::North) && full_towards(Direction::South),
    };
    let s = facing(default, face.opposite());
    let wall = state::set(
        s,
        "attachment",
        if between {
            "double_wall"
        } else {
            "single_wall"
        },
    );
    if let Some(s) = survives(wall) {
        return Some(s);
    }
    let floor = w.face_sturdy(Direction::Down.step(pos), Direction::Up, Support::Full);
    survives(state::set(
        wall,
        "attachment",
        if floor { "floor" } else { "ceiling" },
    ))
}

/// `BellBlock.getConnectedDirection`, turned round: where a bell's support is (down
/// for one on the floor, up for one hanging from the ceiling, its facing on a wall).
fn bell_support(s: u32) -> Direction {
    match state::get(s, "attachment") {
        Some("floor") => Direction::Down,
        Some("ceiling") => Direction::Up,
        _ => facing_of(s),
    }
}

/// `BellBlock.canSurvive`: a hanging bell needs the block above to hold it by the
/// centre; others need a full face on their support's side.
fn bell_can_survive(w: &World<'_>, s: u32, pos: Pos) -> bool {
    match bell_support(s) {
        Direction::Up => w.can_support_center(Direction::Up.step(pos), Direction::Down),
        d => can_attach(w, pos, d),
    }
}

/// `BellBlock.updateShape`. Losing its support breaks a bell (but not one held
/// between two walls). Along the bell's facing axis: a two-wall bell whose neighbour
/// on one side isn't full any more hangs on the other wall, facing it (vanilla reads
/// the neighbour's far face, the one turned away from the bell); a one-wall bell that
/// gets a neighbour opposite its wall, full on the face turned to the bell, hangs
/// between both.
fn bell_update(w: &World<'_>, u: &Update) -> u32 {
    let s = u.state;
    let attachment = state::get(s, "attachment");
    let support = bell_support(s);
    if support == u.direction
        && !super::can_survive(w, s, u.pos)
        && attachment != Some("double_wall")
    {
        return state::AIR;
    }
    let f = facing_of(s);
    if u.direction.axis() == f.axis() {
        let full = |face: Direction| w.blocks.face_sturdy(u.neighbour, face, Support::Full);
        if attachment == Some("double_wall") && !full(u.direction) {
            let s = state::set(s, "attachment", "single_wall");
            return facing(s, u.direction.opposite());
        }
        if attachment == Some("single_wall") && support.opposite() == u.direction && full(f) {
            return state::set(s, "attachment", "double_wall");
        }
    }
    s
}

/// `BellBlock.isProperHit` (from `onHit`): a click rings a bell when it is on a
/// side, no higher than 0.8124 up the block, and on a side the bell swings from:
/// along its facing for a bell on the floor, across it for one on a wall, any for one
/// hanging from the ceiling. (Only the server rings it.)
fn bell_proper_hit(u: &Use<'_>) -> bool {
    let side = u.hit.direction;
    let height = u.hit.location[1] - f64::from(u.hit.pos[1]);
    if side.axis() == Axis::Y || height > 0.812_399_983_406_066_9 {
        return false;
    }
    let along = facing_of(u.state).axis() == side.axis();
    match state::get(u.state, "attachment") {
        Some("floor") => along,
        Some("single_wall" | "double_wall") => !along,
        Some("ceiling") => true,
        _ => false,
    }
}

// Using items on the blocks.

/// `SelectableSlotContainer.getHitSlot`: which of a front's slots (`rows` by
/// `columns`, numbered along each row from the top left as seen from the front) the
/// click is on; none unless the click is on the block's front face.
fn hit_slot(u: &Use<'_>, rows: i32, columns: i32) -> Option<i32> {
    let side = u.hit.direction;
    if facing_of(u.state) != side {
        return None;
    }
    // Relative to the block in front of the face (only the coordinate along the
    // face's normal differs from the block's own).
    let front = side.step(u.hit.pos);
    let rel = |i: usize| u.hit.location[i] - f64::from(front[i]);
    let across = match side {
        Direction::North => 1.0 - rel(0),
        Direction::South => rel(0),
        Direction::West => rel(2),
        Direction::East => 1.0 - rel(2),
        Direction::Up | Direction::Down => return None,
    } as f32;
    let up = rel(1) as f32;
    let section = |at: f32, count: i32| {
        let pixel = at * 16.0;
        let size = 16.0 / count as f32;
        ((pixel / size).floor() as i32).clamp(0, count - 1)
    };
    let row = section(1.0 - up, rows);
    let column = section(across, columns);
    Some(column + row * columns)
}

/// `ShelfBlock.useItemOn` on the client: a main-hand click on one of the shelf's
/// three front slots is taken when the selected item isn't empty (the server swaps
/// it, or the hotbar when powered); an empty hand, the off hand or a click off the
/// slots passes.
fn shelf_use(p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    if u.hand == Hand::Off || hit_slot(u, 1, 3).is_none() {
        return Outcome::Pass;
    }
    if p.inventory.hand(Hand::Main).is_empty() {
        Outcome::Pass
    } else {
        Outcome::Success
    }
}

/// `CopperGolemStatueBlock.updatePose`: the statue takes its next pose (standing,
/// running, sitting, star, round again), set at once on the client too.
fn next_statue_pose(w: &mut World<'_>, u: &Use<'_>) -> Outcome {
    w.set_block_and_update(u.pos, state::cycle(u.state, "copper_golem_pose"));
    Outcome::Success
}

/// `WeatheringCopperGolemStatueBlock.useItemOn`. Honeycomb passes (to wax it); any
/// item but an axe changes the pose. An axe on an unweathered statue brings the
/// golem back: the client makes the golem but can't add it (the server does), and
/// removes the block (leaving its water). An axe on a weathered one passes (to scrape
/// it).
fn weathering_statue_use(w: &mut World<'_>, stack: &Stack, u: &Use<'_>) -> Outcome {
    if !w.item_is(stack, "minecraft:axes") {
        if stack.c.name == "minecraft:honeycomb" {
            return Outcome::Pass;
        }
        return next_statue_pose(w, u);
    }
    let unaffected = w
        .blocks
        .kind(u.state)
        .field("CopperGolemStatueBlock.weatheringState")
        == Some("UNAFFECTED");
    if !unaffected {
        return Outcome::Pass;
    }
    // `Level.removeBlock`: the block's fluid stays, as a block.
    let left = fluid_block(w.fluid(u.pos));
    w.set_block(u.pos, left, super::flags::ALL);
    Outcome::Success
}

/// `FluidState.createLegacyBlock`: the fluid as a block (air for none): a source is
/// level 0, flowing fluid 8 less its amount, plus 8 when falling.
fn fluid_block(f: FluidState) -> u32 {
    let name = match f.fluid {
        Fluid::Empty => return state::AIR,
        Fluid::Water | Fluid::FlowingWater => "minecraft:water",
        Fluid::Lava | Fluid::FlowingLava => "minecraft:lava",
    };
    let level = if f.is_source() {
        0
    } else {
        8 - i32::from(f.amount.min(8)) + if f.falling { 8 } else { 0 }
    };
    state::set_int(state::default_of(name), "level", level)
}

/// `ChiseledBookShelfBlock.useItemOn` on the client: anything but a book goes on to
/// the empty-hand click; a book on an empty slot is taken (the server shelves it), on
/// a full one goes on to the empty-hand click; a click off the slots passes. The
/// slots' fullness is in the block state.
fn bookshelf_use_item(w: &World<'_>, stack: &Stack, u: &Use<'_>) -> Outcome {
    if !w.item_is(stack, "minecraft:bookshelf_books") {
        return Outcome::TryEmptyHand;
    }
    let Some(slot) = hit_slot(u, 2, 3) else {
        return Outcome::Pass;
    };
    if state::get_bool(u.state, &format!("slot_{slot}_occupied")) {
        Outcome::TryEmptyHand
    } else {
        Outcome::Success
    }
}

/// `BeehiveBlock.useItemOn` on the client. A glass bottle on a full hive (honey
/// level 5) fills: one bottle goes (even building without materials), a honey bottle
/// takes its place or goes into the inventory, and the hive's honey goes back to 0
/// (whether or not smoke calms the bees, which is the server's). Shears are the
/// server's alone. Anything else goes on to the empty-hand click.
fn beehive_use(w: &mut World<'_>, p: &mut LocalPlayer, stack: &Stack, u: &Use<'_>) -> Outcome {
    let full = state::get_int(u.state, "honey_level") >= 5;
    if !full || stack.c.name != "minecraft:glass_bottle" {
        return Outcome::TryEmptyHand;
    }
    let honey = w.new_stack("minecraft:honey_bottle", 1);
    let held = p.inventory.hand_mut(u.hand);
    held.shrink(1);
    if held.is_empty() {
        p.inventory.set_hand(u.hand, honey);
    } else {
        add_to_inventory(p, honey);
    }
    w.set_block_and_update(u.pos, state::set_int(u.state, "honey_level", 0));
    Outcome::Success
}

/// `Inventory.add` of one stackable item: onto the first stack of it with room
/// (the selected slot, then the off hand, then the main slots in order), else the
/// first empty main slot. A full inventory drops it, which this client leaves to the
/// server (the item isn't shown dropping).
fn add_to_inventory(p: &mut LocalPlayer, stack: Stack) {
    let inv = &mut p.inventory;
    let selected = inv.slot_of(crate::item::EquipmentSlot::MainHand);
    let has_room = |s: &Stack| {
        !s.is_empty()
            && s.same_item_same_components(&stack)
            && s.c.max_stack_size() > 1
            && s.count < s.c.max_stack_size().min(99)
    };
    let slot = [selected, crate::inventory::SLOT_OFFHAND]
        .into_iter()
        .chain(0..36)
        .find(|&i| has_room(inv.get(i)))
        .or_else(|| (0..36).find(|&i| inv.get(i).is_empty()));
    let Some(slot) = slot else {
        return;
    };
    if inv.get(slot).is_empty() {
        inv.set(slot, stack);
    } else if let Some(s) = inv.get_mut(slot) {
        s.count += stack.count;
    }
}

/// `CampfireBlock.useItemOn` on the client: food the campfire cooks is taken (the
/// server puts it on, if there's room); an item that douses campfires puts out a
/// lit one at once (its wear is the server's); anything else goes on to the
/// empty-hand click.
fn campfire_use(w: &mut World<'_>, stack: &Stack, u: &Use<'_>) -> Outcome {
    if w.in_recipe_set(stack, "minecraft:campfire_input") {
        return Outcome::Success;
    }
    if w.item_is(stack, "minecraft:douses_campfires") && state::get_bool(u.state, "lit") {
        w.set_block_and_update(u.pos, state::set_bool(u.state, "lit", false));
        return Outcome::Success;
    }
    Outcome::TryEmptyHand
}

/// `LecternBlock.useItemOn` on the client: with a book on it, the click goes on to
/// the empty-hand click (reading it); without, a lectern book is taken (the server
/// puts it there), an empty main hand passes, anything else goes on to the
/// empty-hand click.
fn lectern_use_item(w: &World<'_>, stack: &Stack, u: &Use<'_>) -> Outcome {
    if state::get_bool(u.state, "has_book") {
        Outcome::TryEmptyHand
    } else if w.item_is(stack, "minecraft:lectern_books") {
        Outcome::Success
    } else if stack.is_empty() && u.hand == Hand::Main {
        Outcome::Pass
    } else {
        Outcome::TryEmptyHand
    }
}

/// `SignBlock.useItemOn` on the client: every click is taken, with a dye, ink or
/// honeycomb (applied by the server) or without (the server then opens the editor or
/// runs the text's click commands).
fn sign_use_item() -> Outcome {
    Outcome::Success
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::testing::Fixture;
    use crate::pick::BlockHit;

    /// Places `item` on top of the floor at the origin, the player standing 3 blocks
    /// south of it looking north and down; returns what was placed.
    fn place_on_floor(f: &mut Fixture, item: &str, y_rot: f32, x_rot: f32) -> String {
        f.player.position = [0.5, -60.0, 3.5];
        f.player.y_rot = y_rot;
        f.player.x_rot = x_rot;
        f.hold(Hand::Main, item, 4);
        f.click_face([0, -61, 0], Direction::Up);
        f.get([0, -60, 0])
    }

    #[test]
    fn workstations_face_the_player_or_their_view() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        let furnace = place_on_floor(&mut f, "minecraft:furnace", 180.0, 60.0);
        assert!(furnace.contains("facing=south"), "{furnace}");
        f.set([0, -60, 0], "minecraft:air");
        let anvil = place_on_floor(&mut f, "minecraft:anvil", 180.0, 60.0);
        assert!(anvil.contains("facing=east"), "{anvil}");
        f.set([0, -60, 0], "minecraft:air");
        let dispenser = place_on_floor(&mut f, "minecraft:dispenser", 180.0, 90.0);
        assert!(dispenser.contains("facing=up"), "{dispenser}");
        f.set([0, -60, 0], "minecraft:air");
        let observer = place_on_floor(&mut f, "minecraft:observer", 180.0, 90.0);
        assert!(observer.contains("facing=down"), "{observer}");
        f.set([0, -60, 0], "minecraft:air");
        let campfire = place_on_floor(&mut f, "minecraft:campfire", 180.0, 60.0);
        assert!(
            campfire.contains("facing=north") && campfire.contains("lit=true"),
            "{campfire}"
        );
    }

    #[test]
    fn hoppers_point_into_the_clicked_side() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 0], "minecraft:stone");
        f.player.position = [3.5, -60.0, 0.5];
        f.player.y_rot = 90.0;
        f.hold(Hand::Main, "minecraft:hopper", 4);
        f.click_face([0, -60, 0], Direction::East);
        assert_eq!(
            f.get([1, -60, 0]),
            "minecraft:hopper[enabled=true,facing=west]"
        );
        f.click_face([1, -61, 1], Direction::Up);
        assert!(f.get([1, -60, 1]).contains("facing=down"));
    }

    #[test]
    fn levers_go_on_the_clicked_wall() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 0], "minecraft:stone");
        f.player.position = [3.5, -60.0, 0.5];
        f.player.y_rot = 90.0;
        f.player.x_rot = 10.0;
        f.hold(Hand::Main, "minecraft:lever", 4);
        f.click_face([0, -60, 0], Direction::East);
        let lever = f.get([1, -60, 0]);
        assert!(
            lever.contains("face=wall") && lever.contains("facing=east"),
            "{lever}"
        );
        // On the floor, facing the way the player faces.
        f.click_face([2, -61, 0], Direction::Up);
        let lever = f.get([2, -60, 0]);
        assert!(
            lever.contains("face=floor") && lever.contains("facing=west"),
            "{lever}"
        );
        // Losing its floor, it can't stay.
        f.set([2, -61, 0], "minecraft:air");
        let air = state::AIR;
        let mut w = f.world();
        let floor_lever = Fixture::state(&lever);
        let below = [2, -61, 0];
        let s =
            super::super::update_shape(&w, floor_lever, [2, -60, 0], Direction::Down, below, air);
        assert_eq!(s, air);
        // Losing its wall: the client leaves it for the server to break.
        w.set_block([0, -60, 0], air, super::super::flags::ALL);
        assert!(f.get([1, -60, 0]).starts_with("minecraft:lever"));
    }

    #[test]
    fn bells_hang_between_walls_and_let_go_of_one() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 0], "minecraft:stone");
        f.set([2, -60, 0], "minecraft:stone");
        f.player.position = [1.5, -60.0, 3.5];
        f.player.y_rot = 180.0;
        f.hold(Hand::Main, "minecraft:bell", 4);
        f.click_face([0, -60, 0], Direction::East);
        let bell = f.get([1, -60, 0]);
        assert!(
            bell.contains("attachment=double_wall") && bell.contains("facing=west"),
            "{bell}"
        );
        let mut w = f.world();
        w.set_block([2, -60, 0], state::AIR, super::super::flags::ALL);
        let bell = f.get([1, -60, 0]);
        assert!(
            bell.contains("attachment=single_wall") && bell.contains("facing=west"),
            "{bell}"
        );
        // Rung from the side the bell swings to, below its top.
        let s = Fixture::state(&bell);
        let hit = BlockHit {
            location: [1.5, -59.5, 1.0],
            direction: Direction::South,
            pos: [1, -60, 0],
            inside: false,
            miss: false,
        };
        let u = Use {
            hand: Hand::Main,
            state: s,
            pos: [1, -60, 0],
            hit: &hit,
        };
        assert!(bell_proper_hit(&u));
        let high = BlockHit {
            location: [1.5, -59.1, 1.0],
            ..hit
        };
        assert!(!bell_proper_hit(&Use { hit: &high, ..u }));
        let end = BlockHit {
            direction: Direction::East,
            location: [2.0, -59.5, 0.5],
            ..hit
        };
        assert!(!bell_proper_hit(&Use { hit: &end, ..u }));
    }

    #[test]
    fn signs_stand_or_hang_on_walls() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        let sign = place_on_floor(&mut f, "minecraft:oak_sign", 0.0, 60.0);
        assert!(sign.starts_with("minecraft:oak_sign[rotation=8"), "{sign}");
        f.set([3, -60, 0], "minecraft:stone");
        f.player.position = [0.5, -60.0, 0.5];
        f.player.y_rot = -90.0;
        f.player.x_rot = 0.0;
        f.click_face([3, -60, 0], Direction::West);
        let wall = f.get([2, -60, 0]);
        assert!(
            wall.starts_with("minecraft:oak_wall_sign[facing=west"),
            "{wall}"
        );
        // A wall hanging sign hangs off the side of a block.
        f.set([0, -60, 2], "minecraft:stone");
        f.player.position = [3.5, -60.0, 2.5];
        f.player.y_rot = 80.0;
        f.hold(Hand::Main, "minecraft:oak_hanging_sign", 4);
        f.click_face([0, -60, 2], Direction::East);
        let hanging = f.get([1, -60, 2]);
        assert!(
            hanging.starts_with("minecraft:oak_wall_hanging_sign[facing=north"),
            "{hanging}"
        );
    }

    #[test]
    fn rotation_segments_round_halves_up() {
        assert_eq!(rotation_segment(0.0), 0);
        assert_eq!(rotation_segment(180.0), 8);
        assert_eq!(rotation_segment(11.25), 1);
        assert_eq!(rotation_segment(-11.25), 0);
        assert_eq!(rotation_segment(-90.0), 12);
        assert_eq!(rotation_segment(-200.0 + 180.0), 15);
        assert_eq!(direction_segment(Direction::West), 4);
        assert_eq!(segment_direction(4), Some(Direction::East));
    }

    #[test]
    fn bookshelf_slots_read_from_the_front() {
        let Some(_) = Fixture::new() else {
            return;
        };
        let s = Fixture::state("minecraft:chiseled_bookshelf[facing=north]");
        let at = |x: f64, y: f64, side: Direction| {
            let hit = BlockHit {
                location: [x, y, 0.0],
                direction: side,
                pos: [0, 0, 0],
                inside: false,
                miss: false,
            };
            hit_slot(
                &Use {
                    hand: Hand::Main,
                    state: s,
                    pos: [0, 0, 0],
                    hit: &hit,
                },
                2,
                3,
            )
        };
        assert_eq!(at(0.9, 0.75, Direction::North), Some(0));
        assert_eq!(at(0.1, 0.75, Direction::North), Some(2));
        assert_eq!(at(0.1, 0.25, Direction::North), Some(5));
        assert_eq!(at(0.5, 0.5, Direction::South), None);
    }

    #[test]
    fn crafter_orientations_and_statue_ages_exist() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        for front in Direction::ALL {
            let tops: &[Direction] = match front {
                Direction::Up | Direction::Down => &Direction::HORIZONTAL,
                _ => &[Direction::Up],
            };
            for top in tops {
                let name = format!(
                    "minecraft:crafter[orientation={}_{}]",
                    front.name(),
                    top.name()
                );
                assert!(state::parse(&name).is_some(), "{name}");
            }
        }
        let w = f.world();
        let age = |name: &str| {
            w.blocks
                .kind(state::default_of(name))
                .field("CopperGolemStatueBlock.weatheringState")
                .map(str::to_owned)
        };
        assert_eq!(
            age("minecraft:copper_golem_statue").as_deref(),
            Some("UNAFFECTED")
        );
        assert_eq!(
            age("minecraft:exposed_copper_golem_statue").as_deref(),
            Some("EXPOSED")
        );
    }

    #[test]
    fn campfires_douse_and_hives_give_honey() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 0], "minecraft:campfire[lit=true]");
        f.player.position = [0.5, -60.0, 3.5];
        f.player.y_rot = 180.0;
        f.player.x_rot = 30.0;
        f.hold(Hand::Main, "minecraft:wooden_shovel", 1);
        f.click_face([0, -60, 0], Direction::South);
        assert!(f.get([0, -60, 0]).contains("lit=false"));
        f.set([1, -60, 0], "minecraft:beehive[facing=south,honey_level=5]");
        f.hold(Hand::Main, "minecraft:glass_bottle", 2);
        f.click_face([1, -60, 0], Direction::South);
        assert!(f.get([1, -60, 0]).contains("honey_level=0"));
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        let honey = f
            .player
            .inventory
            .items
            .iter()
            .find(|s| s.c.name == "minecraft:honey_bottle");
        assert_eq!(honey.map(|s| s.count), Some(1));
    }
}
