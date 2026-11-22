//! Structural blocks: slabs, stairs, doors, trapdoors, fences, fence gates, walls, panes and bars, chains, lanterns, ladders, scaffolding, snow layers, carpets, leaves, pillars, rods, paths and farmland, falling blocks, liquids and portals, pistons.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Many of these blocks only schedule a tick when a neighbour changes (water in a
//! waterlogged block, falling blocks, leaf decay, scaffolding stability, paths and
//! farmland turning back to dirt). Scheduled ticks run on the server, so on the
//! client those blocks keep their state.

use super::{Behaviour, Class, Pos, Support, Update, Use, flags, state};
use crate::direction::Direction;
use crate::fluid::Fluid;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::place::PlaceContext;
use crate::player::LocalPlayer;
use crate::shape::Axis;

/// The (class, behaviour) pairs this group answers for; every pair the export lists
/// must be in some group's list (see [`super`]'s coverage test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PORTED: &[(Class, Behaviour)] = &[
    (Class::SnowyBlock, Behaviour::GetStateForPlacement),
    (Class::SnowyBlock, Behaviour::UpdateShape),
    (Class::RotatedPillarBlock, Behaviour::GetStateForPlacement),
    (
        Class::InfestedRotatedPillarBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::LiquidBlock, Behaviour::UpdateShape),
    (Class::FallingBlock, Behaviour::UpdateShape),
    (Class::BrushableBlock, Behaviour::UpdateShape),
    (Class::ConcretePowderBlock, Behaviour::GetStateForPlacement),
    (Class::ConcretePowderBlock, Behaviour::UpdateShape),
    (Class::MangroveRootsBlock, Behaviour::GetStateForPlacement),
    (Class::MangroveRootsBlock, Behaviour::UpdateShape),
    (Class::LeavesBlock, Behaviour::GetStateForPlacement),
    (Class::LeavesBlock, Behaviour::UpdateShape),
    (Class::StairBlock, Behaviour::GetStateForPlacement),
    (Class::StairBlock, Behaviour::UpdateShape),
    (Class::SlabBlock, Behaviour::GetStateForPlacement),
    (Class::SlabBlock, Behaviour::UpdateShape),
    (Class::SlabBlock, Behaviour::CanBeReplaced),
    (Class::DoorBlock, Behaviour::GetStateForPlacement),
    (Class::DoorBlock, Behaviour::UpdateShape),
    (Class::DoorBlock, Behaviour::CanSurvive),
    (Class::DoorBlock, Behaviour::UseWithoutItem),
    (Class::DoorBlock, Behaviour::SetPlacedBy),
    (Class::TrapDoorBlock, Behaviour::GetStateForPlacement),
    (Class::TrapDoorBlock, Behaviour::UpdateShape),
    (Class::TrapDoorBlock, Behaviour::UseWithoutItem),
    (Class::FenceBlock, Behaviour::GetStateForPlacement),
    (Class::FenceBlock, Behaviour::UpdateShape),
    (Class::FenceBlock, Behaviour::UseWithoutItem),
    (Class::FenceGateBlock, Behaviour::GetStateForPlacement),
    (Class::FenceGateBlock, Behaviour::UpdateShape),
    (Class::FenceGateBlock, Behaviour::UseWithoutItem),
    (Class::IronBarsBlock, Behaviour::GetStateForPlacement),
    (Class::IronBarsBlock, Behaviour::UpdateShape),
    (Class::WallBlock, Behaviour::GetStateForPlacement),
    (Class::WallBlock, Behaviour::UpdateShape),
    (Class::ChainBlock, Behaviour::GetStateForPlacement),
    (Class::ChainBlock, Behaviour::UpdateShape),
    (Class::LanternBlock, Behaviour::GetStateForPlacement),
    (Class::LanternBlock, Behaviour::UpdateShape),
    (Class::LanternBlock, Behaviour::CanSurvive),
    (Class::LadderBlock, Behaviour::GetStateForPlacement),
    (Class::LadderBlock, Behaviour::UpdateShape),
    (Class::LadderBlock, Behaviour::CanSurvive),
    (Class::ScaffoldingBlock, Behaviour::GetStateForPlacement),
    (Class::ScaffoldingBlock, Behaviour::UpdateShape),
    (Class::ScaffoldingBlock, Behaviour::CanSurvive),
    (Class::ScaffoldingBlock, Behaviour::CanBeReplaced),
    (Class::SnowLayerBlock, Behaviour::GetStateForPlacement),
    (Class::SnowLayerBlock, Behaviour::UpdateShape),
    (Class::SnowLayerBlock, Behaviour::CanSurvive),
    (Class::SnowLayerBlock, Behaviour::CanBeReplaced),
    (Class::CarpetBlock, Behaviour::UpdateShape),
    (Class::CarpetBlock, Behaviour::CanSurvive),
    (Class::BarrierBlock, Behaviour::GetStateForPlacement),
    (Class::BarrierBlock, Behaviour::UpdateShape),
    (
        Class::WaterloggedTransparentBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::WaterloggedTransparentBlock, Behaviour::UpdateShape),
    (Class::HeavyCoreBlock, Behaviour::GetStateForPlacement),
    (Class::HeavyCoreBlock, Behaviour::UpdateShape),
    (Class::ConduitBlock, Behaviour::GetStateForPlacement),
    (Class::ConduitBlock, Behaviour::UpdateShape),
    (Class::LightningRodBlock, Behaviour::GetStateForPlacement),
    (Class::LightningRodBlock, Behaviour::UpdateShape),
    (Class::EndRodBlock, Behaviour::GetStateForPlacement),
    (Class::HugeMushroomBlock, Behaviour::GetStateForPlacement),
    (Class::HugeMushroomBlock, Behaviour::UpdateShape),
    (Class::PathBlock, Behaviour::GetStateForPlacement),
    (Class::PathBlock, Behaviour::UpdateShape),
    (Class::PathBlock, Behaviour::CanSurvive),
    (Class::FarmlandBlock, Behaviour::GetStateForPlacement),
    (Class::FarmlandBlock, Behaviour::UpdateShape),
    (Class::FarmlandBlock, Behaviour::CanSurvive),
    (Class::NetherPortalBlock, Behaviour::UpdateShape),
    (Class::EndPortalBlock, Behaviour::CanBeReplacedByFluid),
    (Class::EndGatewayBlock, Behaviour::CanBeReplacedByFluid),
    (Class::PistonBaseBlock, Behaviour::GetStateForPlacement),
    (Class::PistonBaseBlock, Behaviour::SetPlacedBy),
    (Class::PistonHeadBlock, Behaviour::UpdateShape),
    (Class::PistonHeadBlock, Behaviour::CanSurvive),
    (Class::MovingPistonBlock, Behaviour::UseWithoutItem),
];

