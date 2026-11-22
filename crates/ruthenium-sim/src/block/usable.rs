//! Blocks with a right click of their own: workstations and containers, cauldrons, jukeboxes, composters, respawn anchors, pumpkins, dragon eggs, chests, beds, cakes and candles, flower pots, light blocks, shulker boxes, and the game-master blocks.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Most of these only answer a click: a menu, a sleep, a song or a carving is the
//! server's to do, and the client only learns whether the click was used. A few change
//! the block at once on the client, as vanilla's does: a cake losing a slice, a candle
//! going out, a flower going into or out of a pot, a respawn anchor taking glowstone,
//! a composter being emptied, the second half of a bed or chest.
//!
//! Block entities aren't tracked. Vanilla's client makes one on demand for any block
//! that has one (`Level.getBlockEntity` creates it), so a check that the block entity
//! is of the block's own type always holds here; what one holds (a jukebox's disc, a
//! shulker box's lid) isn't known, and its default is taken.

use ruthenium_data::v26_3::Block;
use ruthenium_data::v26_3::registries::{DataComponentType, Item, Potion};
use ruthenium_protocol::item::Component;

use super::state::{self, AIR};
use super::{Behaviour, Class, Pos, Update, Use, flags};
use crate::direction::Direction;
use crate::fluid::Fluid;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::item::{Food, Stack};
use crate::place::PlaceContext;
use crate::player::LocalPlayer;

/// The (class, behaviour) pairs this group answers for; every pair the export lists
/// must be in some group's list (see [`super`]'s coverage test).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PORTED: &[(Class, Behaviour)] = &[
    (Class::CraftingTableBlock, Behaviour::UseWithoutItem),
    (Class::EnchantingTableBlock, Behaviour::UseWithoutItem),
    (Class::BrewingStandBlock, Behaviour::UseWithoutItem),
    (Class::AbstractCauldronBlock, Behaviour::UseItemOn),
    (Class::BeaconBlock, Behaviour::UseWithoutItem),
    (Class::CartographyTableBlock, Behaviour::UseWithoutItem),
    (Class::SmithingTableBlock, Behaviour::UseWithoutItem),
    (Class::JukeboxBlock, Behaviour::UseItemOn),
    (Class::JukeboxBlock, Behaviour::UseWithoutItem),
    (Class::JukeboxBlock, Behaviour::SetPlacedBy),
    (Class::ComposterBlock, Behaviour::UseItemOn),
    (Class::ComposterBlock, Behaviour::UseWithoutItem),
    (Class::RespawnAnchorBlock, Behaviour::UseItemOn),
    (Class::RespawnAnchorBlock, Behaviour::UseWithoutItem),
    (Class::PumpkinBlock, Behaviour::UseItemOn),
    (Class::DragonEggBlock, Behaviour::UseWithoutItem),
    (Class::DragonEggBlock, Behaviour::Attack),
    (Class::ChestBlock, Behaviour::GetStateForPlacement),
    (Class::ChestBlock, Behaviour::UpdateShape),
    (Class::ChestBlock, Behaviour::UseWithoutItem),
    (Class::CopperChestBlock, Behaviour::GetStateForPlacement),
    (Class::CopperChestBlock, Behaviour::UpdateShape),
    (Class::EnderChestBlock, Behaviour::GetStateForPlacement),
    (Class::EnderChestBlock, Behaviour::UpdateShape),
    (Class::EnderChestBlock, Behaviour::UseWithoutItem),
    (Class::AbstractBedBlock, Behaviour::GetStateForPlacement),
    (Class::AbstractBedBlock, Behaviour::UpdateShape),
    (Class::AbstractBedBlock, Behaviour::UseWithoutItem),
    (Class::AbstractBedBlock, Behaviour::SetPlacedBy),
    (Class::CakeBlock, Behaviour::UpdateShape),
    (Class::CakeBlock, Behaviour::CanSurvive),
    (Class::CakeBlock, Behaviour::UseItemOn),
    (Class::CakeBlock, Behaviour::UseWithoutItem),
    (Class::CandleCakeBlock, Behaviour::UpdateShape),
    (Class::CandleCakeBlock, Behaviour::CanSurvive),
    (Class::CandleCakeBlock, Behaviour::UseItemOn),
    (Class::CandleCakeBlock, Behaviour::UseWithoutItem),
    (Class::CandleBlock, Behaviour::GetStateForPlacement),
    (Class::CandleBlock, Behaviour::UpdateShape),
    (Class::CandleBlock, Behaviour::CanSurvive),
    (Class::CandleBlock, Behaviour::CanBeReplaced),
    (Class::CandleBlock, Behaviour::UseItemOn),
    (Class::FlowerPotBlock, Behaviour::UpdateShape),
    (Class::FlowerPotBlock, Behaviour::UseItemOn),
    (Class::FlowerPotBlock, Behaviour::UseWithoutItem),
    (Class::LightBlock, Behaviour::UpdateShape),
    (Class::LightBlock, Behaviour::UseWithoutItem),
    (Class::ShulkerBoxBlock, Behaviour::GetStateForPlacement),
    (Class::ShulkerBoxBlock, Behaviour::UseWithoutItem),
    (Class::StructureBlock, Behaviour::UseWithoutItem),
    (Class::StructureBlock, Behaviour::SetPlacedBy),
    (Class::TestInstanceBlock, Behaviour::UseWithoutItem),
    (Class::TestBlock, Behaviour::GetStateForPlacement),
    (Class::TestBlock, Behaviour::UseWithoutItem),
    (Class::JigsawBlock, Behaviour::GetStateForPlacement),
    (Class::JigsawBlock, Behaviour::UseWithoutItem),
    (Class::CommandBlock, Behaviour::GetStateForPlacement),
    (Class::CommandBlock, Behaviour::UseWithoutItem),
    (Class::CommandBlock, Behaviour::SetPlacedBy),
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
        Class::ChestBlock => chest_placement(w, ctx, default),
        Class::CopperChestBlock => least_oxidized_pair(w, chest_placement(w, ctx, default), pos),
        Class::EnderChestBlock => {
            let s = state::set_dir(default, "facing", ctx.horizontal_direction().opposite());
            state::set_bool(s, "waterlogged", water_source(w, pos))
        }
        Class::AbstractBedBlock => return Some(bed_placement(w, ctx, default)),
        Class::CandleBlock => candle_placement(w, pos, default),
        Class::ShulkerBoxBlock => state::set_dir(default, "facing", ctx.clicked_face()),
        Class::TestBlock => test_block_placement(ctx, default),
        Class::JigsawBlock => jigsaw_placement(ctx, default),
        Class::CommandBlock => state::set_dir(
            default,
            "facing",
            ctx.nearest_looking_direction().opposite(),
        ),
        _ => return None,
    };
    Some(Some(placed))
}

