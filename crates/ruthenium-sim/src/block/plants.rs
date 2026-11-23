//! Plants on the ground: what each may stand on, crops, tall plants, flowers, grass and the bone-mealable ground blocks, seagrass and kelp, vines that grow up or down, cave vines, berry bushes, flower beds and leaf litter, mossy carpets.
//!
//! Each function answers for the classes this group ports and `None` for any other
//! class, so [`super`]'s dispatch moves on to the next group.
//!
//! Two things the vanilla client reads aren't tracked here. Light levels: crops
//! need a raw brightness of at least 8 and mushrooms one below 13 to stay; both are
//! taken as met, and the server corrects a placement it refuses. And the plant's
//! own support tag where it is a constructor setting (stems, fungi, roots): the
//! dispatch's `mayPlaceOn` doesn't pass the plant, so [`may_place_on`] accepts
//! either variant's tag; the plants' own `canSurvive` here reads the right one.

#![allow(unused_variables)]

use super::{Behaviour, Class, Pos, Support, Update, Use, state};
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
    (Class::VegetationBlock, Behaviour::UpdateShape),
    (Class::VegetationBlock, Behaviour::CanSurvive),
    (Class::VegetationBlock, Behaviour::MayPlaceOn),
    (Class::DryVegetationBlock, Behaviour::MayPlaceOn),
    (Class::WitherRoseBlock, Behaviour::MayPlaceOn),
    (Class::MushroomBlock, Behaviour::CanSurvive),
    (Class::MushroomBlock, Behaviour::MayPlaceOn),
    (Class::MushroomBlock, Behaviour::IsValidBonemealTarget),
    (Class::CropBlock, Behaviour::CanSurvive),
    (Class::CropBlock, Behaviour::MayPlaceOn),
    (Class::CropBlock, Behaviour::IsValidBonemealTarget),
    (Class::CactusFlowerBlock, Behaviour::MayPlaceOn),
    (Class::AttachedStemBlock, Behaviour::UpdateShape),
    (Class::AttachedStemBlock, Behaviour::MayPlaceOn),
    (Class::StemBlock, Behaviour::MayPlaceOn),
    (Class::StemBlock, Behaviour::IsValidBonemealTarget),
    (Class::LilyPadBlock, Behaviour::MayPlaceOn),
    (Class::NetherWartBlock, Behaviour::MayPlaceOn),
    (Class::PitcherCropBlock, Behaviour::GetStateForPlacement),
    (Class::PitcherCropBlock, Behaviour::UpdateShape),
    (Class::PitcherCropBlock, Behaviour::CanSurvive),
    (Class::PitcherCropBlock, Behaviour::CanBeReplaced),
    (Class::PitcherCropBlock, Behaviour::SetPlacedBy),
    (Class::PitcherCropBlock, Behaviour::MayPlaceOn),
    (Class::PitcherCropBlock, Behaviour::IsValidBonemealTarget),
    (Class::DoublePlantBlock, Behaviour::UpdateShape),
    (Class::DoublePlantBlock, Behaviour::SetPlacedBy),
    (Class::DoublePlantBlock, Behaviour::GetStateForPlacement),
    (Class::DoublePlantBlock, Behaviour::CanSurvive),
    (Class::TallFlowerBlock, Behaviour::IsValidBonemealTarget),
    (Class::TallGrassBlock, Behaviour::IsValidBonemealTarget),
    (Class::BushBlock, Behaviour::IsValidBonemealTarget),
    (Class::ShortDryGrassBlock, Behaviour::IsValidBonemealTarget),
    (Class::TallDryGrassBlock, Behaviour::IsValidBonemealTarget),
    (Class::GrassBlock, Behaviour::IsValidBonemealTarget),
    (Class::SaplingBlock, Behaviour::IsValidBonemealTarget),
    (
        Class::MangrovePropaguleBlock,
        Behaviour::GetStateForPlacement,
    ),
    (Class::MangrovePropaguleBlock, Behaviour::UpdateShape),
    (Class::MangrovePropaguleBlock, Behaviour::CanSurvive),
    (Class::MangrovePropaguleBlock, Behaviour::MayPlaceOn),
    (
        Class::MangrovePropaguleBlock,
        Behaviour::IsValidBonemealTarget,
    ),
    (Class::MangroveLeavesBlock, Behaviour::IsValidBonemealTarget),
    (Class::NetherFungusBlock, Behaviour::MayPlaceOn),
    (Class::NetherFungusBlock, Behaviour::IsValidBonemealTarget),
    (Class::NetherRootsBlock, Behaviour::MayPlaceOn),
    (Class::NetherSproutsBlock, Behaviour::MayPlaceOn),
    (Class::NetherrackBlock, Behaviour::IsValidBonemealTarget),
    (Class::NyliumBlock, Behaviour::IsValidBonemealTarget),
    (Class::AzaleaBlock, Behaviour::MayPlaceOn),
    (Class::AzaleaBlock, Behaviour::IsValidBonemealTarget),
    (Class::RootedDirtBlock, Behaviour::IsValidBonemealTarget),
    (Class::FireflyBushBlock, Behaviour::IsValidBonemealTarget),
    (
        Class::BonemealableFeaturePlacerBlock,
        Behaviour::IsValidBonemealTarget,
    ),
    (Class::FlowerBedBlock, Behaviour::GetStateForPlacement),
    (Class::FlowerBedBlock, Behaviour::CanBeReplaced),
    (Class::FlowerBedBlock, Behaviour::IsValidBonemealTarget),
    (Class::LeafLitterBlock, Behaviour::GetStateForPlacement),
    (Class::LeafLitterBlock, Behaviour::CanSurvive),
    (Class::LeafLitterBlock, Behaviour::CanBeReplaced),
    (Class::SweetBerryBushBlock, Behaviour::UseItemOn),
    (Class::SweetBerryBushBlock, Behaviour::UseWithoutItem),
    (Class::SweetBerryBushBlock, Behaviour::IsValidBonemealTarget),
    (Class::MossyCarpetBlock, Behaviour::GetStateForPlacement),
    (Class::MossyCarpetBlock, Behaviour::UpdateShape),
    (Class::MossyCarpetBlock, Behaviour::CanSurvive),
    (Class::MossyCarpetBlock, Behaviour::SetPlacedBy),
    (Class::MossyCarpetBlock, Behaviour::IsValidBonemealTarget),
    (Class::SeagrassBlock, Behaviour::GetStateForPlacement),
    (Class::SeagrassBlock, Behaviour::UpdateShape),
    (Class::SeagrassBlock, Behaviour::MayPlaceOn),
    (Class::SeagrassBlock, Behaviour::IsValidBonemealTarget),
    (Class::TallSeagrassBlock, Behaviour::GetStateForPlacement),
    (Class::TallSeagrassBlock, Behaviour::CanSurvive),
    (Class::TallSeagrassBlock, Behaviour::MayPlaceOn),
    (Class::KelpBlock, Behaviour::GetStateForPlacement),
    (Class::GrowingPlantHeadBlock, Behaviour::UpdateShape),
    (
        Class::GrowingPlantHeadBlock,
        Behaviour::IsValidBonemealTarget,
    ),
    (Class::GrowingPlantBlock, Behaviour::CanSurvive),
    (Class::GrowingPlantBlock, Behaviour::GetStateForPlacement),
    (Class::GrowingPlantBodyBlock, Behaviour::UpdateShape),
    (Class::GrowingPlantBodyBlock, Behaviour::CanBeReplaced),
    (
        Class::GrowingPlantBodyBlock,
        Behaviour::IsValidBonemealTarget,
    ),
    (Class::CaveVinesBlock, Behaviour::UseWithoutItem),
    (Class::CaveVinesBlock, Behaviour::IsValidBonemealTarget),
    (Class::CaveVinesPlantBlock, Behaviour::UseWithoutItem),
    (Class::CaveVinesPlantBlock, Behaviour::IsValidBonemealTarget),
    (Class::GlowLichenBlock, Behaviour::IsValidBonemealTarget),
];

