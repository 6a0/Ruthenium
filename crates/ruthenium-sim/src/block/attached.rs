//! Blocks that need support or attach to faces: cactus and sugar cane, vines and lichen, chorus, corals, bubble columns, amethyst buds, dripstone, torches, fire, rails, sea pickles, turtle eggs, frog spawn, dripleaves, hanging moss and roots, spore blossoms, bamboo, cocoa, shelf mushrooms.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Most of these blocks only check their support when a neighbour changes; a block
//! that can't stay is either turned to air here (the server then breaks it) or, where
//! vanilla only schedules a tick to break it later, left as it is: scheduled ticks,
//! sounds and particles don't happen on a client level.

use super::{Behaviour, Class, Pos, Support, Update, Use, state};
use crate::direction::Direction;
use crate::fluid::Fluid;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::place::PlaceContext;
use crate::player::LocalPlayer;
use crate::shape::Axis;
use state::AIR;

/// The (class, behaviour) pairs this group answers for; every pair the export lists
/// must be in some group's list (see [`super`]'s coverage test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PORTED: &[(Class, Behaviour)] = &[
    (Class::CactusBlock, Behaviour::UpdateShape),
    (Class::CactusBlock, Behaviour::CanSurvive),
    (Class::SugarCaneBlock, Behaviour::UpdateShape),
    (Class::SugarCaneBlock, Behaviour::CanSurvive),
    (Class::VineBlock, Behaviour::GetStateForPlacement),
    (Class::VineBlock, Behaviour::UpdateShape),
    (Class::VineBlock, Behaviour::CanSurvive),
    (Class::VineBlock, Behaviour::CanBeReplaced),
    (Class::MultifaceBlock, Behaviour::GetStateForPlacement),
    (Class::MultifaceBlock, Behaviour::UpdateShape),
    (Class::MultifaceBlock, Behaviour::CanSurvive),
    (Class::MultifaceBlock, Behaviour::CanBeReplaced),
    (Class::ChorusPlantBlock, Behaviour::GetStateForPlacement),
    (Class::ChorusPlantBlock, Behaviour::UpdateShape),
    (Class::ChorusPlantBlock, Behaviour::CanSurvive),
    (Class::ChorusFlowerBlock, Behaviour::UpdateShape),
    (Class::ChorusFlowerBlock, Behaviour::CanSurvive),
    (Class::CoralBlock, Behaviour::GetStateForPlacement),
    (Class::CoralBlock, Behaviour::UpdateShape),
    (
        Class::BaseCoralPlantTypeBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::BaseCoralPlantTypeBlock, Behaviour::UpdateShape),
    (Class::BaseCoralPlantTypeBlock, Behaviour::CanSurvive),
    (Class::CoralPlantBlock, Behaviour::UpdateShape),
    (Class::CoralFanBlock, Behaviour::UpdateShape),
    (
        Class::BaseCoralWallFanBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::BaseCoralWallFanBlock, Behaviour::UpdateShape),
    (Class::BaseCoralWallFanBlock, Behaviour::CanSurvive),
    (Class::CoralWallFanBlock, Behaviour::UpdateShape),
    (Class::BubbleColumnBlock, Behaviour::UpdateShape),
    (Class::BubbleColumnBlock, Behaviour::CanSurvive),
    (Class::AmethystClusterBlock, Behaviour::GetStateForPlacement),
    (Class::AmethystClusterBlock, Behaviour::UpdateShape),
    (Class::AmethystClusterBlock, Behaviour::CanSurvive),
    (Class::SpeleothemBlock, Behaviour::GetStateForPlacement),
    (Class::SpeleothemBlock, Behaviour::UpdateShape),
    (Class::SpeleothemBlock, Behaviour::CanSurvive),
    (Class::BaseTorchBlock, Behaviour::UpdateShape),
    (Class::BaseTorchBlock, Behaviour::CanSurvive),
    (Class::WallTorchBlock, Behaviour::GetStateForPlacement),
    (Class::WallTorchBlock, Behaviour::UpdateShape),
    (Class::WallTorchBlock, Behaviour::CanSurvive),
    (Class::FireBlock, Behaviour::GetStateForPlacement),
    (Class::FireBlock, Behaviour::UpdateShape),
    (Class::FireBlock, Behaviour::CanSurvive),
    (Class::BaseFireBlock, Behaviour::GetStateForPlacement),
    (Class::SoulFireBlock, Behaviour::UpdateShape),
    (Class::SoulFireBlock, Behaviour::CanSurvive),
    (Class::BaseRailBlock, Behaviour::GetStateForPlacement),
    (Class::BaseRailBlock, Behaviour::UpdateShape),
    (Class::BaseRailBlock, Behaviour::CanSurvive),
    (Class::SeaPickleBlock, Behaviour::GetStateForPlacement),
    (Class::SeaPickleBlock, Behaviour::UpdateShape),
    (Class::SeaPickleBlock, Behaviour::CanSurvive),
    (Class::SeaPickleBlock, Behaviour::CanBeReplaced),
    (Class::SeaPickleBlock, Behaviour::MayPlaceOn),
    (Class::SeaPickleBlock, Behaviour::IsValidBonemealTarget),
    (Class::TurtleEggBlock, Behaviour::GetStateForPlacement),
    (Class::TurtleEggBlock, Behaviour::CanBeReplaced),
    (Class::FrogspawnBlock, Behaviour::UpdateShape),
    (Class::FrogspawnBlock, Behaviour::CanSurvive),
    (Class::BigDripleafBlock, Behaviour::GetStateForPlacement),
    (Class::BigDripleafBlock, Behaviour::UpdateShape),
    (Class::BigDripleafBlock, Behaviour::CanSurvive),
    (Class::BigDripleafBlock, Behaviour::IsValidBonemealTarget),
    (Class::BigDripleafStemBlock, Behaviour::UpdateShape),
    (Class::BigDripleafStemBlock, Behaviour::CanSurvive),
    (
        Class::BigDripleafStemBlock,
        Behaviour::IsValidBonemealTarget,
    ),
    (Class::SmallDripleafBlock, Behaviour::GetStateForPlacement),
    (Class::SmallDripleafBlock, Behaviour::UpdateShape),
    (Class::SmallDripleafBlock, Behaviour::CanSurvive),
    (Class::SmallDripleafBlock, Behaviour::SetPlacedBy),
    (Class::SmallDripleafBlock, Behaviour::MayPlaceOn),
    (Class::SmallDripleafBlock, Behaviour::IsValidBonemealTarget),
    (Class::HangingMossBlock, Behaviour::UpdateShape),
    (Class::HangingMossBlock, Behaviour::CanSurvive),
    (Class::HangingMossBlock, Behaviour::IsValidBonemealTarget),
    (Class::HangingRootsBlock, Behaviour::GetStateForPlacement),
    (Class::HangingRootsBlock, Behaviour::UpdateShape),
    (Class::HangingRootsBlock, Behaviour::CanSurvive),
    (Class::SporeBlossomBlock, Behaviour::UpdateShape),
    (Class::SporeBlossomBlock, Behaviour::CanSurvive),
    (Class::BambooSaplingBlock, Behaviour::UpdateShape),
    (Class::BambooSaplingBlock, Behaviour::CanSurvive),
    (Class::BambooSaplingBlock, Behaviour::IsValidBonemealTarget),
    (Class::BambooStalkBlock, Behaviour::GetStateForPlacement),
    (Class::BambooStalkBlock, Behaviour::UpdateShape),
    (Class::BambooStalkBlock, Behaviour::CanSurvive),
    (Class::BambooStalkBlock, Behaviour::IsValidBonemealTarget),
    (Class::CocoaBlock, Behaviour::GetStateForPlacement),
    (Class::CocoaBlock, Behaviour::UpdateShape),
    (Class::CocoaBlock, Behaviour::CanSurvive),
    (Class::CocoaBlock, Behaviour::IsValidBonemealTarget),
    (Class::ShelfMushroomBlock, Behaviour::GetStateForPlacement),
    (Class::ShelfMushroomBlock, Behaviour::UpdateShape),
    (Class::ShelfMushroomBlock, Behaviour::CanSurvive),
    (Class::ShelfMushroomBlock, Behaviour::IsValidBonemealTarget),
];