/// `updateShape`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    Some(match c {
        Class::ChestBlock => chest_update_shape(w, u),
        Class::CopperChestBlock => copper_chest_update_shape(w, u),
        Class::AbstractBedBlock => bed_update_shape(u),
        // A waterlogged block's water tick is the server's.
        Class::EnderChestBlock | Class::CandleBlock | Class::LightBlock => u.state,
        Class::CakeBlock | Class::CandleCakeBlock | Class::FlowerPotBlock => {
            if u.direction == Direction::Down && !super::can_survive(w, u.state, u.pos) {
                AIR
            } else {
                u.state
            }
        }
        _ => return None,
    })
}

/// `canSurvive`.
pub(super) fn can_survive(c: Class, w: &World<'_>, _state: u32, pos: Pos) -> Option<bool> {
    let below = Direction::Down.step(pos);
    match c {
        // `CakeBlock.canSurvive`, `CandleCakeBlock.canSurvive`: something solid
        // (the legacy `isSolid`) underneath.
        Class::CakeBlock | Class::CandleCakeBlock => Some(w.blocks.solid(w.get(below))),
        // `CandleBlock.canSurvive`: the top of the block below holds it by its centre.
        Class::CandleBlock => Some(w.can_support_center(below, Direction::Up)),
        _ => None,
    }
}

/// `canBeReplaced(state, context)`.
pub(super) fn can_be_replaced(
    c: Class,
    w: &World<'_>,
    state: u32,
    ctx: &PlaceContext,
) -> Option<bool> {
    match c {
        // `CandleBlock.canBeReplaced`: the same candle, held without sneaking, adds
        // to a group of fewer than four.
        Class::CandleBlock => {
            let adds = !ctx.secondary_use()
                && super::item_id(ctx.item()) == w.blocks.kind(state).item
                && state::get_int(state, "candles") < 4;
            Some(adds || super::default_can_be_replaced(w, state, ctx))
        }
        _ => None,
    }
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
    Some(match c {
        Class::AbstractCauldronBlock => cauldron_use(w, p, u),
        Class::JukeboxBlock => jukebox_use(p, u),
        Class::ComposterBlock => composter_use(p, u),
        Class::RespawnAnchorBlock => respawn_anchor_use(w, p, u),
        Class::PumpkinBlock => pumpkin_use(p, u),
        Class::CakeBlock => cake_use(w, p, u),
        Class::CandleCakeBlock => candle_cake_use(w, p, u),
        Class::CandleBlock => candle_use(w, p, u),
        Class::FlowerPotBlock => flower_pot_use(w, p, u),
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
        // Workstations, containers and beds: the server opens the menu (or puts the
        // player to bed, or says why not); the dragon egg's jump is the server's too.
        // The light block's level changes on the server only.
        Class::CraftingTableBlock
        | Class::EnchantingTableBlock
        | Class::BrewingStandBlock
        | Class::BeaconBlock
        | Class::CartographyTableBlock
        | Class::SmithingTableBlock
        | Class::ChestBlock
        | Class::EnderChestBlock
        | Class::ShulkerBoxBlock
        | Class::AbstractBedBlock
        | Class::DragonEggBlock
        | Class::LightBlock => Outcome::Success,
        // `JukeboxBlock.useWithoutItem`: a jukebox with a disc gives it back (the
        // server drops it).
        Class::JukeboxBlock => used_if(state::get_bool(u.state, "has_record")),
        Class::ComposterBlock => composter_empty(w, u),
        // `RespawnAnchorBlock.useWithoutItem`: a charged anchor sets the spawn point
        // or blows up, which the server decides.
        Class::RespawnAnchorBlock => used_if(state::get_int(u.state, "charges") != 0),
        Class::CakeBlock => cake_eat(w, p, u),
        Class::CandleCakeBlock => {
            // `CandleCakeBlock.useWithoutItem`: a slice of a whole cake; the candle
            // drops on the server.
            eat_slice(w, p, state::default_of("minecraft:cake"), u.pos)
        }
        Class::FlowerPotBlock => flower_pot_take(w, p, u),
        // The game-master blocks open their screen for operators in creative.
        Class::StructureBlock
        | Class::TestInstanceBlock
        | Class::TestBlock
        | Class::JigsawBlock
        | Class::CommandBlock => used_if(p.can_use_game_master_blocks()),
        _ => return None,
    })
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
    p: &LocalPlayer,
    state: u32,
    pos: Pos,
    hand: Hand,
) -> Option<()> {
    match c {
        Class::JukeboxBlock => jukebox_placed(w, p.hand(hand), state, pos),
        // `AbstractBedBlock.setPlacedBy`: the head goes on the block the bed faces.
        Class::AbstractBedBlock => {
            let head = state::get_dir(state, "facing")
                .unwrap_or(Direction::North)
                .step(pos);
            w.set_block_and_update(head, state::set(state, "part", "head"));
        }
        // Who placed it, the command block's feedback rule and its power are the
        // server's.
        Class::StructureBlock | Class::CommandBlock => {}
        _ => return None,
    }
    Some(())
}