/// The most segments a flower bed or leaf litter holds (`SegmentableBlock.MAX_SEGMENT`).
const MAX_SEGMENTS: i32 = 4;

/// The pitcher crop's last age, and the age from which it is two blocks tall.
const PITCHER_MAX_AGE: i32 = 4;
const PITCHER_TALL_AGE: i32 = 3;

/// `getStateForPlacement` (`Some(None)`: the block can't be placed there).
pub(super) fn state_for_placement(
    c: Class,
    w: &World<'_>,
    ctx: &PlaceContext,
    default: u32,
) -> Option<Option<u32>> {
    let pos = ctx.clicked_pos();
    let placed = match c {
        // `PitcherCropBlock.getStateForPlacement`: the default state, wherever
        // (the bulb is one block tall).
        Class::PitcherCropBlock => Some(default),
        Class::DoublePlantBlock => double_plant_placement(w, ctx, default),
        // `TallSeagrassBlock.getStateForPlacement`: as a tall plant, and the
        // upper half must go into full water too.
        Class::TallSeagrassBlock => double_plant_placement(w, ctx, default)
            .filter(|_| full_water(w, Direction::Up.step(pos))),
        // `MangrovePropaguleBlock.getStateForPlacement`: fully grown, and
        // waterlogged when it replaces a water source.
        Class::MangrovePropaguleBlock => {
            let in_source = w.fluid(pos).fluid == Fluid::Water;
            let s = state::set_bool(default, "waterlogged", in_source);
            Some(state::set_int(s, "age", 4))
        }
        Class::FlowerBedBlock | Class::LeafLitterBlock => {
            Some(segment_placement(w, ctx, segment_property(c), default))
        }
        // `MossyCarpetBlock.getStateForPlacement`: a carpet with a low side
        // against every wall next to it.
        Class::MossyCarpetBlock => Some(carpet_sides(w, default, pos, true)),
        // `SeagrassBlock.getStateForPlacement`: only into full water.
        Class::SeagrassBlock => full_water(w, pos).then_some(default),
        // `KelpBlock.getStateForPlacement`: only into full water, then as any
        // growing plant.
        Class::KelpBlock => full_water(w, pos).then(|| growing_plant_placement(w, pos, default)),
        Class::GrowingPlantBlock => Some(growing_plant_placement(w, pos, default)),
        _ => return None,
    };
    Some(placed)
}

/// `updateShape`.
pub(super) fn update_shape(c: Class, w: &World<'_>, u: &Update) -> Option<u32> {
    let s = u.state;
    let new = match c {
        // `SeagrassBlock.updateShape` only adds a water tick to this.
        Class::VegetationBlock | Class::SeagrassBlock => vegetation_shape(w, u),
        Class::AttachedStemBlock => attached_stem_shape(w, u),
        Class::DoublePlantBlock => double_plant_shape(w, u),
        // `PitcherCropBlock.updateShape`: while one block tall it only checks
        // that it may stay; once tall, as a tall plant.
        Class::PitcherCropBlock => {
            if state::get_int(s, "age") >= PITCHER_TALL_AGE {
                double_plant_shape(w, u)
            } else if super::can_survive(w, s, u.pos) {
                s
            } else {
                state::AIR
            }
        }
        // `MangrovePropaguleBlock.updateShape`: a change above that leaves it
        // unable to stay removes it (a hanging one loses its hold); otherwise as a
        // plant.
        Class::MangrovePropaguleBlock => {
            if u.direction == Direction::Up && !super::can_survive(w, s, u.pos) {
                state::AIR
            } else {
                vegetation_shape(w, u)
            }
        }
        // `MossyCarpetBlock.updateShape`: gone when unsupported, else its sides
        // follow the walls next to it, and gone when no side or base is left.
        Class::MossyCarpetBlock => {
            if super::can_survive(w, s, u.pos) {
                let sides = carpet_sides(w, s, u.pos, false);
                if carpet_has_faces(sides) {
                    sides
                } else {
                    state::AIR
                }
            } else {
                state::AIR
            }
        }
        Class::GrowingPlantHeadBlock => growing_head_shape(w, u),
        Class::GrowingPlantBodyBlock => growing_body_shape(w, u),
        _ => return None,
    };
    Some(new)
}