/// \`getStateForPlacement\` (\`Some(None)\`: the block can't be placed there).
pub(super) fn state_for_placement(
    c: Class,
    w: &World<'_>,
    ctx: &PlaceContext,
    default: u32,
) -> Option<Option<u32>> {
    let pos = ctx.clicked_pos();
    let placed = match c {
        Class::VineBlock => vine_placement(w, ctx, default),
        Class::MultifaceBlock => multiface_placement(w, ctx, default),
        Class::ChorusPlantBlock => Some(chorus_connections(w, pos, default)),
        Class::CoralBlock => Some(default),
        Class::BaseCoralPlantTypeBlock => Some(coral_plant_placement(w, pos, default)),
        Class::BaseCoralWallFanBlock => {
            facing_first_support(w, ctx, coral_plant_placement(w, pos, default), true)
        }
        Class::AmethystClusterBlock => Some(state::set_dir(
            state::set_bool(default, "waterlogged", source_water(w, pos)),
            "facing",
            ctx.clicked_face(),
        )),
        Class::SpeleothemBlock => speleothem_placement(w, ctx, default),
        Class::WallTorchBlock | Class::ShelfMushroomBlock => {
            facing_first_support(w, ctx, default, true)
        }
        Class::CocoaBlock => facing_first_support(w, ctx, default, false),
        Class::FireBlock => Some(fire_with_sides(w, pos)),
        Class::BaseFireBlock => Some(fire_for(w, pos)),
        Class::BaseRailBlock => Some(rail_placement(w, ctx, default)),
        Class::SeaPickleBlock => Some(
            added_to_clicked(w, pos, default, "pickles")
                .unwrap_or_else(|| state::set_bool(default, "waterlogged", source_water(w, pos))),
        ),
        Class::TurtleEggBlock => Some(added_to_clicked(w, pos, default, "eggs").unwrap_or(default)),
        Class::BigDripleafBlock => Some(big_dripleaf_placement(w, ctx, default)),
        Class::SmallDripleafBlock => small_dripleaf_placement(w, ctx, default),
        Class::HangingRootsBlock => Some(state::set_bool(
            default,
            "waterlogged",
            source_water(w, pos),
        )),
        Class::BambooStalkBlock => bamboo_placement(w, pos, default),
        _ => return None,
    };
    Some(placed)
}

/// \`updateShape\`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    let s = u.state;
    let d = u.direction;
    let unless_fallen = |side: bool| {
        if side && !super::can_survive(w, s, u.pos) {
            AIR
        } else {
            s
        }
    };
    Some(match c {
        // These only schedule a tick to break (or turn) them, which the server runs.
        Class::CactusBlock
        | Class::SugarCaneBlock
        | Class::ChorusFlowerBlock
        | Class::CoralBlock
        | Class::BubbleColumnBlock
        | Class::BaseRailBlock
        | Class::BigDripleafStemBlock => s,
        Class::VineBlock => vine_update(w, u),
        Class::MultifaceBlock => multiface_update(w, u),
        Class::ChorusPlantBlock => chorus_plant_update(w, u),
        Class::BaseCoralPlantTypeBlock
        | Class::CoralPlantBlock
        | Class::CoralFanBlock
        | Class::BaseTorchBlock => unless_fallen(d == Direction::Down),
        Class::BigDripleafBlock => {
            if d == Direction::Up && state::same_block(u.neighbour, s) {
                // A leaf with another leaf put on it becomes stem.
                state::with_properties_of(state::default_of("minecraft:big_dripleaf_stem"), s)
            } else {
                unless_fallen(d == Direction::Down)
            }
        }
        Class::BaseCoralWallFanBlock
        | Class::CoralWallFanBlock
        | Class::WallTorchBlock
        | Class::AmethystClusterBlock
        | Class::ShelfMushroomBlock => {
            let behind = state::get_dir(s, "facing").map(Direction::opposite);
            unless_fallen(behind == Some(d))
        }
        Class::CocoaBlock => unless_fallen(state::get_dir(s, "facing") == Some(d)),
        Class::HangingRootsBlock | Class::SporeBlossomBlock => unless_fallen(d == Direction::Up),
        Class::SeaPickleBlock | Class::FrogspawnBlock => unless_fallen(true),
        Class::SpeleothemBlock => speleothem_update(w, u),
        Class::FireBlock => {
            if super::can_survive(w, s, u.pos) {
                let age = state::get_int(s, "age");
                let fresh = fire_for(w, u.pos);
                if state::is(fresh, "minecraft:fire") {
                    state::set_int(fresh, "age", age)
                } else {
                    fresh
                }
            } else {
                AIR
            }
        }
        Class::SoulFireBlock => {
            if super::can_survive(w, s, u.pos) {
                state::default_of("minecraft:soul_fire")
            } else {
                AIR
            }
        }
        Class::SmallDripleafBlock => small_dripleaf_update(w, u),
        Class::HangingMossBlock => {
            let below = w.get(Direction::Down.step(u.pos));
            state::set_bool(s, "tip", !state::same_block(below, s))
        }
        Class::BambooSaplingBlock => {
            if !super::can_survive(w, s, u.pos) {
                AIR
            } else if d == Direction::Up && state::is(u.neighbour, "minecraft:bamboo") {
                state::default_of("minecraft:bamboo")
            } else {
                s
            }
        }
        Class::BambooStalkBlock => {
            let thicker_above = d == Direction::Up
                && state::is(u.neighbour, "minecraft:bamboo")
                && state::get_int(u.neighbour, "age") > state::get_int(s, "age");
            if thicker_above {
                state::cycle(s, "age")
            } else {
                s
            }
        }
        _ => return None,
    })
}