/// `attack` (a left click on the block as breaking starts).
pub(super) fn attack(
    c: Class,
    _w: &mut World<'_>,
    _p: &LocalPlayer,
    _state: u32,
    _pos: Pos,
) -> Option<()> {
    // `DragonEggBlock.attack`: the egg jumps away on the server only.
    (c == Class::DragonEggBlock).then_some(())
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

/// Success when the click does something, else a pass.
fn used_if(used: bool) -> Outcome {
    if used {
        Outcome::Success
    } else {
        Outcome::Pass
    }
}

/// `FluidState.is(Fluids.WATER)`: a water source (flowing water isn't).
fn water_source(w: &World<'_>, pos: Pos) -> bool {
    w.fluid(pos).fluid == Fluid::Water
}

/// Every block registry id, in order.
fn every_block() -> impl Iterator<Item = u32> {
    (0..).map_while(|id| Block::from_id(id).map(|b| b.id()))
}

/// The block registry id of the block a field names (`minecraft:dandelion`).
fn block_field(w: &World<'_>, block: u32, field: &str) -> Option<&'static str> {
    let key = w.blocks.kinds().of_block(block).field(field)?;
    Block::from_name(key).map(|b| b.name())
}

// Chests.

/// `ChestBlock.chestCanConnectTo` of the chest `chest`: a plain or trapped chest
/// pairs with its own block only; a copper chest with any copper chest (the
/// `copper_chests` tag, with a chest type).
fn chest_pairs_with(w: &World<'_>, chest: u32, other: u32) -> bool {
    if w.blocks.is_class(chest, Class::CopperChestBlock) {
        w.tags.is(other, "minecraft:copper_chests") && state::has(other, "type")
    } else {
        state::same_block(chest, other)
    }
}

/// `ChestType.getOpposite`: left and right swap.
fn other_half(kind: &str) -> &'static str {
    match kind {
        "left" => "right",
        "right" => "left",
        _ => "single",
    }
}

/// `ChestBlock.getConnectedDirection`: where the other half is, seen from this one:
/// clockwise from the facing for a left half, counter-clockwise otherwise (a lone
/// chest too).
fn chest_partner_direction(chest: u32) -> Direction {
    let facing = state::get_dir(chest, "facing").unwrap_or(Direction::North);
    if state::get(chest, "type") == Some("left") {
        facing.clockwise()
    } else {
        facing.counter_clockwise()
    }
}

/// `ChestBlock.candidatePartnerFacing`: the facing of the chest towards
/// `direction` from `pos`, if it pairs with `chest` and has no partner yet.
fn lone_chest_facing(
    w: &World<'_>,
    chest: u32,
    pos: Pos,
    direction: Direction,
) -> Option<Direction> {
    let s = w.get(direction.step(pos));
    (chest_pairs_with(w, chest, s) && state::get(s, "type") == Some("single"))
        .then(|| state::get_dir(s, "facing"))
        .flatten()
}

/// `ChestBlock.getStateForPlacement`: the chest faces the player. Sneaking against
/// the side of a lone chest it pairs with joins that chest, taking its facing, as
/// long as it doesn't face along the clicked face's axis (the half is the right one
/// when the clicked chest is counter-clockwise of the new facing, else the left
/// one). Otherwise, unless sneaking, a lone chest facing the same way on its
/// clockwise side makes it a left half, else one on its counter-clockwise side a
/// right half. A chest placed into a water source is waterlogged.
fn chest_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> u32 {
    let pos = ctx.clicked_pos();
    let face = ctx.clicked_face();
    let sneaking = ctx.secondary_use();
    let mut facing = ctx.horizontal_direction().opposite();
    let mut half = "single";
    if sneaking && face.is_horizontal() {
        let into = face.opposite();
        if let Some(theirs) = lone_chest_facing(w, default, pos, into)
            && theirs.axis() != face.axis()
        {
            facing = theirs;
            half = if facing.counter_clockwise() == into {
                "right"
            } else {
                "left"
            };
        }
    }
    if half == "single" && !sneaking {
        if lone_chest_facing(w, default, pos, facing.clockwise()) == Some(facing) {
            half = "left";
        } else if lone_chest_facing(w, default, pos, facing.counter_clockwise()) == Some(facing) {
            half = "right";
        }
    }
    let s = state::set_dir(default, "facing", facing);
    let s = state::set(s, "type", half);
    state::set_bool(s, "waterlogged", water_source(w, pos))
}

/// `ChestBlock.updateShape`: a lone chest beside the other half of a pair it may
/// join (facing the same way, pointing back at it) becomes that pair's missing
/// half; a half whose partner's side now holds something it can't pair with (or
/// that isn't beside it horizontally) is alone again.
fn chest_update_shape(w: &World<'_>, u: &Update) -> u32 {
    let mine = state::get(u.state, "type").unwrap_or("single");
    if chest_pairs_with(w, u.state, u.neighbour) && u.direction.is_horizontal() {
        let theirs = state::get(u.neighbour, "type").unwrap_or("single");
        let joins = mine == "single"
            && theirs != "single"
            && state::get_dir(u.state, "facing") == state::get_dir(u.neighbour, "facing")
            && chest_partner_direction(u.neighbour) == u.direction.opposite();
        if joins {
            return state::set(u.state, "type", other_half(theirs));
        }
    } else if chest_partner_direction(u.state) == u.direction {
        return state::set(u.state, "type", "single");
    }
    u.state
}

/// The copper chests' ages (`WeatheringCopper.WeatherState`, in order).
const WEATHER_STATES: [&str; 4] = ["UNAFFECTED", "EXPOSED", "WEATHERED", "OXIDIZED"];

/// `CopperChestBlock.weatherState`'s place in [`WEATHER_STATES`].
fn copper_age(w: &World<'_>, chest: u32) -> usize {
    w.blocks
        .kind(chest)
        .field("CopperChestBlock.weatherState")
        .and_then(|a| WEATHER_STATES.iter().position(|&s| s == a))
        .unwrap_or(0)
}

/// `CopperChestBlock.isWaxed`: copper chests that don't age are the waxed ones.
fn copper_waxed(w: &World<'_>, chest: u32) -> bool {
    !w.blocks.is_class(chest, Class::WeatheringCopperChestBlock)
}

/// `CopperChestBlock.unwaxBlock`: an unwaxed chest as it is; a waxed one as the
/// unwaxed chest of the same age (`HoneycombItem.WAX_OFF_BY_BLOCK`, the waxed
/// block's name without `waxed_`), if there is one.
fn copper_unwaxed(w: &World<'_>, chest: u32) -> Option<u32> {
    if !copper_waxed(w, chest) {
        return Some(chest);
    }
    let name = state::name(chest).replacen("waxed_", "", 1);
    let block = state::block_named(&name)?;
    Some(state::with_properties_of(
        state::default_of_block(block),
        chest,
    ))
}

/// `CopperChestBlock.getLeastOxidizedChestOfConnectedBlocks`: a copper chest that
/// joins another copper chest takes the younger chest's block of the two (its own
/// when they are as old); when only one of them is waxed, both count as unwaxed.
fn least_oxidized_pair(w: &World<'_>, chest: u32, pos: Pos) -> u32 {
    if state::get(chest, "type") == Some("single") {
        return chest;
    }
    let partner = w.get(chest_partner_direction(chest).step(pos));
    if !w.blocks.is_class(chest, Class::CopperChestBlock)
        || !w.blocks.is_class(partner, Class::CopperChestBlock)
    {
        return chest;
    }
    let (mine, theirs) = if copper_waxed(w, chest) == copper_waxed(w, partner) {
        (chest, partner)
    } else {
        (
            copper_unwaxed(w, chest).unwrap_or(chest),
            copper_unwaxed(w, partner).unwrap_or(partner),
        )
    };
    let younger = if copper_age(w, chest) <= copper_age(w, partner) {
        mine
    } else {
        theirs
    };
    state::with_properties_of(state::default_of_block(state::block_id(younger)), mine)
}

/// `CopperChestBlock.updateShape`: as a chest, then a half whose partner is the
/// copper chest beside it takes that chest's block.
fn copper_chest_update_shape(w: &World<'_>, u: &Update) -> u32 {
    let s = chest_update_shape(w, u);
    let paired = chest_pairs_with(w, u.state, u.neighbour)
        && state::get(s, "type") != Some("single")
        && chest_partner_direction(s) == u.direction;
    if paired {
        state::with_properties_of(state::default_of_block(state::block_id(u.neighbour)), s)
    } else {
        s
    }
}

// Beds.

/// `AbstractBedBlock.getStateForPlacement`: the bed faces the way the player looks,
/// its foot where it is placed and its head on the next block that way, which must
/// be free to build over. (The world border isn't tracked; the head is taken to be
/// inside it.)
fn bed_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let facing = ctx.horizontal_direction();
    let head = facing.step(ctx.clicked_pos());
    super::can_be_replaced(w, w.get(head), ctx).then(|| state::set_dir(default, "facing", facing))
}

