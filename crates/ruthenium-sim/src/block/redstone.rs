//! Redstone: wire, repeaters and comparators, levers, buttons, pressure plates, tripwire and hooks, redstone wall torches, lamps, note blocks, daylight detectors, redstone ore, TNT.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Power itself is the server's business: on the client, wire keeps the power it
//! was given, diodes don't tick, and levers and buttons don't tell their neighbours.
//! What the client does predict is shape: wire connecting to its neighbours, a
//! repeater's delay, a comparator's mode, a button going down, hooks and tripwire
//! attaching, lamps placed lit.

use super::{Behaviour, Class, Pos, Support, Update, Use, state};
use crate::direction::Direction;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::place::{PlaceContext, UseOnContext};
use crate::player::LocalPlayer;
use crate::shape::Axis;

/// The (class, behaviour) pairs this group answers for; every pair the export lists
/// must be in some group's list (see [`super`]'s coverage test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PORTED: &[(Class, Behaviour)] = &[
    (Class::RedstoneWireBlock, Behaviour::GetStateForPlacement),
    (Class::RedstoneWireBlock, Behaviour::UpdateShape),
    (Class::RedstoneWireBlock, Behaviour::CanSurvive),
    (Class::RedstoneWireBlock, Behaviour::UseWithoutItem),
    (
        Class::RedstoneWireBlock,
        Behaviour::UpdateIndirectNeighbourShapes,
    ),
    (Class::RepeaterBlock, Behaviour::GetStateForPlacement),
    (Class::RepeaterBlock, Behaviour::UpdateShape),
    (Class::RepeaterBlock, Behaviour::UseWithoutItem),
    (Class::DiodeBlock, Behaviour::CanSurvive),
    (Class::DiodeBlock, Behaviour::SetPlacedBy),
    (Class::DiodeBlock, Behaviour::GetStateForPlacement),
    (Class::ComparatorBlock, Behaviour::UpdateShape),
    (Class::ComparatorBlock, Behaviour::UseWithoutItem),
    (Class::LeverBlock, Behaviour::UseWithoutItem),
    (Class::ButtonBlock, Behaviour::UseWithoutItem),
    (Class::BasePressurePlateBlock, Behaviour::UpdateShape),
    (Class::BasePressurePlateBlock, Behaviour::CanSurvive),
    (Class::TripWireHookBlock, Behaviour::GetStateForPlacement),
    (Class::TripWireHookBlock, Behaviour::UpdateShape),
    (Class::TripWireHookBlock, Behaviour::CanSurvive),
    (Class::TripWireHookBlock, Behaviour::SetPlacedBy),
    (Class::TripWireBlock, Behaviour::GetStateForPlacement),
    (Class::TripWireBlock, Behaviour::UpdateShape),
    (
        Class::RedstoneWallTorchBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::RedstoneWallTorchBlock, Behaviour::UpdateShape),
    (Class::RedstoneWallTorchBlock, Behaviour::CanSurvive),
    (Class::RedstoneLampBlock, Behaviour::GetStateForPlacement),
    (Class::NoteBlock, Behaviour::GetStateForPlacement),
    (Class::NoteBlock, Behaviour::UpdateShape),
    (Class::NoteBlock, Behaviour::UseItemOn),
    (Class::NoteBlock, Behaviour::UseWithoutItem),
    (Class::NoteBlock, Behaviour::Attack),
    (Class::DaylightDetectorBlock, Behaviour::UseWithoutItem),
    (Class::RedStoneOreBlock, Behaviour::UseItemOn),
    (Class::RedStoneOreBlock, Behaviour::Attack),
    (Class::TntBlock, Behaviour::UseItemOn),
];

/// `getStateForPlacement` (`Some(None)`: the block can't be placed there).
pub(super) fn state_for_placement(
    c: Class,
    w: &World<'_>,
    ctx: &PlaceContext,
    default: u32,
) -> Option<Option<u32>> {
    let pos = ctx.clicked_pos();
    Some(match c {
        Class::RedstoneWireBlock => Some(wire_connections(w, wire_cross(default), pos)),
        Class::DiodeBlock => Some(diode_placement(ctx, default)),
        Class::RepeaterBlock => {
            let s = diode_placement(ctx, default);
            Some(state::set_bool(s, "locked", repeater_locked(w, pos, s)))
        }
        Class::TripWireHookBlock => {
            facing_away_from_support(ctx, default, |s| hook_can_survive(w, s, pos))
        }
        Class::TripWireBlock => Some(Direction::HORIZONTAL.into_iter().fold(default, |s, d| {
            let joins = tripwire_joins(w, s, w.get(d.step(pos)), d);
            state::set_bool(s, d.name(), joins)
        })),
        Class::RedstoneWallTorchBlock => {
            facing_away_from_support(ctx, default, |s| wall_torch_can_survive(w, s, pos))
        }
        Class::RedstoneLampBlock => {
            Some(state::set_bool(default, "lit", w.has_neighbor_signal(pos)))
        }
        Class::NoteBlock => Some(note_block_instrument(w, pos, default)),
        _ => return None,
    })
}