/// `canSurvive`.
pub(super) fn can_survive(c: Class, w: &World<'_>, state: u32, pos: Pos) -> Option<bool> {
    let below_pos = Direction::Down.step(pos);
    let survives = match c {
        Class::VegetationBlock => vegetation_can_survive(w, state, pos),
        // `MushroomBlock.canSurvive`: anything on mycelium-like ground (the
        // light-override tag); elsewhere it needs the dark (taken as met) and a
        // solid block.
        Class::MushroomBlock => {
            let below = w.get(below_pos);
            w.tags
                .is(below, "minecraft:overrides_mushroom_light_requirement")
                || dark_enough_for_mushrooms(w, pos) && may_place_on_for(w, state, below, below_pos)
        }
        // `CropBlock.canSurvive`: enough light (taken as met) and the ground.
        Class::CropBlock => crop_light(w, pos) && vegetation_can_survive(w, state, pos),
        // `PitcherCropBlock.canSurvive`: the lower half also needs crop light.
        Class::PitcherCropBlock => {
            !(is_lower_pitcher(state) && !crop_light(w, pos))
                && double_plant_can_survive(w, state, pos)
        }
        Class::DoublePlantBlock => double_plant_can_survive(w, state, pos),
        // `MangrovePropaguleBlock.canSurvive`: a hanging propagule hangs from a
        // block of its tag; a planted one stands on its ground.
        Class::MangrovePropaguleBlock => {
            if state::get_bool(state, "hanging") {
                let above = w.get(Direction::Up.step(pos));
                w.tags
                    .is(above, "minecraft:supports_hanging_mangrove_propagule")
            } else {
                vegetation_can_survive(w, state, pos)
            }
        }
        // `TallSeagrassBlock.canSurvive`: the top on its own bottom half; the
        // bottom on its ground and in full water.
        Class::TallSeagrassBlock => {
            if is_upper(state) {
                lower_half_below(w, state, pos)
            } else {
                double_plant_can_survive(w, state, pos) && full_water(w, pos)
            }
        }
        // `LeafLitterBlock.canSurvive`: a full top face below.
        Class::LeafLitterBlock => w.face_sturdy(below_pos, Direction::Up, Support::Full),
        // `MossyCarpetBlock.canSurvive`: a carpet with a base on anything but air;
        // a sides-only one on a carpet that has its base.
        Class::MossyCarpetBlock => {
            let below = w.get(below_pos);
            if state::get_bool(state, "bottom") {
                !w.blocks.is_air(below)
            } else {
                state::same_block(below, state) && state::get_bool(below, "bottom")
            }
        }
        Class::GrowingPlantBlock => growing_plant_can_survive(w, state, pos),
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
        // `PitcherCropBlock.canBeReplaced`: never.
        Class::PitcherCropBlock => false,
        // `FlowerBedBlock`/`LeafLitterBlock.canBeReplaced`: the same item adds a
        // segment (unless sneaking, or it is full); otherwise as any block.
        Class::FlowerBedBlock | Class::LeafLitterBlock => {
            let item = ctx.item();
            let adds_segment = !ctx.secondary_use()
                && !item.is_empty()
                && super::item_id(item) == w.blocks.kind(state).item
                && state::get_int(state, segment_property(c)) < MAX_SEGMENTS;
            adds_segment || super::default_can_be_replaced(w, state, ctx)
        }
        // `GrowingPlantBodyBlock.canBeReplaced`: as any block, except that the
        // plant's own item (the head's) doesn't replace its stem.
        Class::GrowingPlantBodyBlock => {
            let head_item = w
                .blocks
                .kinds()
                .of_block(GrowingPlant::of(w, state).head)
                .item;
            let item = ctx.item();
            super::default_can_be_replaced(w, state, ctx)
                && (item.is_empty() || super::item_id(item) != head_item)
        }
        _ => return None,
    };
    Some(replaced)
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
    match c {
        // `SweetBerryBushBlock.useItemOn`: bone meal on an unripe bush is left to
        // the item; anything else tries the empty hand (picking).
        Class::SweetBerryBushBlock => {
            let item = p.hand(u.hand);
            let ripe = state::get_int(u.state, "age") == 3;
            let bone_meal = !item.is_empty() && item.c.name == "minecraft:bone_meal";
            Some(if !ripe && bone_meal {
                Outcome::Pass
            } else {
                Outcome::TryEmptyHand
            })
        }
        _ => None,
    }
}