/// `getStateForPlacement` (`Some(None)`: the block can't be placed there).
pub(super) fn state_for_placement(
    c: Class,
    w: &World<'_>,
    ctx: &PlaceContext,
    default: u32,
) -> Option<Option<u32>> {
    let pos = ctx.clicked_pos();
    let placed = match c {
        Class::SnowyBlock => {
            state::set_bool(default, "snowy", is_snow(w, w.get(Direction::Up.step(pos))))
        }
        Class::RotatedPillarBlock | Class::InfestedRotatedPillarBlock => pillar(ctx, default),
        Class::ChainBlock => with_water_source(w, pos, pillar(ctx, default)),
        Class::MangroveRootsBlock
        | Class::BarrierBlock
        | Class::WaterloggedTransparentBlock
        | Class::HeavyCoreBlock => with_water_source(w, pos, default),
        Class::ConduitBlock => {
            let f = w.fluid(pos);
            state::set_bool(default, "waterlogged", f.fluid.is_water() && f.amount == 8)
        }
        Class::LightningRodBlock => with_water_source(
            w,
            pos,
            state::set_dir(default, "facing", ctx.clicked_face()),
        ),
        Class::ConcretePowderBlock => concrete_powder_placement(w, pos, default),
        Class::LeavesBlock => {
            let s = with_water_source(w, pos, state::set_bool(default, "persistent", true));
            state::set_int(s, "distance", leaf_distance(w, pos))
        }
        Class::StairBlock => stairs_placement(w, ctx, default),
        Class::SlabBlock => slab_placement(w, ctx, default),
        Class::DoorBlock => return Some(door_placement(w, ctx, default)),
        Class::TrapDoorBlock => trapdoor_placement(w, ctx, default),
        Class::FenceBlock => {
            with_water_source(w, pos, connect_sides(w, default, pos, fence_connects))
        }
        Class::IronBarsBlock => {
            with_water_source(w, pos, connect_sides(w, default, pos, bars_connect))
        }
        Class::FenceGateBlock => gate_placement(w, ctx, default),
        Class::WallBlock => wall_placement(w, pos, default),
        Class::LanternBlock => return Some(lantern_placement(w, ctx, default)),
        Class::LadderBlock => return Some(ladder_placement(w, ctx, default)),
        Class::ScaffoldingBlock => {
            let distance = scaffolding_distance(w, pos);
            let s = with_water_source(w, pos, state::set_int(default, "distance", distance));
            state::set_bool(s, "bottom", scaffolding_bottom(w, pos, distance))
        }
        Class::SnowLayerBlock => {
            let here = w.get(pos);
            if state::same_block(here, default) {
                state::set_int(here, "layers", (state::get_int(here, "layers") + 1).min(8))
            } else {
                default
            }
        }
        Class::EndRodBlock => end_rod_placement(w, ctx, default),
        Class::HugeMushroomBlock => Direction::ALL.into_iter().fold(default, |s, d| {
            let same = state::same_block(w.get(d.step(pos)), default);
            state::set_bool(s, d.name(), !same)
        }),
        Class::PathBlock => {
            if super::can_survive(w, default, pos) {
                default
            } else {
                // `Block.pushEntitiesUp` also lifts entities standing where the path
                // was lower than the dirt; this client places blocks without moving
                // anyone (entities in the way then stop the placement instead).
                state::default_of("minecraft:dirt")
            }
        }
        Class::FarmlandBlock => {
            if super::can_survive(w, default, pos) {
                default
            } else {
                base_block(w, default, "FarmlandBlock.baseBlock", "minecraft:dirt")
            }
        }
        Class::PistonBaseBlock => {
            let s = state::set_dir(
                default,
                "facing",
                ctx.nearest_looking_direction().opposite(),
            );
            state::set_bool(s, "extended", false)
        }
        _ => return None,
    };
    Some(Some(placed))
}

/// `updateShape`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    let s = u.state;
    let d = u.direction;
    let updated = match c {
        // These only schedule ticks (fluids, falling, decay, stability, a path or
        // farmland under something solid turning back to dirt): the server's work.
        Class::LiquidBlock
        | Class::FallingBlock
        | Class::BrushableBlock
        | Class::MangroveRootsBlock
        | Class::LeavesBlock
        | Class::SlabBlock
        | Class::TrapDoorBlock
        | Class::ChainBlock
        | Class::ScaffoldingBlock
        | Class::BarrierBlock
        | Class::WaterloggedTransparentBlock
        | Class::HeavyCoreBlock
        | Class::ConduitBlock
        | Class::LightningRodBlock
        | Class::PathBlock
        | Class::FarmlandBlock => s,
        Class::SnowyBlock if d == Direction::Up => {
            state::set_bool(s, "snowy", is_snow(w, u.neighbour))
        }
        Class::SnowyBlock => s,
        Class::ConcretePowderBlock if touches_water(w, u.pos) => concrete_of(w, s),
        Class::ConcretePowderBlock => s,
        Class::StairBlock if d.is_horizontal() => state::set(s, "shape", stairs_shape(w, s, u.pos)),
        Class::StairBlock => s,
        Class::DoorBlock => door_update(w, u),
        Class::FenceBlock if d.is_horizontal() => {
            let joined = fence_connects(
                w,
                s,
                u.neighbour,
                sturdy_towards(w, u.neighbour, d.opposite()),
                d.opposite(),
            );
            state::set_bool(s, d.name(), joined)
        }
        Class::IronBarsBlock if d.is_horizontal() => {
            let joined = bars_connect(
                w,
                s,
                u.neighbour,
                sturdy_towards(w, u.neighbour, d.opposite()),
                d.opposite(),
            );
            state::set_bool(s, d.name(), joined)
        }
        Class::FenceBlock | Class::IronBarsBlock => s,
        Class::FenceGateBlock => gate_update(w, u),
        Class::WallBlock => wall_update(w, u),
        Class::LanternBlock if d == lantern_support_side(s) && !super::can_survive(w, s, u.pos) => {
            state::AIR
        }
        Class::LanternBlock => s,
        Class::LadderBlock
            if Some(d.opposite()) == state::get_dir(s, "facing")
                && !super::can_survive(w, s, u.pos) =>
        {
            state::AIR
        }
        Class::LadderBlock => s,
        Class::SnowLayerBlock | Class::CarpetBlock if !super::can_survive(w, s, u.pos) => {
            state::AIR
        }
        Class::SnowLayerBlock | Class::CarpetBlock => s,
        Class::HugeMushroomBlock if state::same_block(u.neighbour, s) => {
            state::set_bool(s, d.name(), false)
        }
        Class::HugeMushroomBlock => s,
        Class::NetherPortalBlock => portal_update(w, u),
        Class::PistonHeadBlock
            if Some(d.opposite()) == state::get_dir(s, "facing")
                && !super::can_survive(w, s, u.pos) =>
        {
            state::AIR
        }
        Class::PistonHeadBlock => s,
        _ => return None,
    };
    Some(updated)
}

/// `canSurvive`.
pub(super) fn can_survive(c: Class, w: &World<'_>, state: u32, pos: Pos) -> Option<bool> {
    let below = Direction::Down.step(pos);
    let above = Direction::Up.step(pos);
    let survives = match c {
        Class::DoorBlock => door_survives(w, state, pos),
        Class::LanternBlock => {
            let side = lantern_support_side(state);
            w.can_support_center(side.step(pos), side.opposite())
        }
        Class::LadderBlock => {
            let Some(facing) = state::get_dir(state, "facing") else {
                return Some(false);
            };
            w.face_sturdy(facing.opposite().step(pos), facing, Support::Full)
        }
        Class::ScaffoldingBlock => scaffolding_distance(w, pos) < 7,
        Class::SnowLayerBlock => snow_layer_survives(w, state, below),
        Class::CarpetBlock => !w.is_empty_block(below),
        Class::PathBlock => {
            let top = w.get(above);
            !w.blocks.solid(top) || w.blocks.is_class(top, Class::FenceGateBlock)
        }
        Class::FarmlandBlock => {
            let top = w.get(above);
            !w.blocks.solid(top) || w.tags.is(top, "minecraft:maintains_farmland")
        }
        Class::PistonHeadBlock => piston_head_survives(w, state, pos),
        _ => return None,
    };
    Some(survives)
}

/// `canBeReplaced(state, context)`.
pub(super) fn can_be_replaced(
    c: Class,
    w: &World<'_>,
    state: u32,
    ctx: &PlaceContext,
) -> Option<bool> {
    let replaced = match c {
        Class::SlabBlock => slab_can_be_replaced(w, state, ctx),
        // `ScaffoldingBlock.canBeReplaced`: by more scaffolding, whatever the click.
        Class::ScaffoldingBlock => holds_own_item(w, state, ctx),
        Class::SnowLayerBlock => {
            let layers = state::get_int(state, "layers");
            if !holds_own_item(w, state, ctx) || layers >= 8 {
                // Anything may go over a single layer.
                layers == 1
            } else {
                // More snow piles up when it lands on top, or goes in front of it.
                !ctx.replacing_clicked_on_block() || ctx.clicked_face() == Direction::Up
            }
        }
        _ => return None,
    };
    Some(replaced)
}

/// `canBeReplaced(state, fluid)` (`water`: the fluid is water, else lava).
pub(super) fn can_be_replaced_by_fluid(
    c: Class,
    _w: &World<'_>,
    _state: u32,
    _water: bool,
) -> Option<bool> {
    match c {
        // End portals and gateways never wash away.
        Class::EndPortalBlock | Class::EndGatewayBlock => Some(false),
        _ => None,
    }
}

/// `useItemOn`.
pub(super) fn use_item_on(
    _c: Class,
    _w: &mut World<'_>,
    _p: &mut LocalPlayer,
    _u: &Use<'_>,
) -> Option<Outcome> {
    None
}

