//! Items' own `useOn` (what an item that isn't a block item does when used on a
//! block), and the data-driven block transformers of axes, shovels and hoes
//! (`Item.useOn` with a `block_transformer` component), as the client runs them.
//!
//! On the client only the level and the hands change: entities an item would make
//! (armor stands, paintings, minecarts, end crystals, mobs from eggs, rockets) are
//! the server's to add, item damage is the server's, and sounds, particles and game
//! events are left out. What the client still does is answer the click (which ends
//! it, or lets it go on to the other hand), change blocks under the action's
//! prediction (fire, lit campfires and candles, waxed copper, mud, grown plant tips,
//! transformed blocks) and shrink the stack where vanilla's client does.

use ruthenium_data::v26_3::registries::{DataComponentType, Potion};
use ruthenium_protocol::item::Component;

use crate::block::{self, Class, Pos, Support, flags, state};
use crate::direction::Direction;
use crate::entity::Entity;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::item::Stack;
use crate::pick::{self, BlockHit};
use crate::place::{PlaceContext, UseOnContext};
use crate::player::LocalPlayer;
use crate::shape::{Aabb, Axis, Placed};

pub use crate::transform::{Transformer, TransformerRef, Transformers};

/// An item class's `useOn` override: the outcome, and whether it counts as the
/// item's use (`wasItemInteraction`; every override here answers `SUCCESS` or
/// `CONSUME`, which both count).
pub fn use_on(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    hand: Hand,
    hit: &BlockHit,
) -> (Outcome, bool) {
    let c = p.hand(hand).c.clone();
    let outcome = if c.is("ArmorStandItem") {
        armor_stand(w, p, hand, hit)
    } else if c.is("BoneMealItem") {
        bone_meal(w, hit)
    } else if c.is("BrushItem") {
        // `calculateHitResult` looks along the view again, entities included, as
        // far as the player reaches blocks; the click came from the crosshair
        // being on this block, which is taken to be the block it finds.
        p.start_using_item(hand);
        Outcome::Success
    } else if c.is("CompassItem") {
        compass(w, p, hand, hit)
    } else if c.is("CushionItem") {
        cushion(w, p, hand, hit)
    } else if c.is("DebugStickItem") {
        // Cycling a property is the server's; the client only answers.
        Outcome::Success
    } else if c.is("EndCrystalItem") {
        end_crystal(w, p, hand, hit)
    } else if c.is("EnderEyeItem") {
        ender_eye(w, hit)
    } else if c.is("FireChargeItem") {
        fire_charge(w, p, hand, hit)
    } else if c.is("FireworkRocketItem") {
        // The rocket is the server's; a gliding player's rocket is `use`'s.
        if p.fall_flying() {
            Outcome::Pass
        } else {
            Outcome::Success
        }
    } else if c.is("FlintAndSteelItem") {
        flint_and_steel(w, p, hit)
    } else if c.is("HangingEntityItem") {
        hanging_entity(w, p, hand, hit)
    } else if c.is("HoneycombItem") {
        honeycomb(w, p, hand, hit)
    } else if c.is("LeadItem") {
        // Tying leashed mobs to a fence is the server's.
        Outcome::Pass
    } else if c.is("MapItem") {
        // Marking a banner on the map is the server's.
        if w.tags.is(w.get(hit.pos), "minecraft:banners") {
            Outcome::Success
        } else {
            transform_block(w, p, hand, hit)
        }
    } else if c.is("MinecartItem") {
        minecart(w, p, hand, hit)
    } else if c.is("PotionItem") {
        potion(w, p, hand, hit)
    } else if c.is("ShearsItem") {
        shears(w, p, hand, hit)
    } else if c.is("SpawnEggItem") {
        spawn_egg(p, hand)
    } else {
        transform_block(w, p, hand, hit)
    };
    (outcome, true)
}

/// `Item.useOn` itself: the item's `block_transformer` (`BlockTransformer.transformBlock`).
/// A main-hand use waits while a blocking item (a shield) is in the off hand, unless
/// the player sneaks. Then the first transform allowed from the clicked face whose
/// provider has a state for the block turns it into that state (fitted to its
/// neighbours, unless the transform says not to); a stackable item is used up by
/// one (unless the transform keeps it, or the player builds without materials),
/// while a tool's wear is the server's. The block is set with `UPDATE_ALL_IMMEDIATE`.
pub fn transform_block(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    hand: Hand,
    hit: &BlockHit,
) -> Outcome {
    let stack = p.hand(hand).clone();
    let Some(r) = &stack.c.block_transformer else {
        return Outcome::Pass;
    };
    let empty = Transformers::default();
    let known = w.transformers.unwrap_or(&empty);
    let Some(t) = known.get(r) else {
        return Outcome::Pass;
    };
    if hand == Hand::Main && p.hand(Hand::Off).c.blocks_attacks && !p.secondary_use() {
        return Outcome::Pass;
    }
    let pos = hit.pos;
    for transform in &t.transforms {
        if transform.disallowed_faces.contains(&hit.direction) {
            continue;
        }
        let Some(new) = transform.provider.optional_state(w, pos) else {
            continue;
        };
        let fitted = if transform.update_from_neighbors {
            w.update_from_neighbour_shapes(new, pos)
        } else {
            new
        };
        if stackable(&stack) {
            let infinite = p.infinite_materials();
            let used = i32::from(transform.consume_on_use);
            p.inventory.hand_mut(hand).consume(used, infinite);
        }
        w.set_block(pos, fitted, flags::ALL_IMMEDIATE);
        return Outcome::Success;
    }
    Outcome::Pass
}

/// `ItemStack.isStackable`: more than one fits, and it isn't a worn tool.
fn stackable(s: &Stack) -> bool {
    let worn = s.c.damageable() && s.c.damage.unwrap_or(0) > 0;
    s.c.max_stack_size() > 1 && !worn
}

/// `ArmorStandItem.useOn`: never on a block's underside. The stand would stand at
/// the bottom centre of the block the click places into (over the clicked block when
/// that may be built over); with no block shape and no entity (the player included,
/// spectators not) in its 0.5 by 1.975 box the stack shrinks by one.
fn armor_stand(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    if hit.direction == Direction::Down {
        return Outcome::Fail;
    }
    let place = PlaceContext::new(w, UseOnContext::new(p, hand, *hit));
    let at = place.clicked_pos();
    let (half, height) = (f64::from(0.5f32) / 2.0, f64::from(1.975f32));
    let (x, y, z) = (
        f64::from(at[0]) + 0.5,
        f64::from(at[1]),
        f64::from(at[2]) + 0.5,
    );
    let stand = Aabb {
        min: [x - half, y, z - half],
        max: [x + half, y + height, z + half],
    };
    if blocks_collide(w, &stand) || anyone_in(w, &stand) {
        return Outcome::Fail;
    }
    p.inventory.hand_mut(hand).shrink(1);
    Outcome::Success
}