/// `AbstractBedBlock.updateShape`: each half watches the other (the head is the
/// way the foot faces): while the other half is there, the bed's `occupied` follows
/// it; without it, the half can't stay.
fn bed_update_shape(u: &Update) -> u32 {
    let part = state::get(u.state, "part");
    let facing = state::get_dir(u.state, "facing").unwrap_or(Direction::North);
    let towards_other = if part == Some("foot") {
        facing
    } else {
        facing.opposite()
    };
    if u.direction != towards_other {
        return u.state;
    }
    if state::same_block(u.neighbour, u.state) && state::get(u.neighbour, "part") != part {
        state::set_bool(
            u.state,
            "occupied",
            state::get_bool(u.neighbour, "occupied"),
        )
    } else {
        AIR
    }
}

// Game-master blocks.

/// `TestBlock.getStateForPlacement`: the mode the item's `block_state` names, if
/// any.
fn test_block_placement(ctx: &PlaceContext, default: u32) -> u32 {
    let mode = ctx
        .item()
        .c
        .block_state
        .as_ref()
        .and_then(|props| props.iter().find(|(k, _)| k == "mode"));
    mode.map_or(default, |(_, v)| state::try_set(default, "mode", v))
}

/// `JigsawBlock.getStateForPlacement`: the front is the clicked face; the top is
/// up, or for a front up or down, back towards the player.
fn jigsaw_placement(ctx: &PlaceContext, default: u32) -> u32 {
    let front = ctx.clicked_face();
    let top = if front.is_horizontal() {
        Direction::Up
    } else {
        ctx.horizontal_direction().opposite()
    };
    state::set(
        default,
        "orientation",
        &format!("{}_{}", front.name(), top.name()),
    )
}

// Candles and cakes.

/// `CandleBlock.getStateForPlacement`: one more candle on a group of the same
/// candle; else a single candle, waterlogged in a water source.
fn candle_placement(w: &World<'_>, pos: Pos, default: u32) -> u32 {
    let here = w.get(pos);
    if state::same_block(here, default) {
        state::cycle(here, "candles")
    } else {
        state::set_bool(default, "waterlogged", water_source(w, pos))
    }
}

/// `AbstractCandleBlock.extinguish`: the candles go out (`UPDATE_ALL_IMMEDIATE`).
fn extinguish(w: &mut World<'_>, candle: u32, pos: Pos) {
    w.set_block(
        pos,
        state::set_bool(candle, "lit", false),
        flags::ALL_IMMEDIATE,
    );
}

/// `CandleBlock.useItemOn`: an empty hand puts lit candles out, for a player who
/// may build.
fn candle_use(w: &mut World<'_>, p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    if p.hand(u.hand).is_empty() && p.abilities.may_build && state::get_bool(u.state, "lit") {
        extinguish(w, u.state, u.pos);
        Outcome::Success
    } else {
        Outcome::TryEmptyHand
    }
}