/// `useWithoutItem`.
pub(super) fn use_without_item(
    c: Class,
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    u: &Use<'_>,
) -> Option<Outcome> {
    let outcome = match c {
        // `DoorBlock.useWithoutItem`: a door its set lets hands open flips open or
        // shut (`UPDATE_CLIENTS | UPDATE_IMMEDIATE`; the other half follows through
        // its shape update). Iron doors pass.
        Class::DoorBlock if opens_by_hand(w, u.state, "DoorBlock.type") => {
            w.set_block(
                u.pos,
                state::cycle(u.state, "open"),
                flags::CLIENTS | flags::IMMEDIATE,
            );
            Outcome::Success
        }
        // `TrapDoorBlock.useWithoutItem`: the same for trapdoors, with
        // `UPDATE_CLIENTS` only.
        Class::TrapDoorBlock if opens_by_hand(w, u.state, "TrapDoorBlock.type") => {
            w.set_block(u.pos, state::cycle(u.state, "open"), flags::CLIENTS);
            Outcome::Success
        }
        Class::DoorBlock | Class::TrapDoorBlock => Outcome::Pass,
        Class::FenceGateBlock => {
            w.set_block(
                u.pos,
                gate_toggled(p, u.state),
                flags::CLIENTS | flags::IMMEDIATE,
            );
            Outcome::Success
        }
        // `FenceBlock.useWithoutItem` ties the player's leashed mobs to the fence on
        // the server only; `MovingPistonBlock.useWithoutItem` clears a moving piston
        // without its block entity, also only on the server. The client passes.
        Class::FenceBlock | Class::MovingPistonBlock => Outcome::Pass,
        _ => return None,
    };
    Some(outcome)
}

/// `updateIndirectNeighbourShapes`.
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

/// `setPlacedBy`.
pub(super) fn set_placed_by(
    c: Class,
    w: &mut World<'_>,
    _p: &LocalPlayer,
    state: u32,
    pos: Pos,
    _hand: Hand,
) -> Option<()> {
    match c {
        // `DoorBlock.setPlacedBy`: the upper half goes above the placed lower half,
        // with neighbour updates.
        Class::DoorBlock => {
            w.set_block_and_update(Direction::Up.step(pos), state::set(state, "half", "upper"));
        }
        // `PistonBaseBlock.setPlacedBy` only checks for power on the server.
        Class::PistonBaseBlock => {}
        _ => return None,
    }
    Some(())
}

/// `attack` (a left click on the block as breaking starts).
pub(super) fn attack(
    _c: Class,
    _w: &mut World<'_>,
    _p: &LocalPlayer,
    _state: u32,
    _pos: Pos,
) -> Option<()> {
    None
}

/// `mayPlaceOn`: whether a plant may stand on `below` (the block under it, at
/// `below_pos`).
pub(super) fn may_place_on(
    _c: Class,
    _w: &World<'_>,
    _plant: u32,
    _below: u32,
    _below_pos: Pos,
) -> Option<bool> {
    None
}

/// `isValidBonemealTarget`.
pub(super) fn is_valid_bonemeal_target(
    _c: Class,
    _w: &World<'_>,
    _state: u32,
    _pos: Pos,
) -> Option<bool> {
    None
}

/// `ScaffoldingBlock.getDistance`: how far scaffolding at `pos` would be from
/// scaffolding standing on the ground (7: too far to stay). Standing on scaffolding
/// it starts from that one's distance; on a block with a sturdy top it is 0
/// outright; otherwise 7. Then each horizontal neighbour (north, east, south, west)
/// that is scaffolding offers its distance plus one, stopping early at 1.
pub(super) fn scaffolding_distance(w: &World<'_>, pos: Pos) -> i32 {
    let below = Direction::Down.step(pos);
    let under = w.get(below);
    let mut distance = 7;
    if is_scaffolding(under) {
        distance = state::get_int(under, "distance");
    } else if w.face_sturdy(below, Direction::Up, Support::Full) {
        return 0;
    }
    for side in Direction::HORIZONTAL {
        let s = w.get(side.step(pos));
        if is_scaffolding(s) {
            distance = distance.min(state::get_int(s, "distance") + 1);
            if distance == 1 {
                break;
            }
        }
    }
    distance
}

// --- Shared rules -------------------------------------------------------------

/// The `waterlogged` property set when the placed block replaces a water source
/// (`FluidState.is(Fluids.WATER)`: flowing water doesn't count).
fn with_water_source(w: &World<'_>, pos: Pos, s: u32) -> u32 {
    state::set_bool(s, "waterlogged", w.fluid(pos).fluid == Fluid::Water)
}

/// The item in hand is the block's own item (`ItemStack.is(asItem())`).
fn holds_own_item(w: &World<'_>, s: u32, ctx: &PlaceContext) -> bool {
    let item = ctx.item();
    !item.is_empty() && super::item_id(item) == w.blocks.kind(s).item
}

/// Whether a slab or stair placed with `ctx` goes in the lower half: when the top
/// of a block was clicked, or any face but the bottom at or below the middle of the
/// block it goes into.
fn lower_half(ctx: &PlaceContext) -> bool {
    let face = ctx.clicked_face();
    face == Direction::Up
        || face != Direction::Down
            && ctx.click_location()[1] - f64::from(ctx.clicked_pos()[1]) <= 0.5
}

/// A pillar's axis follows the clicked face (`RotatedPillarBlock.getStateForPlacement`).
fn pillar(ctx: &PlaceContext, default: u32) -> u32 {
    let axis = match ctx.clicked_face().axis() {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    };
    state::set(default, "axis", axis)
}

/// `SnowyBlock.isSnowySetting`: the block above is in `#minecraft:snow`.
fn is_snow(w: &World<'_>, above: u32) -> bool {
    w.tags.is(above, "minecraft:snow")
}

/// A block named by one of the block's constructor settings, as its default state
/// (`fallback` if the export lacks it).
fn base_block(w: &World<'_>, s: u32, field: &str, fallback: &str) -> u32 {
    let name = w.blocks.kind(s).field(field).unwrap_or(fallback);
    state::default_of(name)
}

/// Whether the block's `BlockSetType` (in the constructor setting `field`) lets
/// players open it by hand (iron doors and trapdoors don't).
fn opens_by_hand(w: &World<'_>, s: u32, field: &str) -> bool {
    w.blocks.kind(s).record_part(field, "canOpenByHand") != Some("false")
}

fn is_scaffolding(s: u32) -> bool {
    state::is(s, "minecraft:scaffolding")
}

/// `isFaceSturdy` (full support) of `s` on its face towards `direction`.
fn sturdy_towards(w: &World<'_>, s: u32, direction: Direction) -> bool {
    w.blocks.face_sturdy(s, direction, Support::Full)
}

// --- Concrete powder ------------------------------------------------------------

/// `ConcretePowderBlock.getStateForPlacement`: powder placed into water, or next
/// to water, sets at once into its concrete.
fn concrete_powder_placement(w: &World<'_>, pos: Pos, default: u32) -> u32 {
    if holds_water(w, w.get(pos)) || touches_water(w, pos) {
        concrete_of(w, default)
    } else {
        default
    }
}

/// The concrete a powder sets into.
fn concrete_of(w: &World<'_>, powder: u32) -> u32 {
    base_block(
        w,
        powder,
        "ConcretePowderBlock.concrete",
        "minecraft:white_concrete",
    )
}

fn holds_water(w: &World<'_>, s: u32) -> bool {
    w.blocks.fluid(s).fluid.is_water()
}

/// `ConcretePowderBlock.touchesLiquid`: a neighbour holding water whose face
/// towards the powder isn't sturdy, checked in direction order. The one below only
/// counts when the powder's own position holds water (as it does only while powder
/// is placed into water).
fn touches_water(w: &World<'_>, pos: Pos) -> bool {
    Direction::ALL.into_iter().any(|d| {
        if d == Direction::Down && !holds_water(w, w.get(pos)) {
            return false;
        }
        let n = w.get(d.step(pos));
        holds_water(w, n) && !sturdy_towards(w, n, d.opposite())
    })
}

// --- Leaves -----------------------------------------------------------------------