/// \`canSurvive\`.
pub(super) fn can_survive(c: Class, w: &World<'_>, state: u32, pos: Pos) -> Option<bool> {
    let below = Direction::Down.step(pos);
    let above = Direction::Up.step(pos);
    let under = w.get(below);
    Some(match c {
        Class::CactusBlock => cactus_may_stay(w, state, pos),
        Class::SugarCaneBlock => sugar_cane_may_stay(w, state, pos),
        Class::VineBlock => vine_has_faces(vine_supported_faces(w, state, pos)),
        Class::MultifaceBlock => multiface_may_stay(w, state, pos),
        Class::ChorusPlantBlock => chorus_plant_may_stay(w, state, pos),
        Class::ChorusFlowerBlock => chorus_flower_may_stay(w, state, pos),
        Class::BaseCoralPlantTypeBlock => w.face_sturdy(below, Direction::Up, Support::Full),
        Class::BaseCoralWallFanBlock
        | Class::WallTorchBlock
        | Class::AmethystClusterBlock
        | Class::ShelfMushroomBlock => held_from_behind(w, state, pos),
        Class::BubbleColumnBlock => {
            state::same_block(under, state)
                || w.tags.is(under, "minecraft:enables_bubble_column_push_up")
                || w.tags
                    .is(under, "minecraft:enables_bubble_column_drag_down")
        }
        Class::SpeleothemBlock => {
            let tip = state::get_dir(state, "vertical_direction").unwrap_or(Direction::Up);
            speleothem_held(w, state, pos, tip)
        }
        Class::BaseTorchBlock => w.can_support_center(below, Direction::Up),
        Class::FireBlock => {
            w.face_sturdy(below, Direction::Up, Support::Full)
                || Direction::ALL
                    .iter()
                    .any(|d| flammable(w, w.get(d.step(pos))))
        }
        Class::SoulFireBlock => w.tags.is(under, "minecraft:soul_fire_base_blocks"),
        Class::BaseRailBlock => w.can_support_rigid_block(below),
        Class::SeaPickleBlock => sea_pickle_may_stand_on(w, under),
        Class::FrogspawnBlock => {
            let floats_on = w.fluid(below).fluid == Fluid::Water
                || w.tags.is(under, "minecraft:supports_frogspawn");
            floats_on && w.fluid(pos).is_empty()
        }
        Class::BigDripleafBlock => {
            state::same_block(under, state)
                || state::is(under, "minecraft:big_dripleaf_stem")
                || w.tags.is(under, "minecraft:supports_big_dripleaf")
        }
        Class::BigDripleafStemBlock => {
            let over = w.get(above);
            (state::same_block(under, state) || w.tags.is(under, "minecraft:supports_big_dripleaf"))
                && (state::same_block(over, state) || state::is(over, "minecraft:big_dripleaf"))
        }
        Class::SmallDripleafBlock => {
            if state::get(state, "half") == Some("upper") {
                state::same_block(under, state) && state::get(under, "half") == Some("lower")
            } else {
                small_dripleaf_may_stand_on(w, under, below)
            }
        }
        Class::HangingMossBlock => hanging_moss_held(w, state, pos),
        Class::HangingRootsBlock => w.face_sturdy(above, Direction::Down, Support::Full),
        Class::SporeBlossomBlock => {
            w.can_support_center(above, Direction::Down) && !w.is_water_at(pos)
        }
        Class::BambooSaplingBlock | Class::BambooStalkBlock => {
            w.tags.is(under, "minecraft:supports_bamboo")
        }
        Class::CocoaBlock => state::get_dir(state, "facing")
            .is_some_and(|f| w.tags.is(w.get(f.step(pos)), "minecraft:supports_cocoa")),
        _ => return None,
    })
}

/// \`canBeReplaced(state, context)\`.
pub(super) fn can_be_replaced(
    c: Class,
    w: &World<'_>,
    state: u32,
    ctx: &PlaceContext,
) -> Option<bool> {
    let holding_own = || {
        let item = ctx.item();
        !item.is_empty() && super::item_id(item) == w.blocks.kind(state).item
    };
    Some(match c {
        Class::VineBlock => {
            // Asked of the block where the new one would go.
            let there = w.get(ctx.clicked_pos());
            if state::is(there, "minecraft:vine") {
                VINE_SIDES
                    .iter()
                    .filter(|&&d| state::get_bool(there, d.name()))
                    .count()
                    < VINE_SIDES.len()
            } else {
                super::default_can_be_replaced(w, state, ctx)
            }
        }
        Class::MultifaceBlock => {
            !holding_own() || Direction::ALL.iter().any(|&d| !has_face(state, d))
        }
        Class::SeaPickleBlock => {
            (!ctx.secondary_use() && holding_own() && state::get_int(state, "pickles") < 4)
                || super::default_can_be_replaced(w, state, ctx)
        }
        Class::TurtleEggBlock => {
            (!ctx.secondary_use() && holding_own() && state::get_int(state, "eggs") < 4)
                || super::default_can_be_replaced(w, state, ctx)
        }
        _ => return None,
    })
}

/// \`canBeReplaced(state, fluid)\` (\`water\`: the fluid is water, else lava).
pub(super) fn can_be_replaced_by_fluid(
    _c: Class,
    _w: &World<'_>,
    _state: u32,
    _water: bool,
) -> Option<bool> {
    None
}

/// \`useItemOn\`.
pub(super) fn use_item_on(
    _c: Class,
    _w: &mut World<'_>,
    _p: &mut LocalPlayer,
    _u: &Use<'_>,
) -> Option<Outcome> {
    None
}

/// \`useWithoutItem\`.
pub(super) fn use_without_item(
    _c: Class,
    _w: &mut World<'_>,
    _p: &mut LocalPlayer,
    _u: &Use<'_>,
) -> Option<Outcome> {
    None
}

/// \`updateIndirectNeighbourShapes\`.
pub(super) fn update_indirect_neighbour_shapes(
    _c: Class,
    _w: &mut World<'_>,
    _state: u32,
    _pos: Pos,
    _flags: u32,
    _limit: i32,
) -> Option<()> {
    None
}

/// \`setPlacedBy\`.
///
/// \`SmallDripleafBlock.setPlacedBy\` puts the top half above only on the server: the
/// client shows the lower half alone until the server's blocks arrive.
pub(super) fn set_placed_by(
    c: Class,
    _w: &mut World<'_>,
    _p: &LocalPlayer,
    _state: u32,
    _pos: Pos,
    _hand: Hand,
) -> Option<()> {
    match c {
        Class::SmallDripleafBlock => Some(()),
        _ => None,
    }
}

/// \`attack\` (a left click on the block as breaking starts).
pub(super) fn attack(
    _c: Class,
    _w: &mut World<'_>,
    _p: &LocalPlayer,
    _state: u32,
    _pos: Pos,
) -> Option<()> {
    None
}

