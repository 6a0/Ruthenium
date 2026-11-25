//! Using an item on a block (`ItemStack.useOn`) and placing blocks
//! (`BlockItem.place`), as the client predicts them.
//!
//! A right click on a block first lets the block answer (see [`crate::block`]),
//! then the item: a block item places its block where the click points (over the
//! clicked block when that one may be built over, else against the clicked face), in
//! the state the block chooses for the player's view and the neighbours, if it may
//! stay there and no entity is in the way. The new block appears at once, under the
//! action's prediction, with the neighbours' shape updates; the stack shrinks unless
//! the player builds without materials. Other items' `useOn` are in [`crate::use_on`].

use crate::block::{self, CollisionMode, Pos, flags, state};
use crate::direction::Direction;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::item::Stack;
use crate::pick::BlockHit;
use crate::player::LocalPlayer;
use crate::shape::Axis;

/// What placing reads of the player (`UseOnContext.getPlayer()`), taken as the
/// click happens.
#[derive(Debug, Clone)]
pub struct PlayerView {
    pub position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    /// `isSecondaryUseActive` (sneaking).
    pub secondary_use: bool,
    /// `isDescending` (the sneak key, for scaffolding).
    pub descending: bool,
    pub main_hand: Stack,
    pub off_hand: Stack,
    /// Creative's building without materials (`hasInfiniteMaterials`).
    pub infinite: bool,
    /// `Abilities.mayBuild`.
    pub may_build: bool,
    /// `canUseGameMasterBlocks`.
    pub game_master: bool,
}

impl PlayerView {
    pub fn of(p: &LocalPlayer) -> PlayerView {
        PlayerView {
            position: p.position,
            y_rot: p.y_rot,
            x_rot: p.x_rot,
            secondary_use: p.secondary_use(),
            descending: p.secondary_use(),
            main_hand: p.hand(Hand::Main).clone(),
            off_hand: p.hand(Hand::Off).clone(),
            infinite: p.infinite_materials(),
            may_build: p.abilities.may_build,
            game_master: p.can_use_game_master_blocks(),
        }
    }
}

/// `UseOnContext`: who clicks, with which hand and item, and where.
#[derive(Debug, Clone)]
pub struct UseOnContext {
    pub player: PlayerView,
    pub hand: Hand,
    pub item: Stack,
    pub hit: BlockHit,
}

impl UseOnContext {
    pub fn new(p: &LocalPlayer, hand: Hand, hit: BlockHit) -> UseOnContext {
        UseOnContext {
            player: PlayerView::of(p),
            hand,
            item: p.hand(hand).clone(),
            hit,
        }
    }

    pub fn clicked_pos(&self) -> Pos {
        self.hit.pos
    }

    pub fn clicked_face(&self) -> Direction {
        self.hit.direction
    }

    pub fn click_location(&self) -> [f64; 3] {
        self.hit.location
    }

    pub fn is_inside(&self) -> bool {
        self.hit.inside
    }

    /// `getHorizontalDirection`: the way the player faces (`getDirection`).
    pub fn horizontal_direction(&self) -> Direction {
        Direction::from_y_rot(f64::from(self.player.y_rot))
    }

    pub fn secondary_use(&self) -> bool {
        self.player.secondary_use
    }

    /// `getRotation`: the player's yaw.
    pub fn rotation(&self) -> f32 {
        self.player.y_rot
    }
}

/// `BlockPlaceContext`: a use-on context that knows where the block goes: on the
/// clicked block if that one may be replaced, else in front of the clicked face.
#[derive(Debug, Clone)]
pub struct PlaceContext {
    pub on: UseOnContext,
    replace_clicked: bool,
    relative_pos: Pos,
}

impl PlaceContext {
    /// From a use-on context. Whether the clicked block may be replaced is asked of
    /// it with this context, which still counts as replacing it while it is asked.
    pub fn new(w: &World<'_>, on: UseOnContext) -> PlaceContext {
        let relative_pos = on.hit.direction.step(on.hit.pos);
        let mut ctx = PlaceContext {
            on,
            replace_clicked: true,
            relative_pos,
        };
        let clicked = w.get(ctx.on.hit.pos);
        ctx.replace_clicked = block::can_be_replaced(w, clicked, &ctx);
        ctx
    }

