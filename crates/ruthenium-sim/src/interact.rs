//! Using items, attacking and breaking blocks, as the client does it
//! (`Minecraft.startUseItem`, `startAttack`, `continueAttack`, the hands' part of
//! `handleKeybinds`, and `MultiPlayerGameMode`'s `useItem`, `useItemOn`, `interact`,
//! `releaseUsingItem`, `attack`, `piercingAttack` and the block breaking:
//! `startDestroyBlock`, `continueDestroyBlock`, `stopDestroyBlock`, `destroyBlock`):
//! the packets in vanilla's order, and the client's own prediction of what each item
//! does (`Item.use` and its overrides on the client side), which starts and stops the
//! use that slows the player, of what an attack does to the attacker (see
//! [`LocalPlayer::attack`]), and of the block a player breaks disappearing (see
//! [`crate::predict`]).
//!
//! Not predicted yet (the server's answer still arrives): what blocks and entities do
//! when used (a block or item that has its own `useItemOn`, `useWithoutItem` or
//! `useOn` is taken to have handled the click, which is what a placed block or an
//! opened chest does), placing blocks and fluids, the shape updates a broken block
//! makes its neighbours do (a plant on it popping, a fence losing its arm, the other
//! half of a door or bed going), the world border, riding.

use ruthenium_data::v26_3::registries::EntityType;
use ruthenium_protocol::packets::play;

use crate::blocks::{BlockTags, Blocks};
use crate::entity::Entities;
use crate::inventory::{Hand, Inventory};
use crate::item::{Class, Stack};
use crate::mining;
use crate::pick::{self, BlockHit, Direction, FluidMode, HitResult};
use crate::player::{Env, LocalPlayer};
use crate::predict::Predictions;
use crate::world::Level;

/// How an action went, as far as the client acts on it (`InteractionResult`; the
/// swing is the server's to show, and isn't a packet in 26.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `Success` (also `CONSUME` and `SUCCESS_SERVER`): the click is used up.
    Success,
    Fail,
    Pass,
}

/// The use-related keys: whether use is held, and the presses not yet read
/// (`KeyMapping.clickCount`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UseKeys {
    pub use_down: bool,
    pub attack_down: bool,
    pub use_clicks: u32,
    pub swap_clicks: u32,
    pub attack_clicks: u32,
    pub pick_clicks: u32,
}

/// What the client's item use, attacks and block breaking keep between ticks:
/// `Minecraft.rightClickDelay` and `missTime`, and `MultiPlayerGameMode`'s
/// `carriedIndex` and breaking state.
#[derive(Debug, Clone)]
pub struct GameMode {
    pub right_click_delay: i32,
    /// Ticks an attack click is ignored for after a miss (10; 10000 while a screen is
    /// open, until the attack key is let go).
    pub miss_time: i32,
    /// The hotbar slot last sent (0 for a new game mode).
    pub carried_index: i32,
    /// The block being broken (`destroyBlockPos`, kept after it ends), and the item it
    /// is broken with.
    pub destroy_pos: [i32; 3],
    destroying_item: Stack,
    /// How much of it is broken (1 breaks it), and the ticks spent on it.
    pub destroy_progress: f32,
    pub destroy_ticks: f32,
    /// Ticks before breaking goes on, after a block broke (and after a creative
    /// player's hit).
    pub destroy_delay: i32,
    destroy_direction: Option<Direction>,
    /// `isDestroying`: a block is being broken (and the use key waits).
    pub is_destroying: bool,
}

impl Default for GameMode {
    fn default() -> GameMode {
        GameMode {
            right_click_delay: 0,
            miss_time: 0,
            carried_index: 0,
            destroy_pos: [-1, -1, -1],
            destroying_item: Stack::empty(),
            destroy_progress: 0.0,
            destroy_ticks: 0.0,
            destroy_delay: 0,
            destroy_direction: None,
            is_destroying: false,
        }
    }
}