/// `LeavesBlock.updateDistance`: one more than the nearest distance among the six
/// neighbours, at most 7.
fn leaf_distance(w: &World<'_>, pos: Pos) -> i32 {
    let mut distance = 7;
    for d in Direction::ALL {
        distance = distance.min(distance_to_log(w, w.get(d.step(pos))) + 1);
        if distance == 1 {
            break;
        }
    }
    distance
}

/// `LeavesBlock.getDistanceAt`: 0 for blocks that keep leaves alive (logs), a
/// leaf's own distance, else 7.
fn distance_to_log(w: &World<'_>, s: u32) -> i32 {
    if w.tags.is(s, "minecraft:prevents_nearby_leaf_decay") {
        0
    } else if w.blocks.is_class(s, Class::LeavesBlock) {
        state::get_int(s, "distance")
    } else {
        7
    }
}

// --- Stairs and slabs ---------------------------------------------------------------

fn is_stairs(w: &World<'_>, s: u32) -> bool {
    w.blocks.is_class(s, Class::StairBlock)
}

/// `StairBlock.getStateForPlacement`: facing the way the player looks, upside down
/// when placed under a block or high on a side, waterlogged in a water source, and
/// shaped by the stairs around it.
fn stairs_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let s = state::set_dir(default, "facing", ctx.horizontal_direction());
    let s = state::set(s, "half", if lower_half(ctx) { "bottom" } else { "top" });
    let s = with_water_source(w, pos, s);
    state::set(s, "shape", stairs_shape(w, s, pos))
}

/// `StairBlock.getStairsShape`. Stairs of the same half behind (the way they
/// face) turned across them make an outer corner, unless the stairs on the side
/// the corner would open to continue this one; failing that, such stairs in front
/// make an inner corner, unless the stairs on their side continue this one. Left
/// is when the other stairs face this one's counter-clockwise turn.
fn stairs_shape(w: &World<'_>, s: u32, pos: Pos) -> &'static str {
    let Some(facing) = state::get_dir(s, "facing") else {
        return "straight";
    };
    let half = state::get(s, "half");
    let across = |n: u32| -> Option<Direction> {
        let d = state::get_dir(n, "facing")?;
        (is_stairs(w, n) && state::get(n, "half") == half && d.axis() != facing.axis()).then_some(d)
    };
    // The stairs on that side don't run on from this one.
    let free = |side: Direction| {
        let n = w.get(side.step(pos));
        !(is_stairs(w, n)
            && state::get_dir(n, "facing") == Some(facing)
            && state::get(n, "half") == half)
    };
    if let Some(d) = across(w.get(facing.step(pos)))
        && free(d.opposite())
    {
        return if d == facing.counter_clockwise() {
            "outer_left"
        } else {
            "outer_right"
        };
    }
    if let Some(d) = across(w.get(facing.opposite().step(pos)))
        && free(d)
    {
        return if d == facing.counter_clockwise() {
            "inner_left"
        } else {
            "inner_right"
        };
    }
    "straight"
}

/// `SlabBlock.getStateForPlacement`: placed into a slab of its own kind it makes a
/// double slab (never waterlogged); else a bottom or top slab by the click, as
/// stairs choose their half, waterlogged in a water source.
fn slab_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let here = w.get(pos);
    if state::same_block(here, default) {
        let double = state::set(here, "type", "double");
        return state::set_bool(double, "waterlogged", false);
    }
    let s = with_water_source(w, pos, default);
    state::set(s, "type", if lower_half(ctx) { "bottom" } else { "top" })
}

/// `SlabBlock.canBeReplaced`: only a single slab, by its own item. Clicked on
/// directly, a bottom slab takes the second slab from its top face or the upper
/// half of a side; a top slab from its bottom face or the lower half of a side. A
/// slab in front of the clicked face always may be.
fn slab_can_be_replaced(w: &World<'_>, s: u32, ctx: &PlaceContext) -> bool {
    let kind = state::get(s, "type");
    if kind == Some("double") || !holds_own_item(w, s, ctx) {
        return false;
    }
    if !ctx.replacing_clicked_on_block() {
        return true;
    }
    let upper = ctx.click_location()[1] - f64::from(ctx.clicked_pos()[1]) > 0.5;
    let face = ctx.clicked_face();
    if kind == Some("bottom") {
        face == Direction::Up || upper && face.is_horizontal()
    } else {
        face == Direction::Down || !upper && face.is_horizontal()
    }
}

// --- Doors and trapdoors ------------------------------------------------------------

/// `DoorBlock.getStateForPlacement`: only below the top of the level with room for
/// the upper half (the block above may be built over); faces the way the player
/// looks, opens and is powered when either half's position has redstone power.
fn door_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let above = Direction::Up.step(pos);
    if pos[1] >= w.max_y() || !super::can_be_replaced(w, w.get(above), ctx) {
        return None;
    }
    let powered = w.has_neighbor_signal(pos) || w.has_neighbor_signal(above);
    let s = state::set_dir(default, "facing", ctx.horizontal_direction());
    let s = state::set(s, "hinge", door_hinge(w, ctx));
    let s = state::set_bool(state::set_bool(s, "powered", powered), "open", powered);
    Some(state::set(s, "half", "lower"))
}

/// `DoorBlock.getHinge`. Looking the way the door faces, full blocks beside the
/// two halves weigh it: more on the right puts the hinge right, more on the left
/// puts it left. A lone lower door half to one side wins first: on the left it
/// makes this door hinge right (a double door), on the right, left. With nothing to
/// decide, the click's side of the block's middle does: the hinge goes on the side
/// clicked, the right when the click is right of the middle as the player sees it.
fn door_hinge(w: &World<'_>, ctx: &PlaceContext) -> &'static str {
    let pos = ctx.clicked_pos();
    let facing = ctx.horizontal_direction();
    let (left, right) = (facing.counter_clockwise(), facing.clockwise());
    let full = |at: Pos| i32::from(w.blocks.collision_full_block(w.get(at)));
    let above = Direction::Up.step(pos);
    let balance = full(right.step(pos)) + full(right.step(above))
        - full(left.step(pos))
        - full(left.step(above));
    let lower_door = |at: Pos| {
        let s = w.get(at);
        w.blocks.is_class(s, Class::DoorBlock) && state::get(s, "half") == Some("lower")
    };
    let door_left = lower_door(left.step(pos));
    let door_right = lower_door(right.step(pos));
    if door_left && !door_right || balance > 0 {
        return "right";
    }
    if door_right && !door_left || balance < 0 {
        return "left";
    }
    let at = ctx.click_location();
    let x = at[0] - f64::from(pos[0]);
    let z = at[2] - f64::from(pos[2]);
    let [sx, _, sz] = facing.normal();
    let clicked_right =
        sx < 0 && z < 0.5 || sx > 0 && z > 0.5 || sz < 0 && x > 0.5 || sz > 0 && x < 0.5;
    if clicked_right { "right" } else { "left" }
}

/// `DoorBlock.updateShape`. The half that hears from its other half copies that
/// half's state (keeping its own half), or can't stay when the other half is gone.
/// A lower half whose support below goes can't stay either.
fn door_update(w: &World<'_>, u: &Update) -> u32 {
    let half = state::get(u.state, "half");
    let lower = half == Some("lower");
    let towards_partner = u.direction
        == if lower {
            Direction::Up
        } else {
            Direction::Down
        };
    if !towards_partner {
        if lower && u.direction == Direction::Down && !super::can_survive(w, u.state, u.pos) {
            return state::AIR;
        }
        return u.state;
    }
    let partner = u.neighbour;
    if w.blocks.is_class(partner, Class::DoorBlock) && state::get(partner, "half") != half {
        state::set(partner, "half", half.unwrap_or("lower"))
    } else {
        state::AIR
    }
}

/// `DoorBlock.canSurvive`: a lower half needs a sturdy top below it; an upper half
/// needs its own door below.
fn door_survives(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let below = Direction::Down.step(pos);
    if state::get(s, "half") == Some("lower") {
        w.face_sturdy(below, Direction::Up, Support::Full)
    } else {
        state::same_block(w.get(below), s)
    }
}

/// `TrapDoorBlock.getStateForPlacement`. Against the side of a block it hinges on
/// that side (facing away from it), in the half of the block the click was in;
/// otherwise it faces the player, at the bottom when placed on top of a block and at
/// the top when placed under one or over a replaced block. Power opens it.
fn trapdoor_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let face = ctx.clicked_face();
    let (facing, top) = if !ctx.replacing_clicked_on_block() && face.is_horizontal() {
        (face, ctx.click_location()[1] - f64::from(pos[1]) > 0.5)
    } else {
        (ctx.horizontal_direction().opposite(), face != Direction::Up)
    };
    let mut s = state::set_dir(default, "facing", facing);
    s = state::set(s, "half", if top { "top" } else { "bottom" });
    if w.has_neighbor_signal(pos) {
        s = state::set_bool(state::set_bool(s, "open", true), "powered", true);
    }
    with_water_source(w, pos, s)
}