    /// `BlockPlaceContext.at`: the same player and item, clicking the centre of
    /// `pos`'s face towards `direction`.
    pub fn at(w: &World<'_>, ctx: &PlaceContext, pos: Pos, direction: Direction) -> PlaceContext {
        let n = direction.normal();
        let location = [
            f64::from(pos[0]) + 0.5 + f64::from(n[0]) * 0.5,
            f64::from(pos[1]) + 0.5 + f64::from(n[1]) * 0.5,
            f64::from(pos[2]) + 0.5 + f64::from(n[2]) * 0.5,
        ];
        let hit = BlockHit {
            location,
            direction,
            pos,
            inside: false,
            miss: false,
            world_border: false,
        };
        PlaceContext::new(
            w,
            UseOnContext {
                player: ctx.on.player.clone(),
                hand: ctx.on.hand,
                item: ctx.on.item.clone(),
                hit,
            },
        )
    }

    /// Where the block goes.
    pub fn clicked_pos(&self) -> Pos {
        if self.replace_clicked {
            self.on.hit.pos
        } else {
            self.relative_pos
        }
    }

    /// `canPlace`: the clicked block may be replaced, or the block in front of it.
    pub fn can_place(&self, w: &World<'_>) -> bool {
        self.replace_clicked || block::can_be_replaced(w, w.get(self.clicked_pos()), self)
    }

    /// `replacingClickedOnBlock`.
    pub fn replacing_clicked_on_block(&self) -> bool {
        self.replace_clicked
    }

    pub fn item(&self) -> &Stack {
        &self.on.item
    }

    pub fn hand(&self) -> Hand {
        self.on.hand
    }

    pub fn player(&self) -> &PlayerView {
        &self.on.player
    }

    pub fn clicked_face(&self) -> Direction {
        self.on.clicked_face()
    }

    pub fn click_location(&self) -> [f64; 3] {
        self.on.click_location()
    }

    pub fn is_inside(&self) -> bool {
        self.on.is_inside()
    }

    pub fn horizontal_direction(&self) -> Direction {
        self.on.horizontal_direction()
    }

    pub fn secondary_use(&self) -> bool {
        self.on.secondary_use()
    }

    pub fn rotation(&self) -> f32 {
        self.on.rotation()
    }

    /// `getNearestLookingDirection`.
    pub fn nearest_looking_direction(&self) -> Direction {
        Direction::ordered_by_nearest(self.on.player.y_rot, self.on.player.x_rot)[0]
    }

    /// `getNearestLookingVerticalDirection`: up or down, by the pitch.
    pub fn nearest_looking_vertical_direction(&self) -> Direction {
        Direction::facing_axis(self.on.player.y_rot, self.on.player.x_rot, Axis::Y)
    }

    /// `getNearestLookingDirections`: the view's directions, nearest first; when
    /// the block goes in front of the clicked face, the direction into that face
    /// moves to the front.
    pub fn nearest_looking_directions(&self) -> [Direction; 6] {
        let mut d = Direction::ordered_by_nearest(self.on.player.y_rot, self.on.player.x_rot);
        if self.replace_clicked {
            return d;
        }
        let into = self.on.hit.direction.opposite();
        let i = d.iter().position(|&x| x == into).unwrap_or(d.len());
        if i > 0 && i < d.len() {
            d[..=i].rotate_right(1);
        }
        d
    }
}

/// `ItemStack.useOn`: a player who may not build places only against blocks the
/// item's `can_place_on` names; then the item's own `useOn`; a use that counts as
/// the item's leaves its remainder and starts its cooldown.
pub fn stack_use_on(w: &mut World<'_>, p: &mut LocalPlayer, hand: Hand, hit: &BlockHit) -> Outcome {
    let stack = p.hand(hand).clone();
    if !p.abilities.may_build
        && !crate::mining::can_place_on_in_adventure(&stack, w.get(hit.pos), w.tags)
    {
        return Outcome::Pass;
    }
    let before = stack.copy();
    let (result, item_interaction) = item_use_on(w, p, hand, hit);
    if result.consumes_action() && item_interaction {
        p.after_use(hand, &before);
    }
    result
}

