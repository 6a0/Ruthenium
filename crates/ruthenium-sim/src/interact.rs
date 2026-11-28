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
//! A right click on a block asks the block first ([`crate::block`]: doors open,
//! buttons press, cakes are eaten...), then the item ([`crate::place`] places blocks,
//! [`crate::use_on`] runs the other items' `useOn` and the block transformers); what
//! either changes is predicted, with the neighbours' shape updates.
//!
//! Not predicted yet (the server's answer still arrives): what entities do when used
//! (beyond armor stands taking the click), placing fluids from buckets, the world
//! border, riding.

use ruthenium_data::v26_3::registries::EntityType;
use ruthenium_protocol::packets::play;

use crate::blocks::{BlockTags, Blocks};
use crate::entity::{Entities, PlayerInfos};
use crate::inventory::{Hand, Inventory};
use crate::item::{Class, Stack};
use crate::mining;
use crate::pick::{self, BlockHit, Direction, FluidMode, HitResult};
use crate::player::{Env, LocalPlayer};
use crate::predict::Predictions;
use crate::world::Level;

/// The local outcome of an interaction, including who starts its swing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Successful with a predicted swing.
    Success,
    /// Consumes the click without starting a local swing (CONSUME or SUCCESS_SERVER).
    Consume,
    Fail,
    Pass,
    /// `TRY_WITH_EMPTY_HAND`: a block's `useItemOn` has nothing for the item, and
    /// the main hand goes on to `useWithoutItem`.
    TryEmptyHand,
}

impl Outcome {
    /// `consumesAction`.
    pub fn consumes_action(self) -> bool {
        matches!(self, Outcome::Success | Outcome::Consume)
    }
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
    /// The last `destroyBlockProgress` the breaking made, not yet handed to the
    /// level's cracks (`ClientLevel.destroyingBlocks`): the block and its stage, none
    /// (-1) once breaking is given up or the block is broken. The cracks follow these
    /// calls, not `isDestroying`: a block that came back (the server refused to break
    /// it) is broken again with `isDestroying` false, and cracks
    /// (`MultiPlayerGameMode.java:281`).
    pub cracks: Option<([i32; 3], i32)>,
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
            cracks: None,
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
    /// The player list (spectators don't stop blocks being placed).
    pub players: Option<&'a PlayerInfos>,
    /// The local player's position and box as the current action began: predicted
    /// blocks keep the position, and the box stops blocks being placed in it (unless
    /// it spectates).
    pub player_position: [f64; 3],
    pub player_box: Option<crate::shape::Aabb>,
    /// The level's shape updates under way.
    pub updates: crate::block::level::ShapeUpdates,
    /// The items' defaults, for stacks an action makes (an emptied bucket).
    pub items: Option<&'a crate::item::Items>,
    /// The item tags (none known without them).
    pub item_tags: Option<&'a crate::item::ItemTags>,
    /// The block transformers (axes, shovels, hoes; none known without them).
    pub transformers: Option<&'a crate::use_on::Transformers>,
    /// The recipe property sets (none known without them).
    pub recipe_sets: Option<&'a crate::item::RecipeSets>,
    /// When set, every `setBlock` call made while an action is predicted (position,
    /// state, flags, update limit), in the order made (tests compare it with
    /// vanilla's).
    pub set_log: Option<Vec<crate::block::level::SetCall>>,
    /// The block sounds the actions made, for the session to play.
    pub sounds: Vec<crate::sound::BlockSound>,
}