/// The world around the player that item use and breaking read, and the level they
/// change (through the predictions).
#[derive(Debug)]
pub struct World<'a> {
    pub level: &'a mut Level,
    pub predictions: &'a mut Predictions,
    pub blocks: &'a Blocks,
    pub tags: &'a BlockTags,
    pub entities: &'a Entities,
    /// The client's fluid shapes (vanilla caches each at the height it was first met).
    pub fluid_shapes: &'a pick::FluidShapes,
}

impl World<'_> {
    /// The level as movement and picking read it.
    pub fn env(&self) -> Env<'_> {
        Env {
            level: self.level,
            blocks: self.blocks,
            tags: self.tags,
            colliders: &[],
        }
    }

    fn block(&self, pos: [i32; 3]) -> u32 {
        self.level.block(pos[0], pos[1], pos[2])
    }

    fn is_air(&self, pos: [i32; 3]) -> bool {
        self.blocks.is_air(self.block(pos))
    }
}

impl GameMode {
    /// `Minecraft.tick`: the use delay counts down.
    pub fn tick_delay(&mut self) {
        if self.right_click_delay > 0 {
            self.right_click_delay -= 1;
        }
    }

    /// A screen is open this tick (`Minecraft.tick`): attacks wait for the attack key
    /// to be let go once it closes. `handleKeybinds` doesn't run.
    pub fn screen_open(&mut self) {
        self.miss_time = 10000;
    }

    /// `ensureHasSentCarriedItem` (and `MultiPlayerGameMode.tick`).
    pub fn ensure_carried(&mut self, inventory: &Inventory, out: &mut Vec<Vec<u8>>) {
        if inventory.selected != self.carried_index {
            self.carried_index = inventory.selected;
            out.push(play::set_carried_item(self.carried_index));
        }
    }

    /// `handleKeybinds` from the off-hand key to `continueAttack` (the hotbar keys,
    /// read before, are the caller's), then `missTime` counts down. `hit` is this
    /// tick's pick.
    pub fn handle_keybinds(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hit: &HitResult,
        keys: &mut UseKeys,
        out: &mut Vec<Vec<u8>>,
    ) {
        while keys.swap_clicks > 0 {
            keys.swap_clicks -= 1;
            if !p.spectator {
                out.push(play::player_action(
                    play::ACTION_SWAP_ITEM_WITH_OFFHAND,
                    [0; 3],
                    play::DOWN,
                    0,
                ));
            }
        }
        let mut instant_attack = false;
        if p.is_using_item() {
            if !keys.use_down {
                self.release_using_item(p, out);
            }
            keys.attack_clicks = 0;
            keys.use_clicks = 0;
            keys.pick_clicks = 0;
        } else {
            while keys.attack_clicks > 0 {
                keys.attack_clicks -= 1;
                instant_attack |= self.start_attack(p, w, hit, out);
            }
            while keys.use_clicks > 0 {
                keys.use_clicks -= 1;
                self.start_use_item(p, w, hit, out);
            }
            // Picking blocks isn't ported: its presses are dropped.
            keys.pick_clicks = 0;
        }
        if keys.use_down && self.right_click_delay == 0 && !p.is_using_item() {
            self.start_use_item(p, w, hit, out);
        }
        // No screen is open and the mouse is grabbed while this runs.
        self.continue_attack(p, w, hit, !instant_attack && keys.attack_down, out);
        if self.miss_time > 0 {
            self.miss_time -= 1;
        }
    }