/// `Item.useOn` and its overrides: the outcome, and whether it counts as the
/// item's use (`wasItemInteraction`).
fn item_use_on(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    hand: Hand,
    hit: &BlockHit,
) -> (Outcome, bool) {
    let stack = p.hand(hand).clone();
    if stack.c.block.is_some() {
        let ctx = UseOnContext::new(p, hand, *hit);
        return (block_item_use_on(w, p, ctx), true);
    }
    if stack.c.use_on {
        return crate::use_on::use_on(w, p, hand, hit);
    }
    (crate::use_on::transform_block(w, p, hand, hit), true)
}

/// `BlockItem.useOn` (and `SolidBucketItem`'s, `PlaceOnWaterBlockItem`'s): place
/// the block; failing that, the item's plain `useOn` (a block transformer), and for
/// an item that is also eaten, its plain `use`.
pub fn block_item_use_on(w: &mut World<'_>, p: &mut LocalPlayer, ctx: UseOnContext) -> Outcome {
    if ctx.item.c.is("PlaceOnWaterBlockItem") {
        return Outcome::Pass;
    }
    block_item_use_on_as_placed(w, p, ctx)
}

/// `BlockItem.useOn` itself, which lily pads and frog spawn reach through their
/// `use`.
pub fn block_item_use_on_as_placed(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    ctx: UseOnContext,
) -> Outcome {
    let stack = ctx.item.clone();
    let hand = ctx.hand;
    let hit = ctx.hit;
    let placed = place(w, p, PlaceContext::new(w, ctx));
    if placed.consumes_action() {
        if stack.c.is("SolidBucketItem") {
            // `BucketItem.getEmptySuccessItem`: an empty bucket unless building
            // without materials.
            if !p.infinite_materials() {
                let bucket = w.new_stack("minecraft:bucket", 1);
                p.inventory.set_hand(hand, bucket);
            }
        }
        return placed;
    }
    let plain = crate::use_on::transform_block(w, p, hand, &hit);
    if plain.consumes_action() {
        return plain;
    }
    if stack.c.consumable.is_some() {
        return crate::interact::plain_item_use(p, hand);
    }
    placed
}

/// `BlockItem.place`.
pub fn place(w: &mut World<'_>, p: &mut LocalPlayer, ctx: PlaceContext) -> Outcome {
    let stack = ctx.item().clone();
    let Some(block) = stack.c.block else {
        return Outcome::Fail;
    };
    if !ctx.can_place(w) {
        return Outcome::Fail;
    }
    let Some(ctx) = update_placement_context(w, ctx, block) else {
        return Outcome::Fail;
    };
    let Some(placement) = placement_state(w, &ctx, block) else {
        return Outcome::Fail;
    };
    if !place_block(w, &ctx, placement) {
        return Outcome::Fail;
    }
    let pos = ctx.clicked_pos();
    let mut placed = w.get(pos);
    if state::same_block(placed, placement) {
        if let Some(props) = &stack.c.block_state {
            // `updateBlockStateFromTag`: the item's own property values.
            let modified = props
                .iter()
                .fold(placed, |s, (k, v)| state::try_set(s, k, v));
            if modified != placed {
                w.set_block(pos, modified, flags::CLIENTS);
            }
            placed = modified;
        }
        // Block entity data is the server's.
        block::set_placed_by(w, p, placed, pos, ctx.hand());
    }
    w.sounds
        .push(crate::sound::BlockSound::Place { pos, state: placed });
    let infinite = p.infinite_materials();
    p.inventory.hand_mut(ctx.hand()).consume(1, infinite);
    Outcome::Success
}