/// `BoneMealItem.useOn` on the client: a block that may grow from bone meal takes
/// the click (the growth, and the meal used, are the server's); else a full water
/// block in front of a sturdy clicked face, where the server grows seagrass and
/// coral. Nothing changes on the client either way.
fn bone_meal(w: &World<'_>, hit: &BlockHit) -> Outcome {
    let pos = hit.pos;
    if block::is_valid_bonemeal_target(w, w.get(pos), pos) {
        return Outcome::Success;
    }
    let front = hit.direction.step(pos);
    let full_water = state::is(w.get(front), "minecraft:water") && w.fluid(front).amount == 8;
    if w.face_sturdy(pos, hit.direction, Support::Full) && full_water {
        Outcome::Success
    } else {
        Outcome::Pass
    }
}

/// `CompassItem.useOn`: on a lodestone, a compass that is the only one in the hand
/// (outside creative) becomes a lodestone compass where it is; otherwise one is used
/// up and a new compass goes into the inventory (or would be dropped, which is the
/// server's). Off a lodestone, the plain `Item.useOn`.
///
/// The lodestone it tracks (a component holding the dimension and position) isn't
/// kept on the client's stacks; the server's copy of the slots follows.
fn compass(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    if !state::is(w.get(hit.pos), "minecraft:lodestone") {
        return transform_block(w, p, hand, hit);
    }
    let infinite = p.infinite_materials();
    if infinite || p.hand(hand).count != 1 {
        let tracking = w.new_stack("minecraft:compass", 1);
        p.inventory.hand_mut(hand).consume(1, infinite);
        inventory_add(p, tracking);
    }
    Outcome::Success
}