    /// `Minecraft.startAttack`: one attack click. True when it broke a block at once
    /// (`instantAttack`), which keeps `continueAttack` from running this tick.
    pub fn start_attack(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hit: &HitResult,
        out: &mut Vec<Vec<u8>>,
    ) -> bool {
        // A miss's penalty swallows the click. The hands are never busy (riding isn't
        // ported).
        if self.miss_time > 0 {
            return false;
        }
        if p.game_type() == play::SPECTATOR {
            let id = match *hit {
                HitResult::Entity { id, .. } => Some(id),
                HitResult::Block(_) => None,
            };
            out.push(play::spectator_action(id));
            return true;
        }
        // Every item is enabled (no experimental features are tracked).
        let held = p.hand(Hand::Main).clone();
        if p.cannot_attack_with_item(&held, 0) {
            return false;
        }
        let mut end_attack = false;
        if held.c.piercing_weapon {
            // `piercingAttack`: a jab, without `Attack` or `Punch`.
            self.ensure_carried(&p.inventory, out);
            out.push(play::player_action(
                play::ACTION_STAB,
                [0; 3],
                play::DOWN,
                0,
            ));
            p.on_attack();
            return true;
        }
        match *hit {
            HitResult::Entity { id, location } => {
                let in_range = held
                    .c
                    .attack_range
                    .is_none_or(|r| r.is_in_range(p.eye_position(), location, p.creative));
                if in_range && let Some(e) = w.entities.get(id) {
                    self.attack(p, id, e.info().hurt_by_attack, out);
                }
            }
            HitResult::Block(ref b) if !b.miss && !w.is_air(b.pos) => {
                self.start_destroy_block(p, w, b.pos, b.direction, out);
                // A block that broke at once ends the attack: holding the key doesn't
                // dig on this tick.
                end_attack = w.is_air(b.pos);
            }
            HitResult::Block(_) => {
                if p.game_type() != play::CREATIVE {
                    self.miss_time = 10;
                }
                p.reset_attack_strength_ticker();
            }
        }
        out.push(play::punch());
        end_attack
    }

    /// `MultiPlayerGameMode.attack`: the packet, the client's side of the attack, and
    /// the attack strength starting over; a creative player's hit holds off breaking.
    fn attack(&mut self, p: &mut LocalPlayer, id: i32, hurts: bool, out: &mut Vec<Vec<u8>>) {
        self.ensure_carried(&p.inventory, out);
        out.push(play::attack(id));
        p.attack(hurts);
        p.reset_attack_strength_ticker();
        if p.abilities.instabuild {
            self.destroy_delay = 5;
        }
    }

    /// `Minecraft.continueAttack`: letting go of the attack key ends a miss's penalty;
    /// holding it on a block breaks it (with a `Punch` each tick it goes on), and
    /// anything else stops breaking.
    fn continue_attack(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hit: &HitResult,
        down: bool,
        out: &mut Vec<Vec<u8>>,
    ) {
        if !down {
            self.miss_time = 0;
        }
        if self.miss_time > 0 || p.is_using_item() || p.hand(Hand::Main).c.piercing_weapon {
            return;
        }
        match *hit {
            HitResult::Block(ref b) if down && !b.miss => {
                if !w.is_air(b.pos) && self.continue_destroy_block(p, w, b.pos, b.direction, out) {
                    out.push(play::punch());
                }
            }
            _ => self.stop_destroy_block(p, out),
        }
    }

    /// `startDestroyBlock`: an attack click on a block. In creative mode it breaks
    /// at once; otherwise breaking starts (the block it was on given up), or a block
    /// that breaks at once does. False if the player may not break it.
    pub fn start_destroy_block(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        pos: [i32; 3],
        direction: Direction,
        out: &mut Vec<Vec<u8>>,
    ) -> bool {
        // The world border isn't tracked: every block is inside it.
        if p.block_action_restricted(w.block(pos), w.tags) {
            return false;
        }
        let face = i32::from(direction.id());
        if p.abilities.instabuild {
            let sequence = w.predictions.start();
            self.destroy_block(p, w, pos);
            w.predictions.end();
            out.push(play::player_action(
                play::ACTION_START_DESTROY_BLOCK,
                pos,
                face,
                sequence,
            ));
            self.destroy_delay = 5;
        } else if !self.is_destroying || !self.same_destroy_target(p, pos) {
            if self.is_destroying {
                // The old block, given up, with the new block's face.
                out.push(play::player_action(
                    play::ACTION_ABORT_DESTROY_BLOCK,
                    self.destroy_pos,
                    face,
                    0,
                ));
            }
            let state = w.block(pos);
            let sequence = w.predictions.start();
            let not_air = !w.blocks.is_air(state);
            if not_air && self.destroy_progress == 0.0 {
                block_attacked(p, w, pos, state);
            }
            if not_air && p.destroy_progress(state, w.blocks, w.tags) >= 1.0 {
                self.destroy_block(p, w, pos);
            } else {
                self.is_destroying = true;
                self.destroy_pos = pos;
                self.destroy_direction = Some(direction);
                self.destroying_item = p.hand(Hand::Main).clone();
                self.destroy_progress = 0.0;
                self.destroy_ticks = 0.0;
            }
            w.predictions.end();
            out.push(play::player_action(
                play::ACTION_START_DESTROY_BLOCK,
                pos,
                face,
                sequence,
            ));
        }
        true
    }