/// `updatePlacementContext`: scaffolding climbs or reaches out from scaffolding it is
/// placed on; other blocks keep the context.
fn update_placement_context(w: &World<'_>, ctx: PlaceContext, block: u32) -> Option<PlaceContext> {
    if !ctx.item().c.is("ScaffoldingBlockItem") {
        return Some(ctx);
    }
    let pos = ctx.clicked_pos();
    if state::block_id(w.get(pos)) != block {
        return (block::scaffolding_distance(w, pos) != 7).then_some(ctx);
    }
    let direction = if ctx.secondary_use() {
        if ctx.is_inside() {
            ctx.clicked_face().opposite()
        } else {
            ctx.clicked_face()
        }
    } else if ctx.clicked_face() == Direction::Up {
        ctx.horizontal_direction()
    } else {
        Direction::Up
    };
    let mut at = direction.step(pos);
    let mut reach = 0;
    while reach < 7 {
        let s = w.get(at);
        if state::block_id(s) != block {
            if block::can_be_replaced(w, s, &ctx) {
                return Some(PlaceContext::at(w, &ctx, at, direction));
            }
            break;
        }
        at = direction.step(at);
        if direction.is_horizontal() {
            reach += 1;
        }
    }
    None
}

/// `getPlacementState` of the item's class.
fn placement_state(w: &World<'_>, ctx: &PlaceContext, block: u32) -> Option<u32> {
    let c = &ctx.item().c;
    if c.is("GameMasterBlockItem") && !ctx.player().game_master {
        return None;
    }
    if c.is("StandingAndWallBlockItem") {
        return standing_or_wall_state(w, ctx, block);
    }
    let s = block::state_for_placement(w, ctx, block)?;
    can_place(w, ctx, s).then_some(s)
}

/// `BlockItem.canPlace`: the state may stay there (scaffolding needn't), and no
/// entity is in its way.
fn can_place(w: &World<'_>, ctx: &PlaceContext, s: u32) -> bool {
    let pos = ctx.clicked_pos();
    let must_survive = !ctx.item().c.is("ScaffoldingBlockItem");
    (!must_survive || block::can_survive(w, s, pos))
        && w.is_unobstructed(s, pos, CollisionMode::Placement)
}

/// `StandingAndWallBlockItem.getPlacementState`: torches, signs, heads, banners and
/// hanging signs try the standing block when the view points along their attachment
/// direction, else the wall block, in the order of the directions the player looks
/// along, until one may stay; then nothing may be in its way.
fn standing_or_wall_state(w: &World<'_>, ctx: &PlaceContext, block: u32) -> Option<u32> {
    let c = &ctx.item().c;
    let wall = c
        .field("StandingAndWallBlockItem.wallBlock")
        .and_then(state::block_named)?;
    let attachment = c
        .field("StandingAndWallBlockItem.attachmentDirection")
        .and_then(|d| Direction::from_name(&d.to_ascii_lowercase()))?;
    let wall_state = block::state_for_placement(w, ctx, wall);
    let pos = ctx.clicked_pos();
    let mut chosen = None;
    for d in ctx.nearest_looking_directions() {
        if d == attachment.opposite() {
            continue;
        }
        let candidate = if d == attachment {
            block::state_for_placement(w, ctx, block)
        } else {
            wall_state
        };
        if let Some(s) = candidate
            && standing_or_wall_can_place(w, c, s, pos)
        {
            chosen = Some(s);
            break;
        }
    }
    chosen.filter(|&s| w.is_unobstructed(s, pos, CollisionMode::Empty))
}

/// `StandingAndWallBlockItem.canPlace` (and `HangingSignItem`'s, which also asks a
/// wall hanging sign whether it can hang there).
fn standing_or_wall_can_place(
    w: &World<'_>,
    c: &crate::item::Components,
    s: u32,
    pos: Pos,
) -> bool {
    if c.is("HangingSignItem")
        && w.blocks.is_class(s, block::Class::WallHangingSignBlock)
        && !block::wall_hanging_sign_can_place(w, s, pos)
    {
        return false;
    }
    block::can_survive(w, s, pos)
}

/// `placeBlock`: the block is set with `UPDATE_ALL_IMMEDIATE`; a two-high item
/// first clears the block above (water stays water).
fn place_block(w: &mut World<'_>, ctx: &PlaceContext, s: u32) -> bool {
    let pos = ctx.clicked_pos();
    if ctx.item().c.is("DoubleHighBlockItem") {
        let above = Direction::Up.step(pos);
        let fill = if w.is_water_at(above) {
            state::default_of("minecraft:water")
        } else {
            state::AIR
        };
        w.set_block(above, fill, flags::ALL_IMMEDIATE | flags::KNOWN_SHAPE);
    }
    w.set_block(pos, s, flags::ALL_IMMEDIATE)
}