/// `CushionItem.useOn`: for blocks whose cushions sit on their collision shape
/// (`#cushion_uses_collision_shape`) the click is first traced onto that shape.
/// Only a top face takes a cushion; it would sit centred on the block the click
/// places into, at the click's height, and it needs something under it that isn't
/// buried and room that isn't all suffocating blocks. The cushion, and the item it
/// uses, are the server's.
fn cushion(w: &World<'_>, p: &LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let mut on = UseOnContext::new(p, hand, *hit);
    let clicked = w.get(hit.pos);
    if w.tags.is(clicked, "minecraft:cushion_uses_collision_shape")
        && let Some(shape) = w.blocks.collision(clicked, hit.pos)
    {
        let eye = p.eye_position();
        let ray = [
            hit.location[0] - eye[0],
            hit.location[1] - eye[1],
            hit.location[2] - eye[2],
        ];
        let length = (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
        let past = |i: usize| {
            let unit = if length < 1.0E-5 {
                0.0
            } else {
                ray[i] / length
            };
            hit.location[i] + unit * 0.001
        };
        if let Some(traced) = clip_shape(&shape, eye, [past(0), past(1), past(2)], hit.pos) {
            on.hit = traced;
        }
    }
    if on.hit.direction != Direction::Up {
        return Outcome::Fail;
    }
    let click_y = on.hit.location[1];
    let at = PlaceContext::new(w, on).clicked_pos();
    let (x, z) = (f64::from(at[0]) + 0.5, f64::from(at[2]) + 0.5);
    let spawn = Aabb {
        min: [x - 0.5, click_y, z - 0.5],
        max: [x + 0.5, click_y + f64::from(0.25f32), z + 0.5],
    };
    if cushion_fits(w, &spawn) {
        Outcome::Success
    } else {
        Outcome::Fail
    }
}

/// `Cushion.canBePlacedAt`: a block outline just under the box (within an eighth of
/// a block below its 1/64 thick base), the box not wholly in suffocating blocks, and
/// its base slice not wholly covered by block collisions.
fn cushion_fits(w: &World<'_>, b: &Aabb) -> bool {
    const BASE: f64 = 0.015625;
    let anchor = Aabb {
        min: [b.min[0], b.min[1] - BASE, b.min[2]],
        max: [next_down(b.max[0]), b.min[1], next_down(b.max[2])],
    };
    let search = Aabb {
        min: [anchor.min[0], anchor.min[1] - 0.125, anchor.min[2]],
        max: anchor.max,
    };
    let anchored = cells(&search).any(|pos| {
        w.blocks
            .outline(w.get(pos), pos, "minecraft:air")
            .is_some_and(|s| bounds(&s).intersects(&anchor))
    });
    let smothered = cells(&next_deflated(b)).all(|pos| w.blocks.suffocating(w.get(pos)));
    let slice = next_deflated(&Aabb {
        min: b.min,
        max: [b.max[0], b.min[1] + BASE, b.max[2]],
    });
    anchored && !smothered && !covered(w, &slice)
}

/// Whether block collision boxes cover all of `area` (`Shapes.join(area, collisions,
/// ONLY_FIRST)` coming out empty): the area is cut at every box edge inside it, and
/// every piece must lie in some box.
fn covered(w: &World<'_>, area: &Aabb) -> bool {
    let mut boxes = Vec::new();
    for s in w.level.block_collisions(w.blocks, area) {
        for b in world_boxes(&s) {
            let clipped = Aabb {
                min: std::array::from_fn(|i| b.min[i].max(area.min[i])),
                max: std::array::from_fn(|i| b.max[i].min(area.max[i])),
            };
            if (0..3).all(|i| clipped.min[i] < clipped.max[i]) {
                boxes.push(clipped);
            }
        }
    }
    let cuts: [Vec<f64>; 3] = std::array::from_fn(|i| {
        let mut c = vec![area.min[i], area.max[i]];
        c.extend(boxes.iter().flat_map(|b| [b.min[i], b.max[i]]));
        c.sort_by(f64::total_cmp);
        c.dedup();
        c
    });
    let mids = |i: usize| -> Vec<f64> { cuts[i].windows(2).map(|p| (p[0] + p[1]) / 2.0).collect() };
    let (xs, ys, zs) = (mids(0), mids(1), mids(2));
    xs.iter().all(|&x| {
        ys.iter().all(|&y| {
            zs.iter().all(|&z| {
                let c = [x, y, z];
                boxes
                    .iter()
                    .any(|b| (0..3).all(|i| b.min[i] <= c[i] && c[i] <= b.max[i]))
            })
        })
    })
}

/// `EndCrystalItem.useOn`: only on obsidian or bedrock with air above and no entity
/// (the player included) in the two blocks above; the crystal is the server's, but
/// the stack shrinks by one.
fn end_crystal(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let s = w.get(hit.pos);
    if !state::is(s, "minecraft:obsidian") && !state::is(s, "minecraft:bedrock") {
        return Outcome::Fail;
    }
    let above = Direction::Up.step(hit.pos);
    if !w.is_empty_block(above) {
        return Outcome::Fail;
    }
    let (x, y, z) = (
        f64::from(above[0]),
        f64::from(above[1]),
        f64::from(above[2]),
    );
    let room = Aabb {
        min: [x, y, z],
        max: [x + 1.0, y + 2.0, z + 1.0],
    };
    if anyone_in(w, &room) {
        return Outcome::Fail;
    }
    p.inventory.hand_mut(hand).shrink(1);
    Outcome::Success
}

/// `EnderEyeItem.useOn` on the client: an end portal frame without an eye takes
/// the click (setting the eye, and the portal, are the server's); anything else
/// passes.
fn ender_eye(w: &World<'_>, hit: &BlockHit) -> Outcome {
    let s = w.get(hit.pos);
    if state::is(s, "minecraft:end_portal_frame") && !state::get_bool(s, "eye") {
        Outcome::Success
    } else {
        Outcome::Pass
    }
}

/// `FireChargeItem.useOn`: lights an unlit campfire, candle or candle cake, else
/// sets fire in front of the clicked face where fire may go; either way with
/// `UPDATE_ALL`, and the stack shrinks by one. Nowhere to burn fails.
fn fire_charge(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let s = w.get(hit.pos);
    let lit = if can_light(w, s) {
        w.set_block_and_update(hit.pos, state::set_bool(s, "lit", true));
        true
    } else {
        let front = hit.direction.step(hit.pos);
        let forward = UseOnContext::new(p, hand, *hit).horizontal_direction();
        let fits = fire_can_be_placed_at(w, front, forward);
        if fits {
            let fire = fire_state(w, front);
            w.set_block_and_update(front, fire);
        }
        fits
    };
    if !lit {
        return Outcome::Fail;
    }
    p.inventory.hand_mut(hand).shrink(1);
    Outcome::Success
}

/// `FlintAndSteelItem.useOn`: lights an unlit campfire, candle or candle cake, else
/// sets fire in front of the clicked face where fire may go, both with
/// `UPDATE_ALL_IMMEDIATE`; nowhere to burn fails. Its wear is the server's.
fn flint_and_steel(w: &mut World<'_>, p: &LocalPlayer, hit: &BlockHit) -> Outcome {
    let s = w.get(hit.pos);
    if can_light(w, s) {
        w.set_block(
            hit.pos,
            state::set_bool(s, "lit", true),
            flags::ALL_IMMEDIATE,
        );
        return Outcome::Success;
    }
    let front = hit.direction.step(hit.pos);
    let forward = Direction::from_y_rot(f64::from(p.y_rot));
    if !fire_can_be_placed_at(w, front, forward) {
        return Outcome::Fail;
    }
    let fire = fire_state(w, front);
    w.set_block(front, fire, flags::ALL_IMMEDIATE);
    Outcome::Success
}

/// `CampfireBlock.canLight`, `CandleBlock.canLight` and `CandleCakeBlock.canLight`: an
/// unlit campfire or candle that isn't waterlogged, or an unlit candle cake.
pub fn can_light(w: &World<'_>, s: u32) -> bool {
    let unlit = state::has(s, "lit") && !state::get_bool(s, "lit");
    let dry = state::has(s, "waterlogged") && !state::get_bool(s, "waterlogged");
    let campfire_or_candle =
        w.tags.is(s, "minecraft:campfires") || w.tags.is(s, "minecraft:candles");
    unlit && (campfire_or_candle && dry || w.tags.is(s, "minecraft:candle_cakes"))
}

/// `BaseFireBlock.getState`: soul fire over soul fire's bases
/// (`#soul_fire_base_blocks`); else fire, which over a block that neither burns nor
/// has a sturdy top clings to each neighbour that burns (its `north` ... `up`
/// sides; `FireBlock.getStateForPlacement`).
pub fn fire_state(w: &World<'_>, pos: Pos) -> u32 {
    let below_pos = Direction::Down.step(pos);
    let below = w.get(below_pos);
    if w.tags.is(below, "minecraft:soul_fire_base_blocks") {
        return state::default_of("minecraft:soul_fire");
    }
    let fire = state::default_of("minecraft:fire");
    if burns(w, below) || w.face_sturdy(below_pos, Direction::Up, Support::Full) {
        return fire;
    }
    Direction::ALL
        .into_iter()
        .filter(|&d| d != Direction::Down)
        .fold(fire, |s, d| {
            state::set_bool(s, d.name(), burns(w, w.get(d.step(pos))))
        })
}

/// `FireBlock.canBurn`: the block catches fire (its ignite odds), unless it is
/// waterlogged.
fn burns(w: &World<'_>, s: u32) -> bool {
    let wet = state::has(s, "waterlogged") && state::get_bool(s, "waterlogged");
    !wet && w.blocks.kind(s).ignite_odds > 0
}

/// `BaseFireBlock.canBePlacedAt`: an air block where the fire chosen for it may
/// stay, or that is inside an empty nether portal frame.
///
/// Portal frames count in the overworld and the nether only; the client's world here
/// doesn't say which dimension it is, so they count everywhere.
pub fn fire_can_be_placed_at(w: &World<'_>, pos: Pos, forward: Direction) -> bool {
    if !w.blocks.is_air(w.get(pos)) {
        return false;
    }
    block::can_survive(w, fire_state(w, pos), pos) || in_portal_frame(w, pos, forward)
}

/// `BaseFireBlock.isPortal`: next to a portal frame block, and an empty portal frame
/// is found around `pos`, across the player's view first.
fn in_portal_frame(w: &World<'_>, pos: Pos, forward: Direction) -> bool {
    let frame_near = Direction::ALL
        .into_iter()
        .any(|d| portal_frame(w, w.get(d.step(pos))));
    if !frame_near {
        return false;
    }
    let first = forward.counter_clockwise().axis();
    let other = if first == Axis::X { Axis::Z } else { Axis::X };
    [first, other]
        .into_iter()
        .any(|axis| empty_portal(w, pos, axis))
}

fn portal_frame(w: &World<'_>, s: u32) -> bool {
    w.tags.is(s, "minecraft:nether_portal_frame")
}

/// What a portal's inside may hold before it lights: air, fire, portal.
fn portal_space(w: &World<'_>, s: u32) -> bool {
    w.blocks.is_air(s) || w.tags.is(s, "minecraft:fire") || state::is(s, "minecraft:nether_portal")
}

/// `pos` moved `n` blocks towards `d`.
fn moved(pos: Pos, d: Direction, n: i32) -> Pos {
    let s = d.normal();
    [pos[0] + s[0] * n, pos[1] + s[1] * n, pos[2] + s[2] * n]
}

/// `PortalShape.findAnyShape` along `axis`, kept only when it is a valid frame with
/// no portal blocks yet (`findEmptyPortalShape` for one axis). The inside is found
/// from `pos` down to the frame's floor (at most 21 down) and out to the side; it
/// must be 2 to 21 wide and 3 to 21 tall, framed on both sides of every row and
/// along the top.
fn empty_portal(w: &World<'_>, pos: Pos, axis: Axis) -> bool {
    let right = if axis == Axis::X {
        Direction::West
    } else {
        Direction::South
    };
    let left = right.opposite();
    let lowest = w.min_y().max(pos[1] - 21);
    let mut base = pos;
    while base[1] > lowest && portal_space(w, w.get(Direction::Down.step(base))) {
        base = Direction::Down.step(base);
    }
    let Some(edge) = run_above_frame(w, base, left).checked_sub(1) else {
        return false;
    };
    let corner = moved(base, left, edge);
    let width = run_above_frame(w, corner, right);
    if !(2..=21).contains(&width) {
        return false;
    }
    let mut portals = 0;
    let mut height = 21;
    'rows: for h in 0..21 {
        let row = moved(corner, Direction::Up, h);
        let sides = [moved(row, right, -1), moved(row, right, width)];
        if !sides.iter().all(|&s| portal_frame(w, w.get(s))) {
            height = h;
            break;
        }
        for i in 0..width {
            let s = w.get(moved(row, right, i));
            if !portal_space(w, s) {
                height = h;
                break 'rows;
            }
            if state::is(s, "minecraft:nether_portal") {
                portals += 1;
            }
        }
    }
    let top = moved(corner, Direction::Up, height);
    let roofed = (0..width).all(|i| portal_frame(w, w.get(moved(top, right, i))));
    (3..=21).contains(&height) && roofed && portals == 0
}