// --- Fences, bars, gates and walls --------------------------------------------------

/// `Block.isExceptionForConnection`: blocks fences, bars and walls don't join even
/// by a sturdy face (leaves, barriers, pumpkins and jack o'lanterns, melons,
/// shulker boxes).
fn no_connection(w: &World<'_>, s: u32) -> bool {
    w.blocks.is_class(s, Class::LeavesBlock)
        || matches!(
            state::name(s),
            "minecraft:barrier"
                | "minecraft:carved_pumpkin"
                | "minecraft:jack_o_lantern"
                | "minecraft:melon"
                | "minecraft:pumpkin"
        )
        || w.tags.is(s, "minecraft:shulker_boxes")
}

/// `FenceGateBlock.connectsToDirection`: a gate joins blocks beside its posts
/// (`towards` is the direction from the gate to the block joining it).
fn gate_joins(w: &World<'_>, gate: u32, towards: Direction) -> bool {
    w.blocks.is_class(gate, Class::FenceGateBlock)
        && state::get_dir(gate, "facing").is_some_and(|f| f.axis() == towards.clockwise().axis())
}

/// `FenceBlock.connectsTo`: a fence joins a sturdy face (save the exceptions),
/// fences of the same family (wooden with wooden, others with others) and gates
/// turned its way. `towards` is the direction from the neighbour to the fence.
fn fence_connects(w: &World<'_>, fence: u32, other: u32, sturdy: bool, towards: Direction) -> bool {
    let wooden = |s: u32| w.tags.is(s, "minecraft:wooden_fences");
    let same_family = w.tags.is(other, "minecraft:fences") && wooden(other) == wooden(fence);
    !no_connection(w, other) && sturdy || same_family || gate_joins(w, other, towards)
}

/// `IronBarsBlock.attachsTo`: bars and panes join a sturdy face (save the
/// exceptions), other bars and panes, and walls.
fn bars_connect(w: &World<'_>, _bars: u32, other: u32, sturdy: bool, _towards: Direction) -> bool {
    !no_connection(w, other) && sturdy
        || w.blocks.is_class(other, Class::IronBarsBlock)
        || w.tags.is(other, "minecraft:walls")
}

/// `WallBlock.connectsTo`: a wall joins other walls, a sturdy face (save the
/// exceptions), bars and panes, and gates turned its way.
fn wall_connects(w: &World<'_>, other: u32, sturdy: bool, towards: Direction) -> bool {
    w.tags.is(other, "minecraft:walls")
        || !no_connection(w, other) && sturdy
        || w.blocks.is_class(other, Class::IronBarsBlock)
        || gate_joins(w, other, towards)
}

/// Sets each horizontal side's boolean property by whether the block joins its
/// neighbour there (fences, bars and panes as placed).
fn connect_sides(
    w: &World<'_>,
    s: u32,
    pos: Pos,
    joins: fn(&World<'_>, u32, u32, bool, Direction) -> bool,
) -> u32 {
    Direction::HORIZONTAL.into_iter().fold(s, |acc, d| {
        let n = w.get(d.step(pos));
        let joined = joins(w, s, n, sturdy_towards(w, n, d.opposite()), d.opposite());
        state::set_bool(acc, d.name(), joined)
    })
}

/// `FenceGateBlock.getStateForPlacement`: faces the way the player looks, sits
/// lower when a wall is beside a post, and is open and powered with redstone power.
fn gate_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let facing = ctx.horizontal_direction();
    let powered = w.has_neighbor_signal(pos);
    let posts = [facing.clockwise(), facing.counter_clockwise()];
    let in_wall = posts
        .iter()
        .any(|d| w.tags.is(w.get(d.step(pos)), "minecraft:walls"));
    let s = state::set_dir(default, "facing", facing);
    let s = state::set_bool(state::set_bool(s, "open", powered), "powered", powered);
    state::set_bool(s, "in_wall", in_wall)
}

/// `FenceGateBlock.updateShape`: a change beside one of the posts sets `in_wall` by
/// whether either post has a wall beside it.
fn gate_update(w: &World<'_>, u: &Update) -> u32 {
    let Some(facing) = state::get_dir(u.state, "facing") else {
        return u.state;
    };
    if facing.clockwise().axis() != u.direction.axis() {
        return u.state;
    }
    let wall = |s: u32| w.tags.is(s, "minecraft:walls");
    let in_wall = wall(u.neighbour) || wall(w.get(u.direction.opposite().step(u.pos)));
    state::set_bool(u.state, "in_wall", in_wall)
}

/// `FenceGateBlock.useWithoutItem`: an open gate closes; a closed one opens, first
/// turning to face the way the player looks if it faced them.
fn gate_toggled(p: &LocalPlayer, s: u32) -> u32 {
    if state::get_bool(s, "open") {
        return state::set_bool(s, "open", false);
    }
    let looking = Direction::from_y_rot(f64::from(p.y_rot));
    let s = if state::get_dir(s, "facing") == Some(looking.opposite()) {
        state::set_dir(s, "facing", looking)
    } else {
        s
    };
    state::set_bool(s, "open", true)
}

/// `WallBlock.getStateForPlacement`: joined to each side as [`wall_connects`] says,
/// then shaped by the block above.
fn wall_placement(w: &World<'_>, pos: Pos, default: u32) -> u32 {
    let joined = Direction::HORIZONTAL.map(|d| {
        let n = w.get(d.step(pos));
        wall_connects(w, n, sturdy_towards(w, n, d.opposite()), d.opposite())
    });
    let s = with_water_source(w, pos, default);
    wall_shaped(w, s, Direction::Up.step(pos), joined)
}

/// `WallBlock.updateShape`: the block below changes nothing; the block above
/// reshapes the wall's sides and post; a side's neighbour sets whether that side
/// joins, then the same reshaping.
fn wall_update(w: &World<'_>, u: &Update) -> u32 {
    let joined_now = |d: Direction| state::get(u.state, d.name()) != Some("none");
    match u.direction {
        Direction::Down => u.state,
        Direction::Up => wall_shaped(
            w,
            u.state,
            u.neighbour_pos,
            Direction::HORIZONTAL.map(joined_now),
        ),
        changed => {
            let joined = Direction::HORIZONTAL.map(|d| {
                if d == changed {
                    let towards = d.opposite();
                    wall_connects(
                        w,
                        u.neighbour,
                        sturdy_towards(w, u.neighbour, towards),
                        towards,
                    )
                } else {
                    joined_now(d)
                }
            });
            wall_shaped(w, u.state, Direction::Up.step(u.pos), joined)
        }
    }
}

/// The square each wall test covers on the face of the block above, in sixteenths
/// (`[min_x, min_z, max_x, max_z]`): the post's middle, and the middle of each side
/// (in `Direction::HORIZONTAL` order) from the post's edge out to the side.
const POST_TEST: [f64; 4] = [7.0, 7.0, 9.0, 9.0];
const SIDE_TESTS: [[f64; 4]; 4] = [
    [7.0, 0.0, 9.0, 9.0],
    [7.0, 7.0, 16.0, 9.0],
    [7.0, 7.0, 9.0, 16.0],
    [0.0, 7.0, 9.0, 9.0],
];