/// \`mayPlaceOn\`: whether a plant may stand on \`below\` (the block under it, at
/// \`below_pos\`).
pub(super) fn may_place_on(
    c: Class,
    w: &World<'_>,
    _plant: u32,
    below: u32,
    below_pos: Pos,
) -> Option<bool> {
    match c {
        Class::SeaPickleBlock => Some(sea_pickle_may_stand_on(w, below)),
        Class::SmallDripleafBlock => Some(small_dripleaf_may_stand_on(w, below, below_pos)),
        _ => None,
    }
}

/// \`isValidBonemealTarget\`.
pub(super) fn is_valid_bonemeal_target(
    c: Class,
    w: &World<'_>,
    state: u32,
    pos: Pos,
) -> Option<bool> {
    let above = Direction::Up.step(pos);
    Some(match c {
        Class::SeaPickleBlock => {
            state::get_bool(state, "waterlogged")
                && w.tags
                    .is(w.get(Direction::Down.step(pos)), "minecraft:coral_blocks")
        }
        Class::BigDripleafBlock => big_dripleaf_may_grow_into(w, above),
        Class::BigDripleafStemBlock => {
            let mut at = above;
            while state::same_block(w.get(at), state) {
                at = Direction::Up.step(at);
            }
            state::is(w.get(at), "minecraft:big_dripleaf")
                && big_dripleaf_may_grow_into(w, Direction::Up.step(at))
        }
        Class::SmallDripleafBlock => true,
        Class::HangingMossBlock => {
            let mut below = Direction::Down.step(pos);
            while state::same_block(w.get(below), state) {
                below = Direction::Down.step(below);
            }
            w.blocks.is_air(w.get(below)) && w.inside_build_height(below[1])
        }
        Class::BambooSaplingBlock => {
            w.blocks.is_air(w.get(above)) && w.inside_build_height(above[1])
        }
        Class::BambooStalkBlock => bamboo_may_grow(w, pos),
        Class::CocoaBlock => state::get_int(state, "age") < 2,
        Class::ShelfMushroomBlock => state::get_int(state, "age") < 1,
        _ => return None,
    })
}

/// \`Fluids.WATER\` itself (a source, not flowing water) is at \`pos\`.
fn source_water(w: &World<'_>, pos: Pos) -> bool {
    w.fluid(pos).fluid == Fluid::Water
}

/// \`MultifaceBlock.canAttachTo\`: the neighbour lying towards \`toward\` has a full face
/// turned this way, by its support shape or by its collision shape.
fn attaches_to(w: &World<'_>, toward: Direction, neighbour: u32) -> bool {
    let face = toward.opposite();
    w.blocks.face_sturdy(neighbour, face, Support::Full)
        || w.blocks.collision_face_full(neighbour, face)
}

/// \`MultifaceBlock.hasFace\`: the side's property is set (false for a side the block
/// has no property for).
fn has_face(s: u32, side: Direction) -> bool {
    state::get_bool(s, side.name())
}

/// The \`canSurvive\` of blocks hung on the side of another (wall torches, wall fans,
/// amethyst buds, shelf mushrooms): the block behind them, against their \`facing\`,
/// has a sturdy face towards them.
fn held_from_behind(w: &World<'_>, s: u32, pos: Pos) -> bool {
    state::get_dir(s, "facing")
        .is_some_and(|f| w.face_sturdy(f.opposite().step(pos), f, Support::Full))
}

/// Placement of blocks hung on a side (\`WallTorchBlock\`, \`BaseCoralWallFanBlock\`,
/// \`ShelfMushroomBlock\`, \`CocoaBlock.getStateForPlacement\`): the horizontal
/// directions the player looks along, nearest first, are tried until one lets the
/// block stay. Hung blocks face away from the direction looked along (\`away\`);
/// cocoa faces along it, towards its log. None fits: nothing is placed.
fn facing_first_support(w: &World<'_>, ctx: &PlaceContext, base: u32, away: bool) -> Option<u32> {
    let pos = ctx.clicked_pos();
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal())
        .map(|d| state::set_dir(base, "facing", if away { d.opposite() } else { d }))
        .find(|&s| super::can_survive(w, s, pos))
}

/// \`CactusBlock.canSurvive\`: nothing solid (the legacy \`isSolid\`) and no lava beside
/// it, cactus or a block tagged \`supports_cactus\` under it, and no liquid above.
fn cactus_may_stay(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let crowded = Direction::HORIZONTAL.iter().any(|d| {
        let side = d.step(pos);
        w.blocks.solid(w.get(side)) || w.fluid(side).fluid.is_lava()
    });
    if crowded {
        return false;
    }
    let under = w.get(Direction::Down.step(pos));
    (state::same_block(under, s) || w.tags.is(under, "minecraft:supports_cactus"))
        && !w.blocks.liquid(w.get(Direction::Up.step(pos)))
}

/// \`SugarCaneBlock.canSurvive\`: on sugar cane, or on a block tagged
/// \`supports_sugar_cane\` with water (the fluid tag \`supports_sugar_cane_adjacently\`,
/// taken as vanilla's \`#minecraft:water\` since the client keeps no fluid tags) or a
/// block tagged \`supports_sugar_cane_adjacently\` beside that block.
fn sugar_cane_may_stay(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let below = Direction::Down.step(pos);
    let under = w.get(below);
    if state::same_block(under, s) {
        return true;
    }
    w.tags.is(under, "minecraft:supports_sugar_cane")
        && Direction::HORIZONTAL.iter().any(|d| {
            let side = d.step(below);
            w.fluid(side).fluid.is_water()
                || w.tags
                    .is(w.get(side), "minecraft:supports_sugar_cane_adjacently")
        })
}

/// The sides a vine may cover (\`VineBlock.PROPERTY_BY_DIRECTION\`: all but the
/// bottom).
const VINE_SIDES: [Direction; 5] = [
    Direction::Up,
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];

/// Any side covered (\`VineBlock.hasFaces\`).
fn vine_has_faces(s: u32) -> bool {
    VINE_SIDES.iter().any(|&d| has_face(s, d))
}

/// \`VineBlock.canSupportAtFace\`: a vine at \`pos\` may cover \`side\` when the block
/// there has a full face towards it; a wall side may also hang from the vine above
/// covering the same side. Never the bottom.
fn vine_supported_at(w: &World<'_>, vine: u32, pos: Pos, side: Direction) -> bool {
    if side == Direction::Down {
        return false;
    }
    if attaches_to(w, side, w.get(side.step(pos))) {
        return true;
    }
    if side == Direction::Up {
        return false;
    }
    let over = w.get(Direction::Up.step(pos));
    state::same_block(over, vine) && has_face(over, side)
}