/// `CandleCakeBlock.useItemOn`: flint and steel and fire charges pass the click on
/// to the item (which lights the candle); an empty hand on the candle (the upper
/// half of the block) puts a lit one out.
fn candle_cake_use(w: &mut World<'_>, p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    let held = p.hand(u.hand);
    if matches!(
        held.c.name,
        "minecraft:flint_and_steel" | "minecraft:fire_charge"
    ) {
        return Outcome::Pass;
    }
    let on_candle = u.hit.location[1] - f64::from(u.pos[1]) > 0.5;
    if on_candle && held.is_empty() && state::get_bool(u.state, "lit") {
        extinguish(w, u.state, u.pos);
        return Outcome::Success;
    }
    Outcome::TryEmptyHand
}

/// `CandleCakeBlock.byCandle`: the candle cake whose candle is `candle` (a block
/// registry id).
fn candle_cake_of(w: &World<'_>, candle: u32) -> Option<u32> {
    let name = Block::from_id(candle)?.name();
    every_block().find(|&b| {
        w.blocks.kinds().of_block(b).is(Class::CandleCakeBlock)
            && block_field(w, b, "CandleCakeBlock.candleBlock") == Some(name)
    })
}

/// `CakeBlock.useItemOn`: a candle (the `candles` item tag, placing a candle
/// block) goes on a whole cake, using one up.
fn cake_use(w: &mut World<'_>, p: &mut LocalPlayer, u: &Use<'_>) -> Outcome {
    let held = p.hand(u.hand);
    let candle = held
        .c
        .block
        .filter(|&b| w.blocks.kinds().of_block(b).is(Class::CandleBlock));
    let cake = match candle {
        Some(c)
            if w.item_is(held, "minecraft:candles") && state::get_int(u.state, "bites") == 0 =>
        {
            candle_cake_of(w, c)
        }
        _ => None,
    };
    let Some(cake) = cake else {
        return Outcome::TryEmptyHand;
    };
    let infinite = p.infinite_materials();
    p.inventory.hand_mut(u.hand).consume(1, infinite);
    w.set_block_and_update(u.pos, state::default_of_block(cake));
    Outcome::Success
}

/// `CakeBlock.eat`: a player who may eat takes a slice of `cake` at `pos` (2
/// food, a saturation modifier of 0.1): one more bite, or the last slice takes the
/// cake away (`removeBlock`; a cake holds no water, so it leaves air).
fn eat_slice(w: &mut World<'_>, p: &mut LocalPlayer, cake: u32, pos: Pos) -> Outcome {
    if !p.can_eat(false) {
        return Outcome::Pass;
    }
    p.eat(Food {
        nutrition: 2,
        // `FoodConstants.saturationByModifier`: nutrition × modifier × 2.
        saturation: 2.0 * 0.1 * 2.0,
        can_always_eat: false,
    });
    let bites = state::get_int(cake, "bites");
    if bites < 6 {
        w.set_block_and_update(pos, state::set_int(cake, "bites", bites + 1));
    } else {
        w.set_block(pos, AIR, flags::ALL);
    }
    Outcome::Success
}

/// `CakeBlock.useWithoutItem` on the client: a slice if the player may eat; if
/// not, an empty main hand still uses the click up.
fn cake_eat(w: &mut World<'_>, p: &mut LocalPlayer, u: &Use<'_>) -> Outcome {
    let eaten = eat_slice(w, p, u.state, u.pos);
    if eaten.consumes_action() || p.hand(Hand::Main).is_empty() {
        Outcome::Success
    } else {
        // Vanilla eats again for the result, which can't go otherwise.
        eaten
    }
}

// Flower pots.