    /// `continueDestroyBlock`: the attack key held on a block, each tick. Breaking
    /// goes on (a creative player breaks a block every sixth tick), finishes the
    /// block, turns to the face looked at, or starts on another block. False when the
    /// block it was on has gone.
    fn continue_destroy_block(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        pos: [i32; 3],
        direction: Direction,
        out: &mut Vec<Vec<u8>>,
    ) -> bool {
        self.ensure_carried(&p.inventory, out);
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
            return true;
        }
        let face = i32::from(direction.id());
        if p.abilities.instabuild {
            self.destroy_delay = 5;
            let sequence = w.predictions.start();
            self.destroy_block(p, w, pos);
            w.predictions.end();
            out.push(play::player_action(
                play::ACTION_START_DESTROY_BLOCK,
                pos,
                face,
                sequence,
            ));
            return true;
        }
        if !self.same_destroy_target(p, pos) {
            return self.start_destroy_block(p, w, pos, direction, out);
        }
        let state = w.block(pos);
        if w.blocks.is_air(state) {
            self.is_destroying = false;
            return false;
        }
        self.destroy_progress += p.destroy_progress(state, w.blocks, w.tags);
        self.destroy_ticks += 1.0;
        if self.destroy_progress >= 1.0 {
            self.is_destroying = false;
            let sequence = w.predictions.start();
            self.destroy_block(p, w, pos);
            w.predictions.end();
            out.push(play::player_action(
                play::ACTION_STOP_DESTROY_BLOCK,
                pos,
                face,
                sequence,
            ));
            self.destroy_direction = None;
            self.destroy_progress = 0.0;
            self.destroy_ticks = 0.0;
            self.destroy_delay = 5;
        } else if self.destroy_direction != Some(direction) {
            self.destroy_direction = Some(direction);
            out.push(play::player_action(
                play::ACTION_CHANGE_DESTROY_DIRECTION,
                pos,
                face,
                0,
            ));
        }
        true
    }

    /// `stopDestroyBlock`: breaking given up (the key let go, the crosshair off the
    /// block, a teleport, the game paused).
    pub fn stop_destroy_block(&mut self, p: &mut LocalPlayer, out: &mut Vec<Vec<u8>>) {
        if self.is_destroying {
            out.push(play::player_action(
                play::ACTION_ABORT_DESTROY_BLOCK,
                self.destroy_pos,
                play::DOWN,
                0,
            ));
            self.is_destroying = false;
            self.destroy_progress = 0.0;
            p.reset_attack_strength_ticker();
        }
    }

    /// `destroyBlock`: the client's own breaking of a block, under the action's
    /// prediction: the block goes (leaving its fluid). True if it did.
    fn destroy_block(&mut self, p: &LocalPlayer, w: &mut World<'_>, pos: [i32; 3]) -> bool {
        let old = w.block(pos);
        if p.block_action_restricted(old, w.tags)
            || !mining::can_destroy_block(p.hand(Hand::Main), p.abilities.instabuild)
            || w.blocks.is_game_master(old) && !p.can_use_game_master_blocks()
            || w.blocks.is_air(old)
        {
            return false;
        }
        // `playerWillDestroy` and `destroy` only make particles and sounds on the
        // client (the rest is the server's).
        let left = w.blocks.left_behind(old);
        w.predictions.set_block(w.level, pos, left, p.position)
    }

    /// `sameDestroyTarget`: the same block, with the same item in hand.
    fn same_destroy_target(&self, p: &LocalPlayer, pos: [i32; 3]) -> bool {
        pos == self.destroy_pos
            && p.hand(Hand::Main)
                .same_item_same_components(&self.destroying_item)
    }

    /// `getDestroyStage`: the breaking overlay's stage, 0 to 9 (and more past the
    /// end), or -1 when nothing is broken.
    pub fn destroy_stage(&self) -> i32 {
        if self.destroy_progress > 0.0 {
            (self.destroy_progress * 10.0) as i32
        } else {
            -1
        }
    }

    /// `MultiPlayerGameMode.releaseUsingItem`.
    pub fn release_using_item(&mut self, p: &mut LocalPlayer, out: &mut Vec<Vec<u8>>) {
        self.ensure_carried(&p.inventory, out);
        out.push(play::player_action(
            play::ACTION_RELEASE_USE_ITEM,
            [0; 3],
            play::DOWN,
            0,
        ));
        p.release_using_item();
    }

    /// `Minecraft.startUseItem`: the main hand, then the off hand, each on what the
    /// crosshair is on and then on its own, until one is used.
    pub fn start_use_item(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hit: &HitResult,
        out: &mut Vec<Vec<u8>>,
    ) {
        // Nothing is used while a block is being broken; the hands are never busy
        // (riding isn't ported).
        if self.is_destroying {
            return;
        }
        self.right_click_delay = 4;
        for hand in Hand::BOTH {
            let held = p.hand(hand).clone();
            match *hit {
                HitResult::Entity { id, location } => {
                    if let Some(e) = w.entities.get(id)
                        && within_entity_range(p, &e.bounding_box)
                        && self.interact(p, id, e.entity_type, e.position, location, hand, out)
                            == Outcome::Success
                    {
                        return;
                    }
                }
                HitResult::Block(ref b) if !b.miss => match self.use_item_on(p, w, hand, b, out) {
                    Outcome::Success | Outcome::Fail => return,
                    Outcome::Pass => {}
                },
                HitResult::Block(_) => {}
            }
            if !held.is_empty() && self.use_item(p, w, hand, out) == Outcome::Success {
                return;
            }
        }
    }

    /// `MultiPlayerGameMode.interact` and `Player.interactOn`.
    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        p: &mut LocalPlayer,
        id: i32,
        entity_type: i32,
        position: [f64; 3],
        location: [f64; 3],
        hand: Hand,
        out: &mut Vec<Vec<u8>>,
    ) -> Outcome {
        self.ensure_carried(&p.inventory, out);
        let rel = [
            location[0] - position[0],
            location[1] - position[1],
            location[2] - position[2],
        ];
        out.push(play::interact(id, hand.id(), rel, p.secondary_use()));
        if p.game_type() == play::SPECTATOR || p.spectator {
            return Outcome::Pass;
        }
        // `Entity.interact`: an armor stand answers on the server's side; the other
        // entities' own interactions (trading, riding, breeding, leashes...) and items'
        // `interactLivingEntity` aren't ported and count as passing.
        let name = u32::try_from(entity_type)
            .ok()
            .and_then(EntityType::from_id)
            .map_or("", EntityType::name);
        let held = p.hand(hand);
        if name == "minecraft:armor_stand" && held.c.name != "minecraft:name_tag" {
            return Outcome::Success;
        }
        Outcome::Pass
    }

    /// `MultiPlayerGameMode.useItemOn` with `performUseItemOn`.
    fn use_item_on(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hand: Hand,
        b: &BlockHit,
        out: &mut Vec<Vec<u8>>,
    ) -> Outcome {
        self.ensure_carried(&p.inventory, out);
        // The world border isn't tracked: every block is inside it.
        let sequence = w.predictions.start();
        let result = perform_use_item_on(p, w, hand, b);
        w.predictions.end();
        let cursor = [
            (b.location[0] - f64::from(b.pos[0])) as f32,
            (b.location[1] - f64::from(b.pos[1])) as f32,
            (b.location[2] - f64::from(b.pos[2])) as f32,
        ];
        out.push(play::use_item_on(
            hand.id(),
            b.pos,
            i32::from(b.direction.id()),
            cursor,
            b.inside,
            false,
            sequence,
        ));
        result
    }

    /// `MultiPlayerGameMode.useItem`: the item's own use, predicted, then its packet.
    pub fn use_item(
        &mut self,
        p: &mut LocalPlayer,
        w: &mut World<'_>,
        hand: Hand,
        out: &mut Vec<Vec<u8>>,
    ) -> Outcome {
        if p.game_type() == play::SPECTATOR {
            return Outcome::Pass;
        }
        self.ensure_carried(&p.inventory, out);
        let sequence = w.predictions.start();
        let (y_rot, x_rot) = (p.y_rot, p.x_rot);
        let result = if p.cooldowns.is_on_cooldown(p.hand(hand)) {
            Outcome::Pass
        } else {
            stack_use(p, w, hand)
        };
        w.predictions.end();
        out.push(play::use_item(hand.id(), sequence, y_rot, x_rot));
        result
    }
}