/// `useWithoutItem`.
pub(super) fn use_without_item(
    c: Class,
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    u: &Use<'_>,
) -> Option<Outcome> {
    match c {
        // `SweetBerryBushBlock.useWithoutItem`: a bush past age 1 has berries to
        // pick. Dropping them and cutting the bush back to age 1 is the server's;
        // the client only reports success.
        Class::SweetBerryBushBlock => Some(if state::get_int(u.state, "age") > 1 {
            Outcome::Success
        } else {
            Outcome::Pass
        }),
        // `CaveVines.use` (both halves): berries are picked on the server; the
        // client succeeds when there are some.
        Class::CaveVinesBlock | Class::CaveVinesPlantBlock => {
            Some(if state::get_bool(u.state, "berries") {
                Outcome::Success
            } else {
                Outcome::Pass
            })
        }
        _ => None,
    }
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
        // `DoublePlantBlock.setPlacedBy`: the upper half goes above, waterlogged
        // when there is water there.
        Class::DoublePlantBlock => {
            let above = Direction::Up.step(pos);
            let upper = state::set(
                state::default_of_block(state::block_id(state)),
                "half",
                "upper",
            );
            let upper = waterlogged_like(w, above, upper);
            w.set_block_and_update(above, upper);
            Some(())
        }
        // `PitcherCropBlock.setPlacedBy` does nothing (the bulb is one block), and
        // `MossyCarpetBlock.setPlacedBy` grows its random topper on the server only.
        Class::PitcherCropBlock | Class::MossyCarpetBlock => Some(()),
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
/// `below_pos`). The dispatch doesn't say which plant asks, so for the plants whose
/// ground is a constructor setting either variant's tag counts (see the module
/// notes); this group's own `canSurvive` asks with the plant.
pub(super) fn may_place_on(
    c: Class,
    w: &World<'_>,
    plant: u32,
    below: u32,
    below_pos: Pos,
) -> Option<bool> {
    plant_may_place_on(c, w, Some(plant), below, below_pos)
}

/// `isValidBonemealTarget`.
pub(super) fn is_valid_bonemeal_target(
    c: Class,
    w: &World<'_>,
    state: u32,
    pos: Pos,
) -> Option<bool> {
    let above = Direction::Up.step(pos);
    let below = Direction::Down.step(pos);
    let valid = match c {
        // `MushroomBlock`, `SaplingBlock` and `AzaleaBlock` ask the server's
        // feature registry and tree growers; on a client level they say no.
        Class::MushroomBlock | Class::SaplingBlock | Class::AzaleaBlock => false,
        // `CropBlock.isValidBonemealTarget`: not yet fully grown.
        Class::CropBlock => state::get_int(state, "age") < crop_max_age(w, state),
        // `StemBlock.isValidBonemealTarget`: below age 7.
        Class::StemBlock => state::get_int(state, "age") != 7,
        Class::PitcherCropBlock => pitcher_can_be_bonemealed(w, state, pos),
        // Tall flowers drop a copy, short dry grass grows tall, flower beds add a
        // flower or drop one: always something to do.
        Class::TallFlowerBlock | Class::ShortDryGrassBlock | Class::FlowerBedBlock => true,
        // `TallGrassBlock.isValidBonemealTarget`: its tall form (large fern for a
        // fern, else tall grass) could stand here, with an empty block inside the
        // level above.
        Class::TallGrassBlock => {
            let grown = if state::is(state, "minecraft:fern") {
                "minecraft:large_fern"
            } else {
                "minecraft:tall_grass"
            };
            super::can_survive(w, state::default_of(grown), pos)
                && w.is_empty_block(above)
                && w.inside_build_height(above[1])
        }
        // `BushBlock`/`FireflyBushBlock`: a copy could spread next to it.
        Class::BushBlock | Class::FireflyBushBlock => has_spreadable_neighbour(w, pos, state),
        // `TallDryGrassBlock`: short dry grass could spread next to it.
        Class::TallDryGrassBlock => {
            has_spreadable_neighbour(w, pos, state::default_of("minecraft:short_dry_grass"))
        }
        // `GrassBlock`/`NyliumBlock`: air above, inside the level.
        Class::GrassBlock | Class::NyliumBlock => {
            w.blocks.is_air(w.get(above)) && w.inside_build_height(above[1])
        }
        // `MangrovePropaguleBlock`: a planted one may grow a tree; a hanging one
        // until fully grown.
        Class::MangrovePropaguleBlock => {
            !state::get_bool(state, "hanging") || state::get_int(state, "age") != 4
        }
        // `MangroveLeavesBlock`: air below for a new propagule.
        Class::MangroveLeavesBlock => w.blocks.is_air(w.get(below)),
        // `NetherFungusBlock`: on its own nylium, inside the level above.
        Class::NetherFungusBlock => {
            let nylium = w
                .blocks
                .kind(state)
                .field("NetherFungusBlock.requiredBlock");
            nylium.is_some_and(|n| state::is(w.get(below), n)) && w.inside_build_height(above[1])
        }
        Class::NetherrackBlock => netherrack_can_be_bonemealed(w, pos),
        // `RootedDirtBlock`: air below, inside the level, for hanging roots.
        Class::RootedDirtBlock => w.blocks.is_air(w.get(below)) && w.inside_build_height(below[1]),
        // `BonemealableFeaturePlacerBlock` (moss blocks): air above.
        Class::BonemealableFeaturePlacerBlock => w.blocks.is_air(w.get(above)),
        // `SweetBerryBushBlock`: not yet ripe.
        Class::SweetBerryBushBlock => state::get_int(state, "age") < 3,
        // `MossyCarpetBlock`: a carpet with a base that could grow a topper.
        Class::MossyCarpetBlock => {
            state::get_bool(state, "bottom") && !w.blocks.is_air(carpet_topper(w, pos))
        }
        // `SeagrassBlock`: a water block above to grow tall into.
        Class::SeagrassBlock => state::is(w.get(above), "minecraft:water"),
        // `GrowingPlantHeadBlock`: room to grow one more block, inside the level.
        Class::GrowingPlantHeadBlock => {
            let plant = GrowingPlant::of(w, state);
            let next = plant.growth.step(pos);
            plant.can_grow_into(w, w.get(next)) && w.inside_build_height(next[1])
        }
        // `GrowingPlantBodyBlock`: the same for the head at the end of the stem.
        Class::GrowingPlantBodyBlock => {
            let plant = GrowingPlant::of(w, state);
            plant
                .head_after(w, pos, state::block_id(state))
                .is_some_and(|head| {
                    let next = plant.growth.step(head);
                    plant.can_grow_into(w, w.get(next)) && w.inside_build_height(next[1])
                })
        }
        // `CaveVinesBlock`/`CaveVinesPlantBlock`: no berries yet.
        Class::CaveVinesBlock | Class::CaveVinesPlantBlock => !state::get_bool(state, "berries"),
        Class::GlowLichenBlock => lichen_can_spread(w, state, pos),
        _ => return None,
    };
    Some(valid)
}

// Ground rules.

/// The `mayPlaceOn` of the classes here: `plant` is the plant asking, when known
/// (its constructor's support tag is read from it).
fn plant_may_place_on(
    c: Class,
    w: &World<'_>,
    plant: Option<u32>,
    below: u32,
    below_pos: Pos,
) -> Option<bool> {
    let tagged = |tag: &str| w.tags.is(below, tag);
    let own_tag = |field: &str, variants: [&str; 2]| match plant
        .and_then(|p| w.blocks.kind(p).field(field))
    {
        Some(tag) => tagged(tag.trim_start_matches('#')),
        None => variants.iter().any(|t| tagged(t)),
    };
    let may = match c {
        // `VegetationBlock.mayPlaceOn`: dirt-like ground and farmland.
        Class::VegetationBlock => tagged("minecraft:supports_vegetation"),
        Class::DryVegetationBlock => tagged("minecraft:supports_dry_vegetation"),
        Class::WitherRoseBlock => tagged("minecraft:supports_wither_rose"),
        // `MushroomBlock.mayPlaceOn`: any solid-rendering block.
        Class::MushroomBlock => w.blocks.solid_render(below),
        Class::CropBlock | Class::PitcherCropBlock => tagged("minecraft:supports_crops"),
        // `CactusFlowerBlock.mayPlaceOn`: its override tag, or a top face that
        // holds something by its centre.
        Class::CactusFlowerBlock => {
            tagged("minecraft:support_override_cactus_flower")
                || w.face_sturdy(below_pos, Direction::Up, Support::Center)
        }
        Class::AttachedStemBlock => own_tag(
            "AttachedStemBlock.supportBlocks",
            [
                "minecraft:supports_pumpkin_stem",
                "minecraft:supports_melon_stem",
            ],
        ),
        Class::StemBlock => own_tag(
            "StemBlock.stemSupportBlocks",
            [
                "minecraft:supports_pumpkin_stem",
                "minecraft:supports_melon_stem",
            ],
        ),
        // `LilyPadBlock.mayPlaceOn`: on a water source (the fluid tag
        // `supports_lily_pad` holds only `minecraft:water`) or a block of the
        // block tag, with no fluid above it.
        Class::LilyPadBlock => {
            let on_water = w.fluid(below_pos).fluid == Fluid::Water;
            (on_water || tagged("minecraft:supports_lily_pad"))
                && w.fluid(Direction::Up.step(below_pos)).is_empty()
        }
        Class::NetherWartBlock => tagged("minecraft:supports_nether_wart"),
        Class::MangrovePropaguleBlock => tagged("minecraft:supports_mangrove_propagule"),
        Class::NetherFungusBlock => own_tag(
            "NetherFungusBlock.supportBlocks",
            [
                "minecraft:supports_crimson_fungus",
                "minecraft:supports_warped_fungus",
            ],
        ),
        Class::NetherRootsBlock => own_tag(
            "NetherRootsBlock.supportBlocks",
            [
                "minecraft:supports_crimson_roots",
                "minecraft:supports_warped_roots",
            ],
        ),
        Class::NetherSproutsBlock => tagged("minecraft:supports_nether_sprouts"),
        Class::AzaleaBlock => tagged("minecraft:supports_azalea"),
        // `SeagrassBlock`/`TallSeagrassBlock.mayPlaceOn`: a full top face, not of
        // the blocks tagged against seagrass.
        Class::SeagrassBlock | Class::TallSeagrassBlock => {
            w.face_sturdy(below_pos, Direction::Up, Support::Full)
                && !tagged("minecraft:cannot_support_seagrass")
        }
        _ => return None,
    };
    Some(may)
}

/// The plant's own `mayPlaceOn` (a virtual call): this group's version with the
/// plant known, or another group's through the dispatch.
fn may_place_on_for(w: &World<'_>, plant: u32, below: u32, below_pos: Pos) -> bool {
    w.blocks
        .kind(plant)
        .declared(Behaviour::MayPlaceOn)
        .and_then(|c| plant_may_place_on(c, w, Some(plant), below, below_pos))
        .unwrap_or_else(|| super::may_place_on(w, plant, below, below_pos))
}

/// `VegetationBlock.canSurvive`: the plant may stand on the block below.
fn vegetation_can_survive(w: &World<'_>, plant: u32, pos: Pos) -> bool {
    let below_pos = Direction::Down.step(pos);
    may_place_on_for(w, plant, w.get(below_pos), below_pos)
}

/// `VegetationBlock.updateShape`: a plant that can't stay any more becomes air
/// (the server breaks it); otherwise it is unchanged.
fn vegetation_shape(w: &World<'_>, u: &Update) -> u32 {
    if super::can_survive(w, u.state, u.pos) {
        u.state
    } else {
        state::AIR
    }
}

/// `CropBlock.hasSufficientLight` (raw brightness 8 or more). The client keeps no
/// light levels, so it is taken as met.
fn crop_light(_w: &World<'_>, _pos: Pos) -> bool {
    true
}

/// `MushroomBlock.canSurvive`'s light rule (raw brightness below 13), taken as
/// met for want of light levels.
fn dark_enough_for_mushrooms(_w: &World<'_>, _pos: Pos) -> bool {
    true
}

/// `CropBlock.getMaxAge`: torchflower crops bloom after age 2 (their `age`
/// property only reaches 1, so they always take bone meal), beetroots at 3, the
/// other crops at 7.
fn crop_max_age(w: &World<'_>, crop: u32) -> i32 {
    if w.blocks.is_class(crop, Class::TorchflowerCropBlock) {
        2
    } else if w.blocks.is_class(crop, Class::BeetrootBlock) {
        3
    } else {
        7
    }
}

/// A full block of water there (`FluidTags.WATER` and `isFull`: amount 8, a
/// source or falling water).
fn full_water(w: &World<'_>, pos: Pos) -> bool {
    let f = w.fluid(pos);
    f.fluid.is_water() && f.amount == 8
}

/// `DoublePlantBlock.copyWaterloggedFrom`: a waterloggable state takes on whether
/// there is water at `pos`.
fn waterlogged_like(w: &World<'_>, pos: Pos, s: u32) -> u32 {
    if state::has(s, "waterlogged") {
        state::set_bool(s, "waterlogged", w.is_water_at(pos))
    } else {
        s
    }
}

/// `BonemealableBlock.hasSpreadableNeighbourPos`: one of the four blocks beside
/// `pos` is air where `plant` could stand.
fn has_spreadable_neighbour(w: &World<'_>, pos: Pos, plant: u32) -> bool {
    Direction::HORIZONTAL.iter().any(|d| {
        let n = d.step(pos);
        w.is_empty_block(n) && super::can_survive(w, plant, n)
    })
}

/// `AttachedStemBlock.updateShape`: a stem whose fruit (on its facing side) is
/// gone turns back into a fully grown loose stem; otherwise as a plant.
fn attached_stem_shape(w: &World<'_>, u: &Update) -> u32 {
    let kind = w.blocks.kind(u.state);
    let fruit = kind.field("AttachedStemBlock.fruit");
    let faces = state::get_dir(u.state, "facing") == Some(u.direction);
    if faces
        && fruit.is_none_or(|f| !state::is(u.neighbour, f))
        && let Some(stem) = kind
            .field("AttachedStemBlock.stem")
            .and_then(state::block_named)
    {
        return state::try_set(state::default_of_block(stem), "age", "7");
    }
    vegetation_shape(w, u)
}

// Two-block plants.

fn is_upper(s: u32) -> bool {
    state::get(s, "half") == Some("upper")
}

/// The block below `pos` is the lower half of `s`'s plant.
fn lower_half_below(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let below = w.get(Direction::Down.step(pos));
    state::same_block(below, s) && state::get(below, "half") == Some("lower")
}

/// `DoublePlantBlock.getStateForPlacement`: the plant's default state, when it
/// isn't at the top of the level and the block above may be built over.
fn double_plant_placement(w: &World<'_>, ctx: &PlaceContext, default: u32) -> Option<u32> {
    let pos = ctx.clicked_pos();
    let above = Direction::Up.step(pos);
    (pos[1] < w.max_y() && super::can_be_replaced(w, w.get(above), ctx)).then_some(default)
}

/// `DoublePlantBlock.canSurvive`: the top half sits on its own bottom half; the
/// bottom half stands on its ground like any plant.
fn double_plant_can_survive(w: &World<'_>, s: u32, pos: Pos) -> bool {
    if is_upper(s) {
        lower_half_below(w, s, pos)
    } else {
        vegetation_can_survive(w, s, pos)
    }
}

/// `DoublePlantBlock.updateShape`: each half needs the other, above or below it
/// as the half says; the bottom half also needs its ground; otherwise as a plant.
fn double_plant_shape(w: &World<'_>, u: &Update) -> u32 {
    let s = u.state;
    let lower = !is_upper(s);
    let towards_partner = u.direction.axis() == Axis::Y && lower == (u.direction == Direction::Up);
    let partner_there = state::same_block(u.neighbour, s)
        && state::get(u.neighbour, "half") != state::get(s, "half");
    if towards_partner && !partner_there {
        return state::AIR;
    }
    if lower && u.direction == Direction::Down && !super::can_survive(w, s, u.pos) {
        return state::AIR;
    }
    vegetation_shape(w, u)
}

fn is_lower_pitcher(s: u32) -> bool {
    state::is(s, "minecraft:pitcher_crop") && !is_upper(s)
}

/// `PitcherCropBlock.isValidBonemealTarget`: from its lower half (this block, or
/// the one below), it may grow a stage: not fully grown, light (taken as met), the
/// block above inside the level, and once tall, that block air or the crop.
fn pitcher_can_be_bonemealed(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let below_pos = Direction::Down.step(pos);
    let below = w.get(below_pos);
    let (lower_pos, lower) = if is_lower_pitcher(s) {
        (pos, s)
    } else if is_lower_pitcher(below) {
        (below_pos, below)
    } else {
        return false;
    };
    let age = state::get_int(lower, "age");
    let above_pos = Direction::Up.step(lower_pos);
    let above = w.get(above_pos);
    let room = age + 1 < PITCHER_TALL_AGE
        || w.blocks.is_air(above)
        || state::is(above, "minecraft:pitcher_crop");
    age < PITCHER_MAX_AGE && crop_light(w, lower_pos) && w.inside_build_height(above_pos[1]) && room
}

// Flower beds and leaf litter.

/// The property counting a segmentable block's pieces.
fn segment_property(c: Class) -> &'static str {
    if c == Class::FlowerBedBlock {
        "flower_amount"
    } else {
        "segment_amount"
    }
}