/// `updateShape`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    let air = state::AIR;
    Some(match c {
        Class::RedstoneWireBlock => wire_update_shape(w, u),
        // `RepeaterBlock.updateShape`: off a rigid top it can't stay; whether its
        // sides lock it is only worked out on the server. `ComparatorBlock`'s is the
        // same without the locking.
        Class::RepeaterBlock | Class::ComparatorBlock => {
            if u.direction == Direction::Down && !diode_stands_on(w, u.neighbour) {
                air
            } else {
                u.state
            }
        }
        // `BasePressurePlateBlock.updateShape`: a change below may leave it without
        // support.
        Class::BasePressurePlateBlock => {
            if u.direction == Direction::Down && !super::can_survive(w, u.state, u.pos) {
                air
            } else {
                u.state
            }
        }
        // `TripWireHookBlock.updateShape` and `RedstoneWallTorchBlock.updateShape`:
        // only the block they hang on matters.
        Class::TripWireHookBlock | Class::RedstoneWallTorchBlock => {
            let hangs_on = state::get_dir(u.state, "facing").map(Direction::opposite);
            if hangs_on == Some(u.direction) && !super::can_survive(w, u.state, u.pos) {
                air
            } else {
                u.state
            }
        }
        // `TripWireBlock.updateShape`: each side joins tripwire and hooks facing it.
        Class::TripWireBlock => {
            if u.direction.is_horizontal() {
                let joins = tripwire_joins(w, u.state, u.neighbour, u.direction);
                state::set_bool(u.state, u.direction.name(), joins)
            } else {
                u.state
            }
        }
        // `NoteBlock.updateShape`: the blocks above and below choose the instrument.
        Class::NoteBlock => {
            if u.direction.is_horizontal() {
                u.state
            } else {
                note_block_instrument(w, u.pos, u.state)
            }
        }
        _ => return None,
    })
}

/// `canSurvive`.
pub(super) fn can_survive(c: Class, w: &World<'_>, state: u32, pos: Pos) -> Option<bool> {
    let below = Direction::Down.step(pos);
    Some(match c {
        // `RedstoneWireBlock.canSurvive`: on a full top face, or a hopper.
        Class::RedstoneWireBlock => wire_stands_on(w, w.get(below)),
        // `DiodeBlock.canSurvive`: on a rigid top face.
        Class::DiodeBlock => diode_stands_on(w, w.get(below)),
        // `BasePressurePlateBlock.canSurvive`: on a rigid top, or one that holds a
        // block by its centre.
        Class::BasePressurePlateBlock => {
            w.can_support_rigid_block(below) || w.can_support_center(below, Direction::Up)
        }
        Class::TripWireHookBlock => hook_can_survive(w, state, pos),
        Class::RedstoneWallTorchBlock => wall_torch_can_survive(w, state, pos),
        _ => return None,
    })
}

/// `canBeReplaced(state, context)`.
pub(super) fn can_be_replaced(
    _c: Class,
    _w: &World<'_>,
    _state: u32,
    _ctx: &PlaceContext,
) -> Option<bool> {
    None
}