/// `FlowerPotBlock.potted`: what the pot holds (`None` for the empty pot).
fn pot_content(w: &World<'_>, pot: u32) -> Option<&'static str> {
    let block = state::block_id(pot);
    block_field(w, block, "FlowerPotBlock.potted").filter(|&b| b != "minecraft:air")
}

/// `FlowerPotBlock.POTTED_BY_CONTENT`: the pot holding `plant` (a block registry
/// id).
fn pot_holding(w: &World<'_>, plant: u32) -> Option<u32> {
    let name = Block::from_id(plant)?.name();
    every_block().find(|&b| {
        w.blocks.kinds().of_block(b).is(Class::FlowerPotBlock)
            && block_field(w, b, "FlowerPotBlock.potted") == Some(name)
    })
}

/// `FlowerPotBlock.useItemOn`: a block item with a potted form goes into an empty
/// pot, using one up; a full pot takes the click without changing.
fn flower_pot_use(w: &mut World<'_>, p: &mut LocalPlayer, u: &Use<'_>) -> Outcome {
    let Some(pot) = p.hand(u.hand).c.block.and_then(|b| pot_holding(w, b)) else {
        return Outcome::TryEmptyHand;
    };
    if pot_content(w, u.state).is_some() {
        return Outcome::Success;
    }
    w.set_block_and_update(u.pos, state::default_of_block(pot));
    let infinite = p.infinite_materials();
    p.inventory.hand_mut(u.hand).consume(1, infinite);
    Outcome::Success
}

/// `FlowerPotBlock.useWithoutItem`: the plant goes into the inventory (or is
/// dropped, which the server shows) and the pot is empty again; an empty pot takes
/// the click without changing.
fn flower_pot_take(w: &mut World<'_>, p: &mut LocalPlayer, u: &Use<'_>) -> Outcome {
    let Some(plant) = pot_content(w, u.state) else {
        return Outcome::Success;
    };
    let item = state::block_named(plant)
        .map(|b| w.blocks.kinds().of_block(b).item)
        .and_then(Item::from_id);
    if let Some(item) = item {
        let stack = w.new_stack(item.name(), 1);
        // A full inventory drops it: the server's item entity follows.
        let _ = p.inventory.add(stack);
    }
    w.set_block_and_update(u.pos, state::default_of("minecraft:flower_pot"));
    Outcome::Success
}

// Jukeboxes, composters, respawn anchors, pumpkins.

/// Whether a stack has a component its patch may add or remove, given whether its
/// item has it by default.
fn has_component(stack: &Stack, kind: DataComponentType, by_default: bool) -> bool {
    if stack.patch.get(kind).is_some() {
        true
    } else if stack.patch.removes(kind) {
        false
    } else {
        by_default
    }
}

/// `jukebox_playable`.
fn jukebox_playable(stack: &Stack) -> bool {
    stack.c.jukebox_playable
}

/// `compostable`.
fn compostable(stack: &Stack) -> bool {
    stack.c.compostable
}

/// `JukeboxBlock.useItemOn` (with `JukeboxPlayable.tryInsertIntoJukebox`): a
/// playable item goes into a jukebox without a disc (the server takes it).
fn jukebox_use(p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    let inserts = !state::get_bool(u.state, "has_record")
        && jukebox_playable(p.hand(u.hand))
        && state::is(u.state, "minecraft:jukebox");
    if inserts {
        Outcome::Success
    } else {
        Outcome::TryEmptyHand
    }
}

/// `JukeboxBlock.setPlacedBy`: a jukebox placed from an item whose block entity
/// data holds a disc (`RecordItem`) shows it has one.
fn jukebox_placed(w: &mut World<'_>, stack: &Stack, jukebox: u32, pos: Pos) {
    let Some(Component::Other { data, .. }) = stack.patch.get(DataComponentType::BlockEntityData)
    else {
        return;
    };
    // The block entity type (a VarInt), then the data.
    let Some(start) = data.iter().position(|b| b & 0x80 == 0).map(|i| i + 1) else {
        return;
    };
    let has_disc = ruthenium_protocol::nbt::read_network(&data[start..])
        .ok()
        .and_then(|(tag, _)| tag)
        .is_some_and(|t| {
            t.as_compound()
                .is_some_and(|c| c.get("RecordItem").is_some())
        });
    if has_disc {
        w.set_block(
            pos,
            state::set_bool(jukebox, "has_record", true),
            flags::CLIENTS,
        );
    }
}

/// `ComposterBlock.useItemOn`: a compostable item goes into a composter that isn't
/// ready (the layer it adds is rolled on the server).
fn composter_use(p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    if state::get_int(u.state, "level") < 8 && compostable(p.hand(u.hand)) {
        Outcome::Success
    } else {
        Outcome::TryEmptyHand
    }
}

/// `ComposterBlock.useWithoutItem`: a ready composter (level 8) is emptied at once
/// (`extractProduce`; the bone meal is the server's).
fn composter_empty(w: &mut World<'_>, u: &Use<'_>) -> Outcome {
    if state::get_int(u.state, "level") != 8 {
        return Outcome::Pass;
    }
    w.set_block_and_update(u.pos, state::set_int(u.state, "level", 0));
    Outcome::Success
}

/// `RespawnAnchorBlock.useItemOn`: glowstone adds a charge to an anchor with fewer
/// than four, using one up. Glowstone in the off hand while the main hand holds
/// something else passes the main hand's click on, so the off hand charges it.
fn respawn_anchor_use(w: &mut World<'_>, p: &mut LocalPlayer, u: &Use<'_>) -> Outcome {
    let fuel = |s: &Stack| s.c.name == "minecraft:glowstone";
    let charges = state::get_int(u.state, "charges");
    if charges >= 4 {
        return Outcome::TryEmptyHand;
    }
    if fuel(p.hand(u.hand)) {
        w.set_block_and_update(u.pos, state::set_int(u.state, "charges", charges + 1));
        let infinite = p.infinite_materials();
        p.inventory.hand_mut(u.hand).consume(1, infinite);
        return Outcome::Success;
    }
    if u.hand == Hand::Main && fuel(p.hand(Hand::Off)) {
        Outcome::Pass
    } else {
        Outcome::TryEmptyHand
    }
}

/// `PumpkinBlock.useItemOn`: shears carve it, on the server.
fn pumpkin_use(p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    if p.hand(u.hand).c.name == "minecraft:shears" {
        Outcome::Success
    } else {
        Outcome::TryEmptyHand
    }
}

// Cauldrons.

/// Which of `CauldronInteractions`' tables a cauldron uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cauldron {
    Empty,
    Water,
    Lava,
    PowderSnow,
}

/// The table of the cauldron `state` is (by its block).
fn cauldron_table(state: u32) -> Option<Cauldron> {
    Some(match state::name(state) {
        "minecraft:cauldron" => Cauldron::Empty,
        "minecraft:water_cauldron" => Cauldron::Water,
        "minecraft:lava_cauldron" => Cauldron::Lava,
        "minecraft:powder_snow_cauldron" => Cauldron::PowderSnow,
        _ => return None,
    })
}

/// The item tags and items a cauldron's table has an entry for, as the export writes
/// the `interactions` field (`Dispatcher{tags={#minecraft:x};items={minecraft:y,...}}`).
fn cauldron_entries(table: &str) -> (Vec<&str>, Vec<&str>) {
    let list = |key: &str| -> Vec<&str> {
        table
            .split_once(key)
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(inside, _)| inside.split(',').filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    };
    (list("tags={"), list("items={"))
}

/// `AbstractCauldronBlock.useItemOn`: the cauldron's table answers for the held item
/// (its tags first, then the item itself; without an entry, the click goes on to the
/// empty hand). Filling and emptying are the server's: on the client each entry
/// only decides whether the click is used.
fn cauldron_use(w: &World<'_>, p: &LocalPlayer, u: &Use<'_>) -> Outcome {
    let held = p.hand(u.hand);
    let Some(table) = cauldron_table(u.state) else {
        return Outcome::TryEmptyHand;
    };
    let field = w
        .blocks
        .kind(u.state)
        .field("AbstractCauldronBlock.interactions")
        .unwrap_or("");
    let (tags, items) = cauldron_entries(field);
    let by_tag = tags
        .iter()
        .find(|t| w.item_is(held, t.trim_start_matches('#')));
    let used = if let Some(tag) = by_tag {
        // `dyedItemIteration`: only a dyed item is washed.
        *tag == "#minecraft:cauldron_can_remove_dye"
            && has_component(held, DataComponentType::DyedColor, false)
    } else if items.contains(&held.c.name) {
        cauldron_item_used(w, table, held, u.state)
    } else {
        false
    };
    if used {
        Outcome::Success
    } else {
        Outcome::TryEmptyHand
    }
}