/// `WallBlock.updateShape(level, state, topPos, ...)`: each joined side is tall
/// when the block above covers that side's middle with the bottom of its collision
/// shape, else low; then whether the post rises (`shouldRaisePost`).
fn wall_shaped(w: &World<'_>, s: u32, above_pos: Pos, joined: [bool; 4]) -> u32 {
    let above = w.get(above_pos);
    let sides: [&str; 4] = std::array::from_fn(|i| match joined[i] {
        false => "none",
        true if bottom_covers(w, above, above_pos, SIDE_TESTS[i]) => "tall",
        true => "low",
    });
    let s = Direction::HORIZONTAL
        .into_iter()
        .zip(sides)
        .fold(s, |acc, (d, side)| state::set(acc, d.name(), side));
    state::set_bool(s, "up", wall_post_rises(w, above, above_pos, sides))
}

/// `WallBlock.shouldRaisePost` (`sides` in `Direction::HORIZONTAL` order: north,
/// east, south, west). The post rises under a wall with a post; at an end, a corner
/// or alone; not in a straight run of tall sides; otherwise only under a block that
/// forces it (`#minecraft:wall_post_override`) or whose bottom covers the post's
/// middle.
fn wall_post_rises(w: &World<'_>, above: u32, above_pos: Pos, sides: [&str; 4]) -> bool {
    if w.blocks.is_class(above, Class::WallBlock) && state::get_bool(above, "up") {
        return true;
    }
    let [north, east, south, west] = sides;
    let none = |s: &str| s == "none";
    let alone = sides.iter().all(|s| none(s));
    if alone || none(north) != none(south) || none(west) != none(east) {
        return true;
    }
    if north == "tall" && south == "tall" || east == "tall" && west == "tall" {
        return false;
    }
    w.tags.is(above, "minecraft:wall_post_override")
        || bottom_covers(w, above, above_pos, POST_TEST)
}

/// Whether the bottom face of the collision shape of `above` (`getFaceShape(DOWN)`:
/// its cell layer just above its bottom) covers the square `rect` (sixteenths,
/// `[min_x, min_z, max_x, max_z]`) wholly: no part of the square lies outside it.
fn bottom_covers(w: &World<'_>, above: u32, above_pos: Pos, rect: [f64; 4]) -> bool {
    const EPSILON: f64 = 1.0e-7;
    let Some(shape) = w.blocks.collision(above, above_pos) else {
        return false;
    };
    let layer = shape.find_index_local(Axis::Y, EPSILON);
    if layer < 0 || layer as usize >= shape.shape.size(Axis::Y) {
        return false;
    }
    let [x0, z0, x1, z1] = rect.map(|v| v / 16.0);
    // The cells along an axis that overlap `lo..hi`, or none if the square reaches
    // past the shape.
    let cells = |axis: Axis, lo: f64, hi: f64| -> Option<Vec<usize>> {
        let n = shape.shape.size(axis);
        if lo < shape.local(axis, 0) - EPSILON || hi > shape.local(axis, n) + EPSILON {
            return None;
        }
        Some(
            (0..n)
                .filter(|&i| {
                    shape.local(axis, i + 1) > lo + EPSILON && shape.local(axis, i) < hi - EPSILON
                })
                .collect(),
        )
    };
    let (Some(xs), Some(zs)) = (cells(Axis::X, x0, x1), cells(Axis::Z, z0, z1)) else {
        return false;
    };
    xs.iter().all(|&x| {
        zs.iter()
            .all(|&z| shape.shape.is_full_at(x as isize, layer, z as isize))
    })
}

// --- Hanging and attached blocks ----------------------------------------------------

/// The side a lantern hangs from or stands on: up when hanging, else down.
fn lantern_support_side(s: u32) -> Direction {
    if state::get_bool(s, "hanging") {
        Direction::Up
    } else {
        Direction::Down
    }
}

/// `LanternBlock.getStateForPlacement`: of the vertical directions the player looks
/// along, nearest first, the first whose block can hold the lantern: hanging when
/// that is up. None may: it can't be placed.
fn lantern_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.axis() == Axis::Y)
        .map(|d| state::set_bool(default, "hanging", d == Direction::Up))
        .find(|&s| super::can_survive(w, s, pos))
        .map(|s| with_water_source(w, pos, s))
}

/// `LadderBlock.getStateForPlacement`: never onto the front of a ladder facing out
/// of the clicked face (from there it would be placed again in front of itself);
/// else against the first horizontal direction the player looks along whose block
/// has a sturdy face for it, facing away from that block.
fn ladder_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let face = ctx.clicked_face();
    if !ctx.replacing_clicked_on_block() {
        let behind = w.get(face.opposite().step(pos));
        if state::same_block(behind, default) && state::get_dir(behind, "facing") == Some(face) {
            return None;
        }
    }
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal())
        .map(|d| state::set_dir(default, "facing", d.opposite()))
        .find(|&s| super::can_survive(w, s, pos))
        .map(|s| with_water_source(w, pos, s))
}

/// `EndRodBlock.getStateForPlacement`: points out of the clicked face, unless it
/// goes onto the tip of an end rod pointing that way: then it points back into it.
fn end_rod_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let face = ctx.clicked_face();
    let behind = w.get(face.opposite().step(ctx.clicked_pos()));
    let onto_tip =
        state::same_block(behind, default) && state::get_dir(behind, "facing") == Some(face);
    state::set_dir(
        default,
        "facing",
        if onto_tip { face.opposite() } else { face },
    )
}

/// `ScaffoldingBlock.isBottom`: scaffolding off the ground with no scaffolding under
/// it shows its bottom frame.
fn scaffolding_bottom(w: &World<'_>, pos: Pos, distance: i32) -> bool {
    distance > 0 && !is_scaffolding(w.get(Direction::Down.step(pos)))
}

/// `SnowLayerBlock.canSurvive`: not on blocks tagged against it; always on blocks
/// tagged for it; else on a block whose collision shape covers its top, or on a
/// full stack of eight layers.
fn snow_layer_survives(w: &World<'_>, s: u32, below: Pos) -> bool {
    let under = w.get(below);
    if w.tags.is(under, "minecraft:cannot_support_snow_layer") {
        return false;
    }
    w.tags.is(under, "minecraft:support_override_snow_layer")
        || w.blocks.collision_face_full(under, Direction::Up)
        || state::same_block(under, s) && state::get_int(under, "layers") == 8
}

// --- Portals and pistons --------------------------------------------------------------

/// `NetherPortalBlock.updateShape`: unless the change is across the portal's plane,
/// a portal block whose new neighbour isn't portal can't stay once its frame no
/// longer holds a complete portal.
fn portal_update(w: &World<'_>, u: &Update) -> u32 {
    let axis = if state::get(u.state, "axis") == Some("z") {
        Axis::Z
    } else {
        Axis::X
    };
    let across = u.direction.axis() != axis && u.direction.is_horizontal();
    if across
        || state::is(u.neighbour, "minecraft:nether_portal")
        || portal_complete(w, u.pos, axis)
    {
        u.state
    } else {
        state::AIR
    }
}

/// `PortalShape.findAnyShape(...).isComplete()`: the portal at `pos` along `axis`
/// is framed (2 to 21 wide, 3 to 21 high) and every block inside is portal.
fn portal_complete(w: &World<'_>, pos: Pos, axis: Axis) -> bool {
    let right = if axis == Axis::X {
        Direction::West
    } else {
        Direction::South
    };
    let frame = |at: Pos| w.tags.is(w.get(at), "minecraft:nether_portal_frame");
    let open = |s: u32| {
        w.blocks.is_air(s)
            || w.tags.is(s, "minecraft:fire")
            || state::is(s, "minecraft:nether_portal")
    };
    let step = |at: Pos, d: Direction, n: i32| {
        let v = d.normal();
        [at[0] + v[0] * n, at[1] + v[1] * n, at[2] + v[2] * n]
    };
    // How far open blocks standing on frame run from `start` before a frame block
    // (0 when the run ends any other way).
    let run_to_frame = |start: Pos, d: Direction| -> i32 {
        for n in 0..=21 {
            let at = step(start, d, n);
            let s = w.get(at);
            if !open(s) {
                return if frame(at) { n } else { 0 };
            }
            if !frame(Direction::Down.step(at)) {
                return 0;
            }
        }
        0
    };
    // Down through the opening, at most 21 blocks and not below the level.
    let lowest = w.min_y().max(pos[1] - 21);
    let mut floor = pos;
    while floor[1] > lowest && open(w.get(Direction::Down.step(floor))) {
        floor = Direction::Down.step(floor);
    }
    let edge = run_to_frame(floor, right.opposite()) - 1;
    if edge < 0 {
        return false;
    }
    let corner = step(floor, right.opposite(), edge);
    let width = run_to_frame(corner, right);
    if !(2..=21).contains(&width) {
        return false;
    }
    // Rows upwards while framed at both ends and open inside, counting portal blocks.
    let mut portals = 0;
    let mut height = 21;
    'rows: for h in 0..21 {
        let row = step(corner, Direction::Up, h);
        if !frame(step(row, right, -1)) || !frame(step(row, right, width)) {
            height = h;
            break;
        }
        for i in 0..width {
            let s = w.get(step(row, right, i));
            if !open(s) {
                height = h;
                break 'rows;
            }
            if state::is(s, "minecraft:nether_portal") {
                portals += 1;
            }
        }
    }
    let roofed = (0..width).all(|i| frame(step(step(corner, Direction::Up, height), right, i)));
    (3..=21).contains(&height) && roofed && portals == width * height
}