/// \`VineBlock.getUpdatedState\`: the vine without the sides nothing holds any more.
/// The top stays only while the block above has a full *top* face (vanilla asks
/// \`canAttachTo\` with the direction reversed here); the walls are checked north,
/// east, south, west.
fn vine_supported_faces(w: &World<'_>, vine: u32, pos: Pos) -> u32 {
    let mut s = vine;
    if has_face(s, Direction::Up) {
        let over = w.get(Direction::Up.step(pos));
        s = state::set_bool(s, "up", attaches_to(w, Direction::Down, over));
    }
    for d in Direction::HORIZONTAL {
        if has_face(s, d) {
            s = state::set_bool(s, d.name(), vine_supported_at(w, s, pos, d));
        }
    }
    s
}

/// \`VineBlock.updateShape\`: news from below changes nothing; otherwise the
/// unsupported sides go, and a vine left with none can't stay.
fn vine_update(w: &World<'_>, u: &Update) -> u32 {
    if u.direction == Direction::Down {
        return u.state;
    }
    let kept = vine_supported_faces(w, u.state, u.pos);
    if vine_has_faces(kept) { kept } else { AIR }
}

/// \`VineBlock.getStateForPlacement\`: the first side, in the order the player looks
/// along (not the bottom), that the vine may cover and doesn't yet. Placing into a
/// vine adds that side to it, or leaves it as it is when no side fits.
fn vine_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let there = w.get(pos);
    let adding = state::same_block(there, default);
    let base = if adding { there } else { default };
    let side = ctx.nearest_looking_directions().into_iter().find(|&d| {
        d != Direction::Down
            && !(adding && has_face(there, d))
            && vine_supported_at(w, default, pos, d)
    });
    match side {
        Some(d) => Some(state::set_bool(base, d.name(), true)),
        None => adding.then_some(base),
    }
}

/// \`MultifaceBlock.canSurvive\`: at least one side covered, each against a block it
/// attaches to.
fn multiface_may_stay(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let mut covered = Direction::ALL
        .into_iter()
        .filter(|&d| has_face(s, d))
        .peekable();
    covered.peek().is_some() && covered.all(|d| attaches_to(w, d, w.get(d.step(pos))))
}

/// \`MultifaceBlock.updateShape\`: a side whose neighbour no longer holds it goes; no
/// side left, the block can't stay.
fn multiface_update(w: &World<'_>, u: &Update) -> u32 {
    let any = |s: u32| Direction::ALL.iter().any(|&d| has_face(s, d));
    let s = u.state;
    if !any(s) {
        return AIR;
    }
    if has_face(s, u.direction) && !attaches_to(w, u.direction, u.neighbour) {
        let without = state::set_bool(s, u.direction.name(), false);
        return if any(without) { without } else { AIR };
    }
    s
}

/// \`MultifaceBlock.getStateForPlacement\`: the first direction the player looks along
/// whose block it may attach to and which isn't covered yet. Placed into the same
/// block, the side is added to it; into a water source, the new block is
/// waterlogged. (Every side is supported by every multiface block of 26.3.)
fn multiface_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let old = w.get(pos);
    let adding = state::same_block(old, default);
    let side = ctx
        .nearest_looking_directions()
        .into_iter()
        .find(|&d| !(adding && has_face(old, d)) && attaches_to(w, d, w.get(d.step(pos))))?;
    let base = if adding {
        old
    } else if w.blocks.fluid(old).fluid == Fluid::Water {
        state::set_bool(default, "waterlogged", true)
    } else {
        default
    };
    Some(state::set_bool(base, side.name(), true))
}

/// Whether a chorus plant joins its neighbour towards \`toward\`: another of the same
/// plant, a chorus flower, or (below only) a block tagged \`supports_chorus_plant\`.
fn chorus_joins(w: &World<'_>, plant: u32, toward: Direction, neighbour: u32) -> bool {
    state::same_block(neighbour, plant)
        || state::is(neighbour, "minecraft:chorus_flower")
        || toward == Direction::Down && w.tags.is(neighbour, "minecraft:supports_chorus_plant")
}

/// \`ChorusPlantBlock.getStateWithConnections\`: an arm towards each neighbour it joins.
fn chorus_connections(w: &World<'_>, pos: Pos, plant: u32) -> u32 {
    Direction::ALL.into_iter().fold(plant, |s, d| {
        let joined = chorus_joins(w, plant, d, w.get(d.step(pos)));
        state::try_set(s, d.name(), if joined { "true" } else { "false" })
    })
}

/// \`ChorusPlantBlock.updateShape\`: the arm towards the changed neighbour follows
/// it; a plant that can't stay keeps its arms (its break is a scheduled tick).
fn chorus_plant_update(w: &World<'_>, u: &Update) -> u32 {
    if !super::can_survive(w, u.state, u.pos) {
        return u.state;
    }
    let joined = chorus_joins(w, u.state, u.direction, u.neighbour);
    state::set_bool(u.state, u.direction.name(), joined)
}

/// \`ChorusPlantBlock.canSurvive\`: a plant beside another plant may hang from it when
/// that one stands on plant or end stone (\`supports_chorus_plant\`), unless blocks are
/// both above and below it; otherwise it must stand on plant or end stone itself.
fn chorus_plant_may_stay(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let under = w.get(Direction::Down.step(pos));
    let boxed_in = !w.blocks.is_air(w.get(Direction::Up.step(pos))) && !w.blocks.is_air(under);
    let stands =
        |b: u32| state::same_block(b, s) || w.tags.is(b, "minecraft:supports_chorus_plant");
    for d in Direction::HORIZONTAL {
        let side = d.step(pos);
        if !state::same_block(w.get(side), s) {
            continue;
        }
        if boxed_in {
            return false;
        }
        if stands(w.get(Direction::Down.step(side))) {
            return true;
        }
    }
    stands(under)
}

/// \`ChorusFlowerBlock.canSurvive\`: on its plant (the \`plant\` setting) or a block
/// tagged \`supports_chorus_flower\`; or over air with exactly one plant beside it and
/// nothing else around.
fn chorus_flower_may_stay(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let plant = w
        .blocks
        .kind(s)
        .field("ChorusFlowerBlock.plant")
        .unwrap_or("minecraft:chorus_plant");
    let under = w.get(Direction::Down.step(pos));
    if state::is(under, plant) || w.tags.is(under, "minecraft:supports_chorus_flower") {
        return true;
    }
    if !w.blocks.is_air(under) {
        return false;
    }
    let mut plants = 0;
    for d in Direction::HORIZONTAL {
        let side = w.get(d.step(pos));
        if state::is(side, plant) {
            plants += 1;
        } else if !w.blocks.is_air(side) {
            return false;
        }
    }
    plants == 1
}

/// \`BaseCoralPlantTypeBlock.getStateForPlacement\`: waterlogged in a full block of
/// water (\`#minecraft:water\`, amount 8). Whether the coral dies is the server's
/// tick.
fn coral_plant_placement(w: &World<'_>, pos: Pos, default: u32) -> u32 {
    let f = w.fluid(pos);
    state::set_bool(default, "waterlogged", f.fluid.is_water() && f.amount == 8)
}