/// `SegmentableBlock.getStateForPlacement`: one more piece on a block of the
/// same kind (at most four); a new one has one piece and faces the player.
fn segment_placement(w: &World<'_>, ctx: &PlaceContext, amount: &str, default: u32) -> u32 {
    let there = w.get(ctx.clicked_pos());
    if state::same_block(there, default) {
        let more = (state::get_int(there, amount) + 1).min(MAX_SEGMENTS);
        state::set_int(there, amount, more)
    } else {
        state::set_dir(default, "facing", ctx.horizontal_direction().opposite())
    }
}

// Pale moss carpets.

fn is_moss_carpet(s: u32) -> bool {
    state::is(s, "minecraft:pale_moss_carpet")
}

/// `MultifaceBlock.canAttachTo`: the block towards `direction` has a full face
/// towards `pos`, by its support shape or its collision shape.
fn face_holds(w: &World<'_>, pos: Pos, direction: Direction) -> bool {
    let neighbour = w.get(direction.step(pos));
    let face = direction.opposite();
    w.blocks.face_sturdy(neighbour, face, Support::Full)
        || w.blocks.collision_face_full(neighbour, face)
}

/// `MossyCarpetBlock.getUpdatedState`: each horizontal side against a wall is
/// low (a new side when `create` or the carpet has a base, else the side as it
/// was), tall where the carpet above hangs a side there too (and has no base); a
/// sides-only carpet drops a side the carpet below lacks. Sides with no wall go.
fn carpet_sides(w: &World<'_>, carpet: u32, pos: Pos, create: bool) -> u32 {
    let base = state::get_bool(carpet, "bottom");
    let create = create || base;
    let above = w.get(Direction::Up.step(pos));
    let below = w.get(Direction::Down.step(pos));
    let mut out = carpet;
    for d in Direction::HORIZONTAL {
        let side = d.name();
        let mut value = if !face_holds(w, pos, d) {
            "none"
        } else if create {
            "low"
        } else {
            state::get(carpet, side).unwrap_or("none")
        };
        if value == "low" {
            if is_moss_carpet(above)
                && state::get(above, side) != Some("none")
                && !state::get_bool(above, "bottom")
            {
                value = "tall";
            }
            if !base && is_moss_carpet(below) && state::get(below, side) == Some("none") {
                value = "none";
            }
        }
        out = state::set(out, side, value);
    }
    out
}