/// `BlockState.attack` on the client: a redstone ore lights up (its other effects,
/// a note block's note and a dragon egg's jump, are the server's).
fn block_attacked(p: &LocalPlayer, w: &mut World<'_>, pos: [i32; 3], state: u32) {
    let name = w.blocks.name(state);
    if !matches!(
        name,
        "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
    ) {
        return;
    }
    let lit = ruthenium_data::v26_3::BlockState::from_id(state)
        .and_then(|s| s.with("lit", "true"))
        .map(|s| s.id());
    if let Some(lit) = lit
        && lit != state
    {
        w.predictions.set_block(w.level, pos, lit, p.position);
    }
}

/// `Player.isWithinEntityInteractionRange(entity, 0)`: the box's nearest point to the
/// eyes within the entity interaction range.
fn within_entity_range(p: &LocalPlayer, b: &crate::shape::Aabb) -> bool {
    let eye = p.eye_position();
    let mut d = 0.0;
    for i in 0..3 {
        let v = (b.min[i] - eye[i]).max(eye[i] - b.max[i]).max(0.0);
        d += v * v;
    }
    let range = p.attributes.entity_interaction_range.value();
    d < range * range
}

/// `performUseItemOn`: the block's own use, then the item's `useOn`.
fn perform_use_item_on(p: &mut LocalPlayer, w: &World<'_>, hand: Hand, b: &BlockHit) -> Outcome {
    if p.game_type() == play::SPECTATOR {
        return Outcome::Success;
    }
    let something = !p.hand(Hand::Main).is_empty() || !p.hand(Hand::Off).is_empty();
    let suppress = p.secondary_use() && something;
    if !suppress {
        let state = w.level.block(b.pos[0], b.pos[1], b.pos[2]);
        let s = w.blocks.state(state);
        use ruthenium_prepare::exporters::StatePhysics;
        // A block with its own use is taken to use the click (doors, chests, buttons...).
        if s.interaction & StatePhysics::USE_ITEM_ON != 0 {
            return Outcome::Success;
        }
        // `TRY_WITH_EMPTY_HAND`: `useWithoutItem`, with the main hand only.
        if hand == Hand::Main && s.interaction & StatePhysics::USE_WITHOUT_ITEM != 0 {
            return Outcome::Success;
        }
    }
    let stack = p.hand(hand);
    if stack.is_empty() || p.cooldowns.is_on_cooldown(stack) {
        return Outcome::Pass;
    }
    // `Item.useOn`.
    if stack.c.block_transformer {
        // `BlockTransformer.transformBlock`: an axe or the like with a shield in the
        // other hand leaves the click to the shield; else it transforms (not predicted).
        let blocking =
            hand == Hand::Main && p.hand(Hand::Off).c.blocks_attacks && !p.secondary_use();
        return if blocking {
            Outcome::Pass
        } else {
            Outcome::Success
        };
    }
    if stack.c.block_item || stack.c.use_on {
        return Outcome::Success;
    }
    Outcome::Pass
}