/// `PortalShape.getDistanceUntilEdgeAboveFrame`: going towards `d` over frame
/// blocks through portal space, how far the first frame block across the way is
/// (0 when the way ends otherwise, or runs past 21).
fn run_above_frame(w: &World<'_>, from: Pos, d: Direction) -> i32 {
    for i in 0..=21 {
        let at = moved(from, d, i);
        let s = w.get(at);
        if !portal_space(w, s) {
            return if portal_frame(w, s) { i } else { 0 };
        }
        if !portal_frame(w, w.get(Direction::Down.step(at))) {
            return 0;
        }
    }
    0
}

/// `HangingEntityItem.useOn` (paintings, item frames): only from a side face (an
/// item frame also from the top or bottom, within the build height) where the player
/// may use the item. The entity would hang in front of the clicked face; where it
/// would stay (`survives`) the stack shrinks by one, else the click is used up all
/// the same.
///
/// A painting picks the largest of the placeable variants that fits, at random; the
/// variants (a synced registry) aren't known here, and vanilla's placeable ones
/// include 1 by 1 paintings, so a painting counts as fitting where a 1 by 1 one does
/// (an item naming its variant is taken the same way). The other hanging entities'
/// boxes are the client's boxes for them, and their facing is read from their
/// rotation.
fn hanging_entity(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let stack = p.hand(hand).clone();
    let face = hit.direction;
    let at = face.step(hit.pos);
    let may_use = p.abilities.may_build
        || crate::mining::can_place_on_in_adventure(&stack, w.get(hit.pos), w.tags);
    let side_ok = if stack.c.is("ItemFrameItem") {
        w.inside_build_height(at[1])
    } else {
        face.is_horizontal()
    };
    if !(side_ok && may_use) {
        return Outcome::Fail;
    }
    let stays = match stack.c.name {
        "minecraft:painting" => painting_survives(w, at, face, 1, 1),
        "minecraft:item_frame" | "minecraft:glow_item_frame" => frame_survives(w, at, face),
        _ => return Outcome::Success,
    };
    if stays {
        p.inventory.hand_mut(hand).shrink(1);
    }
    Outcome::Success
}

/// `Painting.calculateBoundingBox`: a sixteenth thick against the wall behind
/// `pos`, `width` by `height` blocks, shifted half a block left and up for even
/// sizes.
fn painting_box(pos: Pos, face: Direction, width: i32, height: i32) -> Aabb {
    let shift = |n: i32| if n % 2 == 0 { 0.5 } else { 0.0 };
    let back = face.normal();
    let left = face.counter_clockwise().normal();
    let centre: [f64; 3] = std::array::from_fn(|i| {
        f64::from(pos[i]) + 0.5 - f64::from(back[i]) * 0.46875
            + f64::from(left[i]) * shift(width)
            + if i == 1 { shift(height) } else { 0.0 }
    });
    let size = [
        if face.axis() == Axis::X {
            0.0625
        } else {
            f64::from(width)
        },
        f64::from(height),
        if face.axis() == Axis::Z {
            0.0625
        } else {
            f64::from(width)
        },
    ];
    of_size(centre, size)
}

/// `HangingEntity.survives` for a painting: no block collision in its box, every
/// block behind it solid (or a repeater or comparator), and no other painting, nor
/// any hanging entity facing the same way, overlapping it.
fn painting_survives(w: &World<'_>, pos: Pos, face: Direction, width: i32, height: i32) -> bool {
    let b = painting_box(pos, face, width, height);
    if blocks_collide(w, &b) {
        return false;
    }
    let n = face.normal();
    let behind = b.moved([
        -f64::from(n[0]) * 0.5,
        -f64::from(n[1]) * 0.5,
        -f64::from(n[2]) * 0.5,
    ]);
    let support = behind.inflate([-1.0E-7; 3]);
    let supported = cells(&support).all(|c| {
        let s = w.get(c);
        w.blocks.solid(s) || w.blocks.is_class(s, Class::DiodeBlock)
    });
    supported && !hanging_in_the_way(w, &b, face, Some("minecraft:painting"))
}

/// `ItemFrame.survives`: no block collision in its 3/4 block box, a solid block
/// behind it (or, on a wall, a repeater or comparator), and no hanging entity facing
/// the same way overlapping it.
fn frame_survives(w: &World<'_>, pos: Pos, face: Direction) -> bool {
    let n = face.normal();
    let centre: [f64; 3] =
        std::array::from_fn(|i| f64::from(pos[i]) + 0.5 - f64::from(n[i]) * 0.46875);
    let size: [f64; 3] = std::array::from_fn(|i| {
        if n[i] != 0 {
            0.0625
        } else {
            f64::from(0.75f32)
        }
    });
    let b = of_size(centre, size);
    if blocks_collide(w, &b) {
        return false;
    }
    let behind = w.get(face.opposite().step(pos));
    let held = w.blocks.solid(behind)
        || face.is_horizontal() && w.blocks.is_class(behind, Class::DiodeBlock);
    held && !hanging_in_the_way(w, &b, face, None)
}

/// `HangingEntity.canCoexist`: another hanging entity overlapping `b` facing the
/// same way, or (when `same_type` is given) of that type.
fn hanging_in_the_way(w: &World<'_>, b: &Aabb, face: Direction, same_type: Option<&str>) -> bool {
    const HANGING: [&str; 3] = [
        "minecraft:painting",
        "minecraft:item_frame",
        "minecraft:glow_item_frame",
    ];
    entities_in(w, b).any(|e| {
        let key = e.info().key.as_str();
        HANGING.contains(&key) && (same_type == Some(key) || hanging_facing(e) == face)
    })
}

/// A hanging entity's facing from its rotation (an item frame on a floor or ceiling
/// looks up or down).
fn hanging_facing(e: &Entity) -> Direction {
    if e.x_rot <= -45.0 {
        Direction::Up
    } else if e.x_rot >= 45.0 {
        Direction::Down
    } else {
        Direction::from_y_rot(f64::from(e.y_rot))
    }
}

/// `HoneycombItem.useOn`: a copper block that may be waxed becomes its waxed
/// counterpart (keeping its properties) with `UPDATE_ALL_IMMEDIATE`, and the stack
/// shrinks by one; other blocks pass. Only the clicked half of a double copper
/// chest is set.
fn honeycomb(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let Some(waxed) = waxed(w.get(hit.pos)) else {
        return Outcome::Pass;
    };
    p.inventory.hand_mut(hand).shrink(1);
    w.set_block(hit.pos, waxed, flags::ALL_IMMEDIATE);
    Outcome::Success
}