/// `MossyCarpetBlock.hasFaces`: a base or any side.
fn carpet_has_faces(s: u32) -> bool {
    state::get_bool(s, "bottom")
        || Direction::HORIZONTAL
            .iter()
            .any(|d| state::get(s, d.name()) != Some("none"))
}

/// `MossyCarpetBlock.createTopperWithSideChance` with every side kept (as bone
/// meal asks): the sides-only carpet that could go above `pos` (on air or
/// something replaceable, or a sides-only carpet it changes), or air.
fn carpet_topper(w: &World<'_>, pos: Pos) -> u32 {
    let above = Direction::Up.step(pos);
    let there = w.get(above);
    let carpet_there = is_moss_carpet(there);
    let blocked = if carpet_there {
        state::get_bool(there, "bottom")
    } else {
        !w.blocks.replaceable(there)
    };
    if blocked {
        return state::AIR;
    }
    let sides_only = state::set_bool(
        state::default_of("minecraft:pale_moss_carpet"),
        "bottom",
        false,
    );
    let topper = carpet_sides(w, sides_only, above, true);
    if carpet_has_faces(topper) && topper != there {
        topper
    } else {
        state::AIR
    }
}

// Plants that grow up or down: kelp, weeping, twisting and cave vines.

/// A growing plant's two blocks (the head at its tip, the body behind it) and
/// the way it grows. The body is the head's block with `_plant` after its name.
struct GrowingPlant {
    head: u32,
    body: u32,
    growth: Direction,
}

impl GrowingPlant {
    fn of(w: &World<'_>, s: u32) -> GrowingPlant {
        let name = state::name(s);
        let head_name = name.strip_suffix("_plant").unwrap_or(name);
        let growth = w
            .blocks
            .kind(s)
            .field("GrowingPlantBlock.growthDirection")
            .and_then(|d| Direction::from_name(&d.to_ascii_lowercase()))
            .unwrap_or(Direction::Up);
        GrowingPlant {
            head: state::block_named(head_name).unwrap_or(u32::MAX),
            body: state::block_named(&format!("{head_name}_plant")).unwrap_or(u32::MAX),
            growth,
        }
    }

    /// The state is the head or the body.
    fn is_part(&self, s: u32) -> bool {
        let b = state::block_id(s);
        b == self.head || b == self.body
    }

    fn is_kelp(&self, w: &World<'_>) -> bool {
        w.blocks.kinds().of_block(self.head).is(Class::KelpBlock)
    }

    /// `canAttachTo`: kelp won't hang on blocks tagged against it; the vines on
    /// anything.
    fn can_attach_to(&self, w: &World<'_>, s: u32) -> bool {
        !self.is_kelp(w) || !w.tags.is(s, "minecraft:cannot_support_kelp")
    }

    /// The head's `canGrowInto`: kelp into a water block, the vines into air.
    fn can_grow_into(&self, w: &World<'_>, s: u32) -> bool {
        if self.is_kelp(w) {
            state::is(s, "minecraft:water")
        } else {
            w.blocks.is_air(s)
        }
    }

    /// `BlockUtil.getTopConnectedBlock`: past the run of `body` blocks from
    /// `pos` in the growth direction, the head's position if a head is there.
    fn head_after(&self, w: &World<'_>, pos: Pos, body: u32) -> Option<Pos> {
        let mut at = self.growth.step(pos);
        while state::block_id(w.get(at)) == body {
            at = self.growth.step(at);
        }
        (state::block_id(w.get(at)) == self.head).then_some(at)
    }

    /// A head turned into body (`updateBodyAfterConvertedFromHead`: cave vines
    /// keep their berries).
    fn body_from(&self, w: &World<'_>, head: u32) -> u32 {
        let body = state::default_of_block(self.body);
        if w.blocks.is_class(head, Class::CaveVinesBlock) {
            state::set_bool(body, "berries", state::get_bool(head, "berries"))
        } else {
            body
        }
    }

    /// A body turned into head (`updateHeadAfterConvertedFromBody`: cave vines
    /// keep their berries). The game picks the new head's age at random
    /// (`getStateForPlacement(random)`); age 0 stands in for it here.
    fn head_from(&self, w: &World<'_>, body: u32) -> u32 {
        let head = state::default_of_block(self.head);
        if w.blocks.is_class(body, Class::CaveVinesPlantBlock) {
            state::set_bool(head, "berries", state::get_bool(body, "berries"))
        } else {
            head
        }
    }
}

/// `GrowingPlantBlock.getStateForPlacement`: placed behind the plant's head or
/// body (where it grows from), it becomes body; otherwise the class's own state:
/// a head with a random age in the game (the default age here), or the body.
fn growing_plant_placement(w: &World<'_>, pos: Pos, default: u32) -> u32 {
    let plant = GrowingPlant::of(w, default);
    if plant.is_part(w.get(plant.growth.step(pos))) {
        state::default_of_block(plant.body)
    } else {
        default
    }
}