/// A speleothem (\`#minecraft:speleothems\`: pointed dripstone, sulfur spikes)
/// pointing \`tip\` (\`SpeleothemBlock.isSpeleothemWithDirection\`).
fn speleothem_pointing(w: &World<'_>, s: u32, tip: Direction) -> bool {
    w.tags.is(s, "minecraft:speleothems") && state::get_dir(s, "vertical_direction") == Some(tip)
}

/// \`SpeleothemBlock.isValidSpeleothemPlacement\`: the block at its base (against
/// \`tip\`) has a sturdy face towards the tip, or is the same speleothem pointing the
/// same way.
fn speleothem_held(w: &World<'_>, s: u32, pos: Pos, tip: Direction) -> bool {
    let base = w.get(tip.opposite().step(pos));
    w.blocks.face_sturdy(base, tip, Support::Full)
        || speleothem_pointing(w, base, tip) && state::same_block(base, s)
}

/// \`SpeleothemBlock.calculateSpeleothemThickness\`: the same block pointing back at
/// the tip makes a merged tip (when \`merge\`, or it is merged already) or a plain tip;
/// nothing pointing on ahead, a tip; a tip ahead, a frustum; thicker ahead, a middle
/// with more of the column behind, else the base.
fn speleothem_thickness(
    w: &World<'_>,
    s: u32,
    pos: Pos,
    tip: Direction,
    merge: bool,
) -> &'static str {
    let back = tip.opposite();
    let ahead = w.get(tip.step(pos));
    if speleothem_pointing(w, ahead, back) && state::same_block(ahead, s) {
        let merged = merge || state::get(ahead, "thickness") == Some("tip_merge");
        return if merged { "tip_merge" } else { "tip" };
    }
    if !speleothem_pointing(w, ahead, tip) {
        return "tip";
    }
    if matches!(state::get(ahead, "thickness"), Some("tip" | "tip_merge")) {
        return "frustum";
    }
    if speleothem_pointing(w, w.get(back.step(pos)), tip) {
        "middle"
    } else {
        "base"
    }
}

/// \`SpeleothemBlock.getStateForPlacement\`: it points against the player's vertical
/// view (up when looking down) if it may hang that way, else the other way, else not
/// at all; it merges with a tip it meets unless the player sneaks; waterlogged in a
/// water source.
fn speleothem_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let preferred = ctx.nearest_looking_vertical_direction().opposite();
    let tip = [preferred, preferred.opposite()]
        .into_iter()
        .find(|&t| speleothem_held(w, default, pos, t))?;
    let thickness = speleothem_thickness(w, default, pos, tip, !ctx.secondary_use());
    let s = state::set_dir(default, "vertical_direction", tip);
    let s = state::set(s, "thickness", thickness);
    Some(state::set_bool(s, "waterlogged", source_water(w, pos)))
}

/// \`SpeleothemBlock.updateShape\`: only blocks above and below matter. Losing its base
/// keeps the state (the fall or break is a scheduled tick; a client level has none
/// scheduled, so a hanging one never waits on one here); otherwise the thickness
/// follows the column, a merged tip staying merged.
fn speleothem_update(w: &World<'_>, u: &Update) -> u32 {
    let s = u.state;
    if u.direction.is_horizontal() {
        return s;
    }
    let tip = state::get_dir(s, "vertical_direction").unwrap_or(Direction::Up);
    if u.direction == tip.opposite() && !super::can_survive(w, s, u.pos) {
        return s;
    }
    let merge = state::get(s, "thickness") == Some("tip_merge");
    state::set(
        s,
        "thickness",
        speleothem_thickness(w, s, u.pos, tip, merge),
    )
}

/// \`FireBlock.canBurn\`: the block catches fire (its ignite odds aren't 0), unless it
/// is waterlogged.
fn flammable(w: &World<'_>, s: u32) -> bool {
    !state::get_bool(s, "waterlogged") && w.blocks.kind(s).ignite_odds > 0
}

/// \`FireBlock.getStateForPlacement(level, pos)\`: plain fire over a flammable or
/// sturdy floor; otherwise fire clinging to each flammable side (north, east, south,
/// west, up).
fn fire_with_sides(w: &World<'_>, pos: Pos) -> u32 {
    let fire = state::default_of("minecraft:fire");
    let under = w.get(Direction::Down.step(pos));
    if flammable(w, under) || w.blocks.face_sturdy(under, Direction::Up, Support::Full) {
        return fire;
    }
    [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
        Direction::Up,
    ]
    .into_iter()
    .fold(fire, |s, d| {
        state::set_bool(s, d.name(), flammable(w, w.get(d.step(pos))))
    })
}

/// \`BaseFireBlock.getState\`: soul fire on a block tagged \`soul_fire_base_blocks\`,
/// else fire as [\`fire_with_sides\`] shapes it.
fn fire_for(w: &World<'_>, pos: Pos) -> u32 {
    if w.tags.is(
        w.get(Direction::Down.step(pos)),
        "minecraft:soul_fire_base_blocks",
    ) {
        state::default_of("minecraft:soul_fire")
    } else {
        fire_with_sides(w, pos)
    }
}

/// \`BaseRailBlock.getStateForPlacement\`: straight along the way the player faces
/// (east-west or north-south), waterlogged in a water source.
///
/// The rail's turn towards its neighbours (\`RailState.place\`) isn't predicted:
/// vanilla only runs it from \`onPlace\`, which a client level never calls, and
/// \`updateDir\` leaves a client's rail as it is. The server's shape arrives with its
/// block update.
fn rail_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let shape = match ctx.horizontal_direction() {
        Direction::East | Direction::West => "east_west",
        _ => "north_south",
    };
    let s = state::set(default, "shape", shape);
    state::set_bool(s, "waterlogged", source_water(w, ctx.clicked_pos()))
}

/// Placing sea pickles or turtle eggs into a block of their own adds one to its
/// count (\`property\`), up to four.
fn added_to_clicked(w: &World<'_>, pos: Pos, default: u32, property: &str) -> Option<u32> {
    let there = w.get(pos);
    state::same_block(there, default).then(|| {
        state::set_int(
            there,
            property,
            (state::get_int(there, property) + 1).min(4),
        )
    })
}

/// \`SeaPickleBlock.mayPlaceOn\`: the block below has some collision shape at its top
/// face, or a sturdy top.
fn sea_pickle_may_stand_on(w: &World<'_>, below: u32) -> bool {
    collision_reaches_top(w, below) || w.blocks.face_sturdy(below, Direction::Up, Support::Full)
}

/// Whether the collision shape's top face (\`getFaceShape(UP)\`: the layer of cells a
/// ten-millionth below the top) holds any cell.
fn collision_reaches_top(w: &World<'_>, s: u32) -> bool {
    let shape = w.blocks.shape(s);
    if shape.is_empty() {
        return false;
    }
    let ys = &shape.coords[Axis::Y as usize];
    let at = 1.0 - 1.0e-7;
    let Some(layer) = (0..shape.size(Axis::Y)).find(|&i| ys[i] <= at && at < ys[i + 1]) else {
        return false;
    };
    (0..shape.size(Axis::X)).any(|x| {
        (0..shape.size(Axis::Z)).any(|z| shape.is_full_at(x as isize, layer as isize, z as isize))
    })
}