/// `PistonHeadBlock.canSurvive`: behind the head is an extended piston of its kind
/// (sticky for a sticky head) facing the same way, or a moving piston facing that way.
fn piston_head_survives(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let Some(facing) = state::get_dir(s, "facing") else {
        return false;
    };
    let base = w.get(facing.opposite().step(pos));
    let same_facing = state::get_dir(base, "facing") == Some(facing);
    let piston = if state::get(s, "type") == Some("sticky") {
        "minecraft:sticky_piston"
    } else {
        "minecraft:piston"
    };
    let fitting = state::is(base, piston) && state::get_bool(base, "extended");
    same_facing && (fitting || state::is(base, "minecraft:moving_piston"))
}

#[cfg(test)]
mod tests {
    use super::super::testing::Fixture;
    use super::*;

    /// A property of the block at `pos`.
    fn prop(f: &Fixture, pos: Pos, name: &str) -> String {
        state::get(f.level.block(pos[0], pos[1], pos[2]), name)
            .unwrap_or("-")
            .to_owned()
    }

    /// A floor, and the player a few blocks south looking north and down.
    fn fixture() -> Option<Fixture> {
        let mut f = Fixture::new()?;
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.y_rot = 180.0;
        f.player.x_rot = 30.0;
        Some(f)
    }