impl<'a> World<'a> {
    /// The world of one tick's actions.
    pub fn new(
        level: &'a mut Level,
        predictions: &'a mut Predictions,
        blocks: &'a Blocks,
        tags: &'a BlockTags,
        entities: &'a Entities,
        fluid_shapes: &'a pick::FluidShapes,
    ) -> World<'a> {
        World {
            level,
            predictions,
            blocks,
            tags,
            entities,
            fluid_shapes,
            players: None,
            player_position: [0.0; 3],
            player_box: None,
            updates: Default::default(),
            items: None,
            item_tags: None,
            transformers: None,
            recipe_sets: None,
            set_log: None,
            sounds: Vec::new(),
        }
    }

    /// The recipe property sets.
    pub fn with_recipe_sets(mut self, sets: &'a crate::item::RecipeSets) -> World<'a> {
        self.recipe_sets = Some(sets);
        self
    }

    /// Whether a stack's item is in a recipe property set (`minecraft:campfire_input`).
    pub fn in_recipe_set(&self, stack: &Stack, key: &str) -> bool {
        self.recipe_sets.is_some_and(|s| s.contains(key, stack))
    }

    /// The block transformers.
    pub fn with_transformers(mut self, t: &'a crate::use_on::Transformers) -> World<'a> {
        self.transformers = Some(t);
        self
    }

    /// The item tags.
    pub fn with_item_tags(mut self, tags: &'a crate::item::ItemTags) -> World<'a> {
        self.item_tags = Some(tags);
        self
    }

    /// `ItemStack.is(tag)` for an item tag (`minecraft:bookshelf_books`).
    pub fn item_is(&self, stack: &Stack, tag: &str) -> bool {
        self.item_tags.is_some_and(|t| t.is(stack, tag))
    }

    /// The items' defaults.
    pub fn with_items(mut self, items: &'a crate::item::Items) -> World<'a> {
        self.items = Some(items);
        self
    }

    /// A new stack of an item, by name (empty without the items' defaults).
    pub fn new_stack(&self, name: &str, count: i32) -> Stack {
        let id =
            ruthenium_data::v26_3::registries::Item::from_name(name).map_or(0, |i| i.id() as i32);
        self.items
            .map_or_else(Stack::empty, |items| items.new_stack(id, count))
    }

    /// The player list, for spectators.
    pub fn with_players(mut self, players: &'a PlayerInfos) -> World<'a> {
        self.players = Some(players);
        self
    }

    /// Where the local player is as an action begins.
    pub fn track_player(&mut self, p: &LocalPlayer) {
        self.player_position = p.position;
        self.player_box = (!p.spectator).then(|| p.bounding_box());
    }

    /// The level as movement and picking read it.
    pub fn env(&self) -> Env<'_> {
        Env {
            level: self.level,
            blocks: self.blocks,
            tags: self.tags,
            colliders: &[],
            others: None,
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
        w.track_player(p);
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
                self.release_using_item(p, w, out);
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
        // A miss's penalty swallows the click, as do hands busy paddling a boat.
        if self.miss_time > 0 || p.hands_busy {
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
            p.swing(Hand::Main, held.c.attack_swing());
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
        p.swing(Hand::Main, held.c.attack_swing());
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
                    let swing = p.hand(Hand::Main).c.attack_swing();
                    p.swing(Hand::Main, swing);
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
        if p.block_action_restricted(w.block(pos), w.tags) {
            return false;
        }
        if !w.level.border.block_within_bounds(pos) {
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
                // No cracks yet (stage -1 at no progress, `MultiPlayerGameMode.java:204`).
                self.cracks = Some((pos, self.destroy_stage()));
                // `addBreakingBlockEffects` with its sound.
                w.sounds.push(crate::sound::BlockSound::Hit { pos, state });
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
        if p.abilities.instabuild && w.level.border.block_within_bounds(pos) {
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
        // Each tick on the same block, broken or not, and whether or not
        // `isDestroying` (`MultiPlayerGameMode.java:281`).
        self.cracks = Some((self.destroy_pos, self.destroy_stage()));
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
            self.cracks = Some((self.destroy_pos, -1));
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
        // client (the rest is the server's). The fluid stays, and the neighbours hear of
        // it (`UPDATE_ALL_IMMEDIATE`, `MultiPlayerGameMode.java:141`).
        w.sounds
            .push(crate::sound::BlockSound::Break { pos, state: old });
        let left = w.blocks.left_behind(old);
        w.track_player(p);
        w.set_block(pos, left, crate::block::flags::ALL_IMMEDIATE)
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
    pub fn release_using_item(
        &mut self,
        p: &mut LocalPlayer,
        w: &World<'_>,
        out: &mut Vec<Vec<u8>>,
    ) {
        self.ensure_carried(&p.inventory, out);
        out.push(play::player_action(
            play::ACTION_RELEASE_USE_ITEM,
            [0; 3],
            play::DOWN,
            0,
        ));
        let colliders = w.entities.colliders(p);
        let env = crate::player::Env {
            level: w.level,
            blocks: w.blocks,
            tags: w.tags,
            colliders: &colliders,
            others: None,
        };
        p.release_using_item(&env);
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
        // Nothing is used while a block is being broken, nor while paddling a boat.
        if self.is_destroying {
            return;
        }
        self.right_click_delay = 4;
        if p.hands_busy {
            return;
        }
        for hand in Hand::BOTH {
            let held = p.hand(hand).clone();
            match *hit {
                HitResult::Entity { id, location } => {
                    // Nothing outside the world border is used, with either hand.
                    if let Some(e) = w.entities.get(id)
                        && !w
                            .level
                            .border
                            .block_within_bounds(e.position.map(ruthenium_javamath::mth::floor))
                    {
                        return;
                    }
                    if let Some(e) = w.entities.get(id)
                        && within_entity_range(p, &e.bounding_box)
                    {
                        let rideable = e.rideable_interaction(p.secondary_use());
                        let outcome =
                            self.interact(p, id, e.entity_type, e.position, location, hand, out);
                        if outcome == Outcome::Success {
                            p.swing(hand, held.c.interact_swing());
                        }
                        if outcome.consumes_action() || rideable == Some(true) && !p.spectator {
                            return;
                        }
                    }
                }
                HitResult::Block(ref b) if !b.miss => {
                    let count = held.count;
                    match self.use_item_on(p, w, hand, b, out) {
                        Outcome::Success => {
                            // The hand swings, and comes up anew with what is left of
                            // what it held (or with all of it, where nothing is used
                            // up).
                            p.swing(hand, held.c.interact_swing());
                            let now = p.hand(hand);
                            if !now.is_empty() && (now.count != count || p.abilities.instabuild) {
                                p.first_person.item_used(hand == Hand::Off);
                            }
                            return;
                        }
                        Outcome::Consume | Outcome::Fail => return,
                        Outcome::Pass | Outcome::TryEmptyHand => {}
                    }
                }
                HitResult::Block(_) => {}
            }
            if !held.is_empty() {
                let result = self.use_item(p, w, hand, out);
                if result.consumes_action() {
                    if result == Outcome::Success {
                        p.swing(hand, held.c.interact_swing());
                    }
                    p.first_person.item_used(hand == Hand::Off);
                    return;
                }
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
            return Outcome::Consume;
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
        if !w.level.border.block_within_bounds(b.pos) {
            return Outcome::Fail;
        }
        w.track_player(p);
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
            b.world_border,
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

/// `BlockState.attack` on the client (a redstone ore lights up; the other blocks'
/// attacks are the server's).
fn block_attacked(p: &LocalPlayer, w: &mut World<'_>, pos: [i32; 3], state: u32) {
    w.track_player(p);
    crate::block::attack(w, p, state, pos);
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

/// `performUseItemOn`: the block's own use (`useItemOn`, then for the main hand
/// `useWithoutItem` when the block has nothing for the item), unless the player
/// sneaks holding something; then the item's `useOn` (whose stack a player building
/// without materials keeps whole).
fn perform_use_item_on(
    p: &mut LocalPlayer,
    w: &mut World<'_>,
    hand: Hand,
    b: &BlockHit,
) -> Outcome {
    if p.game_type() == play::SPECTATOR {
        return Outcome::Success;
    }
    let something = !p.hand(Hand::Main).is_empty() || !p.hand(Hand::Off).is_empty();
    let suppress = p.secondary_use() && something;
    if !suppress {
        // Every block's feature is enabled (none are tracked).
        let state = w.get(b.pos);
        let r = crate::block::use_item_on(w, p, hand, state, b.pos, b);
        if r.consumes_action() {
            return r;
        }
        if r == Outcome::TryEmptyHand && hand == Hand::Main {
            let r = crate::block::use_without_item(w, p, state, b.pos, b);
            if r.consumes_action() {
                return r;
            }
        }
    }
    let stack = p.hand(hand).clone();
    if stack.is_empty() || p.cooldowns.is_on_cooldown(&stack) {
        return Outcome::Pass;
    }
    let result = crate::place::stack_use_on(w, p, hand, b);
    if p.infinite_materials() {
        // The count is put back on the stack that was used, if it is still held.
        let held = p.inventory.hand_mut(hand);
        if held.same_object(&stack) {
            held.count = stack.count;
        }
    }
    result
}

/// `ItemStack.use`: the item's use, and for an item used at once (no use
/// duration), its remainder and cooldown.
fn stack_use(p: &mut LocalPlayer, w: &mut World<'_>, hand: Hand) -> Outcome {
    let before = p.hand(hand).copy();
    let instantly = before.c.use_duration <= 0;
    let (result, transformed) = item_use(p, w, hand);
    if result.consumes_action() {
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
fn item_use(p: &mut LocalPlayer, w: &mut World<'_>, hand: Hand) -> (Outcome, Option<Stack>) {
    let stack = p.hand(hand).clone();
    let infinite = p.infinite_materials();
    let ok = (Outcome::Success, None);
    let consume = (Outcome::Consume, None);
    match stack.c.class {
        Class::Plain => plain_use(p, hand, &stack),
        Class::Bow => {
            if !infinite && projectile(&p.inventory, &stack).is_none() {
                return (Outcome::Fail, None);
            }
            p.start_using_item(hand);
            consume
        }
        Class::Crossbow => {
            if stack.c.charged {
                // `performShooting` happens on the server.
                consume
            } else if projectile(&p.inventory, &stack).is_some() || infinite {
                p.start_using_item(hand);
                consume
            } else {
                (Outcome::Fail, None)
            }
        }
        Class::Trident => {
            let env = Env {
                level: w.level,
                blocks: w.blocks,
                tags: w.tags,
                colliders: &[],
                others: None,
            };
            if stack.next_damage_will_break()
                || stack.c.spin_attack_strength > 0.0 && !p.is_in_water_or_rain(&env)
            {
                return (Outcome::Fail, None);
            }
            p.start_using_item(hand);
            consume
        }
        Class::Bundle => {
            p.start_using_item(hand);
            ok
        }
        Class::Spyglass => {
            p.start_using_item(hand);
            consume
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
            consume
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
            consume
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
        Class::PlaceOnWater => {
            // `PlaceOnWaterBlockItem.use`: `BlockItem.useOn` one block above the water
            // (or whatever) the view's ray found, missed or not.
            let hit = pov_hit(p, w, FluidMode::SourceOnly);
            let above = BlockHit {
                pos: Direction::Up.step(hit.pos),
                ..hit
            };
            let ctx = crate::place::UseOnContext::new(p, hand, above);
            (crate::place::block_item_use_on_as_placed(w, p, ctx), None)
        }
        Class::Bucket | Class::Boat | Class::SpawnEgg | Class::Bottle => {
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

/// `Item.use` itself (not an override), as `BlockItem.useOn` falls back on it for
/// a block item that is also eaten.
pub fn plain_item_use(p: &mut LocalPlayer, hand: Hand) -> Outcome {
    let stack = p.hand(hand).clone();
    let (result, transformed) = plain_use(p, hand, &stack);
    if let Some(t) = transformed {
        p.inventory.set_hand(hand, t);
    }
    result
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
            return (Outcome::Consume, None);
        }
        // Consumed at once (`onConsume`).
        if let Some(food) = stack.c.food {
            p.eat(food);
        }
        let infinite = p.infinite_materials();
        p.inventory.hand_mut(hand).consume(1, infinite);
        return (Outcome::Consume, None);
    }
    if let Some(e) = stack.c.equippable
        && e.swappable
    {
        return swap_with_equipment_slot(p, hand, e.slot);
    }
    if stack.c.blocks_attacks || stack.c.spear.is_some() {
        p.start_using_item(hand);
        return (Outcome::Consume, None);
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

#[cfg(test)]
mod tests {
    use crate::block::testing::Fixture;
    use crate::direction::Direction;
    use crate::first_person::Destruction;

    /// A block the server puts back cracks again while the player keeps breaking it,
    /// as vanilla's does: the cracks follow `destroyBlockProgress`, which
    /// `continueDestroyBlock` calls on the same block whether or not `isDestroying`
    /// (false once the client broke the block).
    #[test]
    fn a_block_put_back_cracks_again() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.on_ground = true;
        let pos = [0, -60, 2];
        f.set(pos, "minecraft:dirt");
        let blocks = f.blocks();
        let mut cracks = Destruction::default();
        let mut stages = Vec::new();
        let mut tick = |f: &mut Fixture, clicks: u32, game_time: i64| {
            f.attack_face(pos, Direction::North, clicks);
            if let Some((at, stage)) = f.game_mode.cracks.take() {
                cracks.set(f.player.entity_id, at, stage, game_time);
            }
            let mut out = Vec::new();
            cracks.stages(&mut out);
            out.first().map(|&(_, s)| s)
        };
        // The click starts breaking; the server refuses (spawn protection: only the
        // acknowledgement; a protection plugin also sends the block, unchanged).
        stages.push(tick(&mut f, 1, 0));
        f.predictions
            .server_block(&mut f.level, pos, Fixture::state("minecraft:dirt"));
        f.predictions
            .acknowledge(1, &mut f.level, blocks, &mut f.player);
        // Dirt by hand: a fifteenth a tick, broken (predicted) on the fifteenth.
        for t in 1..15 {
            stages.push(tick(&mut f, 0, t));
        }
        assert_eq!(f.get(pos), "minecraft:air");
        assert!(!f.game_mode.is_destroying);
        let first: Vec<_> = (0..10).map(|s| Some(s as u8)).collect();
        // Stages 0 to 9 over the fifteen ticks, then none once broken.
        assert_eq!(stages.iter().filter(|s| s.is_some()).count(), 14);
        assert!(first.iter().all(|s| stages.contains(s)));
        assert_eq!(stages.last(), Some(&None));
        // The server refuses the break: its acknowledgement puts the block back.
        f.predictions
            .acknowledge(2, &mut f.level, blocks, &mut f.player);
        assert_eq!(f.get(pos), "minecraft:dirt");
        // Five ticks' delay, then breaking goes on from nothing, with cracks though
        // `isDestroying` is false.
        stages.clear();
        for t in 15..30 {
            stages.push(tick(&mut f, 0, t));
        }
        assert!(!f.game_mode.is_destroying);
        assert_eq!(&stages[..5], &[None; 5]);
        assert_eq!(
            &stages[5..10],
            &[Some(0), Some(1), Some(2), Some(2), Some(3)]
        );
        // Letting go while not `isDestroying` sends nothing and leaves the cracks, as
        // vanilla's `stopDestroyBlock` does (they go after 400 ticks without news).
        let mut out = Vec::new();
        f.game_mode.stop_destroy_block(&mut f.player, &mut out);
        assert!(out.is_empty() && f.game_mode.cracks.is_none());
    }

    /// Letting go of a block being broken takes its cracks away.
    #[test]
    fn letting_go_takes_the_cracks_away() {
        let Some(mut f) = Fixture::new() else {
            return;
        };
        f.floor();
        f.player.on_ground = true;
        let pos = [0, -60, 2];
        f.set(pos, "minecraft:dirt");
        f.attack_face(pos, Direction::North, 1);
        f.attack_face(pos, Direction::North, 0);
        assert_eq!(f.game_mode.cracks, Some((pos, 1)));
        let mut out = Vec::new();
        f.game_mode.stop_destroy_block(&mut f.player, &mut out);
        assert_eq!(out.len(), 1, "abort_destroy_block");
        assert_eq!(f.game_mode.cracks, Some((pos, -1)));
    }
}