/// `canBeReplaced(state, fluid)` (`water`: the fluid is water, else lava).
pub(super) fn can_be_replaced_by_fluid(
    _c: Class,
    _w: &World<'_>,
    _state: u32,
    _water: bool,
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
    let stack = p.hand(u.hand);
    Some(match c {
        // `NoteBlock.useItemOn`: a head (an instrument that plays from on top) used
        // on the top face goes on to be placed there; anything else falls through to
        // tuning.
        Class::NoteBlock => {
            if u.hit.direction == Direction::Up
                && w.item_is(stack, "minecraft:noteblock_top_instruments")
            {
                Outcome::Pass
            } else {
                Outcome::TryEmptyHand
            }
        }
        // `RedStoneOreBlock.useItemOn`: the client only shows particles (the ore lights
        // up on the server). A block item that could be placed against the ore goes on
        // to be placed; anything else is used up on the ore.
        Class::RedStoneOreBlock => {
            let block_item = stack.c.is("BlockItem");
            let ctx = UseOnContext::new(p, u.hand, *u.hit);
            if block_item && PlaceContext::new(w, ctx).can_place(w) {
                Outcome::Pass
            } else {
                Outcome::Success
            }
        }
        // `TntBlock.useItemOn`: flint and steel or a fire charge lights it, but only
        // a server level primes TNT, so the client changes nothing (the block, the
        // item's damage or count) and the click is used up.
        Class::TntBlock => {
            if matches!(
                stack.c.name,
                "minecraft:flint_and_steel" | "minecraft:fire_charge"
            ) {
                Outcome::Success
            } else {
                Outcome::TryEmptyHand
            }
        }
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
    let may_build = p.abilities.may_build;
    Some(match c {
        Class::RedstoneWireBlock => {
            if may_build {
                wire_toggle(w, u.state, u.pos)
            } else {
                Outcome::Pass
            }
        }
        // `RepeaterBlock.useWithoutItem`: the next delay, 1 to 4 and round again.
        Class::RepeaterBlock => {
            if !may_build {
                return Some(Outcome::Pass);
            }
            w.set_block_and_update(u.pos, state::cycle(u.state, "delay"));
            Outcome::Success
        }
        Class::ComparatorBlock => {
            if may_build {
                comparator_toggle(w, u.state, u.pos);
                Outcome::Success
            } else {
                Outcome::Pass
            }
        }
        // `LeverBlock.useWithoutItem`: the client only shows particles; the server
        // pulls the lever.
        Class::LeverBlock => Outcome::Success,
        // `ButtonBlock.useWithoutItem`: a button that is up goes down at once on the
        // client too (the neighbours' power and the timer to release it are the
        // server's); one already down takes the click and stays as it is.
        Class::ButtonBlock => {
            if state::get_bool(u.state, "powered") {
                Outcome::Consume
            } else {
                w.set_block_and_update(u.pos, state::set_bool(u.state, "powered", true));
                Outcome::Success
            }
        }
        // `NoteBlock.useWithoutItem`: the server tunes it and plays the note.
        Class::NoteBlock => Outcome::Success,
        // `DaylightDetectorBlock.useWithoutItem`: the server inverts it; a player who
        // may not build gets the plain block's answer.
        Class::DaylightDetectorBlock => {
            if may_build {
                Outcome::Success
            } else {
                Outcome::Pass
            }
        }
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
    match c {
        Class::RedstoneWireBlock => {
            wire_tell_diagonals(w, state, pos, flags, limit);
            Some(())
        }
        _ => None,
    }
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
        // `DiodeBlock.setPlacedBy`: a powered diode schedules its first tick, which
        // a client level doesn't.
        Class::DiodeBlock => Some(()),
        // `TripWireHookBlock.setPlacedBy`: the hook looks along its tripwire for a
        // hook facing it and attaches both.
        Class::TripWireHookBlock => {
            hook_calculate_state(w, pos, state, false);
            Some(())
        }
        _ => None,
    }
}

/// `attack` (a left click on the block as breaking starts).
pub(super) fn attack(
    c: Class,
    w: &mut World<'_>,
    _p: &LocalPlayer,
    state: u32,
    pos: Pos,
) -> Option<()> {
    match c {
        // `NoteBlock.attack`: the server plays the note.
        Class::NoteBlock => Some(()),
        // `RedStoneOreBlock.attack`: unlit ore lights up, on the client too.
        Class::RedStoneOreBlock => {
            if !state::get_bool(state, "lit") {
                w.set_block_and_update(pos, state::set_bool(state, "lit", true));
            }
            Some(())
        }
        _ => None,
    }
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

// Redstone wire.

/// A wire side's `RedstoneSide` (`none`, `side` or `up`).
fn wire_side(s: u32, d: Direction) -> &'static str {
    state::get(s, d.name()).unwrap_or("none")
}

/// `RedstoneSide.isConnected`: anything but `none`.
fn wire_connected(s: u32, d: Direction) -> bool {
    wire_side(s, d) != "none"
}

/// `RedstoneWireBlock.isDot`: no side connects.
fn wire_is_dot(s: u32) -> bool {
    !Direction::HORIZONTAL
        .into_iter()
        .any(|d| wire_connected(s, d))
}

/// `RedstoneWireBlock.isCross`: every side connects.
fn wire_is_cross(s: u32) -> bool {
    Direction::HORIZONTAL
        .into_iter()
        .all(|d| wire_connected(s, d))
}

/// The wire's `crossState`: every side `side`, with `s`'s power.
fn wire_cross(s: u32) -> u32 {
    Direction::HORIZONTAL
        .into_iter()
        .fold(s, |s, d| state::set(s, d.name(), "side"))
}

/// The wire's default state (a dot) with `s`'s power.
fn wire_dot(s: u32) -> u32 {
    let plain = state::default_of_block(state::block_id(s));
    state::set_int(plain, "power", state::get_int(s, "power"))
}

/// `RedstoneWireBlock.canSurviveOn`: a full top face holds wire, and so does a hopper.
fn wire_stands_on(w: &World<'_>, below: u32) -> bool {
    w.blocks.face_sturdy(below, Direction::Up, Support::Full)
        || state::is(below, "minecraft:hopper")
}

/// `shouldRedstoneWireConnectTo` of the block `s`, reached going `towards` from the
/// wire (`None` for the blocks above and below a neighbour): wire always; a repeater
/// along its axis; an observer reached in the direction it watches (at its output);
/// any other power source from the side.
fn wire_joins(w: &World<'_>, s: u32, towards: Option<Direction>) -> bool {
    let b = w.blocks;
    if b.is_class(s, Class::RedstoneWireBlock) {
        return true;
    }
    let facing = state::get_dir(s, "facing");
    if b.is_class(s, Class::RepeaterBlock) {
        return towards.is_some()
            && (facing == towards || facing.map(Direction::opposite) == towards);
    }
    if b.is_class(s, Class::ObserverBlock) {
        return towards.is_some() && facing == towards;
    }
    b.signal_source(s) && towards.is_some()
}

/// `RedstoneWireBlock.getConnectingSide`: how the wire at `pos` meets its side
/// towards `d`. With nothing that conducts above the wire, it climbs (`up`) onto a
/// neighbour it could stand on (or a trapdoor) when wire-joining is on top of that
/// neighbour, provided its face towards the wire is full (else it only reaches it,
/// `side`). Otherwise it reaches a neighbour that joins wire, and wire one below a
/// neighbour that doesn't conduct.
fn wire_side_towards(w: &World<'_>, pos: Pos, d: Direction, may_climb: bool) -> &'static str {
    let next = d.step(pos);
    let s = w.get(next);
    if may_climb {
        let holds_wire = w.blocks.is_class(s, Class::TrapDoorBlock) || wire_stands_on(w, s);
        if holds_wire && wire_joins(w, w.get(Direction::Up.step(next)), None) {
            return if w.blocks.face_sturdy(s, d.opposite(), Support::Full) {
                "up"
            } else {
                "side"
            };
        }
    }
    let reaches = wire_joins(w, s, Some(d))
        || !w.blocks.redstone_conductor(s)
            && wire_joins(w, w.get(Direction::Down.step(next)), None);
    if reaches { "side" } else { "none" }
}

/// Whether wire at `pos` may climb its neighbours: the block above doesn't conduct.
fn wire_may_climb(w: &World<'_>, pos: Pos) -> bool {
    !w.blocks.redstone_conductor(w.get(Direction::Up.step(pos)))
}

/// `RedstoneWireBlock.getConnectionState`: every side worked out afresh from the
/// neighbours (keeping the power). Wire that was a dot and still meets nothing stays a
/// dot; otherwise wire that leads nowhere along an axis runs straight across it: no
/// connection north or south fills in east and west, and the other way round (so a
/// lone wire is a cross, and one connection makes a line).
fn wire_connections(w: &World<'_>, s: u32, pos: Pos) -> u32 {
    let was_dot = wire_is_dot(s);
    let may_climb = wire_may_climb(w, pos);
    let mut out = Direction::HORIZONTAL
        .into_iter()
        .fold(wire_dot(s), |acc, d| {
            state::set(acc, d.name(), wire_side_towards(w, pos, d, may_climb))
        });
    if was_dot && wire_is_dot(out) {
        return out;
    }
    let joined = |d| wire_connected(out, d);
    let north_south_open = !joined(Direction::North) && !joined(Direction::South);
    let east_west_open = !joined(Direction::East) && !joined(Direction::West);
    for d in Direction::HORIZONTAL {
        let open_across = if d.axis() == Axis::X {
            north_south_open
        } else {
            east_west_open
        };
        if open_across && !wire_connected(out, d) {
            out = state::set(out, d.name(), "side");
        }
    }
    out
}

/// `RedstoneWireBlock.updateShape`: support going from below leaves it to the
/// server; a change above may let it climb or stop it; a side's change keeps the
/// rest of its shape when that side stays connected or unconnected (a cross is
/// always worked out afresh), else the whole shape is worked out again from a cross
/// with that side set.
fn wire_update_shape(w: &World<'_>, u: &Update) -> u32 {
    match u.direction {
        Direction::Down => {
            if wire_stands_on(w, u.neighbour) {
                u.state
            } else {
                state::AIR
            }
        }
        Direction::Up => wire_connections(w, u.state, u.pos),
        d => {
            let side = wire_side_towards(w, u.pos, d, wire_may_climb(w, u.pos));
            let same = (side != "none") == wire_connected(u.state, d);
            if same && !wire_is_cross(u.state) {
                state::set(u.state, d.name(), side)
            } else {
                let start = state::set(wire_cross(u.state), d.name(), side);
                wire_connections(w, start, u.pos)
            }
        }
    }
}

/// `RedstoneWireBlock.updateIndirectNeighbourShapes`: on each side that connects to
/// something other than wire, wire one block lower and one block higher there hears
/// of its neighbour back towards this wire (lower first; sides in north, east,
/// south, west order).
fn wire_tell_diagonals(w: &mut World<'_>, s: u32, pos: Pos, flags: u32, limit: i32) {
    for d in Direction::HORIZONTAL {
        let beside = d.step(pos);
        if !wire_connected(s, d) || state::same_block(w.get(beside), s) {
            continue;
        }
        for diagonal in [Direction::Down.step(beside), Direction::Up.step(beside)] {
            if state::same_block(w.get(diagonal), s) {
                let back = d.opposite().step(diagonal);
                let neighbour = w.get(back);
                w.neighbour_shape_changed(d.opposite(), diagonal, back, neighbour, flags, limit);
            }
        }
    }
}

/// `RedstoneWireBlock.useWithoutItem` for a player who may build: a cross becomes a
/// dot and a dot a cross (each then fitted to the neighbours); wire that is neither,
/// or that the neighbours keep as it is, passes the click on.
fn wire_toggle(w: &mut World<'_>, s: u32, pos: Pos) -> Outcome {
    if !wire_is_cross(s) && !wire_is_dot(s) {
        return Outcome::Pass;
    }
    let start = if wire_is_cross(s) {
        wire_dot(s)
    } else {
        wire_cross(s)
    };
    let toggled = wire_connections(w, start, pos);
    if toggled == s {
        return Outcome::Pass;
    }
    // The neighbours that conduct hear of it only on the server.
    w.set_block_and_update(pos, toggled);
    Outcome::Success
}

// Repeaters and comparators.

/// `DiodeBlock.getStateForPlacement`: facing the player (its output away from them).
fn diode_placement(ctx: &PlaceContext, default: u32) -> u32 {
    state::set_dir(default, "facing", ctx.horizontal_direction().opposite())
}

/// `DiodeBlock.canSurviveOn`: the block below has a rigid top.
fn diode_stands_on(w: &World<'_>, below: u32) -> bool {
    w.blocks.face_sturdy(below, Direction::Up, Support::Rigid)
}

/// `SignalGetter.getControlInputSignal`: the power the block at `pos` gives a diode
/// from the side (`d` points from the diode to it). Repeaters only take diodes'
/// strong power; comparators take a redstone block's 15, wire's power level, and
/// any other source's strong power.
fn control_input(w: &World<'_>, pos: Pos, d: Direction, diodes_only: bool) -> i32 {
    let s = w.get(pos);
    if diodes_only {
        return if w.blocks.is_class(s, Class::DiodeBlock) {
            w.direct_signal(pos, d)
        } else {
            0
        };
    }
    if state::is(s, "minecraft:redstone_block") {
        15
    } else if state::is(s, "minecraft:redstone_wire") {
        state::get_int(s, "power")
    } else if w.blocks.signal_source(s) {
        w.direct_signal(pos, d)
    } else {
        0
    }
}

/// `DiodeBlock.getAlternateSignal`: the stronger of the two side inputs (clockwise
/// first).
fn side_input(w: &World<'_>, pos: Pos, s: u32, diodes_only: bool) -> i32 {
    let Some(facing) = state::get_dir(s, "facing") else {
        return 0;
    };
    let right = facing.clockwise();
    let left = facing.counter_clockwise();
    control_input(w, right.step(pos), right, diodes_only).max(control_input(
        w,
        left.step(pos),
        left,
        diodes_only,
    ))
}

/// `RepeaterBlock.isLocked`: a powered diode at either side.
fn repeater_locked(w: &World<'_>, pos: Pos, s: u32) -> bool {
    side_input(w, pos, s, true) > 0
}

/// `ComparatorBlock.getInputSignal`: the power behind it (`DiodeBlock`'s: the
/// signal from that side, at least wire's power level there), replaced by the block
/// there's comparator output, or, when that block conducts and the power is under
/// 15, by the comparator output of the block beyond it. Item frames beyond it aren't
/// tracked with their items here, so they are left out.
fn comparator_input(w: &World<'_>, pos: Pos, s: u32) -> i32 {
    let Some(facing) = state::get_dir(s, "facing") else {
        return 0;
    };
    let behind = facing.step(pos);
    let mut input = w.signal(behind, facing);
    let there = w.get(behind);
    if input < 15 && state::is(there, "minecraft:redstone_wire") {
        input = input.max(state::get_int(there, "power"));
    }
    if has_analog_output(w, there) {
        return analog_output(w, there);
    }
    if input < 15 && w.blocks.redstone_conductor(there) {
        let beyond = w.get(facing.step(behind));
        if has_analog_output(w, beyond) {
            return analog_output(w, beyond);
        }
    }
    input
}

/// `hasAnalogOutputSignal`.
fn has_analog_output(w: &World<'_>, s: u32) -> bool {
    w.behaviour(s).flags & ruthenium_prepare::exporters::StateBehaviour::ANALOG_OUTPUT != 0
}

/// `getAnalogOutputSignal` as the client can tell it from the state alone: cake by
/// bites left, candle cake 14, composters, cauldrons and beehives by their level,
/// lava cauldrons 3, a lit copper bulb or a filled end portal frame 15, a respawn
/// anchor by its charges, a copper golem statue by its pose. The rest read block
/// entities (containers, jukeboxes, lecterns, sculk sensors...) or entities
/// (detector rails), which this client doesn't track: 0.
fn analog_output(w: &World<'_>, s: u32) -> i32 {
    let b = w.blocks;
    let level = |p| state::get_int(s, p);
    if b.is_class(s, Class::CandleCakeBlock) {
        14
    } else if b.is_class(s, Class::CakeBlock) {
        (7 - level("bites")) * 2
    } else if b.is_class(s, Class::ComposterBlock) || b.is_class(s, Class::LayeredCauldronBlock) {
        level("level")
    } else if b.is_class(s, Class::BeehiveBlock) {
        level("honey_level")
    } else if b.is_class(s, Class::LavaCauldronBlock) {
        3
    } else if b.is_class(s, Class::CopperBulbBlock) {
        if state::get_bool(s, "lit") { 15 } else { 0 }
    } else if b.is_class(s, Class::EndPortalFrameBlock) {
        if state::get_bool(s, "eye") { 15 } else { 0 }
    } else if b.is_class(s, Class::RespawnAnchorBlock) {
        // `getScaledChargeLevel`: charges out of 4, scaled to 15 and rounded down.
        (level("charges") as f32 / 4.0 * 15.0).floor() as i32
    } else if b.is_class(s, Class::CopperGolemStatueBlock) {
        let pose = state::get(s, "copper_golem_pose");
        let values = state::values(s, "copper_golem_pose");
        pose.and_then(|v| values.iter().position(|&x| x == v))
            .map_or(0, |i| i as i32 + 1)
    } else {
        0
    }
}

/// `ComparatorBlock.useWithoutItem` for a player who may build: the mode flips
/// (`compare`/`subtract`, `UPDATE_CLIENTS`), and the client then refreshes the output
/// (`refreshOutputState`): the comparator turns on or off by whether its input
/// beats (or, comparing, matches) its stronger side. The output the client last
/// worked out lives in the comparator's block entity, which this client doesn't
/// keep: it counts as 0, as it is before the client has refreshed it, so a
/// comparator in subtract mode with no output left over isn't refreshed.
fn comparator_toggle(w: &mut World<'_>, s: u32, pos: Pos) {
    let flipped = state::cycle(s, "mode");
    w.set_block(pos, flipped, super::flags::CLIENTS);
    if !state::same_block(w.get(pos), flipped) {
        return;
    }
    let input = comparator_input(w, pos, flipped);
    let side = side_input(w, pos, flipped, false);
    let compare = state::get(flipped, "mode") == Some("compare");
    let output = if input == 0 || side > input {
        0
    } else if compare {
        input
    } else {
        input - side
    };
    if output == 0 && !compare {
        return;
    }
    let turn_on = input > 0 && (input > side || input == side && compare);
    if state::get_bool(flipped, "powered") != turn_on {
        w.set_block(
            pos,
            state::set_bool(flipped, "powered", turn_on),
            super::flags::CLIENTS,
        );
    }
}

// Tripwire and hooks, and blocks hung on a wall.

/// `getStateForPlacement` of blocks that hang on a wall facing away from it
/// (`TripWireHookBlock`, and `WallTorchBlock` for the redstone wall torch): the
/// horizontal directions the player looks along, nearest first, each tried as the
/// side it hangs on until it may stay; `None` if none will do.
fn facing_away_from_support(
    ctx: &PlaceContext,
    default: u32,
    survives: impl Fn(u32) -> bool,
) -> Option<u32> {
    ctx.nearest_looking_directions()
        .into_iter()
        .filter(|d| d.is_horizontal())
        .map(|d| state::set_dir(default, "facing", d.opposite()))
        .find(|&s| survives(s))
}

/// `TripWireHookBlock.canSurvive`: facing sideways, off a full face of the block
/// behind it.
fn hook_can_survive(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let Some(facing) = state::get_dir(s, "facing") else {
        return false;
    };
    facing.is_horizontal() && w.face_sturdy(facing.opposite().step(pos), facing, Support::Full)
}

/// `RedstoneWallTorchBlock.canSurvive` (`WallTorchBlock.canSurvive`): the block
/// behind it has a full face towards it.
fn wall_torch_can_survive(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let Some(facing) = state::get_dir(s, "facing") else {
        return false;
    };
    w.face_sturdy(facing.opposite().step(pos), facing, Support::Full)
}

/// The hook block tripwire attaches to (`TripWireBlock.hook`).
fn tripwire_hook_of<'a>(w: &'a World<'_>, wire: u32) -> &'a str {
    w.blocks
        .kind(wire)
        .field("TripWireBlock.hook")
        .unwrap_or("minecraft:tripwire_hook")
}

/// `TripWireBlock.shouldConnectTo`: tripwire joins tripwire, and a hook facing back
/// at it.
fn tripwire_joins(w: &World<'_>, wire: u32, neighbour: u32, d: Direction) -> bool {
    if state::is(neighbour, tripwire_hook_of(w, wire)) {
        state::get_dir(neighbour, "facing") == Some(d.opposite())
    } else {
        state::same_block(neighbour, wire)
    }
}

/// Where along a hook's line the furthest is (`TripWireHookBlock.WIRE_DIST_MAX`).
const WIRE_DIST_MAX: i32 = 42;

/// `TripWireHookBlock.calculateState` as the client runs it (the sounds, game events,
/// neighbour updates and ticks are the server's), for the hook at `pos` as `hook`
/// (`removing`: it is being taken away). It looks along its facing up to 41 blocks
/// for a hook facing back: every block before that must be tripwire for it to be
/// attached, and it is powered if attached and any of that wire is armed and
/// powered. A hook found facing back is set to match (facing back); if the hook at
/// `pos` has meanwhile gone, it is treated as removed instead. Unless removing, the
/// hook at `pos` is set (a fresh state with only the facing kept). When the attached
/// flag changed, the tripwire between the two hooks follows.
fn hook_calculate_state(w: &mut World<'_>, pos: Pos, hook: u32, removing: bool) {
    let Some(facing) = state::get_dir(hook, "facing") else {
        return;
    };
    let was_attached = state::get_bool(hook, "attached");
    let mut attached = !removing;
    let mut powered = false;
    let mut far_hook = 0;
    let mut wires = vec![None; WIRE_DIST_MAX as usize];
    let along = |i: i32| {
        let n = facing.normal();
        [pos[0] + n[0] * i, pos[1] + n[1] * i, pos[2] + n[2] * i]
    };
    for i in 1..WIRE_DIST_MAX {
        let s = w.get(along(i));
        if state::is(s, "minecraft:tripwire_hook") {
            if state::get_dir(s, "facing") == Some(facing.opposite()) {
                far_hook = i;
            }
            break;
        }
        if state::is(s, "minecraft:tripwire") {
            powered |= !state::get_bool(s, "disarmed") && state::get_bool(s, "powered");
            wires[i as usize] = Some(s);
        } else {
            attached = false;
        }
    }
    attached &= far_hook > 1;
    powered &= attached;
    let fresh = state::default_of_block(state::block_id(hook));
    let fresh = state::try_set(fresh, "attached", if attached { "true" } else { "false" });
    let fresh = state::try_set(fresh, "powered", if powered { "true" } else { "false" });
    if far_hook > 0 {
        w.set_block_and_update(
            along(far_hook),
            state::set_dir(fresh, "facing", facing.opposite()),
        );
        if !state::is(w.get(pos), "minecraft:tripwire_hook") {
            hook_removed(w, pos, fresh);
            return;
        }
    }
    if !removing {
        w.set_block_and_update(pos, state::set_dir(fresh, "facing", facing));
    }
    if was_attached != attached {
        for i in 1..far_hook {
            let Some(wire) = wires[i as usize] else {
                continue;
            };
            let here = w.get(along(i));
            if state::is(here, "minecraft:tripwire") || state::is(here, "minecraft:tripwire_hook") {
                let value = if attached { "true" } else { "false" };
                w.set_block_and_update(along(i), state::try_set(wire, "attached", value));
            }
        }
    }
}

/// `TripWireHookBlock.onRemoved`: a hook that was attached or powered works its line
/// out again as removed.
fn hook_removed(w: &mut World<'_>, pos: Pos, hook: u32) {
    if state::get_bool(hook, "attached") || state::get_bool(hook, "powered") {
        hook_calculate_state(w, pos, hook, true);
    }
}

// Note blocks.

/// `NoteBlock.setInstrument`: a block on top that plays from above a note block (a
/// mob head) sets the instrument; otherwise the block below does, except that such
/// a head below gives the harp.
fn note_block_instrument(w: &World<'_>, pos: Pos, s: u32) -> u32 {
    let kinds = w.blocks.kinds();
    let (above, from_above) = kinds.instrument(w.get(Direction::Up.step(pos)));
    if from_above {
        return state::set(s, "instrument", above);
    }
    let (below, head) = kinds.instrument(w.get(Direction::Down.step(pos)));
    state::set(s, "instrument", if head { "harp" } else { below })
}

#[cfg(test)]
mod tests {
    use crate::block::testing::Fixture;
    use crate::direction::Direction;
    use crate::inventory::Hand;

    #[test]
    fn lone_wire_is_a_cross_and_a_second_makes_a_line() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        f.hold(Hand::Main, "minecraft:redstone", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:redstone_wire[east=side,north=side,power=0,south=side,west=side]"
        );
        f.click_face([1, -61, 0], Direction::Up);
        let line = "minecraft:redstone_wire[east=side,north=none,power=0,south=none,west=side]";
        assert_eq!(f.get([1, -60, 0]), line);
        assert_eq!(f.get([0, -60, 0]), line, "the first wire turns to meet it");
        // On air it can't go.
        f.set([3, -61, 0], "minecraft:air");
        f.click_face([4, -61, 0], Direction::West);
        assert_eq!(f.get([3, -61, 0]), "minecraft:air");
    }

    #[test]
    fn wire_climbs_onto_a_block_with_wire_on_top() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        f.set([1, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:redstone", 64);
        f.click_face([0, -61, 0], Direction::Up);
        // Then wire on the block: the lower wire hears of it diagonally.
        f.click_face([1, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:redstone_wire[east=up,north=none,power=0,south=none,west=side]"
        );
        assert_eq!(
            f.get([1, -59, 0]),
            "minecraft:redstone_wire[east=side,north=none,power=0,south=none,west=side]"
        );
        // A block that conducts on top of the lower wire stops it climbing.
        let stone = Fixture::state("minecraft:stone");
        f.world()
            .set_block([0, -59, 0], stone, crate::block::flags::ALL);
        // Nothing to meet any more: a cross again.
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:redstone_wire[east=side,north=side,power=0,south=side,west=side]"
        );
    }

    #[test]
    fn right_clicking_wire_toggles_cross_and_dot() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        f.set(
            [0, -60, 0],
            "minecraft:redstone_wire[east=side,north=side,south=side,west=side,power=3]",
        );
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:redstone_wire[east=none,north=none,power=3,south=none,west=none]"
        );
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:redstone_wire[east=side,north=side,power=3,south=side,west=side]"
        );
    }

    #[test]
    fn repeaters_face_the_player_lock_and_change_delay() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        // A powered repeater pointing into the side of the one to be placed.
        f.set([1, -60, 0], "minecraft:repeater[facing=east,powered=true]");
        f.hold(Hand::Main, "minecraft:repeater", 64);
        f.click_face([0, -61, 0], Direction::Up);
        // Looking north (yaw 180) places it facing south, towards the player.
        f.player.y_rot = 180.0;
        f.click_face([-2, -61, 0], Direction::Up);
        assert_eq!(
            f.get([-2, -60, 0]),
            "minecraft:repeater[delay=1,facing=south,locked=false,powered=false]"
        );
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:repeater[delay=1,facing=south,locked=true,powered=false]"
        );
        f.hold(Hand::Main, "minecraft:air", 0);
        f.click_face([-2, -60, 0], Direction::Up);
        assert_eq!(
            f.get([-2, -60, 0]),
            "minecraft:repeater[delay=2,facing=south,locked=false,powered=false]"
        );
    }

    #[test]
    fn comparators_flip_mode_and_buttons_go_down_but_levers_wait() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        f.set([0, -60, 0], "minecraft:comparator[facing=north]");
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:comparator[facing=north,mode=subtract,powered=false]"
        );
        // With a redstone block behind it, comparing turns it on.
        f.set([0, -60, -1], "minecraft:redstone_block");
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:comparator[facing=north,mode=compare,powered=true]"
        );

        f.set(
            [2, -60, 0],
            "minecraft:stone_button[face=floor,facing=north]",
        );
        f.click_face([2, -60, 0], Direction::Up);
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:stone_button[face=floor,facing=north,powered=true]"
        );
        f.set([3, -60, 0], "minecraft:lever[face=floor,facing=north]");
        f.click_face([3, -60, 0], Direction::Up);
        assert_eq!(
            f.get([3, -60, 0]),
            "minecraft:lever[face=floor,facing=north,powered=false]"
        );
    }

    #[test]
    fn a_lamp_by_a_power_source_is_placed_lit() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.x_rot = 60.0;
        f.set([1, -60, 0], "minecraft:redstone_block");
        f.hold(Hand::Main, "minecraft:redstone_lamp", 64);
        f.click_face([0, -61, 0], Direction::Up);
        f.click_face([-2, -61, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:redstone_lamp[lit=true]");
        assert_eq!(f.get([-2, -60, 0]), "minecraft:redstone_lamp[lit=false]");
    }

    #[test]
    fn a_second_hook_attaches_the_line() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [3.5, -60.0, 3.5];
        f.set([0, -60, 0], "minecraft:stone");
        f.set([0, -60, 5], "minecraft:stone");
        f.set([0, -60, 1], "minecraft:tripwire_hook[facing=south]");
        f.set([0, -60, 2], "minecraft:tripwire[north=true,south=true]");
        f.set([0, -60, 3], "minecraft:tripwire[north=true,south=true]");
        f.hold(Hand::Main, "minecraft:tripwire_hook", 64);
        f.click_face([0, -60, 5], Direction::North);
        assert_eq!(
            f.get([0, -60, 4]),
            "minecraft:tripwire_hook[attached=true,facing=north,powered=false]"
        );
        assert_eq!(
            f.get([0, -60, 1]),
            "minecraft:tripwire_hook[attached=true,facing=south,powered=false]"
        );
        for z in 2..4 {
            assert!(
                f.get([0, -60, z]).contains("attached=true"),
                "{}",
                f.get([0, -60, z])
            );
        }
        // Tripwire placed by hand joins what faces it.
        f.hold(Hand::Main, "minecraft:string", 64);
        f.set([3, -60, 0], "minecraft:tripwire_hook[facing=west]");
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:tripwire[attached=false,disarmed=false,east=true,north=false,powered=false,south=false,west=false]"
        );
    }

    #[test]
    fn redstone_wall_torches_hang_on_the_clicked_wall() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [3.5, -60.0, 3.5];
        f.set([0, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:redstone_torch", 64);
        f.click_face([0, -60, 0], Direction::East);
        assert_eq!(
            f.get([1, -60, 0]),
            "minecraft:redstone_wall_torch[facing=east,lit=true]"
        );
    }

    #[test]
    fn note_blocks_take_a_head_above_and_ore_lights_when_hit() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [3.5, -60.0, 3.5];
        f.player.x_rot = 60.0;
        f.set([0, -59, 0], "minecraft:zombie_head");
        f.hold(Hand::Main, "minecraft:note_block", 64);
        f.click_face([0, -61, 0], Direction::Up);
        assert!(
            f.get([0, -60, 0]).contains("instrument=zombie"),
            "{}",
            f.get([0, -60, 0])
        );

        f.set([2, -60, 0], "minecraft:redstone_ore");
        let ore = Fixture::state("minecraft:redstone_ore");
        let player = f.player.clone();
        super::super::attack(&mut f.world(), &player, ore, [2, -60, 0]);
        assert_eq!(f.get([2, -60, 0]), "minecraft:redstone_ore[lit=true]");

        // Flint and steel on TNT: only the server primes it.
        f.set([4, -60, 0], "minecraft:tnt");
        f.hold(Hand::Main, "minecraft:flint_and_steel", 1);
        f.click_face([4, -60, 0], Direction::Up);
        assert_eq!(f.get([4, -60, 0]), "minecraft:tnt[unstable=false]");
        assert_eq!(f.get([4, -59, 0]), "minecraft:air");
    }
}