/// `GrowingPlantBlock.canSurvive`: the block it grows from must be one it may
/// attach to, and be its own head or body or have a full face towards it.
fn growing_plant_can_survive(w: &World<'_>, s: u32, pos: Pos) -> bool {
    let plant = GrowingPlant::of(w, s);
    let from_pos = plant.growth.opposite().step(pos);
    let from = w.get(from_pos);
    plant.can_attach_to(w, from)
        && (plant.is_part(from) || w.face_sturdy(from_pos, plant.growth, Support::Full))
}

/// `GrowingPlantHeadBlock.updateShape`: a head with the plant continuing beyond
/// it becomes body: heard from behind while it may stay and the block ahead is
/// part of the plant, or heard from ahead that the neighbour is. A head that
/// can't stay is the server's to break.
fn growing_head_shape(w: &World<'_>, u: &Update) -> u32 {
    let plant = GrowingPlant::of(w, u.state);
    let continued = if u.direction == plant.growth.opposite() {
        super::can_survive(w, u.state, u.pos) && plant.is_part(w.get(plant.growth.step(u.pos)))
    } else {
        u.direction == plant.growth && plant.is_part(u.neighbour)
    };
    if continued {
        plant.body_from(w, u.state)
    } else {
        u.state
    }
}

/// `GrowingPlantBodyBlock.updateShape`: a body whose block ahead is no longer
/// part of the plant becomes the head. One that can't stay is the server's.
fn growing_body_shape(w: &World<'_>, u: &Update) -> u32 {
    let plant = GrowingPlant::of(w, u.state);
    if u.direction == plant.growth && !plant.is_part(u.neighbour) {
        plant.head_from(w, u.state)
    } else {
        u.state
    }
}

// Ground that bone meal spreads over.

/// `NetherrackBlock.isValidBonemealTarget`: the block above lets sky light
/// through and a nylium block is within one block (diagonals included).
fn netherrack_can_be_bonemealed(w: &World<'_>, pos: Pos) -> bool {
    if !propagates_skylight_down(w, w.get(Direction::Up.step(pos))) {
        return false;
    }
    (-1..=1).any(|dx| {
        (-1..=1).any(|dy| {
            (-1..=1).any(|dz| {
                w.tags.is(
                    w.get([pos[0] + dx, pos[1] + dy, pos[2] + dz]),
                    "minecraft:nylium",
                )
            })
        })
    })
}

/// `propagatesSkylightDown`, by the nearest class that sets it: by default a
/// state whose shape isn't the full block and holds no fluid.
fn propagates_skylight_down(w: &World<'_>, s: u32) -> bool {
    let dry = || w.blocks.fluid(s).is_empty();
    for &c in &w.blocks.kind(s).classes {
        match c {
            Class::BarrierBlock
            | Class::LightBlock
            | Class::VegetationBlock
            | Class::GlowLichenBlock => {
                return dry();
            }
            Class::CrossCollisionBlock | Class::WallBlock => {
                return !state::get_bool(s, "waterlogged");
            }
            Class::TintedGlassBlock
            | Class::PipeBlock
            | Class::LiquidBlock
            | Class::ShulkerBoxBlock => {
                return false;
            }
            Class::HangingMossBlock
            | Class::BambooStalkBlock
            | Class::TransparentBlock
            | Class::VineBlock
            | Class::MossyCarpetBlock => return true,
            _ => {}
        }
    }
    !w.blocks.outline_shape(s).is_block() && dry()
}

// Glow lichen.

fn has_face(s: u32, face: Direction) -> bool {
    state::get_bool(s, face.name())
}

/// `GlowLichenBlock.isValidBonemealTarget` (`MultifaceSpreader.canSpreadInAnyDirection`
/// from every face): from a face it has, towards a side it lacks (on another
/// axis), a new face fits on the same block, on the next block in the same plane,
/// or around the edge.
fn lichen_can_spread(w: &World<'_>, lichen: u32, pos: Pos) -> bool {
    Direction::ALL.iter().any(|&from| {
        has_face(lichen, from)
            && Direction::ALL.iter().any(|&towards| {
                towards.axis() != from.axis()
                    && !has_face(lichen, towards)
                    && [
                        (pos, towards),
                        (towards.step(pos), from),
                        (from.step(towards.step(pos)), towards.opposite()),
                    ]
                    .into_iter()
                    .any(|(at, face)| lichen_fits(w, lichen, at, face))
            })
    })
}