/// \`BigDripleafBlock.getStateForPlacement\`: on a dripleaf or its stem, it faces the
/// same way; otherwise towards the player. Waterlogged in a water source.
fn big_dripleaf_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let under = w.get(Direction::Down.step(pos));
    let on_dripleaf = state::is(under, "minecraft:big_dripleaf")
        || state::is(under, "minecraft:big_dripleaf_stem");
    let facing = match state::get_dir(under, "facing") {
        Some(f) if on_dripleaf => f,
        _ => ctx.horizontal_direction().opposite(),
    };
    let s = state::set_bool(default, "waterlogged", source_water(w, pos));
    state::set_dir(s, "facing", facing)
}

/// \`BigDripleafBlock.canGrowInto\`: inside the build height, and air, water or a
/// small dripleaf there.
fn big_dripleaf_may_grow_into(w: &World<'_>, pos: Pos) -> bool {
    let s = w.get(pos);
    w.inside_build_height(pos[1])
        && (w.blocks.is_air(s)
            || state::is(s, "minecraft:water")
            || state::is(s, "minecraft:small_dripleaf"))
}

/// \`SmallDripleafBlock.mayPlaceOn\`: a block tagged \`supports_small_dripleaf\`, or,
/// with a water source where the dripleaf goes, one tagged \`supports_vegetation\`.
fn small_dripleaf_may_stand_on(w: &World<'_>, below: u32, below_pos: Pos) -> bool {
    w.tags.is(below, "minecraft:supports_small_dripleaf")
        || source_water(w, Direction::Up.step(below_pos))
            && w.tags.is(below, "minecraft:supports_vegetation")
}

/// \`SmallDripleafBlock.getStateForPlacement\` (through \`DoublePlantBlock\`'s): only
/// below the top of the level with room above (the block above may be replaced by
/// this placement); it faces the player and is waterlogged in any water.
fn small_dripleaf_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let above = Direction::Up.step(pos);
    if pos[1] >= w.max_y() || !super::can_be_replaced(w, w.get(above), ctx) {
        return None;
    }
    let s = state::set_dir(default, "facing", ctx.horizontal_direction().opposite());
    Some(state::set_bool(s, "waterlogged", w.is_water_at(pos)))
}

/// \`DoublePlantBlock.updateShape\` as small dripleaves run it: a half whose other half
/// is no longer next to it can't stay, nor one that can't survive.
fn small_dripleaf_update(w: &World<'_>, u: &Update) -> u32 {
    let s = u.state;
    let half = state::get(s, "half");
    let towards_other = u.direction
        == if half == Some("lower") {
            Direction::Up
        } else {
            Direction::Down
        };
    let other_half = state::same_block(u.neighbour, s) && state::get(u.neighbour, "half") != half;
    if towards_other && !other_half || !super::can_survive(w, s, u.pos) {
        AIR
    } else {
        s
    }
}

/// \`HangingMossBlock.canStayAtPosition\`: under a block it attaches to, or under more
/// moss.
fn hanging_moss_held(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let over = w.get(Direction::Up.step(pos));
    attaches_to(w, Direction::Up, over) || state::same_block(over, s)
}

/// \`BambooStalkBlock.getStateForPlacement\`: never in a fluid, only on a block tagged
/// \`supports_bamboo\`. On a sapling it is thin; on bamboo, as thick as that one; on
/// other ground, as thick as bamboo just above, or else a sapling.
fn bamboo_placement(w: &World<'_>, pos: Pos, default: u32) -> Option<u32> {
    if !w.fluid(pos).is_empty() {
        return None;
    }
    let under = w.get(Direction::Down.step(pos));
    if !w.tags.is(under, "minecraft:supports_bamboo") {
        return None;
    }
    let over = w.get(Direction::Up.step(pos));
    Some(if state::is(under, "minecraft:bamboo_sapling") {
        state::set_int(default, "age", 0)
    } else if state::is(under, "minecraft:bamboo") {
        state::set_int(default, "age", i32::from(state::get_int(under, "age") > 0))
    } else if state::is(over, "minecraft:bamboo") {
        state::set_int(default, "age", state::get_int(over, "age"))
    } else {
        state::default_of("minecraft:bamboo_sapling")
    })
}