/// `HoneycombItem.getWaxed`: every unwaxed stage of the weathering copper blocks has
/// a `waxed_` counterpart, and nothing else does.
pub fn waxed(s: u32) -> Option<u32> {
    let path = state::name(s).strip_prefix("minecraft:")?;
    if path.starts_with("waxed_") {
        return None;
    }
    let block = state::block_named(&format!("minecraft:waxed_{path}"))?;
    Some(state::with_properties_of(state::default_of_block(block), s))
}

/// `MinecartItem.useOn`: only on rails; the cart is the server's, and the stack
/// shrinks by one.
///
/// With the minecart improvements experiment a cart already where the new one would
/// sit refuses it; the client's world here doesn't say whether the experiment is on,
/// so that check is left out.
fn minecart(w: &World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    if !w.tags.is(w.get(hit.pos), "minecraft:rails") {
        return Outcome::Fail;
    }
    p.inventory.hand_mut(hand).shrink(1);
    Outcome::Success
}

/// `PotionItem.useOn`: a water bottle on a block that turns to mud
/// (`#convertible_to_mud`), from any face but the bottom, empties into a glass
/// bottle (`ItemUtils.createFilledResult`) and the block becomes mud with
/// `UPDATE_ALL`.
fn potion(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let stack = p.hand(hand).clone();
    let water = Potion::Water.id() as i32;
    let is_water =
        stack.patch.added.iter().any(
            |(_, c)| matches!(c, Component::PotionContents { potion: Some(id) } if *id == water),
        );
    let s = w.get(hit.pos);
    if hit.direction == Direction::Down
        || !w.tags.is(s, "minecraft:convertible_to_mud")
        || !is_water
    {
        return Outcome::Pass;
    }
    let bottle = w.new_stack("minecraft:glass_bottle", 1);
    filled_result(p, hand, bottle);
    w.set_block_and_update(hit.pos, state::default_of("minecraft:mud"));
    Outcome::Success
}

/// `ItemUtils.createFilledResult` (with creative's stack limit): a player who builds
/// without materials keeps the stack and gets the new item unless they have one
/// already; else one of the stack is used and the new item replaces it if it was the
/// last, or goes into the inventory.
fn filled_result(p: &mut LocalPlayer, hand: Hand, made: Stack) {
    if p.infinite_materials() {
        let has = (0..crate::inventory::SIZE).any(|i| {
            let s = p.inventory.get(i);
            !s.is_empty() && s.same_item_same_components(&made)
        });
        if !has {
            inventory_add(p, made);
        }
        return;
    }
    p.inventory.hand_mut(hand).consume(1, false);
    if p.hand(hand).is_empty() {
        p.inventory.set_hand(hand, made);
    } else {
        inventory_add(p, made);
    }
}

/// `Inventory.add` of an undamaged stack: onto stacks of the same item and
/// components with room (the selected slot first, then the off hand, then the main
/// slots in order), then into empty main slots, until it is all in. What doesn't fit
/// would be dropped, which is the server's; a player building without materials
/// loses it.
fn inventory_add(p: &mut LocalPlayer, mut stack: Stack) {
    let selected = usize::try_from(p.inventory.selected).unwrap_or(0).min(8);
    let max = stack.c.max_stack_size();
    while stack.count > 0 {
        let has_room = |s: &Stack| {
            !s.is_empty()
                && s.same_item_same_components(&stack)
                && stackable(s)
                && s.count < s.c.max_stack_size()
        };
        let inv = &p.inventory;
        let slot = [selected, crate::inventory::SLOT_OFFHAND]
            .into_iter()
            .chain(0..36)
            .find(|&i| has_room(inv.get(i)))
            .or_else(|| (0..36).find(|&i| inv.get(i).is_empty()));
        let Some(slot) = slot else {
            return;
        };
        let target = p.inventory.get(slot);
        let room = if target.is_empty() {
            max
        } else {
            target.c.max_stack_size() - target.count
        };
        let moving = stack.count.min(room);
        if moving <= 0 {
            return;
        }
        if target.is_empty() {
            let mut placed = stack.copy();
            placed.count = moving;
            p.inventory.set(slot, placed);
        } else if let Some(t) = p.inventory.get_mut(slot) {
            t.count += moving;
        }
        stack.count -= moving;
    }
}

/// `ShearsItem.useOn`: the growing tip of kelp or vines that may still grow stops
/// growing (its `age` goes to the last, 25) with `UPDATE_ALL`; their wear is the
/// server's. Other blocks get the plain `Item.useOn`.
fn shears(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let s = w.get(hit.pos);
    if w.blocks.is_class(s, Class::GrowingPlantHeadBlock) && state::get_int(s, "age") != 25 {
        w.set_block_and_update(hit.pos, state::set_int(s, "age", 25));
        return Outcome::Success;
    }
    transform_block(w, p, hand, hit)
}

/// `SpawnEggItem.useOn` on the client: an egg that names an entity takes the click;
/// the mob, and the egg used, are the server's.
///
/// Vanilla also refuses an entity type its features don't enable, and monsters on
/// peaceful; neither is known here, so those eggs take the click too.
fn spawn_egg(p: &LocalPlayer, hand: Hand) -> Outcome {
    let entity_data = DataComponentType::EntityData.id() as i32;
    if p.hand(hand).patch.removed.contains(&entity_data) {
        Outcome::Fail
    } else {
        Outcome::Success
    }
}

/// Some block's collision shape overlaps `b` (`noBlockCollision` failing; the world
/// border isn't checked).
fn blocks_collide(w: &World<'_>, b: &Aabb) -> bool {
    w.level
        .block_collisions(w.blocks, b)
        .iter()
        .any(|s| s.overlaps(b))
}

/// The entities `getEntities(null, box)` finds: the level's (not removed, not
/// spectating players) whose boxes touch `b`.
fn entities_in<'w>(w: &'w World<'_>, b: &Aabb) -> impl Iterator<Item = &'w Entity> + 'w {
    let players = w.players;
    w.entities.in_box(b).into_iter().filter(move |e| {
        !e.removed && !players.is_some_and(|p| p.get(e.uuid).is_some_and(|i| i.game_mode == 3))
    })
}

/// Any entity in `b`, the local player included.
fn anyone_in(w: &World<'_>, b: &Aabb) -> bool {
    w.player_box.is_some_and(|p| p.intersects(b)) || entities_in(w, b).next().is_some()
}

/// The block positions a box covers (`BlockPos.betweenClosed` of its floored corners).
fn cells(b: &Aabb) -> impl Iterator<Item = Pos> {
    let lo: [i32; 3] = std::array::from_fn(|i| b.min[i].floor() as i32);
    let hi: [i32; 3] = std::array::from_fn(|i| b.max[i].floor() as i32);
    (lo[0]..=hi[0]).flat_map(move |x| {
        (lo[1]..=hi[1]).flat_map(move |y| (lo[2]..=hi[2]).map(move |z| [x, y, z]))
    })
}

/// `AABB.ofSize`.
fn of_size(centre: [f64; 3], size: [f64; 3]) -> Aabb {
    Aabb {
        min: std::array::from_fn(|i| centre[i] - size[i] / 2.0),
        max: std::array::from_fn(|i| centre[i] + size[i] / 2.0),
    }
}