/// `DefaultSpreaderConfig.canSpreadInto`: the block there is air, lichen without
/// that face, or a water source, and the face has something full to hold on to.
fn lichen_fits(w: &World<'_>, lichen: u32, at: Pos, face: Direction) -> bool {
    let there = w.get(at);
    let free = if state::same_block(there, lichen) {
        !has_face(there, face)
    } else {
        w.blocks.is_air(there)
            || state::is(there, "minecraft:water") && w.blocks.fluid(there).is_source()
    };
    free && face_holds(w, at, face)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::testing::Fixture;
    use crate::pick::BlockHit;

    /// A fixture with a stone floor and the player standing back from the blocks
    /// at x = 2..6, looking down at them.
    fn fixture() -> Option<Fixture> {
        let mut f = Fixture::new()?;
        f.floor();
        f.player.position = [0.5, -60.0, 3.5];
        f.player.y_rot = 0.0;
        f.player.x_rot = 60.0;
        Some(f)
    }

    #[test]
    fn tall_flowers_need_room_above_and_put_their_top_half_there() {
        let Some(mut f) = fixture() else { return };
        for x in 2..=4 {
            f.set([x, -61, 0], "minecraft:grass_block");
        }
        f.set([3, -59, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:sunflower", 4);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(f.get([2, -60, 0]), "minecraft:sunflower[half=lower]");
        assert_eq!(f.get([2, -59, 0]), "minecraft:sunflower[half=upper]");
        assert_eq!(f.player.hand(Hand::Main).count, 3);
        f.click_face([3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -60, 0]), "minecraft:air", "no room above");
        f.click_face([5, -61, 0], Direction::Up);
        assert_eq!(
            f.get([5, -60, 0]),
            "minecraft:air",
            "stone isn't ground for flowers"
        );
        assert_eq!(f.player.hand(Hand::Main).count, 3);
    }

    #[test]
    fn seeds_go_on_farmland_only() {
        let Some(mut f) = fixture() else { return };
        f.set([2, -61, 0], "minecraft:farmland");
        f.hold(Hand::Main, "minecraft:wheat_seeds", 4);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(f.get([2, -60, 0]), "minecraft:wheat[age=0]");
        f.click_face([3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -60, 0]), "minecraft:air");
    }

    #[test]
    fn flower_beds_add_a_flower_per_click_up_to_four() {
        let Some(mut f) = fixture() else { return };
        f.set([2, -61, 0], "minecraft:grass_block");
        f.hold(Hand::Main, "minecraft:pink_petals", 8);
        f.click_face([2, -61, 0], Direction::Up);
        // Yaw 0 faces south, so the petals face north.
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:pink_petals[facing=north,flower_amount=1]"
        );
        for _ in 0..4 {
            f.click_face([2, -60, 0], Direction::Up);
        }
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:pink_petals[facing=north,flower_amount=4]"
        );
        // The fifth click couldn't add one and goes above, onto the petals: not ground.
        assert_eq!(f.get([2, -59, 0]), "minecraft:air");
        assert_eq!(f.player.hand(Hand::Main).count, 4);
    }

    #[test]
    fn kelp_grows_from_its_stem_and_needs_full_water() {
        let Some(mut f) = fixture() else { return };
        for y in -60..=-57 {
            f.set([2, y, 0], "minecraft:water");
        }
        f.hold(Hand::Main, "minecraft:kelp", 4);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(f.get([2, -60, 0]), "minecraft:kelp[age=0]");
        f.click_face([2, -60, 0], Direction::Up);
        assert_eq!(f.get([2, -59, 0]), "minecraft:kelp[age=0]");
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:kelp_plant",
            "the old head became stem"
        );
        // Out of water: refused.
        f.click_face([3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -60, 0]), "minecraft:air");
    }

    #[test]
    fn weeping_vines_hang_and_turn_into_stem_behind_a_new_tip() {
        let Some(mut f) = fixture() else { return };
        f.set([2, -55, 0], "minecraft:netherrack");
        f.player.x_rot = -60.0;
        f.player.position = [0.5, -60.0, 3.5];
        f.hold(Hand::Main, "minecraft:weeping_vines", 4);
        f.click_face([2, -55, 0], Direction::Down);
        assert_eq!(f.get([2, -56, 0]), "minecraft:weeping_vines[age=0]");
        f.click_face([2, -56, 0], Direction::Down);
        assert_eq!(f.get([2, -57, 0]), "minecraft:weeping_vines[age=0]");
        assert_eq!(f.get([2, -56, 0]), "minecraft:weeping_vines_plant");
        // Vines don't stand on the floor.
        f.click_face([4, -61, 0], Direction::Up);
        assert_eq!(f.get([4, -60, 0]), "minecraft:air");
    }

    #[test]
    fn moss_carpets_grow_sides_against_walls() {
        let Some(mut f) = fixture() else { return };
        f.set([3, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:pale_moss_carpet", 4);
        f.click_face([2, -61, 0], Direction::Up);
        assert_eq!(
            f.get([2, -60, 0]),
            "minecraft:pale_moss_carpet[bottom=true,east=low,north=none,south=none,west=none]"
        );
    }

    #[test]
    fn bone_meal_targets() {
        let Some(mut f) = fixture() else { return };
        let cases = [
            ("minecraft:wheat[age=7]", false),
            ("minecraft:wheat[age=3]", true),
            ("minecraft:beetroots[age=3]", false),
            ("minecraft:torchflower_crop[age=1]", true),
            ("minecraft:oak_sapling", false),
            ("minecraft:sweet_berry_bush[age=3]", false),
            ("minecraft:sweet_berry_bush[age=2]", true),
            ("minecraft:cave_vines[berries=true]", false),
            ("minecraft:cave_vines_plant[berries=false]", true),
            ("minecraft:sunflower[half=lower]", true),
        ];
        for (i, (text, expected)) in cases.into_iter().enumerate() {
            let pos = [i as i32 - 5, -60, -5];
            f.set(pos, text);
            let w = f.world();
            assert_eq!(
                super::super::is_valid_bonemeal_target(&w, Fixture::state(text), pos),
                expected,
                "{text}"
            );
        }
        // Grass needs air above.
        f.set([0, -60, 5], "minecraft:grass_block");
        f.set([1, -60, 5], "minecraft:grass_block");
        f.set([1, -59, 5], "minecraft:stone");
        let w = f.world();
        let grass = Fixture::state("minecraft:grass_block");
        assert!(super::super::is_valid_bonemeal_target(
            &w,
            grass,
            [0, -60, 5]
        ));
        assert!(!super::super::is_valid_bonemeal_target(
            &w,
            grass,
            [1, -60, 5]
        ));
    }

    #[test]
    fn short_grass_grows_tall_only_with_room() {
        let Some(mut f) = fixture() else { return };
        f.set([2, -61, 0], "minecraft:grass_block");
        f.set([2, -60, 0], "minecraft:short_grass");
        f.set([3, -61, 0], "minecraft:grass_block");
        f.set([3, -60, 0], "minecraft:short_grass");
        f.set([3, -59, 0], "minecraft:stone");
        let w = f.world();
        let grass = Fixture::state("minecraft:short_grass");
        assert!(super::super::is_valid_bonemeal_target(
            &w,
            grass,
            [2, -60, 0]
        ));
        assert!(!super::super::is_valid_bonemeal_target(
            &w,
            grass,
            [3, -60, 0]
        ));
    }

    #[test]
    fn berries_are_picked_from_ripe_bushes_and_vines() {
        let Some(mut f) = fixture() else { return };
        let hit = |pos: Pos| BlockHit {
            location: [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 1.0,
                f64::from(pos[2]) + 0.5,
            ],
            direction: Direction::Up,
            pos,
            inside: false,
            miss: false,
            world_border: false,
        };
        let cases = [
            ("minecraft:sweet_berry_bush[age=3]", Outcome::Success),
            ("minecraft:sweet_berry_bush[age=1]", Outcome::Pass),
            ("minecraft:cave_vines[berries=true]", Outcome::Success),
            ("minecraft:cave_vines_plant[berries=false]", Outcome::Pass),
        ];
        for (text, expected) in cases {
            let pos = [2, -60, 0];
            f.set(pos, text);
            let s = Fixture::state(text);
            let (blocks, tags) = (f.blocks(), f.tags());
            let mut w = World::new(
                &mut f.level,
                &mut f.predictions,
                blocks,
                tags,
                &f.entities,
                &f.fluid_shapes,
            );
            let got = super::super::use_without_item(&mut w, &mut f.player, s, pos, &hit(pos));
            assert_eq!(got, expected, "{text}");
            assert_eq!(w.get(pos), s, "the server picks them");
        }
        // Bone meal on an unripe bush is the item's to use.
        f.hold(Hand::Main, "minecraft:bone_meal", 1);
        let pos = [2, -60, 0];
        f.set(pos, "minecraft:sweet_berry_bush[age=1]");
        let s = Fixture::state("minecraft:sweet_berry_bush[age=1]");
        let blocks = f.blocks();
        let tags = f.tags();
        let mut w = World::new(
            &mut f.level,
            &mut f.predictions,
            blocks,
            tags,
            &f.entities,
            &f.fluid_shapes,
        );
        let got = super::super::use_item_on(&mut w, &mut f.player, Hand::Main, s, pos, &hit(pos));
        assert_eq!(got, Outcome::Pass);
    }
}