/// `ItemStack.use`: the item's use, and for an item used at once (no use
/// duration), its remainder and cooldown.
fn stack_use(p: &mut LocalPlayer, w: &World<'_>, hand: Hand) -> Outcome {
    let before = p.hand(hand).copy();
    let instantly = before.c.use_duration <= 0;
    let (result, transformed) = item_use(p, w, hand);
    if result == Outcome::Success {
        if let Some(t) = transformed {
            p.inventory.set_hand(hand, t);
        }
        if instantly {
            p.after_use(hand, &before);
        }
    }
    result
}

/// `Item.use` and its overrides, on the client: the outcome and the stack the hand
/// holds afterwards when it is a new one.
fn item_use(p: &mut LocalPlayer, w: &World<'_>, hand: Hand) -> (Outcome, Option<Stack>) {
    let stack = p.hand(hand).clone();
    let infinite = p.infinite_materials();
    let ok = (Outcome::Success, None);
    match stack.c.class {
        Class::Plain => plain_use(p, hand, &stack),
        Class::Bow => {
            if !infinite && projectile(&p.inventory, &stack).is_none() {
                return (Outcome::Fail, None);
            }
            p.start_using_item(hand);
            ok
        }
        Class::Crossbow => {
            if stack.c.charged {
                // `performShooting` happens on the server.
                ok
            } else if projectile(&p.inventory, &stack).is_some() || infinite {
                p.start_using_item(hand);
                ok
            } else {
                (Outcome::Fail, None)
            }
        }
        Class::Trident => {
            // Rain isn't tracked: only water lets a riptide trident start.
            if stack.next_damage_will_break() || stack.c.riptide && !p.in_water() {
                return (Outcome::Fail, None);
            }
            p.start_using_item(hand);
            ok
        }
        Class::Spyglass | Class::Bundle => {
            p.start_using_item(hand);
            ok
        }
        Class::Instrument => {
            if stack.c.instrument.is_none() {
                return (Outcome::Fail, None);
            }
            p.start_using_item(hand);
            if stack.c.use_duration > 0 {
                p.cooldowns
                    .add(&stack.cooldown_group(), stack.c.use_duration);
            }
            ok
        }
        Class::Throwable => {
            p.inventory.hand_mut(hand).consume(1, infinite);
            ok
        }
        Class::EnderEye => {
            let hit = pov_hit(p, w, FluidMode::None);
            if !hit.miss {
                let state = w.level.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if w.blocks.name(state) == "minecraft:end_portal_frame" {
                    return (Outcome::Pass, None);
                }
            }
            p.start_using_item(hand);
            ok
        }
        Class::FishingRod | Class::Book | Class::EmptyMap => ok,
        Class::FireworkRocket => {
            if p.fall_flying() {
                ok
            } else {
                (Outcome::Pass, None)
            }
        }
        Class::KnowledgeBook => {
            // Its recipes never reach the client, so it always fails there, used up.
            p.inventory.hand_mut(hand).consume(1, infinite);
            (Outcome::Fail, None)
        }
        Class::FoodOnAStick | Class::Unknown => (Outcome::Pass, None),
        Class::Bucket | Class::Boat | Class::SpawnEgg | Class::Bottle | Class::PlaceOnWater => {
            let fluid = match stack.c.class {
                Class::Boat => FluidMode::Any,
                Class::Bucket if stack.c.name == "minecraft:bucket" => FluidMode::SourceOnly,
                Class::Bucket => FluidMode::None,
                _ => FluidMode::SourceOnly,
            };
            let hit = pov_hit(p, w, fluid);
            if hit.miss {
                (Outcome::Pass, None)
            } else if stack.c.class == Class::Bottle {
                let state = w.level.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if w.blocks.fluid(state).fluid.is_water() {
                    ok
                } else {
                    (Outcome::Pass, None)
                }
            } else {
                // Placing fluids, boats and mobs isn't predicted.
                ok
            }
        }
    }
}