/// Whether a cauldron table's entry for the held item uses the click.
fn cauldron_item_used(w: &World<'_>, table: Cauldron, held: &Stack, cauldron: u32) -> bool {
    let full = state::get_int(cauldron, "level") == 3;
    match held.c.name {
        // Emptying a bucket into any cauldron (lava and powder snow under water
        // only take the click).
        "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:powder_snow_bucket" => true,
        // `fillBucket`: water and powder snow only from a full cauldron.
        "minecraft:bucket" => table == Cauldron::Lava || full,
        "minecraft:glass_bottle" => true,
        // A water bottle fills an empty cauldron, or tops up one that isn't full.
        "minecraft:potion" => !(table == Cauldron::Water && full) && water_bottle(held),
        name if name.ends_with("_banner") => banner_has_patterns(held),
        // `shulkerBoxInteraction`: a dyed shulker box loses its colour.
        _ => held
            .c
            .block
            .is_some_and(|b| w.blocks.kinds().of_block(b).is(Class::ShulkerBoxBlock)),
    }
}

/// `PotionContents.is(Potions.WATER)`: a water bottle (a potion's default holds no
/// potion).
fn water_bottle(stack: &Stack) -> bool {
    let water = Potion::Water.id() as i32;
    matches!(
        stack.patch.get(DataComponentType::PotionContents),
        Some(Component::PotionContents { potion: Some(id) }) if *id == water
    )
}