    #[test]
    fn slabs_double_up_and_take_the_clicked_half() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:oak_slab", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "type"), "bottom");
        // On top of the bottom slab: it becomes a double slab.
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "type"), "double");
        assert_eq!(f.get([0, -59, 0]), "minecraft:air");
        assert_eq!(f.player.hand(Hand::Main).count, 62);
        // The upper half of a side: a top slab in front of it.
        f.set([2, -60, 0], "minecraft:stone");
        f.click([2, -60, 0], Direction::West, [0.0, 0.75, 0.5]);
        assert_eq!(prop(&f, [1, -60, 0], "type"), "top");
        // The lower half of the top slab's side fills it.
        f.click([1, -60, 0], Direction::North, [0.5, 0.25, 0.0]);
        assert_eq!(prop(&f, [1, -60, 0], "type"), "double");
        // Another kind of slab doesn't merge.
        f.hold(Hand::Main, "minecraft:stone_slab", 64);
        f.click_face([0, -61, 1], Direction::Up);
        f.hold(Hand::Main, "minecraft:oak_slab", 64);
        f.click_face([0, -60, 1], Direction::Up);
        assert!(f.get([0, -60, 1]).starts_with("minecraft:stone_slab"));
        assert!(f.get([0, -59, 1]).starts_with("minecraft:oak_slab"));
    }

    #[test]
    fn stairs_face_the_player_and_turn_corners() {
        let Some(mut f) = fixture() else { return };
        // Looking east.
        f.player.position = [-3.5, -60.0, 0.5];
        f.player.y_rot = -90.0;
        f.set([1, -60, 0], "minecraft:oak_stairs[facing=north]");
        f.hold(Hand::Main, "minecraft:oak_stairs", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "facing"), "east");
        assert_eq!(prop(&f, [0, -60, 0], "half"), "bottom");
        // The stairs behind face north, east's counter-clockwise turn.
        assert_eq!(prop(&f, [0, -60, 0], "shape"), "outer_left");
        // High on a side: upside down, and not turned by bottom stairs.
        f.set([0, -60, 3], "minecraft:stone");
        f.click([0, -60, 3], Direction::West, [0.0, 0.8, 0.5]);
        assert_eq!(prop(&f, [-1, -60, 3], "half"), "top");
        assert_eq!(prop(&f, [-1, -60, 3], "shape"), "straight");
        // Stairs placed later turn their neighbour's shape.
        f.click_face([5, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [5, -60, 0], "shape"), "straight");
        f.player.y_rot = 180.0;
        f.click_face([6, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [6, -60, 0], "facing"), "north");
        assert_eq!(prop(&f, [5, -60, 0], "shape"), "outer_left");
    }

    #[test]
    fn stairs_take_inner_corners_from_stairs_in_front() {
        let Some(mut f) = fixture() else { return };
        f.player.position = [-3.5, -60.0, 0.5];
        f.player.y_rot = -90.0;
        f.set([-1, -60, 0], "minecraft:oak_stairs[facing=south]");
        f.hold(Hand::Main, "minecraft:oak_stairs", 64);
        f.click_face([0, -61, 0], Direction::Up);
        // In front faces south, east's clockwise turn.
        assert_eq!(prop(&f, [0, -60, 0], "shape"), "inner_right");
    }

    #[test]
    fn doors_place_both_halves_and_open_by_hand() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:oak_door", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "half"), "lower");
        assert_eq!(prop(&f, [0, -60, 0], "facing"), "north");
        assert_eq!(prop(&f, [0, -60, 0], "hinge"), "left");
        assert_eq!(prop(&f, [0, -59, 0], "half"), "upper");
        assert_eq!(prop(&f, [0, -59, 0], "hinge"), "left");
        // A right click with an item that does nothing opens both halves.
        f.hold(Hand::Main, "minecraft:stick", 1);
        f.click([0, -60, 0], Direction::South, [0.5, 0.5, 1.0]);
        assert_eq!(prop(&f, [0, -60, 0], "open"), "true");
        assert_eq!(prop(&f, [0, -59, 0], "open"), "true");
        f.click([0, -59, 0], Direction::South, [0.5, 0.5, 1.0]);
        assert_eq!(prop(&f, [0, -60, 0], "open"), "false");
        // A second door to the right of the first (east, facing north) hinges right.
        f.hold(Hand::Main, "minecraft:oak_door", 64);
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [1, -60, 0], "hinge"), "right");
        // Iron doors stay shut.
        f.hold(Hand::Main, "minecraft:iron_door", 64);
        f.click_face([3, -61, 0], Direction::Up);
        f.hold(Hand::Main, "minecraft:stick", 1);
        f.click([3, -60, 0], Direction::South, [0.5, 0.5, 1.0]);
        assert_eq!(prop(&f, [3, -60, 0], "open"), "false");
        // No room above: no door.
        f.set([5, -59, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:oak_door", 64);
        f.click_face([5, -61, 0], Direction::Up);
        assert_eq!(f.get([5, -60, 0]), "minecraft:air");
    }

    #[test]
    fn door_hinges_follow_the_click_and_solid_neighbours() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:oak_door", 64);
        // Facing north, right is east: a click east of the middle hinges right.
        f.click([0, -61, 0], Direction::Up, [0.8, 1.0, 0.5]);
        assert_eq!(prop(&f, [0, -60, 0], "hinge"), "right");
        // Full blocks to the west (left) outweigh the click.
        f.set([1, -60, 0], "minecraft:stone");
        f.set([1, -59, 0], "minecraft:stone");
        f.click([2, -61, 0], Direction::Up, [0.8, 1.0, 0.5]);
        assert_eq!(prop(&f, [2, -60, 0], "hinge"), "left");
    }

    #[test]
    fn trapdoors_take_their_half_from_the_click() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:oak_trapdoor", 64);
        f.set([2, -60, 0], "minecraft:stone");
        f.click([2, -60, 0], Direction::West, [0.0, 0.7, 0.5]);
        assert_eq!(prop(&f, [1, -60, 0], "half"), "top");
        assert_eq!(prop(&f, [1, -60, 0], "facing"), "west");
        f.click([2, -60, 0], Direction::North, [0.5, 0.3, 0.0]);
        assert_eq!(prop(&f, [2, -60, -1], "half"), "bottom");
        // On top of a block: the bottom, facing the player.
        f.click_face([-2, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [-2, -60, 0], "half"), "bottom");
        assert_eq!(prop(&f, [-2, -60, 0], "facing"), "south");
        f.hold(Hand::Main, "minecraft:stick", 1);
        f.click_face([-2, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [-2, -60, 0], "open"), "true");
    }

    #[test]
    fn fences_join_sturdy_faces_and_their_own_kind() {
        let Some(mut f) = fixture() else { return };
        f.set([1, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:oak_fence", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "east"), "true");
        assert_eq!(prop(&f, [0, -60, 0], "west"), "false");
        f.click_face([-1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [-1, -60, 0], "east"), "true");
        // The first fence heard of the second.
        assert_eq!(prop(&f, [0, -60, 0], "west"), "true");
        // Nether brick fences don't join wooden ones; leaves don't join either.
        f.set([-1, -60, 1], "minecraft:oak_leaves");
        f.hold(Hand::Main, "minecraft:nether_brick_fence", 64);
        f.click_face([-2, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [-2, -60, 0], "east"), "false");
        f.hold(Hand::Main, "minecraft:oak_fence", 64);
        f.click_face([-1, -61, 2], Direction::Up);
        assert_eq!(prop(&f, [-1, -60, 2], "north"), "false");
    }

    #[test]
    fn panes_join_walls_and_bars() {
        let Some(mut f) = fixture() else { return };
        f.set([1, -60, 0], "minecraft:cobblestone_wall");
        f.set([-1, -60, 0], "minecraft:iron_bars");
        f.hold(Hand::Main, "minecraft:glass_pane", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "east"), "true");
        assert_eq!(prop(&f, [0, -60, 0], "west"), "true");
        assert_eq!(prop(&f, [0, -60, 0], "north"), "false");
    }

    #[test]
    fn walls_rise_at_ends_and_grow_tall_under_blocks() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:cobblestone_wall", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "up"), "true");
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "east"), "low");
        assert_eq!(prop(&f, [1, -60, 0], "west"), "low");
        assert_eq!(prop(&f, [0, -60, 0], "up"), "true");
        // In the middle of a straight run the post goes down.
        f.click_face([-1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "west"), "low");
        assert_eq!(prop(&f, [0, -60, 0], "up"), "false");
        // A block on top makes the sides tall.
        f.hold(Hand::Main, "minecraft:stone", 64);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "east"), "tall");
        assert_eq!(prop(&f, [0, -60, 0], "west"), "tall");
        assert_eq!(prop(&f, [0, -60, 0], "up"), "false");
        // A bottom slab above covers it too; a top slab doesn't.
        f.set([5, -59, 0], "minecraft:oak_slab[type=top]");
        f.hold(Hand::Main, "minecraft:cobblestone_wall", 64);
        f.click_face([5, -61, 0], Direction::Up);
        f.click_face([6, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [5, -60, 0], "east"), "low");
    }

    #[test]
    fn gates_open_away_from_the_player() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:oak_fence_gate", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "facing"), "north");
        // From the far side, looking south: it turns to swing away.
        f.player.position = [0.5, -60.0, -3.5];
        f.player.y_rot = 0.0;
        f.hold(Hand::Main, "minecraft:stick", 1);
        f.click_face([0, -60, 0], Direction::North);
        assert_eq!(prop(&f, [0, -60, 0], "open"), "true");
        assert_eq!(prop(&f, [0, -60, 0], "facing"), "south");
        f.click_face([0, -60, 0], Direction::North);
        assert_eq!(prop(&f, [0, -60, 0], "open"), "false");
        // Beside a wall it sits in the wall.
        f.set([2, -60, 0], "minecraft:cobblestone_wall");
        f.player.position = [1.5, -60.0, 4.5];
        f.player.y_rot = 180.0;
        f.hold(Hand::Main, "minecraft:oak_fence_gate", 64);
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [1, -60, 0], "in_wall"), "true");
    }

    #[test]
    fn snow_piles_up_and_scaffolding_counts_its_distance() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:snow", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "layers"), "1");
        f.click_face([0, -60, 0], Direction::Up);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "layers"), "3");
        f.hold(Hand::Main, "minecraft:scaffolding", 64);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [2, -60, 0], "distance"), "0");
        assert_eq!(prop(&f, [2, -60, 0], "bottom"), "false");
        // Clicking its top reaches out the way the player looks, here over air.
        f.set([3, -61, 0], "minecraft:air");
        f.set([4, -61, 0], "minecraft:air");
        f.player.position = [-2.5, -60.0, 0.5];
        f.player.y_rot = -90.0;
        f.click_face([2, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [3, -60, 0], "distance"), "1");
        assert_eq!(prop(&f, [3, -60, 0], "bottom"), "true");
        let mut w = f.world();
        assert_eq!(scaffolding_distance(&w, [4, -60, 0]), 2);
        assert_eq!(scaffolding_distance(&w, [7, -60, 7]), 0);
        w.set_block([3, -60, 0], state::AIR, flags::ALL);
        assert_eq!(scaffolding_distance(&w, [4, -60, 0]), 7);
    }

    #[test]
    fn leaves_logs_lanterns_and_ladders() {
        let Some(mut f) = fixture() else { return };
        f.set([1, -60, 0], "minecraft:oak_log");
        f.hold(Hand::Main, "minecraft:oak_leaves", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [0, -60, 0], "distance"), "1");
        assert_eq!(prop(&f, [0, -60, 0], "persistent"), "true");
        f.click_face([-1, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [-1, -60, 0], "distance"), "2");
        // A log on a side lies along the clicked face's axis.
        f.hold(Hand::Main, "minecraft:oak_log", 64);
        f.click([1, -60, 0], Direction::North, [0.5, 0.5, 0.0]);
        assert_eq!(prop(&f, [1, -60, -1], "axis"), "z");
        // A lantern under a block hangs.
        f.set([3, -58, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:lantern", 64);
        f.click_face([3, -58, 0], Direction::Down);
        assert_eq!(prop(&f, [3, -59, 0], "hanging"), "true");
        f.click_face([4, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [4, -60, 0], "hanging"), "false");
        // A ladder on the west face of a block faces west.
        f.set([6, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:ladder", 64);
        f.click([6, -60, 0], Direction::West, [0.0, 0.5, 0.5]);
        assert_eq!(prop(&f, [5, -60, 0], "facing"), "west");
        // Nothing to hold it: no ladder.
        f.click_face([-4, -61, 0], Direction::Up);
        assert_eq!(f.get([-4, -60, 0]), "minecraft:air");
    }

    #[test]
    fn concrete_powder_sets_beside_water() {
        let Some(mut f) = fixture() else { return };
        f.set([1, -60, 0], "minecraft:water");
        f.hold(Hand::Main, "minecraft:red_concrete_powder", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:red_concrete");
        f.click_face([-2, -61, 0], Direction::Up);
        assert_eq!(f.get([-2, -60, 0]), "minecraft:red_concrete_powder");
        // The constructor settings these ports read are exported.
        let kinds = f.blocks().kinds();
        for (block, field) in [
            ("minecraft:farmland", "FarmlandBlock.baseBlock"),
            ("minecraft:oak_trapdoor", "TrapDoorBlock.type"),
            ("minecraft:iron_door", "DoorBlock.type"),
        ] {
            assert!(
                kinds.of(state::default_of(block)).field(field).is_some(),
                "{block} {field}"
            );
        }
    }

    #[test]
    fn pistons_face_the_player_and_rods_point_out() {
        let Some(mut f) = fixture() else { return };
        f.hold(Hand::Main, "minecraft:piston", 64);
        f.click_face([0, -61, 0], Direction::Up);
        // Looking north and a little down: it faces back at the player.
        assert_eq!(prop(&f, [0, -60, 0], "facing"), "south");
        f.hold(Hand::Main, "minecraft:end_rod", 64);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(prop(&f, [2, -60, 0], "facing"), "up");
        f.click_face([2, -60, 0], Direction::Up);
        assert_eq!(prop(&f, [2, -59, 0], "facing"), "down");
    }
}