/// \`BambooStalkBlock.isValidBonemealTarget\`: the stalk (counted up to 16 each way) is
/// shorter than 16, its top still grows (\`stage\` 0), and there is empty room inside
/// the level above the top.
fn bamboo_may_grow(w: &World<'_>, pos: Pos) -> bool {
    let run = |d: Direction| {
        let mut n = 0;
        let mut at = d.step(pos);
        while n < 16 && state::is(w.get(at), "minecraft:bamboo") {
            n += 1;
            at = d.step(at);
        }
        n
    };
    let up = run(Direction::Up);
    let down = run(Direction::Down);
    let top = [pos[0], pos[1] + up, pos[2]];
    let room = Direction::Up.step(top);
    up + down + 1 < 16
        && state::get_int(w.get(top), "stage") != 1
        && w.inside_build_height(room[1])
        && w.is_empty_block(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::testing::Fixture;

    fn has(f: &Fixture, pos: Pos, text: &str) -> bool {
        f.get(pos).contains(text)
    }

    #[test]
    fn torches_hang_on_the_wall_looked_at_and_stand_on_floors() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 2], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:torch", 64);
        f.click_face([0, -60, 2], Direction::North);
        assert_eq!(f.get([0, -60, 1]), "minecraft:wall_torch[facing=north]");
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(f.get([1, -60, 0]), "minecraft:torch");
        // The floor going: the torch can't stay.
        f.set([1, -61, 0], "minecraft:air");
        let torch = Fixture::state("minecraft:torch");
        let w = f.world();
        let after =
            crate::block::update_shape(&w, torch, [1, -60, 0], Direction::Down, [1, -61, 0], AIR);
        assert_eq!(after, AIR);
    }

    #[test]
    fn sea_pickles_and_turtle_eggs_gather_up_to_four() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.hold(Hand::Main, "minecraft:sea_pickle", 64);
        f.click_face([0, -61, 1], Direction::Up);
        assert!(has(&f, [0, -60, 1], "pickles=1"), "{}", f.get([0, -60, 1]));
        assert!(has(&f, [0, -60, 1], "waterlogged=false"));
        for n in 2..=4 {
            f.click_face([0, -60, 1], Direction::Up);
            assert!(
                has(&f, [0, -60, 1], &format!("pickles={n}")),
                "{}",
                f.get([0, -60, 1])
            );
        }
        // A fifth would go on top, where a pickle can't stand.
        f.click_face([0, -60, 1], Direction::Up);
        assert!(has(&f, [0, -60, 1], "pickles=4"));
        assert_eq!(f.get([0, -59, 1]), "minecraft:air");
        assert_eq!(f.player.hand(Hand::Main).count, 60);
        f.hold(Hand::Main, "minecraft:turtle_egg", 64);
        f.click_face([2, -61, 0], Direction::Up);
        f.click_face([2, -60, 0], Direction::Up);
        assert!(has(&f, [2, -60, 0], "eggs=2"), "{}", f.get([2, -60, 0]));
    }

    #[test]
    fn dripstone_points_away_from_its_base_and_thickens_into_a_column() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.x_rot = 90.0;
        f.hold(Hand::Main, "minecraft:pointed_dripstone", 64);
        f.click_face([0, -61, 1], Direction::Up);
        assert!(
            has(&f, [0, -60, 1], "thickness=tip,"),
            "{}",
            f.get([0, -60, 1])
        );
        assert!(has(&f, [0, -60, 1], "vertical_direction=up"));
        f.click_face([0, -60, 1], Direction::Up);
        assert!(
            has(&f, [0, -59, 1], "thickness=tip,"),
            "{}",
            f.get([0, -59, 1])
        );
        assert!(
            has(&f, [0, -60, 1], "thickness=frustum"),
            "{}",
            f.get([0, -60, 1])
        );
        f.click_face([0, -59, 1], Direction::Up);
        assert!(
            has(&f, [0, -60, 1], "thickness=base"),
            "{}",
            f.get([0, -60, 1])
        );
        assert!(
            has(&f, [0, -59, 1], "thickness=frustum"),
            "{}",
            f.get([0, -59, 1])
        );
        // Hung from a ceiling, looking up, it points down.
        f.set([3, -55, 0], "minecraft:stone");
        f.player.x_rot = -90.0;
        f.click_face([3, -55, 0], Direction::Down);
        assert!(
            has(&f, [3, -56, 0], "vertical_direction=down"),
            "{}",
            f.get([3, -56, 0])
        );
    }

    #[test]
    fn vines_and_lichen_cover_the_faces_they_are_put_against() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 2], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:vine", 64);
        f.click_face([0, -60, 2], Direction::North);
        assert!(has(&f, [0, -60, 1], "south=true"), "{}", f.get([0, -60, 1]));
        assert!(has(&f, [0, -60, 1], "up=false"));
        // Nothing behind it any more: the vine has no face left.
        f.set([0, -60, 2], "minecraft:air");
        let vine = f.level.block(0, -60, 1);
        let w = f.world();
        let after =
            crate::block::update_shape(&w, vine, [0, -60, 1], Direction::South, [0, -60, 2], AIR);
        assert_eq!(after, AIR);
        assert!(!crate::block::can_survive(&w, vine, [0, -60, 1]));

        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.x_rot = 90.0;
        f.hold(Hand::Main, "minecraft:glow_lichen", 64);
        f.click_face([0, -61, 1], Direction::Up);
        assert!(has(&f, [0, -60, 1], "down=true"), "{}", f.get([0, -60, 1]));
        f.set([0, -60, 2], "minecraft:stone");
        f.player.x_rot = 0.0;
        f.click_face([0, -60, 1], Direction::Up);
        let both = f.get([0, -60, 1]);
        assert!(
            both.contains("down=true") && both.contains("south=true"),
            "{both}"
        );
        assert_eq!(f.player.hand(Hand::Main).count, 62);
    }

    #[test]
    fn rails_lie_along_the_view_and_amethyst_faces_the_clicked_face() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.y_rot = -90.0;
        f.hold(Hand::Main, "minecraft:rail", 64);
        f.click_face([2, -61, 0], Direction::Up);
        assert!(
            has(&f, [2, -60, 0], "shape=east_west"),
            "{}",
            f.get([2, -60, 0])
        );
        f.player.y_rot = 0.0;
        f.set([0, -60, 2], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:amethyst_cluster", 64);
        f.click_face([0, -60, 2], Direction::North);
        assert!(
            has(&f, [0, -60, 1], "facing=north"),
            "{}",
            f.get([0, -60, 1])
        );
    }

    #[test]
    fn small_dripleaf_shows_its_lower_half_until_the_server_answers() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([2, -61, 0], "minecraft:clay");
        f.player.y_rot = 0.0;
        f.hold(Hand::Main, "minecraft:small_dripleaf", 64);
        f.click_face([2, -61, 0], Direction::Up);
        assert!(has(&f, [2, -60, 0], "half=lower"), "{}", f.get([2, -60, 0]));
        assert!(
            has(&f, [2, -60, 0], "facing=north"),
            "{}",
            f.get([2, -60, 0])
        );
        assert_eq!(f.get([2, -59, 0]), "minecraft:air");
        // Not on stone without water.
        f.click_face([3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -60, 0]), "minecraft:air");
    }

    #[test]
    fn bamboo_starts_as_a_sapling_that_turns_to_bamboo_when_built_on() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([2, -61, 0], "minecraft:dirt");
        f.hold(Hand::Main, "minecraft:bamboo", 64);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(f.get([2, -60, 0]), "minecraft:bamboo_sapling");
        f.click_face([2, -60, 0], Direction::Up);
        assert!(
            has(&f, [2, -59, 0], "minecraft:bamboo[age=0"),
            "{}",
            f.get([2, -59, 0])
        );
        assert!(
            has(&f, [2, -60, 0], "minecraft:bamboo[age=0"),
            "{}",
            f.get([2, -60, 0])
        );
        // Not on stone.
        f.click_face([3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -60, 0]), "minecraft:air");
    }

    #[test]
    fn fire_clings_to_what_burns_and_turns_to_soul_fire_on_soul_sand() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -55, 1], "minecraft:oak_planks");
        f.set([4, -61, 0], "minecraft:soul_sand");
        let w = f.world();
        let hanging = fire_for(&w, [0, -55, 0]);
        assert_eq!(
            state::describe(hanging),
            "minecraft:fire[age=0,east=false,north=false,south=true,up=false,west=false]"
        );
        assert!(crate::block::can_survive(&w, hanging, [0, -55, 0]));
        assert!(!crate::block::can_survive(&w, hanging, [0, -55, 5]));
        assert_eq!(
            fire_for(&w, [1, -60, 0]),
            state::default_of("minecraft:fire")
        );
        assert_eq!(
            fire_for(&w, [4, -60, 0]),
            state::default_of("minecraft:soul_fire")
        );
    }

    #[test]
    fn wall_corals_need_a_wall_and_take_water() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([0, -60, 2], "minecraft:stone");
        f.set([0, -60, 1], "minecraft:water");
        f.hold(Hand::Main, "minecraft:tube_coral_fan", 64);
        f.click_face([0, -60, 2], Direction::North);
        let fan = f.get([0, -60, 1]);
        assert!(fan.starts_with("minecraft:tube_coral_wall_fan"), "{fan}");
        assert!(
            fan.contains("facing=north") && fan.contains("waterlogged=true"),
            "{fan}"
        );
    }
}