/// A banner with at least one pattern layer (a banner's default has none).
fn banner_has_patterns(stack: &Stack) -> bool {
    match stack.patch.get(DataComponentType::BannerPatterns) {
        // The layers' count, a VarInt, leads.
        Some(Component::Other { data, .. }) => data.first().is_some_and(|&n| n != 0),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::testing::Fixture;
    use crate::pick::BlockHit;

    /// The block's own answer to a main-hand right click on the middle of `face`.
    fn answer(f: &mut Fixture, pos: Pos, face: Direction) -> Outcome {
        let n = face.normal();
        let hit = BlockHit {
            location: [0, 1, 2].map(|i| f64::from(pos[i]) + 0.5 + f64::from(n[i]) * 0.5),
            direction: face,
            pos,
            inside: false,
            miss: false,
        };
        let mut p = f.player.clone();
        let state = Fixture::state(&f.get(pos));
        let r = crate::block::use_item_on(&mut f.world(), &mut p, Hand::Main, state, pos, &hit);
        f.player = p;
        r
    }

    /// A floor, and a player south of the origin looking north and down.
    fn standing() -> Option<Fixture> {
        let mut f = Fixture::new()?;
        f.floor();
        f.player.position = [0.5, -60.0, 4.5];
        f.player.y_rot = 180.0;
        f.player.x_rot = 60.0;
        Some(f)
    }

    #[test]
    fn a_chest_beside_a_lone_chest_makes_a_pair() {
        let Some(mut f) = standing() else { return };
        f.set([0, -60, 0], "minecraft:chest[facing=south]");
        f.hold(Hand::Main, "minecraft:chest", 2);
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(
            f.get([1, -60, 0]),
            "minecraft:chest[facing=south,type=left,waterlogged=false]"
        );
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:chest[facing=south,type=right,waterlogged=false]"
        );
        // A third one has no lone chest to join.
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:chest[facing=south,type=single,waterlogged=false]"
        );
        // Trapped chests keep to themselves.
        f.hold(Hand::Main, "minecraft:trapped_chest", 1);
        f.set([-2, -60, 0], "minecraft:chest[facing=south]");
        f.click_face([-1, -61, 0], Direction::Up);
        assert!(f.get([-1, -60, 0]).contains("type=single"));
        assert!(f.get([-2, -60, 0]).contains("type=single"));
    }

    #[test]
    fn copper_chests_pair_as_the_younger_unwaxed_chest() {
        let Some(mut f) = standing() else { return };
        f.set(
            [0, -60, 0],
            "minecraft:waxed_exposed_copper_chest[facing=south]",
        );
        f.hold(Hand::Main, "minecraft:copper_chest", 1);
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(
            f.get([1, -60, 0]),
            "minecraft:copper_chest[facing=south,type=left,waterlogged=false]"
        );
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:copper_chest[facing=south,type=right,waterlogged=false]"
        );
    }

    #[test]
    fn a_bed_puts_its_head_the_way_the_player_looks() {
        let Some(mut f) = standing() else { return };
        f.hold(Hand::Main, "minecraft:red_bed", 1);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:red_bed[facing=north,occupied=false,part=foot]"
        );
        assert_eq!(
            f.get([0, -60, -1]),
            "minecraft:red_bed[facing=north,occupied=false,part=head]"
        );
        // No room for the head: nothing placed.
        f.set([1, -60, -1], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:red_bed", 1);
        f.click_face([1, -61, 0], Direction::Up);
        assert_eq!(f.get([1, -60, 0]), "minecraft:air");
        assert_eq!(
            answer(&mut f, [0, -60, 0], Direction::Up),
            Outcome::TryEmptyHand
        );
    }

    #[test]
    fn cake_is_eaten_a_slice_at_a_time() {
        let Some(mut f) = standing() else { return };
        f.set([0, -60, 0], "minecraft:cake[bites=5]");
        f.player.food = 10;
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:cake[bites=6]");
        assert_eq!(f.player.food, 12);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:air");
        // Full: nothing eaten.
        f.set([0, -60, 0], "minecraft:cake");
        f.player.food = 20;
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:cake[bites=0]");
    }

    #[test]
    fn candles_group_go_on_cakes_and_go_out() {
        let Some(mut f) = standing() else { return };
        f.hold(Hand::Main, "minecraft:white_candle", 4);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:white_candle[candles=1,lit=false,waterlogged=false]"
        );
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -60, 0]),
            "minecraft:white_candle[candles=2,lit=false,waterlogged=false]"
        );
        f.set([1, -60, 0], "minecraft:cake");
        f.click_face([1, -60, 0], Direction::Up);
        assert_eq!(f.get([1, -60, 0]), "minecraft:white_candle_cake[lit=false]");
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        // Lit ones go out under an empty hand.
        f.hold(Hand::Main, "minecraft:air", 0);
        f.set([1, -60, 0], "minecraft:white_candle_cake[lit=true]");
        f.click([1, -60, 0], Direction::Up, [0.5, 0.9, 0.5]);
        assert_eq!(f.get([1, -60, 0]), "minecraft:white_candle_cake[lit=false]");
        f.set([0, -60, 0], "minecraft:white_candle[lit=true,candles=2]");
        f.click_face([0, -60, 0], Direction::Up);
        assert!(f.get([0, -60, 0]).contains("lit=false"));
    }

    #[test]
    fn flowers_go_into_pots_and_come_out() {
        let Some(mut f) = standing() else { return };
        f.set([0, -60, 0], "minecraft:flower_pot");
        f.hold(Hand::Main, "minecraft:dandelion", 2);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:potted_dandelion");
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        // A full pot takes the click, nothing else.
        assert_eq!(answer(&mut f, [0, -60, 0], Direction::Up), Outcome::Success);
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        f.hold(Hand::Main, "minecraft:air", 0);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:flower_pot");
        assert_eq!(f.player.hand(Hand::Main).c.name, "minecraft:dandelion");
        let w = f.world();
        let azalea = state::block_named("minecraft:azalea").unwrap();
        let pot = pot_holding(&w, azalea).unwrap();
        assert_eq!(
            Block::from_id(pot).unwrap().name(),
            "minecraft:potted_azalea_bush"
        );
    }

    #[test]
    fn glowstone_charges_a_respawn_anchor_and_a_ready_composter_empties() {
        let Some(mut f) = standing() else { return };
        f.set([0, -60, 0], "minecraft:respawn_anchor");
        f.hold(Hand::Main, "minecraft:glowstone", 3);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), "minecraft:respawn_anchor[charges=1]");
        assert_eq!(f.player.hand(Hand::Main).count, 2);
        f.set([1, -60, 0], "minecraft:composter[level=8]");
        f.hold(Hand::Main, "minecraft:air", 0);
        f.click_face([1, -60, 0], Direction::Up);
        assert_eq!(f.get([1, -60, 0]), "minecraft:composter[level=0]");
    }

    #[test]
    fn cauldrons_answer_by_their_table() {
        let Some(mut f) = standing() else { return };
        let mut check = |cauldron: &str, item: &str, expect: Outcome| {
            f.set([0, -60, 0], cauldron);
            f.hold(Hand::Main, item, 1);
            assert_eq!(
                answer(&mut f, [0, -60, 0], Direction::Up),
                expect,
                "{item} on {cauldron}"
            );
        };
        use Outcome::{Success, TryEmptyHand};
        check("minecraft:cauldron", "minecraft:water_bucket", Success);
        check("minecraft:cauldron", "minecraft:bucket", TryEmptyHand);
        check("minecraft:cauldron", "minecraft:potion", TryEmptyHand);
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:bucket",
            TryEmptyHand,
        );
        check(
            "minecraft:water_cauldron[level=3]",
            "minecraft:bucket",
            Success,
        );
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:glass_bottle",
            Success,
        );
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:white_banner",
            TryEmptyHand,
        );
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:red_shulker_box",
            Success,
        );
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:shulker_box",
            TryEmptyHand,
        );
        check(
            "minecraft:water_cauldron[level=1]",
            "minecraft:leather_helmet",
            TryEmptyHand,
        );
        check("minecraft:lava_cauldron", "minecraft:bucket", Success);
        check(
            "minecraft:lava_cauldron",
            "minecraft:glass_bottle",
            TryEmptyHand,
        );
        check(
            "minecraft:powder_snow_cauldron[level=2]",
            "minecraft:bucket",
            TryEmptyHand,
        );
        check(
            "minecraft:powder_snow_cauldron[level=3]",
            "minecraft:bucket",
            Success,
        );
    }

    #[test]
    fn facing_blocks_and_game_master_blocks() {
        let Some(mut f) = standing() else { return };
        f.set([0, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:red_shulker_box", 1);
        f.click_face([0, -60, 0], Direction::East);
        assert_eq!(f.get([1, -60, 0]), "minecraft:red_shulker_box[facing=east]");
        f.hold(Hand::Main, "minecraft:ender_chest", 1);
        f.click_face([0, -60, 0], Direction::Up);
        assert_eq!(
            f.get([0, -59, 0]),
            "minecraft:ender_chest[facing=south,waterlogged=false]"
        );
        let jigsaw = Fixture::state("minecraft:jigsaw");
        let p = f.player.clone();
        let w = f.world();
        let on = |face| {
            let hit = BlockHit {
                location: [0.5, 0.5, 0.5],
                direction: face,
                pos: [0, -61, 0],
                inside: false,
                miss: false,
            };
            let ctx = PlaceContext::new(&w, crate::place::UseOnContext::new(&p, Hand::Main, hit));
            jigsaw_placement(&ctx, jigsaw)
        };
        assert_eq!(
            state::describe(on(Direction::Up)),
            "minecraft:jigsaw[orientation=up_south]"
        );
        assert_eq!(
            state::describe(on(Direction::West)),
            "minecraft:jigsaw[orientation=west_up]"
        );
        drop(w);
        // Not an operator: the structure block passes.
        f.set([2, -60, 0], "minecraft:structure_block");
        assert_eq!(
            answer(&mut f, [2, -60, 0], Direction::Up),
            Outcome::TryEmptyHand
        );
        let mut p = f.player.clone();
        let hit = BlockHit {
            location: [2.5, -59.0, 0.5],
            direction: Direction::Up,
            pos: [2, -60, 0],
            inside: false,
            miss: false,
        };
        let s = Fixture::state("minecraft:structure_block");
        let r = crate::block::use_without_item(&mut f.world(), &mut p, s, [2, -60, 0], &hit);
        assert_eq!(r, Outcome::Pass);
    }
}
