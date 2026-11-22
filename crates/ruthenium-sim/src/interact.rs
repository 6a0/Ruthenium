//! Using items and attacking, as the client does it (`Minecraft.startUseItem`,
//! `startAttack`, `continueAttack`, the hands' part of `handleKeybinds`, and
//! `MultiPlayerGameMode`'s `useItem`, `useItemOn`, `interact`, `releaseUsingItem`,
//! `attack` and `piercingAttack`): the packets in vanilla's order, and the client's
//! own prediction of what each item does (`Item.use` and its overrides on the client
//! side), which starts and stops the use that slows the player, and of what an attack
//! does to the attacker (see [`LocalPlayer::attack`]).
//!
//! Not predicted yet (the server's answer still arrives): what blocks and entities do
//! when used (a block or item that has its own `useItemOn`, `useWithoutItem` or
//! `useOn` is taken to have handled the click, which is what a placed block or an
//! opened chest does), placing blocks and fluids, block breaking (an attack on a block
//! is dropped: no packets), riding.

use ruthenium_data::v26_3::registries::EntityType;
use ruthenium_protocol::packets::play;

use crate::entity::Entities;
use crate::inventory::{Hand, Inventory};
use crate::item::{Class, Stack};
use crate::pick::{self, BlockHit, FluidMode, HitResult};
use crate::player::{Env, LocalPlayer};

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

/// What the client's item use and attacks keep between ticks:
/// `Minecraft.rightClickDelay` and `missTime`, `MultiPlayerGameMode.carriedIndex` and
/// the block prediction sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameMode {
    pub right_click_delay: i32,
    /// Ticks an attack click is ignored for after a miss (10; 10000 while a screen is
    /// open, until the attack key is let go).
    pub miss_time: i32,
    /// The hotbar slot last sent (0 for a new game mode).
    pub carried_index: i32,
    /// `BlockStatePredictionHandler.currentSequenceNr`.
    pub sequence: i32,
}

/// The world around the player that item use reads.
#[derive(Debug)]
pub struct World<'a, 'e> {
    pub env: &'a Env<'e>,
    pub entities: &'a Entities,
    /// The client's fluid shapes (vanilla caches each at the height it was first met).
    pub fluid_shapes: &'a pick::FluidShapes,
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

    fn next_sequence(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }

    /// `handleKeybinds` from the off-hand key to `continueAttack` (the hotbar keys,
    /// read before, are the caller's), then `missTime` counts down. `hit` is this
    /// tick's pick.
    pub fn handle_keybinds(
        &mut self,
        p: &mut LocalPlayer,
        w: &World<'_, '_>,
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
        self.continue_attack(p, !instant_attack && keys.attack_down);
        if self.miss_time > 0 {
            self.miss_time -= 1;
        }
    }

    /// `Minecraft.startAttack`: one attack click. True when it broke a block at once
    /// (`instantAttack`), which keeps `continueAttack` from running this tick.
    pub fn start_attack(
        &mut self,
        p: &mut LocalPlayer,
        w: &World<'_, '_>,
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
            HitResult::Block(ref b) if !b.miss && !is_air(w, b.pos) => {
                // `startDestroyBlock` isn't ported: the click is dropped whole (no
                // digging packet, and so no `Punch`).
                return false;
            }
            HitResult::Block(_) => {
                if p.game_type() != play::CREATIVE {
                    self.miss_time = 10;
                }
                p.reset_attack_strength_ticker();
            }
        }
        out.push(play::punch());
        false
    }

    /// `MultiPlayerGameMode.attack`: the packet, the client's side of the attack, and
    /// the attack strength starting over. (`destroyDelay`, which creative players' hits
    /// set, belongs to block breaking, which isn't ported.)
    fn attack(&mut self, p: &mut LocalPlayer, id: i32, hurts: bool, out: &mut Vec<Vec<u8>>) {
        self.ensure_carried(&p.inventory, out);
        out.push(play::attack(id));
        p.attack(hurts);
        p.reset_attack_strength_ticker();
    }

    /// `Minecraft.continueAttack`: letting go of the attack key ends a miss's penalty.
    /// Holding it on a block digs, which isn't ported (and `stopDestroyBlock` has
    /// nothing to stop).
    fn continue_attack(&mut self, _p: &LocalPlayer, down: bool) {
        if !down {
            self.miss_time = 0;
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
        w: &World<'_, '_>,
        hit: &HitResult,
        out: &mut Vec<Vec<u8>>,
    ) {
        // Breaking a block isn't ported, so the player never is (`isDestroying`);
        // nor are the hands ever busy (riding).
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
        w: &World<'_, '_>,
        hand: Hand,
        b: &BlockHit,
        out: &mut Vec<Vec<u8>>,
    ) -> Outcome {
        self.ensure_carried(&p.inventory, out);
        // The world border isn't tracked: every block is inside it.
        let sequence = self.next_sequence();
        let result = perform_use_item_on(p, w, hand, b);
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
        w: &World<'_, '_>,
        hand: Hand,
        out: &mut Vec<Vec<u8>>,
    ) -> Outcome {
        if p.game_type() == play::SPECTATOR {
            return Outcome::Pass;
        }
        self.ensure_carried(&p.inventory, out);
        let sequence = self.next_sequence();
        let (y_rot, x_rot) = (p.y_rot, p.x_rot);
        let result = if p.cooldowns.is_on_cooldown(p.hand(hand)) {
            Outcome::Pass
        } else {
            stack_use(p, w, hand)
        };
        out.push(play::use_item(hand.id(), sequence, y_rot, x_rot));
        result
    }
}

/// Whether the block at `pos` is air (`BlockState.isAir`).
fn is_air(w: &World<'_, '_>, pos: [i32; 3]) -> bool {
    let state = w.env.level.block(pos[0], pos[1], pos[2]);
    w.env.blocks.is_air(state)
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
fn perform_use_item_on(
    p: &mut LocalPlayer,
    w: &World<'_, '_>,
    hand: Hand,
    b: &BlockHit,
) -> Outcome {
    if p.game_type() == play::SPECTATOR {
        return Outcome::Success;
    }
    let something = !p.hand(Hand::Main).is_empty() || !p.hand(Hand::Off).is_empty();
    let suppress = p.secondary_use() && something;
    if !suppress {
        let state = w.env.level.block(b.pos[0], b.pos[1], b.pos[2]);
        let s = w.env.blocks.state(state);
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
fn stack_use(p: &mut LocalPlayer, w: &World<'_, '_>, hand: Hand) -> Outcome {
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
fn item_use(p: &mut LocalPlayer, w: &World<'_, '_>, hand: Hand) -> (Outcome, Option<Stack>) {
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
                let state = w.env.level.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if w.env.blocks.name(state) == "minecraft:end_portal_frame" {
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
                let state = w.env.level.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if w.env.blocks.fluid(state).fluid.is_water() {
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
fn pov_hit(p: &LocalPlayer, w: &World<'_, '_>, fluid: FluidMode) -> BlockHit {
    let from = p.eye_position();
    let v = pick::view_vector(p.x_rot, p.y_rot);
    let r = p.attributes.block_interaction_range.value();
    let to = [from[0] + v[0] * r, from[1] + v[1] * r, from[2] + v[2] * r];
    let held = p.hand(Hand::Main).c.name;
    pick::clip(w.env, from, to, fluid, held, w.fluid_shapes)
}