/// `AABB.nextDeflated`: one representable step inwards on every side.
fn next_deflated(b: &Aabb) -> Aabb {
    Aabb {
        min: b.min.map(next_up),
        max: b.max.map(next_down),
    }
}

/// `Math.nextUp`.
fn next_up(v: f64) -> f64 {
    if v.is_nan() || v == f64::INFINITY {
        v
    } else if v == 0.0 {
        f64::from_bits(1)
    } else if v > 0.0 {
        f64::from_bits(v.to_bits() + 1)
    } else {
        f64::from_bits(v.to_bits() - 1)
    }
}

/// `Math.nextDown`.
fn next_down(v: f64) -> f64 {
    -next_up(-v)
}

/// A placed shape's bounds in the world.
fn bounds(s: &Placed<'_>) -> Aabb {
    let last = |a: Axis| s.get(a, s.shape.size(a));
    Aabb {
        min: [s.get(Axis::X, 0), s.get(Axis::Y, 0), s.get(Axis::Z, 0)],
        max: [last(Axis::X), last(Axis::Y), last(Axis::Z)],
    }
}

/// A placed shape's boxes in the world.
fn world_boxes(s: &Placed<'_>) -> Vec<Aabb> {
    s.shape
        .boxes()
        .into_iter()
        .map(|b| Aabb {
            min: [
                s.get(Axis::X, b[0]),
                s.get(Axis::Y, b[1]),
                s.get(Axis::Z, b[2]),
            ],
            max: [
                s.get(Axis::X, b[3]),
                s.get(Axis::Y, b[4]),
                s.get(Axis::Z, b[5]),
            ],
        })
        .collect()
}

/// `VoxelShape.clip` of a block's shape: a segment starting in one of the shape's
/// cells (tested a thousandth of the way along) hits there, inside, facing back
/// along it; else the nearest box face it enters.
fn clip_shape(s: &Placed<'_>, from: [f64; 3], to: [f64; 3], pos: Pos) -> Option<BlockHit> {
    let d: [f64; 3] = std::array::from_fn(|i| to[i] - from[i]);
    if d.iter().map(|v| v * v).sum::<f64>() < 1.0E-7 {
        return None;
    }
    let test: [f64; 3] = std::array::from_fn(|i| from[i] + d[i] * 0.001);
    let cell = |a: Axis| s.find_index_local(a, test[a as usize] - f64::from(pos[a as usize]));
    if s.shape
        .is_full_at(cell(Axis::X), cell(Axis::Y), cell(Axis::Z))
    {
        return Some(BlockHit {
            location: test,
            direction: Direction::approximate_nearest(d).opposite(),
            pos,
            inside: true,
            miss: false,
        });
    }
    let mut best: Option<(f64, BlockHit)> = None;
    for b in world_boxes(s) {
        let Some(location) = pick::clip_box(&b, from, to) else {
            continue;
        };
        let along = (0..3).map(|i| (location[i] - from[i]) * d[i]).sum::<f64>();
        if best.as_ref().is_some_and(|(t, _)| *t <= along) {
            continue;
        }
        // The face entered is the one on the axis the segment reaches last.
        let mut entered = None;
        let mut latest = f64::NEG_INFINITY;
        for (i, axis) in [Axis::X, Axis::Y, Axis::Z].into_iter().enumerate() {
            if d[i] == 0.0 {
                continue;
            }
            let plane = if d[i] > 0.0 { b.min[i] } else { b.max[i] };
            let t = (plane - from[i]) / d[i];
            if t > latest {
                latest = t;
                entered = Some(Direction::from_axis(axis, d[i] < 0.0));
            }
        }
        if let Some(direction) = entered {
            best = Some((
                along,
                BlockHit {
                    location,
                    direction,
                    pos,
                    inside: false,
                    miss: false,
                },
            ));
        }
    }
    best.map(|(_, h)| h)
}

#[cfg(test)]
mod tests {
    use ruthenium_protocol::item::ComponentPatch;
    use ruthenium_protocol::nbt::{Compound, Tag};
    use ruthenium_protocol::packets::configuration::RegistryEntry;

    use super::*;
    use crate::block::testing::Fixture;

    /// Fire as it is placed over a sturdy block, or with nothing that burns around.
    const NO_SIDES: &str =
        "minecraft:fire[age=0,east=false,north=false,south=false,up=false,west=false]";