/// `Item.use` itself: a consumable, an equippable that swaps, a blocking item or a
/// kinetic weapon.
fn plain_use(p: &mut LocalPlayer, hand: Hand, stack: &Stack) -> (Outcome, Option<Stack>) {
    if let Some(c) = stack.c.consumable {
        // `Consumable.startConsuming`.
        let can = stack.c.food.is_none_or(|f| p.can_eat(f.can_always_eat));
        if !can {
            return (Outcome::Fail, None);
        }
        if c.ticks() > 0 {
            p.start_using_item(hand);
            return (Outcome::Success, None);
        }
        // Consumed at once (`onConsume`).
        if let Some(food) = stack.c.food {
            p.eat(food);
        }
        let infinite = p.infinite_materials();
        p.inventory.hand_mut(hand).consume(1, infinite);
        return (Outcome::Success, None);
    }
    if let Some(e) = stack.c.equippable
        && e.swappable
    {
        return swap_with_equipment_slot(p, hand, e.slot);
    }
    if stack.c.blocks_attacks || stack.c.kinetic_weapon {
        p.start_using_item(hand);
        return (Outcome::Success, None);
    }
    (Outcome::Pass, None)
}

/// `Equippable.swapWithEquipmentSlot` (the curse of binding isn't read).
fn swap_with_equipment_slot(
    p: &mut LocalPlayer,
    hand: Hand,
    slot: crate::item::EquipmentSlot,
) -> (Outcome, Option<Stack>) {
    let creative = p.game_type() == play::CREATIVE;
    let index = p.inventory.slot_of(slot);
    let in_slot = p.inventory.get(index).clone();
    let in_hand = p.hand(hand).clone();
    if in_hand.same_item_same_components(&in_slot) {
        return (Outcome::Fail, None);
    }
    // `copyAndClear`: a copy, and the original emptied.
    if in_hand.count <= 1 {
        let to_hand = if in_slot.is_empty() {
            let mut h = in_hand.clone();
            if !creative {
                h.count = 0;
            }
            h
        } else {
            in_slot.copy()
        };
        let to_equipment = in_hand.copy();
        p.inventory.set(index, to_equipment);
        return (Outcome::Success, Some(to_hand));
    }
    let mut to_equipment = in_hand.copy();
    to_equipment.count = 1;
    if !creative {
        p.inventory.hand_mut(hand).shrink(1);
    }
    p.inventory.set(index, to_equipment);
    let _ = p.inventory.add(in_slot.copy());
    (Outcome::Success, Some(p.hand(hand).clone()))
}