    /// Vanilla's transformers from the prepared game's jar.
    fn jar_transformers() -> Option<Transformers> {
        let dir = std::path::PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?);
        let jar = ruthenium_assets::zip::Zip::open(&dir.join("versions/26.3/client.jar")).ok()?;
        let files: Vec<(String, Vec<u8>)> = jar
            .with_prefix("data/minecraft/block_transformer/")
            .map(|i| (jar.name(i).to_owned(), jar.read(i).unwrap().into_owned()))
            .collect();
        Some(Transformers::from_json_files(files))
    }

    /// A click in the middle of a face.
    fn hit(pos: Pos, face: Direction) -> BlockHit {
        let n = face.normal();
        BlockHit {
            location: std::array::from_fn(|i| f64::from(pos[i]) + 0.5 + f64::from(n[i]) * 0.5),
            direction: face,
            pos,
            inside: false,
            miss: false,
        }
    }

    /// The fixture's world, with transformers.
    fn world<'f>(
        f: &'f mut Fixture,
        t: Option<&'f Transformers>,
    ) -> (World<'f>, &'f mut LocalPlayer) {
        let (blocks, tags) = (f.blocks(), f.tags());
        let mut w = World::new(
            &mut f.level,
            &mut f.predictions,
            blocks,
            tags,
            &f.entities,
            &f.fluid_shapes,
        );
        w.transformers = t;
        w.track_player(&f.player);
        (w, &mut f.player)
    }

    fn use_item(f: &mut Fixture, pos: Pos, face: Direction) -> Outcome {
        let (mut w, p) = world(f, None);
        use_on(&mut w, p, Hand::Main, &hit(pos, face)).0
    }

    fn transform(f: &mut Fixture, t: &Transformers, pos: Pos, face: Direction) -> Outcome {
        let (mut w, p) = world(f, Some(t));
        transform_block(&mut w, p, Hand::Main, &hit(pos, face))
    }

    #[test]
    fn flint_and_steel_sets_fire_on_the_clicked_face() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.position = [0.5, -60.0, 3.5];
        f.player.x_rot = 60.0;
        f.player.y_rot = 180.0;
        f.hold(Hand::Main, "minecraft:flint_and_steel", 1);
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(f.get([0, -60, 0]), NO_SIDES);
        assert_eq!(
            f.player.hand(Hand::Main).count,
            1,
            "its wear is the server's"
        );
        // Fire only goes into air.
        assert_eq!(use_item(&mut f, [0, -61, 0], Direction::Up), Outcome::Fail);
    }

    #[test]
    fn fire_clings_to_what_burns_and_soul_fire_to_soul_soil() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.hold(Hand::Main, "minecraft:flint_and_steel", 1);
        f.set([1, -50, 0], "minecraft:oak_planks");
        assert_eq!(
            use_item(&mut f, [1, -50, 0], Direction::West),
            Outcome::Success
        );
        assert_eq!(
            f.get([0, -50, 0]),
            "minecraft:fire[age=0,east=true,north=false,south=false,up=false,west=false]"
        );
        // Stone doesn't burn, and nothing holds fire up.
        f.set([1, -40, 0], "minecraft:stone");
        assert_eq!(
            use_item(&mut f, [1, -40, 0], Direction::West),
            Outcome::Fail
        );
        assert_eq!(f.get([0, -40, 0]), "minecraft:air");
        f.set([5, -61, 5], "minecraft:soul_soil");
        use_item(&mut f, [5, -61, 5], Direction::Up);
        assert_eq!(f.get([5, -60, 5]), "minecraft:soul_fire");
    }

    #[test]
    fn flint_and_steel_lights_campfires_and_fire_charges_are_used_up() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.set([0, -61, 0], "minecraft:campfire[lit=false]");
        f.hold(Hand::Main, "minecraft:flint_and_steel", 1);
        assert_eq!(
            use_item(&mut f, [0, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert!(f.get([0, -61, 0]).contains("lit=true"));
        f.set([2, -61, 0], "minecraft:candle[lit=false,waterlogged=true]");
        f.set([2, -62, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:fire_charge", 3);
        // A wet candle can't be lit, and fire above it would have nothing to stay on.
        assert_eq!(use_item(&mut f, [2, -61, 0], Direction::Up), Outcome::Fail);
        f.set([4, -61, 0], "minecraft:candle_cake[lit=false]");
        assert_eq!(
            use_item(&mut f, [4, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert!(f.get([4, -61, 0]).contains("lit=true"));
        assert_eq!(f.player.hand(Hand::Main).count, 2);
    }

    #[test]
    fn honeycomb_waxes_copper_keeping_its_shape() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.hold(Hand::Main, "minecraft:honeycomb", 2);
        f.set(
            [0, -61, 0],
            "minecraft:exposed_cut_copper_stairs[facing=east,half=top]",
        );
        assert_eq!(
            use_item(&mut f, [0, -61, 0], Direction::Up),
            Outcome::Success
        );
        let waxed = f.get([0, -61, 0]);
        assert!(
            waxed.starts_with("minecraft:waxed_exposed_cut_copper_stairs["),
            "{waxed}"
        );
        assert!(
            waxed.contains("facing=east") && waxed.contains("half=top"),
            "{waxed}"
        );
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        // Waxed copper, and stone, pass.
        assert_eq!(use_item(&mut f, [0, -61, 0], Direction::Up), Outcome::Pass);
        f.set([1, -61, 0], "minecraft:stone");
        assert_eq!(use_item(&mut f, [1, -61, 0], Direction::Up), Outcome::Pass);
        assert_eq!(f.player.hand(Hand::Main).count, 1);
    }

    #[test]
    fn a_water_bottle_turns_dirt_to_mud() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.set([0, -61, 0], "minecraft:dirt");
        let mut water = f.stack("minecraft:potion", 1);
        water.patch = std::sync::Arc::new(ComponentPatch {
            added: vec![(
                DataComponentType::PotionContents.id() as i32,
                Component::PotionContents {
                    potion: Some(Potion::Water.id() as i32),
                },
            )],
            removed: Vec::new(),
        });
        f.player.inventory.set_hand(Hand::Main, water.clone());
        assert_eq!(
            use_item(&mut f, [0, -61, 0], Direction::Down),
            Outcome::Pass
        );
        // The whole click, for the world's items (the glass bottle).
        f.player.position = [0.5, -60.0, 2.5];
        f.player.x_rot = 60.0;
        f.player.y_rot = 180.0;
        f.click_face([0, -61, 0], Direction::Up);
        assert_eq!(f.get([0, -61, 0]), "minecraft:mud");
        assert_eq!(f.player.hand(Hand::Main).c.name, "minecraft:glass_bottle");
        // A potion that isn't water does nothing.
        f.set([1, -61, 0], "minecraft:dirt");
        f.hold(Hand::Main, "minecraft:potion", 1);
        assert_eq!(use_item(&mut f, [1, -61, 0], Direction::Up), Outcome::Pass);
    }

    #[test]
    fn shears_stop_a_plant_tip_growing() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.hold(Hand::Main, "minecraft:shears", 1);
        f.set([0, -50, 0], "minecraft:weeping_vines[age=3]");
        f.set([0, -49, 0], "minecraft:stone");
        assert_eq!(
            use_item(&mut f, [0, -50, 0], Direction::North),
            Outcome::Success
        );
        assert_eq!(f.get([0, -50, 0]), "minecraft:weeping_vines[age=25]");
        assert_eq!(
            use_item(&mut f, [0, -50, 0], Direction::North),
            Outcome::Pass
        );
    }

    #[test]
    fn hanging_items_shrink_only_where_they_would_hang() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.set([2, -60, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:painting", 4);
        assert_eq!(
            use_item(&mut f, [2, -60, 0], Direction::Up),
            Outcome::Fail,
            "not on a top"
        );
        assert_eq!(
            use_item(&mut f, [2, -60, 0], Direction::West),
            Outcome::Success
        );
        assert_eq!(f.player.hand(Hand::Main).count, 3);
        // Against grass there is nothing solid to hang on: the click is used all the
        // same.
        f.set([2, -59, 3], "minecraft:short_grass");
        assert_eq!(
            use_item(&mut f, [2, -59, 3], Direction::West),
            Outcome::Success
        );
        assert_eq!(f.player.hand(Hand::Main).count, 3);
        f.hold(Hand::Main, "minecraft:item_frame", 2);
        assert_eq!(
            use_item(&mut f, [0, -61, 3], Direction::Up),
            Outcome::Success
        );
        assert_eq!(f.player.hand(Hand::Main).count, 1);
    }

    #[test]
    fn armor_stands_and_crystals_need_room() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.hold(Hand::Main, "minecraft:armor_stand", 2);
        assert_eq!(
            use_item(&mut f, [0, -61, 0], Direction::Up),
            Outcome::Fail,
            "the player is there"
        );
        assert_eq!(
            use_item(&mut f, [3, -61, 3], Direction::Up),
            Outcome::Success
        );
        assert_eq!(f.player.hand(Hand::Main).count, 1);
        assert_eq!(
            use_item(&mut f, [3, -61, 3], Direction::Down),
            Outcome::Fail
        );
        f.hold(Hand::Main, "minecraft:end_crystal", 1);
        assert_eq!(
            use_item(&mut f, [3, -61, 3], Direction::Up),
            Outcome::Fail,
            "stone"
        );
        f.set([3, -61, 3], "minecraft:obsidian");
        assert_eq!(
            use_item(&mut f, [3, -61, 3], Direction::Up),
            Outcome::Success
        );
        assert!(f.player.hand(Hand::Main).is_empty());
    }

    #[test]
    fn items_that_only_answer() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.set([0, -61, 0], "minecraft:end_portal_frame");
        f.hold(Hand::Main, "minecraft:ender_eye", 1);
        assert_eq!(
            use_item(&mut f, [0, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert_eq!(
            f.get([0, -61, 0]),
            "minecraft:end_portal_frame[eye=false,facing=north]"
        );
        f.set([1, -61, 0], "minecraft:water");
        f.set([2, -61, 0], "minecraft:stone");
        f.hold(Hand::Main, "minecraft:bone_meal", 5);
        assert_eq!(
            use_item(&mut f, [2, -61, 0], Direction::West),
            Outcome::Success
        );
        assert_eq!(use_item(&mut f, [2, -61, 0], Direction::Up), Outcome::Pass);
        assert_eq!(
            f.player.hand(Hand::Main).count,
            5,
            "the meal used is the server's"
        );
        f.hold(Hand::Main, "minecraft:pig_spawn_egg", 1);
        assert_eq!(
            use_item(&mut f, [2, -61, 0], Direction::Up),
            Outcome::Success
        );
        f.hold(Hand::Main, "minecraft:lead", 1);
        assert_eq!(use_item(&mut f, [2, -61, 0], Direction::Up), Outcome::Pass);
        f.set([3, -61, 0], "minecraft:rail");
        f.hold(Hand::Main, "minecraft:minecart", 1);
        assert_eq!(use_item(&mut f, [2, -61, 0], Direction::Up), Outcome::Fail);
        assert_eq!(
            use_item(&mut f, [3, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert!(f.player.hand(Hand::Main).is_empty());
    }

    #[test]
    fn nether_portal_frames_take_fire_where_it_wouldnt_stay() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        // A 2 wide, 3 tall frame along x, at z = 0, its floor at y = -51.
        for x in -1..=2 {
            f.set([x, -51, 0], "minecraft:obsidian");
            f.set([x, -47, 0], "minecraft:obsidian");
        }
        for y in -50..=-48 {
            f.set([-1, y, 0], "minecraft:obsidian");
            f.set([2, y, 0], "minecraft:obsidian");
        }
        f.hold(Hand::Main, "minecraft:flint_and_steel", 1);
        // Into the top of the inside from the frame's side: no floor under it.
        f.player.y_rot = -90.0;
        assert_eq!(
            use_item(&mut f, [-1, -48, 0], Direction::East),
            Outcome::Success
        );
        assert_eq!(f.get([0, -48, 0]), NO_SIDES);
        // The same spot without the frame's roof: fire wouldn't stay.
        for x in -1..=2 {
            f.set([x, -47, 0], "minecraft:air");
        }
        f.set([0, -48, 0], "minecraft:air");
        assert_eq!(
            use_item(&mut f, [-1, -48, 0], Direction::East),
            Outcome::Fail
        );
    }

    #[test]
    fn vanilla_transformers_read_from_the_jar() {
        let Some(t) = jar_transformers() else {
            return;
        };
        assert_eq!(t.len(), 3);
        let axe = t.named("minecraft:axe").unwrap();
        assert!(axe.transforms.len() >= 3, "strip, scrape, unwax");
        let shovel = t.named("minecraft:shovel").unwrap();
        assert_eq!(shovel.transforms[0].disallowed_faces, vec![Direction::Down]);
    }

    #[test]
    fn shovels_make_paths_and_hoes_till() {
        let (Some(mut f), Some(t)) = (Fixture::new(), jar_transformers()) else {
            return;
        };
        f.set([0, -61, 0], "minecraft:grass_block");
        f.hold(Hand::Main, "minecraft:wooden_shovel", 1);
        assert_eq!(
            transform(&mut f, &t, [0, -61, 0], Direction::Down),
            Outcome::Pass
        );
        assert_eq!(
            transform(&mut f, &t, [0, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert_eq!(f.get([0, -61, 0]), "minecraft:dirt_path");
        // Not under a block.
        f.set([1, -61, 0], "minecraft:dirt");
        f.set([1, -60, 0], "minecraft:stone");
        assert_eq!(
            transform(&mut f, &t, [1, -61, 0], Direction::Up),
            Outcome::Pass
        );
        f.hold(Hand::Main, "minecraft:wooden_hoe", 1);
        f.set([2, -61, 0], "minecraft:dirt");
        assert_eq!(
            transform(&mut f, &t, [2, -61, 0], Direction::North),
            Outcome::Success
        );
        assert_eq!(f.get([2, -61, 0]), "minecraft:farmland[moisture=0]");
        f.set([3, -61, 0], "minecraft:coarse_dirt");
        transform(&mut f, &t, [3, -61, 0], Direction::Up);
        assert_eq!(f.get([3, -61, 0]), "minecraft:dirt");
        // Rooted dirt from below too.
        f.set([4, -61, 0], "minecraft:rooted_dirt");
        transform(&mut f, &t, [4, -61, 0], Direction::Down);
        assert_eq!(f.get([4, -61, 0]), "minecraft:dirt");
        assert_eq!(
            f.player.hand(Hand::Main).count,
            1,
            "tools wear on the server"
        );
    }

    #[test]
    fn axes_strip_logs_unless_a_shield_is_raised() {
        let (Some(mut f), Some(t)) = (Fixture::new(), jar_transformers()) else {
            return;
        };
        f.set([0, -61, 0], "minecraft:oak_log[axis=x]");
        f.hold(Hand::Main, "minecraft:iron_axe", 1);
        f.hold(Hand::Off, "minecraft:shield", 1);
        assert_eq!(
            transform(&mut f, &t, [0, -61, 0], Direction::Up),
            Outcome::Pass
        );
        f.hold(Hand::Off, "minecraft:stick", 1);
        assert_eq!(
            transform(&mut f, &t, [0, -61, 0], Direction::Up),
            Outcome::Success
        );
        assert_eq!(f.get([0, -61, 0]), "minecraft:stripped_oak_log[axis=x]");
        // Stripped logs stay.
        assert_eq!(
            transform(&mut f, &t, [0, -61, 0], Direction::Up),
            Outcome::Pass
        );
        // Weathered copper scrapes back a stage.
        f.set([1, -61, 0], "minecraft:weathered_copper");
        transform(&mut f, &t, [1, -61, 0], Direction::Up);
        assert_eq!(f.get([1, -61, 0]), "minecraft:exposed_copper");
    }

    #[test]
    fn registry_entries_without_data_are_vanilla_s() {
        let Some(vanilla) = jar_transformers() else {
            return;
        };
        let mut provider = Compound::new();
        provider.put("id", Tag::String("minecraft:stone".into()));
        let mut transform = Compound::new();
        transform.put("block_state_provider", Tag::Compound(provider));
        transform.put("consume_on_use", Tag::Byte(0));
        let entries = [
            RegistryEntry {
                id: "minecraft:shovel".into(),
                data: None,
            },
            RegistryEntry {
                id: "example:petrify".into(),
                data: Some(Tag::List(vec![Tag::Compound(transform)])),
            },
        ];
        let t = Transformers::from_registry(&entries, &vanilla);
        assert_eq!(
            t.get(&TransformerRef::Id(0)),
            vanilla.named("minecraft:shovel")
        );
        let petrify = t.get(&TransformerRef::Id(1)).unwrap();
        assert_eq!(
            petrify.transforms[0].provider,
            crate::transform::Provider::State(state::default_of("minecraft:stone"))
        );
        assert!(!petrify.transforms[0].consume_on_use);
        assert!(t.get(&TransformerRef::Id(2)).is_none());
    }
}