/// `Player.getProjectile` for a bow or crossbow: the hands first (a crossbow also
/// takes fireworks there), then the inventory in slot order.
fn projectile(inv: &Inventory, weapon: &Stack) -> Option<usize> {
    let held = |s: &Stack| match weapon.c.class {
        Class::Crossbow => is_arrow(s) || s.c.name == "minecraft:firework_rocket",
        _ => is_arrow(s),
    };
    if held(inv.hand(Hand::Off)) {
        return Some(crate::inventory::SLOT_OFFHAND);
    }
    if held(inv.hand(Hand::Main)) {
        return Some(inv.selected as usize);
    }
    (0..crate::inventory::SIZE).find(|&i| is_arrow(inv.get(i)))
}

/// `#minecraft:arrows` as vanilla's data pack has it.
fn is_arrow(s: &Stack) -> bool {
    !s.is_empty()
        && matches!(
            s.c.name,
            "minecraft:arrow" | "minecraft:tipped_arrow" | "minecraft:spectral_arrow"
        )
}

/// `Item.getPlayerPOVHitResult`: from the eyes along the view, as far as the block
/// interaction range reaches.
fn pov_hit(p: &LocalPlayer, w: &World<'_>, fluid: FluidMode) -> BlockHit {
    let from = p.eye_position();
    let v = pick::view_vector(p.x_rot, p.y_rot);
    let r = p.attributes.block_interaction_range.value();
    let to = [from[0] + v[0] * r, from[1] + v[1] * r, from[2] + v[2] * r];
    let held = p.hand(Hand::Main).c.name;
    pick::clip(&w.env(), from, to, fluid, held, w.fluid_shapes)
}
